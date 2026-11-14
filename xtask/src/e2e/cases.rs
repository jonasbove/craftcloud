//! The cases, in order: each changes something on one client, syncs the two, and checks the other
//! side (and the cloud, where it matters). Later cases build on the files earlier ones left.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{Run, Verdict, Who};

/// A case: a detail for the table, or why it failed.
type Case = fn(&mut Run) -> Result<String, String>;

const LAPTOP: Who = Who::Laptop;
const STUDIO: Who = Who::Studio;

pub fn all(run: &mut Run) {
    let cases: [(&str, Case); 19] = [
        ("create Laptop → Studio", create_on_laptop),
        ("create Studio → Laptop", create_on_studio),
        ("edit Laptop → Studio", edit_on_laptop),
        ("edit Studio → Laptop", edit_on_studio),
        ("rename Laptop → Studio", rename_on_laptop),
        ("rename Studio → Laptop", rename_on_studio),
        ("move into a new folder Laptop → Studio", move_on_laptop),
        ("move into another folder Studio → Laptop", move_on_studio),
        ("unicode names (é, 日本, 🐟) Laptop → Studio", unicode_names),
        ("20 MB file Laptop → Studio, same SHA-256", big_file),
        ("200 small files Studio → Laptop", many_files),
        ("delete a file Studio → Laptop", delete_file),
        ("the deleted file is in the cloud trash (REST)", file_in_trash),
        ("delete a folder Laptop → Studio", delete_folder),
        ("the deleted folder is in the cloud trash as one item (REST)", folder_in_trash),
        ("conflict: both edits kept, a conflict copy on both sides", conflict),
        ("T-063: notes.tmp stays local, the projekt.tmp/ folder syncs", temp_names),
        ("T-064: projekt.tmp/ and notes.tmp still there after 3 more pass pairs", temp_names_stay),
        ("the cloud, the Laptop and the Studio hold the same files at the end", same_everywhere),
    ];
    for (name, case) in cases {
        let (verdict, detail) = match case(run) {
            Ok(detail) => (Verdict::Pass, detail),
            Err(why) => (Verdict::Fail, why),
        };
        run.row(name, verdict, &detail);
    }
}

fn write(run: &Run, who: Who, rel: &str, content: &[u8]) -> Result<(), String> {
    let path = run.path(who, rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))
}

fn rename(run: &Run, who: Who, from: &str, to: &str) -> Result<(), String> {
    let (from, to) = (run.path(who, from), run.path(who, to));
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::rename(&from, &to).map_err(|e| format!("{} → {}: {e}", from.display(), to.display()))
}

fn read(run: &Run, who: Who, rel: &str) -> Option<String> {
    fs::read_to_string(run.path(who, rel)).ok()
}

/// `rel` on `who` holds exactly `want`.
fn has(run: &Run, who: Who, rel: &str, want: &str) -> Result<(), String> {
    match read(run, who, rel) {
        Some(got) if got == want => Ok(()),
        Some(got) => Err(format!("{rel} on {} holds {got:?}, not {want:?}", name(who))),
        None => Err(format!("{rel} is missing on {}", name(who))),
    }
}

fn gone(run: &Run, who: Who, rel: &str) -> Result<(), String> {
    if run.path(who, rel).exists() { Err(format!("{rel} is still on {}", name(who))) } else { Ok(()) }
}

fn name(who: Who) -> &'static str {
    match who {
        Who::Laptop => "Laptop",
        Who::Studio => "Studio",
    }
}

fn create_on_laptop(run: &mut Run) -> Result<String, String> {
    write(run, LAPTOP, "a1.txt", b"made on Laptop")?;
    run.pair(LAPTOP);
    has(run, STUDIO, "a1.txt", "made on Laptop").map(|()| String::new())
}

fn create_on_studio(run: &mut Run) -> Result<String, String> {
    write(run, STUDIO, "b1.txt", b"made on Studio")?;
    run.pair(STUDIO);
    has(run, LAPTOP, "b1.txt", "made on Studio").map(|()| String::new())
}

fn edit_on_laptop(run: &mut Run) -> Result<String, String> {
    write(run, LAPTOP, "b1.txt", b"edited on Laptop")?;
    run.pair(LAPTOP);
    has(run, STUDIO, "b1.txt", "edited on Laptop").map(|()| String::new())
}

