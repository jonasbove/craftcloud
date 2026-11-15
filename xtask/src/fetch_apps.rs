//! `cargo xtask fetch-apps`: download, verify and unpack the Craft web builds pinned in `apps.toml`,
//! then precompress them. It runs in the Dockerfile's `apps` stage (and locally for testing).
//!
//! For each `[apps.<id>]`:
//! 1. download `https://github.com/<repo>/releases/download/v<version>/<asset>` (cached by sha256);
//! 2. refuse it unless its sha256 matches the pin;
//! 3. unpack it to `<out>/<id>/`, stripping the zip's single top-level folder and refusing unsafe
//!    entries (`..`, absolute paths, symlinks) and oversized ones, then apply the app's `fixups`;
//! 4. write `.gz` and `.br` siblings for compressible files, so Caddy never compresses at request
//!    time. Files that shrink by less than 10% (already-compressed wasm parts, say) are left alone.
//!
//! Also writes `<out>/apps.json`: id, name, tagline, color, ink, version, bridge, query, opens,
//! default, entry.
//!
//! The zips come from the network and are treated as hostile; the sha256 pin is the trust anchor,
//! and the size limits below hold even for a pinned zip.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::manifest::{self, AppSpec, Fixup};

/// File types that get `.gz` and `.br` siblings (compared without case).
const COMPRESSIBLE: [&str; 13] = ["wasm", "js", "mjs", "html", "css", "json", "svg", "txt", "md", "map", "xml", "ttf", "otf"];
/// A sibling is kept only when gzip saves at least this share of the file.
const MIN_SAVING: f64 = 0.10;
/// Bump when unpacking or precompression changes, so cached trees are rebuilt. `p1` was the
/// Python script's; its trees must not be taken for this one's.
const PROCESS_VERSION: &str = "p2";

/// Limits for hostile input. The largest web build today is about 70 MB unpacked.
const MAX_DOWNLOAD: u64 = 1 << 30;
const MAX_ENTRIES: usize = 100_000;
const MAX_ENTRY_BYTES: u64 = 1 << 30;
const MAX_UNPACKED_BYTES: u64 = 4 << 30;
/// Folder depth when copying trees (fixups and the cache).
const MAX_DEPTH: usize = 64;

pub const USAGE: &str = "usage: cargo xtask fetch-apps <apps.toml> <out-dir> [--cache DIR] [--only app,app] [--jobs N]";

pub fn cmd(args: &[String]) -> Result<(), String> {
    let mut cache = std::env::temp_dir().join("craftcloud-cache");
    let mut only: Option<Vec<String>> = None;
    let mut jobs = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--cache" => cache = PathBuf::from(it.next().ok_or(USAGE)?),
            "--only" => only = Some(it.next().ok_or(USAGE)?.split(',').map(str::to_owned).collect()),
            "--jobs" => jobs = it.next().and_then(|n| n.parse().ok()).filter(|&n| n > 0).ok_or(USAGE)?,
            _ if arg.starts_with("--") => return Err(format!("unknown option `{arg}`\n{USAGE}")),
            _ => positional.push(arg),
        }
    }
    let [manifest_path, out] = positional.as_slice() else { return Err(USAGE.to_owned()) };
    let out = Path::new(out);
    let text = fs::read_to_string(manifest_path).map_err(|e| format!("{manifest_path}: {e}"))?;
    let apps = manifest::parse(&text)?;
    fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;

    let index: Vec<(&str, &AppSpec)> =
        apps.iter().filter(|(id, _)| only.as_ref().is_none_or(|only| only.contains(id))).map(|(id, spec)| (id.as_str(), spec)).collect();
    // One large wasm file dominates each app, so the apps run side by side (`jobs` at a time) and
    // the threads left over share an app's files.
    let app_jobs = jobs.min(index.len()).max(1);
    let file_jobs = (jobs / app_jobs).max(1);
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let work = || -> Vec<(usize, Result<(), String>)> {
        let mut done = Vec::new();
        while !failed.load(Ordering::Relaxed) {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some((id, spec)) = index.get(i) else { break };
            let result = fetch_one(id, spec, out, &cache, file_jobs);
            if result.is_err() {
                failed.store(true, Ordering::Relaxed);
            }
            done.push((i, result));
        }
        done
    };
    let mut results: Vec<(usize, Result<(), String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..app_jobs).map(|_| s.spawn(work)).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|_| vec![(0, Err("a worker thread stopped".to_owned()))])).collect()
    });
    // Report the first failure in manifest order.
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().try_for_each(|(_, r)| r)?;
    write_file(&out.join("apps.json"), apps_json(&index).as_bytes())
}

/// Puts one app's finished tree at `<out>/<id>/`, from the cache or from its zip.
fn fetch_one(id: &str, spec: &AppSpec, out: &Path, cache: &Path, jobs: usize) -> Result<(), String> {
    let dest = out.join(id);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    // The finished tree (unpacked and precompressed) is cached per zip sha256 and per version of
    // this processing, so a pin bump only recompresses the apps that changed.
    let fix_key = if spec.fixups.is_empty() {
        String::new()
    } else {
        format!("-fx{}", hex(&Sha256::digest(fixups_json(&spec.fixups).as_bytes())).get(..8).unwrap_or_default())
    };
    let tree = cache.join(format!("{}-{PROCESS_VERSION}{fix_key}.tree", spec.sha256));
    if tree.is_dir() {
        copy_tree(&tree, &dest, 0)?;
        println!("{id:<14} {:<8} from cache", spec.version);
        return Ok(());
    }
    let url = format!("https://github.com/{}/releases/download/v{}/{}", spec.repo, spec.version, spec.asset);
    let zip_path = download(&url, &spec.sha256, cache)?;
    fs::create_dir(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    unpack(&zip_path, &dest)?;
    if !dest.join("index.html").is_file() {
        return Err(format!("{id}: no index.html at the top of {}", spec.asset));
    }
    apply_fixups(id, &dest, &spec.fixups)?;
    let (raw, packed) = precompress(&dest, jobs)?;
    let tmp_tree = tree.with_extension("tmp");
    if tmp_tree.exists() {
        fs::remove_dir_all(&tmp_tree).map_err(|e| format!("{}: {e}", tmp_tree.display()))?;
    }
    copy_tree(&dest, &tmp_tree, 0)?;
    fs::rename(&tmp_tree, &tree).map_err(|e| format!("{}: {e}", tree.display()))?;
    println!("{id:<14} {:<8} unpacked, {:7.1} MB compressible, {:7.1} MB of .gz/.br", spec.version, raw as f64 / 1e6, packed as f64 / 1e6);
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex(&hasher.finalize()))
}

/// `<cache>/<sha256>.zip`, downloaded unless the cache already holds it intact.
fn download(url: &str, sha: &str, cache: &Path) -> Result<PathBuf, String> {
    let target = cache.join(format!("{sha}.zip"));
    if target.is_file() && sha256_file(&target)? == sha {
        return Ok(target);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(120)))
        .build()
        .into();
    let response = agent.get(url).header("User-Agent", "craftcloud-fetch-apps").call().map_err(|e| format!("{url}: {e}"))?;
    let reader = response.into_body().into_with_config().limit(MAX_DOWNLOAD).reader();
    store_verified(reader, sha, &target.with_extension("part"), &target, url)?;
    Ok(target)
}

/// Streams `reader` to `part` while hashing it, then renames it to `target` only if the sha256
/// matches. On any failure nothing is left behind.
fn store_verified(mut reader: impl Read, sha: &str, part: &Path, target: &Path, what: &str) -> Result<(), String> {
    let result = (|| {
        let mut file = File::create(part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0; 1 << 20];
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("{what}: {e}"))?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
            hasher.update(chunk);
            file.write_all(chunk).map_err(|e| format!("{}: {e}", part.display()))?;
        }
        file.sync_all().map_err(|e| format!("{}: {e}", part.display()))?;
        let got = hex(&hasher.finalize());
        if got != sha {
            return Err(format!("sha256 mismatch for {what}: expected {sha}, got {got}"));
        }
        Ok(())
    })();
    match result {
        Ok(()) => fs::rename(part, target).map_err(|e| format!("{}: {e}", target.display())),
        Err(e) => {
            let _ = fs::remove_file(part);
            Err(e)
        }
    }
}