fn edit_on_studio(run: &mut Run) -> Result<String, String> {
    write(run, STUDIO, "a1.txt", b"edited on Studio")?;
    run.pair(STUDIO);
    has(run, LAPTOP, "a1.txt", "edited on Studio").map(|()| String::new())
}

fn rename_on_laptop(run: &mut Run) -> Result<String, String> {
    rename(run, LAPTOP, "a1.txt", "a1-renamed.txt")?;
    run.pair(LAPTOP);
    has(run, STUDIO, "a1-renamed.txt", "edited on Studio")?;
    gone(run, STUDIO, "a1.txt").map(|()| String::new())
}

fn rename_on_studio(run: &mut Run) -> Result<String, String> {
    rename(run, STUDIO, "b1.txt", "b1-renamed.txt")?;
    run.pair(STUDIO);
    has(run, LAPTOP, "b1-renamed.txt", "edited on Laptop")?;
    gone(run, LAPTOP, "b1.txt").map(|()| String::new())
}

fn move_on_laptop(run: &mut Run) -> Result<String, String> {
    rename(run, LAPTOP, "a1-renamed.txt", "sub/deep/a1-renamed.txt")?;
    run.pair(LAPTOP);
    has(run, STUDIO, "sub/deep/a1-renamed.txt", "edited on Studio")?;
    gone(run, STUDIO, "a1-renamed.txt").map(|()| String::new())
}

fn move_on_studio(run: &mut Run) -> Result<String, String> {
    rename(run, STUDIO, "b1-renamed.txt", "sub/b1-renamed.txt")?;
    run.pair(STUDIO);
    has(run, LAPTOP, "sub/b1-renamed.txt", "edited on Laptop")?;
    gone(run, LAPTOP, "b1-renamed.txt").map(|()| String::new())
}

const UNICODE: [(&str, &str); 3] = [("Crème brûlée é ü 日本.txt", "café"), ("Été/fichier.txt", "x"), ("🐟 makrel.txt", "fisk")];

fn unicode_names(run: &mut Run) -> Result<String, String> {
    for (rel, content) in UNICODE {
        write(run, LAPTOP, rel, content.as_bytes())?;
    }
    run.pair(LAPTOP);
    UNICODE.iter().try_for_each(|(rel, content)| has(run, STUDIO, rel, content)).map(|()| String::new())
}