/// A zip name split like Python's `PurePosixPath(name).parts`: empty and `.` parts dropped, and an
/// absolute name's root kept as a first part `/`.
fn posix_parts(name: &str) -> (bool, Vec<&str>) {
    let absolute = name.starts_with('/');
    let mut parts = Vec::new();
    if absolute {
        parts.push("/");
    }
    parts.extend(name.split('/').filter(|p| !p.is_empty() && *p != "."));
    (absolute, parts)
}

/// A part that names one thing inside its folder on this platform (no `..`, no drive or root).
fn plain_part(part: &str) -> bool {
    let mut components = Path::new(part).components();
    part != ".." && matches!(components.next(), Some(Component::Normal(p)) if p == part) && components.next().is_none()
}

/// Unpacks `zip_path` into `dest`. When every file sits in one top-level folder, that folder is
/// stripped. Directory entries are skipped (folders are made as needed); files get mode 644.
fn unpack(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let label = zip_path.file_name().map_or_else(|| zip_path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let file = File::open(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| format!("{label}: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{label}: {} entries, more than {MAX_ENTRIES}", zip.len()));
    }
    if zip.has_duplicate_names() {
        return Err(format!("{label}: duplicate names refused"));
    }
    let mut names = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let name = zip.name_for_index(i).ok_or_else(|| format!("{label}: entry {i} missing"))?;
        names.push(name.map_err(|e| format!("{label}: {e}"))?.into_owned());
    }
    let files: Vec<&str> = names.iter().map(String::as_str).filter(|n| !n.ends_with('/')).collect();
    let mut tops: Vec<Option<&str>> = files.iter().map(|n| posix_parts(n).1.first().copied()).collect();
    tops.sort_unstable();
    tops.dedup();
    let strip = usize::from(tops.len() == 1 && files.iter().all(|n| posix_parts(n).1.len() > 1));

    let mut total = 0u64;
    for (i, name) in names.iter().enumerate() {
        if name.ends_with('/') {
            continue;
        }
        let (absolute, parts) = posix_parts(name);
        let rel = parts.get(strip..).unwrap_or_default();
        if absolute || rel.is_empty() || !rel.iter().all(|p| plain_part(p)) {
            return Err(format!("{label}: unsafe path {name:?}"));
        }
        let entry = zip.by_index(i).map_err(|e| format!("{label}: {name}: {e}"))?;
        if (entry.external_attributes() >> 16) & 0o170000 == 0o120000 {
            return Err(format!("{label}: symlink {name:?} refused"));
        }
        let out = rel.iter().fold(dest.to_path_buf(), |p, part| p.join(part));
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut file = File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        // Declared sizes can lie: count what actually comes out.
        let written = io::copy(&mut entry.take(MAX_ENTRY_BYTES + 1), &mut file).map_err(|e| format!("{label}: {name}: {e}"))?;
        if written > MAX_ENTRY_BYTES {
            return Err(format!("{label}: {name:?} is larger than {MAX_ENTRY_BYTES} bytes"));
        }
        total += written;
        if total > MAX_UNPACKED_BYTES {
            return Err(format!("{label}: unpacks to more than {MAX_UNPACKED_BYTES} bytes"));
        }
        set_mode_644(&out)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode_644(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644)).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_mode_644(_: &Path) -> Result<(), String> {
    Ok(())
}

/// `rel` under `root`, refused when it is absolute, names the root itself, or leaves it.
fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let mut parts = Vec::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => parts.push(p),
            Component::CurDir => {}
            Component::ParentDir if parts.pop().is_some() => {}
            _ => return Err(format!("fixup path {rel:?} leaves the app directory")),
        }
    }
    if parts.is_empty() {
        return Err(format!("fixup path {rel:?} names the app directory itself"));
    }
    Ok(parts.iter().fold(root.to_path_buf(), |p, part| p.join(part)))
}

/// Repairs for packaging bugs in a pinned release (each one is also in docs/UPSTREAM.md).
fn apply_fixups(app: &str, dest: &Path, fixups: &[Fixup]) -> Result<(), String> {
    for fx in fixups {
        let (src, dst) = (inside(dest, &fx.copy)?, inside(dest, &fx.to)?);
        let Ok(meta) = fs::symlink_metadata(&src) else {
            return Err(format!("{app}: fixup source {:?} not found (fixed upstream? drop the fixup)", fx.copy));
        };
        if meta.is_dir() {
            if dst.starts_with(&src) {
                return Err(format!("{app}: fixup copies {:?} into itself", fx.copy));
            }
            copy_tree(&src, &dst, 0)?;
        } else {
            // Like a shell `cp`: a destination folder receives the file under its own name.
            let dst = match src.file_name() {
                Some(name) if dst.is_dir() => dst.join(name),
                _ => dst,
            };
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        }
        println!("{app:<14} fixup: copied {} -> {}", fx.copy, fx.to);
    }
    Ok(())
}

/// Copies the folder `src` into `dst` (made if missing; files already there are overwritten).
/// Symlinks are not followed or copied: unpacking never makes any.
fn copy_tree(src: &Path, dst: &Path, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: folders nested deeper than {MAX_DEPTH}", src.display()));
    }
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", src.display()))?;
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let to = dst.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &to, depth + 1)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &to).map_err(|e| format!("{}: {e}", to.display()))?;
        }
    }
    Ok(())
}

fn compressible(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| COMPRESSIBLE.contains(&e.to_ascii_lowercase().as_str()))
}

/// Every compressible file under `root`, sorted.
fn compressible_files(root: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: folders nested deeper than {MAX_DEPTH}", root.display()));
    }
    for entry in fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", root.display()))?;
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", entry.path().display()))?;
        if kind.is_dir() {
            compressible_files(&entry.path(), depth + 1, out)?;
        } else if kind.is_file() && compressible(&entry.path()) {
            out.push(entry.path());
        }
    }
    if depth == 0 {
        out.sort();
    }
    Ok(())
}

/// Writes `.gz` and `.br` siblings for the compressible files under `root`, on `jobs` threads.
/// Returns the bytes of compressible files and of the siblings written.
fn precompress(root: &Path, jobs: usize) -> Result<(u64, u64), String> {
    let mut files = Vec::new();
    compressible_files(root, 0, &mut files)?;
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let work = || -> Result<(u64, u64), String> {
        let (mut raw, mut packed) = (0, 0);
        while !failed.load(Ordering::Relaxed) {
            let Some(file) = files.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
            let (r, p) = precompress_file(file).inspect_err(|_| failed.store(true, Ordering::Relaxed))?;
            raw += r;
            packed += p;
        }
        Ok((raw, packed))
    };
    let results: Vec<Result<(u64, u64), String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..jobs.max(1)).map(|_| s.spawn(work)).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a compression thread stopped".to_owned()))).collect()
    });
    results.into_iter().try_fold((0, 0), |(raw, packed), r| r.map(|(r, p)| (raw + r, packed + p)))
}

/// One file's siblings: `.gz` only if it saves at least [`MIN_SAVING`], and then `.br` too.
fn precompress_file(path: &Path) -> Result<(u64, u64), String> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let size = data.len() as u64;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let gz_path = path.with_file_name(format!("{name}.gz"));
    let gz = gzip(&data, &name).map_err(|e| format!("{}: {e}", gz_path.display()))?;
    if gz.len() as f64 > size as f64 * (1.0 - MIN_SAVING) {
        if gz_path.exists() {
            fs::remove_file(&gz_path).map_err(|e| format!("{}: {e}", gz_path.display()))?;
        }
        return Ok((size, 0));
    }
    write_file(&gz_path, &gz)?;
    let br_path = path.with_file_name(format!("{name}.br"));
    let br = brotli(&data).map_err(|e| format!("{}: {e}", br_path.display()))?;
    write_file(&br_path, &br)?;
    Ok((size, (gz.len() + br.len()) as u64))
}