/// 20 MB that don't compress: xorshift from the clock (no need for good randomness, only for
/// bytes a server or a transfer can't shortcut).
fn noise(len: usize) -> Vec<u8> {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut x = (seed as u64) | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

fn big_file(run: &mut Run) -> Result<String, String> {
    write(run, LAPTOP, "big.bin", &noise(20_000_000))?;
    let want = sha256(&run.path(LAPTOP, "big.bin"))?;
    run.pair(LAPTOP);
    let got = sha256(&run.path(STUDIO, "big.bin")).map_err(|_| "big.bin is missing on Studio".to_owned())?;
    if got == want {
        Ok(format!("sha256 {}…", &want[..16]))
    } else {
        Err(format!("sha256 {}… on Laptop, {}… on Studio", &want[..16], &got[..16]))
    }
}

fn many_files(run: &mut Run) -> Result<String, String> {
    for i in 1..=200 {
        write(run, STUDIO, &format!("many/f{i:03}.txt"), i.to_string().as_bytes())?;
    }
    run.pair(STUDIO);
    let arrived = fs::read_dir(run.path(LAPTOP, "many")).map(|d| d.flatten().count()).unwrap_or(0);
    let wrong: Vec<String> = (1..=200)
        .filter(|i| read(run, LAPTOP, &format!("many/f{i:03}.txt")).as_deref() != Some(i.to_string().as_str()))
        .map(|i| format!("f{i:03}.txt"))
        .collect();
    match (arrived, wrong.first()) {
        (200, None) => Ok(String::new()),
        (n, first) => Err(format!("{n} files on Laptop, {} missing or wrong (first: {})", wrong.len(), first.map_or("-", String::as_str))),
    }
}

fn delete_file(run: &mut Run) -> Result<String, String> {
    let path = run.path(STUDIO, "sub/b1-renamed.txt");
    fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    run.pair(STUDIO);
    gone(run, LAPTOP, "sub/b1-renamed.txt").map(|()| String::new())
}

/// Trashed items as paths inside the run's folder (`Personal/<run>/x` → `x`), with whether each is
/// a folder.
fn trashed(run: &Run) -> Result<Vec<(String, bool)>, String> {
    let prefix = format!("{}/", run.cloud);
    Ok(run
        .trashed()?
        .into_iter()
        .filter_map(|t| {
            let rel = t.path.split_once('/').map_or(t.path.as_str(), |(_, rest)| rest);
            rel.strip_prefix(&prefix).map(|r| (r.to_owned(), t.folder))
        })
        .collect())
}

fn file_in_trash(run: &mut Run) -> Result<String, String> {
    let items = trashed(run)?;
    if items.iter().any(|(p, folder)| p == "sub/b1-renamed.txt" && !folder) {
        Ok(String::new())
    } else {
        Err(format!("no sub/b1-renamed.txt in the trash; the run's trashed items: {:?}", items.iter().map(|(p, _)| p).collect::<Vec<_>>()))
    }
}

fn delete_folder(run: &mut Run) -> Result<String, String> {
    let path = run.path(LAPTOP, "Été");
    fs::remove_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    run.pair(LAPTOP);
    gone(run, STUDIO, "Été").map(|()| String::new())
}

fn folder_in_trash(run: &mut Run) -> Result<String, String> {
    let items = trashed(run)?;
    let folder = items.iter().any(|(p, folder)| p == "Été" && *folder);
    let loose = items.iter().any(|(p, _)| p.starts_with("Été/"));
    match (folder, loose) {
        (true, false) => Ok(String::new()),
        (true, true) => Err("Été is in the trash, but so are files from it, one by one".to_owned()),
        (false, _) => Err(format!("no folder Été in the trash; the run's trashed items: {:?}", items.iter().map(|(p, _)| p).collect::<Vec<_>>())),
    }
}

/// The files at the top of the run's folder whose names start with `conf`, with their contents.
fn conf_files(run: &Run, who: Who) -> Vec<(String, String)> {
    let Ok(dir) = fs::read_dir(run.path(who, "")) else { return Vec::new() };
    let mut out: Vec<(String, String)> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("conf").then(|| (name, fs::read_to_string(e.path()).unwrap_or_default()))
        })
        .collect();
    out.sort();
    out
}

fn conflict(run: &mut Run) -> Result<String, String> {
    write(run, LAPTOP, "conf.txt", b"base")?;
    run.pair(LAPTOP);
    has(run, STUDIO, "conf.txt", "base")?;
    write(run, LAPTOP, "conf.txt", b"Laptop's edit")?;
    write(run, STUDIO, "conf.txt", b"Studio's edit")?;
    run.sync(LAPTOP);
    run.sync(STUDIO);
    run.sync(LAPTOP);
    let mut copy = String::new();
    for who in [LAPTOP, STUDIO] {
        let files = conf_files(run, who);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        let mut contents: Vec<&str> = files.iter().map(|(_, c)| c.as_str()).collect();
        contents.sort_unstable();
        let conflict_copy = names.iter().find(|n| n.contains(" (conflict "));
        if files.len() != 2 || !names.contains(&"conf.txt") || conflict_copy.is_none() || contents != ["Laptop's edit", "Studio's edit"] {
            return Err(format!("on {}: {files:?}", name(who)));
        }
        copy = conflict_copy.map(|n| (*n).to_owned()).unwrap_or_default();
    }
    Ok(copy)
}

fn temp_names(run: &mut Run) -> Result<String, String> {
    write(run, LAPTOP, "notes.tmp", b"scratch")?;
    write(run, LAPTOP, "projekt.tmp/inside.txt", b"real work")?;
    run.pair(LAPTOP);
    gone(run, STUDIO, "notes.tmp")?;
    has(run, STUDIO, "projekt.tmp/inside.txt", "real work")?;
    // The Laptop's pass says why notes.tmp stayed.
    let json = run.last_pass(LAPTOP);
    let listed = json["not_synced"]
        .as_array()
        .is_some_and(|l| l.iter().any(|n| n["reason"] == "temporary" && n["path"].as_str().is_some_and(|p| p.ends_with("notes.tmp"))));
    if !listed {
        return Err("the Laptop's pass doesn't list notes.tmp under not_synced as temporary".to_owned());
    }
    in_cloud(run, &["projekt.tmp"], &["notes.tmp"]).map(|()| "listed as temporary in not_synced".to_owned())
}