/// gzip at level 9 with the header Python's `gzip.GzipFile(path, "wb", mtime=0)` writes: the
/// original name (when it is Latin-1), no time, OS "unknown".
fn gzip(data: &[u8], name: &str) -> io::Result<Vec<u8>> {
    let mut builder = flate2::GzBuilder::new().mtime(0).operating_system(255);
    if name.chars().all(|c| u32::from(c) <= 0xff) && !name.is_empty() {
        builder = builder.filename(name.chars().map(|c| c as u8).collect::<Vec<u8>>());
    }
    let mut encoder = builder.write(Vec::with_capacity(data.len() / 3), flate2::Compression::best());
    encoder.write_all(data)?;
    encoder.finish()
}

/// brotli at quality 11 with the window the `brotli` command picks: the smallest that holds the
/// file, between 2^10 and 2^24.
fn brotli(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut lgwin = 10;
    while (1usize << lgwin) - 16 < data.len() && lgwin < 24 {
        lgwin += 1;
    }
    let params = brotli::enc::BrotliEncoderParams { quality: 11, lgwin, size_hint: data.len(), ..Default::default() };
    let mut out = Vec::with_capacity(data.len() / 4);
    brotli::BrotliCompress(&mut &data[..], &mut out, &params)?;
    Ok(out)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// A JSON string as Python's `json.dumps` writes it (`ensure_ascii`).
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
    out
}

/// A list of strings as Python's `json.dumps(indent=2)` writes it, `indent` spaces in.
fn json_list(items: &[String], indent: usize) -> String {
    if items.is_empty() {
        return "[]".to_owned();
    }
    let inner = " ".repeat(indent + 2);
    let body: Vec<String> = items.iter().map(|i| format!("{inner}{}", json_str(i))).collect();
    format!("[\n{}\n{}]", body.join(",\n"), " ".repeat(indent))
}

/// The fixups as Python's `json.dumps(fixups, sort_keys=True)` writes them (the cache key).
fn fixups_json(fixups: &[Fixup]) -> String {
    let items: Vec<String> = fixups.iter().map(|f| format!("{{\"copy\": {}, \"to\": {}}}", json_str(&f.copy), json_str(&f.to))).collect();
    format!("[{}]", items.join(", "))
}

/// `apps.json`, byte for byte as the Python script wrote it (`json.dumps(indent=2)` plus a newline).
fn apps_json(apps: &[(&str, &AppSpec)]) -> String {
    if apps.is_empty() {
        return "{\n  \"apps\": []\n}\n".to_owned();
    }
    let objects: Vec<String> = apps
        .iter()
        .map(|(id, spec)| {
            let s = |v: &str| json_str(v);
            let fields = [
                ("id", s(id)),
                ("name", s(&spec.name)),
                ("tagline", s(&spec.tagline)),
                ("color", s(&spec.color)),
                ("ink", json_list(&spec.ink, 6)),
                ("version", s(&spec.version)),
                ("bridge", s(&spec.bridge)),
                ("query", s(&spec.query)),
                ("opens", json_list(&spec.opens, 6)),
                ("default", json_list(&spec.defaults, 6)),
                ("entry", s(&format!("/apps/{id}/index.html"))),
            ];
            let body: Vec<String> = fields.iter().map(|(k, v)| format!("      {}: {v}", json_str(k))).collect();
            format!("    {{\n{}\n    }}", body.join(",\n"))
        })
        .collect();
    format!("{{\n  \"apps\": [\n{}\n  ]\n}}\n", objects.join(",\n"))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read as _};

    use zip::write::SimpleFileOptions;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-fetch-apps-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip with the given files; `None` contents make a symlink to `../../etc/passwd`.
    fn make_zip(dir: &Path, entries: &[(&str, Option<&[u8]>)]) -> PathBuf {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            match data {
                Some(data) => {
                    zip.start_file(*name, opts).unwrap();
                    zip.write_all(data).unwrap();
                }
                None => zip.add_symlink(*name, "../../etc/passwd", opts).unwrap(),
            }
        }
        let path = dir.join("app.zip");
        fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();
        path
    }

    fn spec(fixups: Vec<Fixup>) -> AppSpec {
        AppSpec {
            name: "WordCraft".into(),
            tagline: "Writing \"and\" d\u{e9}sign".into(),
            color: "#3f5fe0".into(),
            ink: vec!["#8ea2ff".into(), "#2a44b8".into()],
            repo: "storytold/wordcraft".into(),
            version: "0.18.1".into(),
            asset: "wordcraft-web-0.18.1.zip".into(),
            sha256: "0".repeat(64),
            bridge: "native".into(),
            query: String::new(),
            opens: vec!["docx".into(), "md".into()],
            defaults: vec![],
            fixups,
        }
    }

    #[test]
    fn sha256_mismatch_is_refused_and_leaves_nothing() {
        let dir = scratch("sha");
        let (part, target) = (dir.join("x.part"), dir.join("x.zip"));
        let good = hex(&Sha256::digest(b"zip bytes"));
        let err = store_verified(&b"other bytes"[..], &good, &part, &target, "test").unwrap_err();
        assert!(err.contains("sha256 mismatch"), "{err}");
        assert!(!part.exists() && !target.exists());
        store_verified(&b"zip bytes"[..], &good, &part, &target, "test").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"zip bytes");
        assert!(!part.exists());
    }

    #[test]
    fn unpack_strips_the_single_top_folder() {
        let dir = scratch("strip");
        let zip = make_zip(&dir, &[("app-1.0/index.html", Some(b"<html>")), ("app-1.0/a/./b.js", Some(b"js"))]);
        let dest = dir.join("out");
        fs::create_dir(&dest).unwrap();
        unpack(&zip, &dest).unwrap();
        assert_eq!(fs::read(dest.join("index.html")).unwrap(), b"<html>");
        assert_eq!(fs::read(dest.join("a/b.js")).unwrap(), b"js");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(dest.join("index.html")).unwrap().permissions().mode() & 0o777, 0o644);
        }
        // Two top-level names: nothing is stripped.
        let zip = make_zip(&dir, &[("index.html", Some(b"x")), ("app/x.js", Some(b"y"))]);
        let dest = dir.join("out2");
        fs::create_dir(&dest).unwrap();
        unpack(&zip, &dest).unwrap();
        assert!(dest.join("index.html").is_file() && dest.join("app/x.js").is_file());
    }

    #[test]
    fn unpack_refuses_path_traversal_absolute_paths_and_symlinks() {
        let dir = scratch("traversal");
        type Entries<'a> = &'a [(&'a str, Option<&'a [u8]>)];
        let cases: [Entries; 4] = [
            &[("app/index.html", Some(b"x")), ("app/../../evil.js", Some(b"x"))],
            &[("index.html", Some(b"x")), ("../evil.js", Some(b"x"))],
            &[("index.html", Some(b"x")), ("/tmp/evil.js", Some(b"x"))],
            &[("index.html", Some(b"x")), ("link", None)],
        ];
        for entries in cases {
            let zip = make_zip(&dir, entries);
            let dest = dir.join("out");
            let _ = fs::remove_dir_all(&dest);
            fs::create_dir(&dest).unwrap();
            let err = unpack(&zip, &dest).unwrap_err();
            assert!(err.contains("unsafe path") || err.contains("symlink"), "{entries:?}: {err}");
        }
        assert!(!dir.join("evil.js").exists() && !dir.parent().unwrap().join("evil.js").exists());
    }

    #[test]
    fn fixup_copies_a_folder_and_stays_inside() {
        let dir = scratch("fixup");
        fs::create_dir_all(dir.join("snippets/x")).unwrap();
        fs::write(dir.join("snippets/x/a.js"), "a").unwrap();
        let fixups = [Fixup { copy: "snippets".into(), to: "app/snippets".into() }];
        apply_fixups("photocraft", &dir, &fixups).unwrap();
        assert_eq!(fs::read(dir.join("app/snippets/x/a.js")).unwrap(), b"a");
        // A file into an existing folder lands under its own name.
        fs::write(dir.join("one.txt"), "1").unwrap();
        apply_fixups("t", &dir, &[Fixup { copy: "one.txt".into(), to: "app".into() }]).unwrap();
        assert_eq!(fs::read(dir.join("app/one.txt")).unwrap(), b"1");

        let refused = [("../snippets", "x"), ("snippets", "../../x"), ("/etc", "x"), ("missing", "x"), ("snippets", "snippets/y")];
        for (copy, to) in refused {
            assert!(apply_fixups("t", &dir, &[Fixup { copy: copy.into(), to: to.into() }]).is_err(), "{copy} -> {to}");
        }
    }

    #[test]
    fn apps_json_has_the_python_shape() {
        let a = spec(vec![]);
        let json = apps_json(&[("wordcraft", &a)]);
        let expected = r##"{
  "apps": [
    {
      "id": "wordcraft",
      "name": "WordCraft",
      "tagline": "Writing \"and\" d\u00e9sign",
      "color": "#3f5fe0",
      "ink": [
        "#8ea2ff",
        "#2a44b8"
      ],
      "version": "0.18.1",
      "bridge": "native",
      "query": "",
      "opens": [
        "docx",
        "md"
      ],
      "default": [],
      "entry": "/apps/wordcraft/index.html"
    }
  ]
}
"##;
        assert_eq!(json, expected);
        // It is JSON, and the shape the web frontend reads.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["apps"][0]["tagline"], "Writing \"and\" d\u{e9}sign");
        assert_eq!(apps_json(&[]), "{\n  \"apps\": []\n}\n");
        let two = apps_json(&[("a", &a), ("b", &a)]);
        assert!(two.contains("    },\n    {\n      \"id\": \"b\""), "{two}");
    }

    #[test]
    fn fixup_cache_key_matches_python() {
        let fx = [Fixup { copy: "snippets".into(), to: "app/snippets".into() }];
        assert_eq!(fixups_json(&fx), r#"[{"copy": "snippets", "to": "app/snippets"}]"#);
    }

    #[test]
    fn precompression_picks_the_right_files() {
        let dir = scratch("precompress");
        let text = "function f() { return 42; }\n".repeat(200);
        // Incompressible bytes (a simple generator, so the test needs no randomness crate).
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let noise: Vec<u8> = (0..4096)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x.to_le_bytes()[0]
            })
            .collect();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("app.js"), &text).unwrap();
        fs::write(dir.join("sub/STYLE.CSS"), &text).unwrap();
        fs::write(dir.join("image.png"), &text).unwrap();
        fs::write(dir.join("noise.wasm"), &noise).unwrap();
        fs::write(dir.join("noise.wasm.gz"), b"stale").unwrap();
        fs::write(dir.join("empty.json"), b"").unwrap();
        let (raw, packed) = precompress(&dir, 2).unwrap();
        assert_eq!(raw, (text.len() * 2 + noise.len()) as u64);
        assert!(packed > 0);
        for kept in ["app.js", "sub/STYLE.CSS"] {
            let gz = fs::read(dir.join(format!("{kept}.gz"))).unwrap();
            let mut back = String::new();
            flate2::read::GzDecoder::new(&gz[..]).read_to_string(&mut back).unwrap();
            assert_eq!(back, text);
            let br = fs::read(dir.join(format!("{kept}.br"))).unwrap();
            let mut back = String::new();
            brotli::Decompressor::new(&br[..], 4096).read_to_string(&mut back).unwrap();
            assert_eq!(back, text);
        }
        // Not a compressible type, too little saving (a stale sibling goes), or empty.
        for absent in ["image.png.gz", "image.png.br", "noise.wasm.gz", "noise.wasm.br", "empty.json.gz", "empty.json.br"] {
            assert!(!dir.join(absent).exists(), "{absent}");
        }
        // The gzip header names the file and carries no time, as Python's does.
        let gz = fs::read(dir.join("app.js.gz")).unwrap();
        assert_eq!(gz.get(..10).unwrap(), [0x1f, 0x8b, 8, 8, 0, 0, 0, 0, 2, 255]);
        assert_eq!(gz.get(10..17).unwrap(), b"app.js\0");
    }
}