/// The run's cloud folder (listed by the Laptop) has every name in `present` and none in `absent`.
fn in_cloud(run: &mut Run, present: &[&str], absent: &[&str]) -> Result<(), String> {
    let cloud = run.cloud.clone();
    let listed = run.cli_json(LAPTOP, &["ls", &cloud])?;
    let names: Vec<&str> = listed.as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|i| i["name"].as_str()).collect();
    if let Some(missing) = present.iter().find(|p| !names.contains(p)) {
        return Err(format!("{missing} is not in the cloud"));
    }
    if let Some(there) = absent.iter().find(|a| names.contains(a)) {
        return Err(format!("{there} is in the cloud"));
    }
    Ok(())
}

fn temp_names_stay(run: &mut Run) -> Result<String, String> {
    for _ in 0..3 {
        run.pair(LAPTOP);
    }
    has(run, STUDIO, "projekt.tmp/inside.txt", "real work")?;
    has(run, LAPTOP, "notes.tmp", "scratch")?;
    in_cloud(run, &["projekt.tmp"], &["notes.tmp"]).map(|()| String::new())
}

/// Files (path → size) under `dir` on this computer, without CraftCloud's own folder and the
/// temporary-looking files the sync leaves alone.
fn local_tree(dir: &Path, prefix: &str, depth: u32, out: &mut BTreeMap<String, u64>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = format!("{prefix}{name}");
        match e.metadata() {
            Ok(m) if m.is_dir() && name != ".craftcloud" && depth < 16 => local_tree(&e.path(), &format!("{rel}/"), depth + 1, out),
            Ok(m) if m.is_file() && !name.ends_with(".tmp") => {
                out.insert(rel, m.len());
            }
            _ => {}
        }
    }
}

/// Files (path → size) in the run's cloud folder, listed folder by folder with the Laptop's `ls`.
fn cloud_tree(run: &mut Run, rel: &str, depth: u32, out: &mut BTreeMap<String, u64>) -> Result<(), String> {
    let path = if rel.is_empty() { run.cloud.clone() } else { format!("{}/{rel}", run.cloud) };
    let listed = run.cli_json(LAPTOP, &["ls", &path])?;
    for item in listed.as_array().map(Vec::as_slice).unwrap_or_default() {
        let name = item["name"].as_str().unwrap_or_default();
        let child = if rel.is_empty() { name.to_owned() } else { format!("{rel}/{name}") };
        if item["type"] == "folder" && depth < 16 {
            cloud_tree(run, &child, depth + 1, out)?;
        } else if item["type"] == "file" {
            out.insert(child, item["size"].as_u64().unwrap_or(u64::MAX));
        }
    }
    Ok(())
}

/// What differs between two trees, for the table (the first few).
fn differences(a: &BTreeMap<String, u64>, b: &BTreeMap<String, u64>) -> Vec<String> {
    let mut out: Vec<String> = a.iter().filter(|(p, s)| b.get(*p) != Some(s)).map(|(p, s)| format!("{p} ({s} B)")).collect();
    out.extend(b.keys().filter(|p| !a.contains_key(*p)).map(|p| format!("only in the other: {p}")));
    out
}

/// A stray client syncing the account, or a pass that dropped something, leaves the three apart.
fn same_everywhere(run: &mut Run) -> Result<String, String> {
    let mut cloud = BTreeMap::new();
    cloud_tree(run, "", 0, &mut cloud)?;
    for who in [LAPTOP, STUDIO] {
        let mut local = BTreeMap::new();
        local_tree(&run.path(who, ""), "", 0, &mut local);
        let diff = differences(&cloud, &local);
        if !diff.is_empty() {
            let shown: Vec<&str> = diff.iter().take(5).map(String::as_str).collect();
            return Err(format!("{} differs from the cloud in {} places: {}", name(who), diff.len(), shown.join(", ")));
        }
    }
    Ok(format!("{} files", cloud.len()))
}
