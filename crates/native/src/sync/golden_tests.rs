//! Golden request logs: what the engine sends a scripted OxiCloud, cycle by cycle, with the
//! activity lines, the sync state and what it saved. Recorded before the storage layer was put
//! between the engine and the server (docs/WEBDAV-STORAGE.md § 14, pull request 1), and compared
//! on every run: a change that alters a request (method, path, a guard header, the order where it
//! matters), an activity line or the saved state fails here.
//!
//! The logs are in `golden/`. `CRAFTCLOUD_BLESS=1 cargo test -p craftcloud-native golden` writes
//! them again, only for a change that is meant to change what goes over the wire.
//!
//! What is left out, because it varies from run to run: the server's port (`http://SERVER`), the
//! order of the folders listed at the same time (each run of `PROPFIND`s is sorted), the time in
//! a version's name (`<STAMP>`) and in a conflict copy's name (`<WHEN>`), and headers the HTTP
//! library adds (only the ones in [`HEADERS`] are logged).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use craftcloud_core::reconcile::{Known, State};
use craftcloud_core::sync::SyncState;

use super::engine_tests::engine;
use super::{Deletes, Engine, Saved, local_meta, part_name, save_saved, synced_state};
use crate::fake_server::{Answer, FakeServer, Seen};

/// The request headers that say what a request does (and its guards), in the order logged.
const HEADERS: [&str; 8] = ["authorization", "depth", "destination", "overwrite", "if-match", "if-none-match", "range", "content-type"];

/// When every local file in these scenarios was last written: long settled.
const T0: u64 = 1_767_225_600;

/// What the cloud's listings say a file was last changed.
const MODIFIED: &str = "Mon, 16 Nov 2026 10:00:00 GMT";

// --- A small OxiCloud: WebDAV for the bytes, REST for ids, the trash and the account ---------

/// Files taken out of the cloud or put back: path → (ETag, bytes).
type Files = Vec<(String, (String, String))>;

#[derive(Clone)]
struct Trashed {
    id: String,
    path: String,
    folder: bool,
    files: Files,
    folders: Vec<String>,
}

/// The fake cloud's contents. File ETags are kept without quotes and sent with them.
struct Cloud {
    /// Path → (ETag, bytes).
    files: BTreeMap<String, (String, String)>,
    /// Every folder but the drive's root (`""`).
    folders: BTreeSet<String>,
    trash: Vec<Trashed>,
    next: u32,
    /// The account: (role, bytes used, quota; 0 is none).
    me: (&'static str, u64, u64),
}

/// A scenario's own answers, asked before the cloud's.
type Special = Box<dyn Fn(&Seen, &mut Cloud) -> Option<Answer> + Send + Sync>;

impl Cloud {
    fn new(files: &[(&str, &str, &str)], folders: &[&str]) -> Self {
        let mut cloud = Self { files: BTreeMap::new(), folders: BTreeSet::new(), trash: Vec::new(), next: 0, me: ("user", 0, 0) };
        for f in folders {
            cloud.add_parents(&format!("{f}/x"));
        }
        for (path, etag, bytes) in files {
            cloud.add_parents(path);
            cloud.files.insert((*path).to_owned(), ((*etag).to_owned(), (*bytes).to_owned()));
        }
        cloud
    }

    fn add_parents(&mut self, path: &str) {
        let mut at = path;
        while let Some((parent, _)) = at.rsplit_once('/') {
            self.folders.insert(parent.to_owned());
            at = parent;
        }
    }

    fn is_folder(&self, path: &str) -> bool {
        path.is_empty() || self.folders.contains(path)
    }

    fn exists(&self, path: &str) -> bool {
        self.is_folder(path) || self.files.contains_key(path)
    }

    fn parent_of(path: &str) -> &str {
        path.rsplit_once('/').map_or("", |(p, _)| p)
    }

    fn etag(&mut self) -> String {
        self.next += 1;
        format!("n{}", self.next)
    }

    fn under<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
        path.strip_prefix(prefix).and_then(|rest| rest.strip_prefix('/'))
    }

    /// Takes `path` (a file, or a folder with everything in it) out of the cloud.
    fn take(&mut self, path: &str) -> (Files, Vec<String>) {
        let files: Files =
            self.files.iter().filter(|(p, _)| *p == path || Self::under(p, path).is_some()).map(|(p, f)| (p.clone(), f.clone())).collect();
        let folders: Vec<String> = self.folders.iter().filter(|f| *f == path || Self::under(f, path).is_some()).cloned().collect();
        for (p, _) in &files {
            self.files.remove(p);
        }
        for f in &folders {
            self.folders.remove(f);
        }
        (files, folders)
    }

    fn put_back(&mut self, files: Files, folders: Vec<String>) {
        self.folders.extend(folders);
        self.files.extend(files);
    }

    fn answer(&mut self, r: &Seen) -> Answer {
        let (path, query) = r.path.split_once('?').unwrap_or((r.path.as_str(), ""));
        if let Some(dav) = path.strip_prefix("/webdav") {
            return self.dav(r, &decode(dav.trim_matches('/')));
        }
        self.rest(r, path, query)
    }

    fn dav(&mut self, r: &Seen, path: &str) -> Answer {
        match r.method.as_str() {
            "PROPFIND" if r.header("depth") == Some("0") => match self.entry(path) {
                Some(e) => multistatus(&[e]),
                None => Answer::new(404, ""),
            },
            "PROPFIND" => {
                if !self.is_folder(path) {
                    return Answer::new(404, "");
                }
                let mut entries = vec![self.entry(path).unwrap_or_default()];
                let children = |p: &&String| Self::parent_of(p) == path && !p.is_empty();
                entries.extend(self.folders.iter().filter(children).filter_map(|f| self.entry(f)));
                entries.extend(self.files.keys().filter(children).filter_map(|f| self.entry(f)));
                multistatus(&entries)
            }
            "GET" => {
                let Some((etag, bytes)) = self.files.get(path) else { return Answer::new(404, "") };
                let from = r.header("range").and_then(|v| v.strip_prefix("bytes=")).and_then(|v| v.strip_suffix('-')).and_then(|v| v.parse().ok());
                let answer = match from {
                    Some(from) if from < bytes.len() => {
                        Answer::new(206, &bytes[from..]).header("Content-Range", format!("bytes {from}-{}/{}", bytes.len() - 1, bytes.len()))
                    }
                    _ => Answer::new(200, bytes.clone()),
                };
                answer.header("Content-Type", "application/octet-stream").header("ETag", format!("\"{etag}\""))
            }
            "PUT" => {
                if !self.is_folder(Self::parent_of(path)) {
                    return Answer::new(409, "");
                }
                let current = self.files.get(path).map(|(e, _)| e.clone());
                let allowed = match (r.header("if-none-match"), r.header("if-match")) {
                    (Some("*"), _) => current.is_none(),
                    (_, Some(want)) => current.as_deref() == Some(want.trim_matches('"')),
                    _ => true,
                };
                if !allowed {
                    return Answer::new(412, "");
                }
                let etag = self.etag();
                self.files.insert(path.to_owned(), (etag.clone(), r.body.clone()));
                Answer::new(if current.is_some() { 204 } else { 201 }, "").header("ETag", format!("\"{etag}\""))
            }
            "MKCOL" if self.exists(path) => Answer::new(405, ""),
            "MKCOL" if !self.is_folder(Self::parent_of(path)) => Answer::new(409, ""),
            "MKCOL" => {
                self.folders.insert(path.to_owned());
                Answer::new(201, "")
            }
            "MOVE" | "COPY" => {
                let Some(to) = r.header("destination").and_then(|d| d.split_once("/webdav")).map(|(_, p)| decode(p.trim_matches('/'))) else {
                    return Answer::new(400, "");
                };
                if !self.exists(path) {
                    return Answer::new(404, "");
                }
                if self.exists(&to) && r.header("overwrite") != Some("T") {
                    return Answer::new(412, "");
                }
                if !self.is_folder(Self::parent_of(&to)) {
                    return Answer::new(409, "");
                }
                let (files, folders) = self.take(path);
                let moved = |p: &String| format!("{to}{}", &p[path.len()..]);
                let copies: Files = files.iter().map(|(p, f)| (moved(p), f.clone())).collect();
                let new_folders: Vec<String> = folders.iter().map(moved).collect();
                if r.method == "COPY" {
                    self.put_back(files, folders);
                }
                self.put_back(copies, new_folders);
                Answer::new(201, "")
            }
            "DELETE" => {
                if !self.exists(path) {
                    return Answer::new(404, "");
                }
                self.take(path);
                Answer::new(204, "")
            }
            _ => Answer::new(405, ""),
        }
    }

    fn entry(&self, path: &str) -> Option<String> {
        let href = if path.is_empty() { "/webdav/".to_owned() } else { format!("/webdav/{}", encode(path)) };
        if self.is_folder(path) {
            let href = if path.is_empty() { href } else { format!("{href}/") };
            return Some(format!(
                "<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype>\
                 <D:getetag>&quot;folder&quot;</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
            ));
        }
        let (etag, bytes) = self.files.get(path)?;
        Some(format!(
            "<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:resourcetype/><D:getetag>&quot;{etag}&quot;</D:getetag>\
             <D:getcontentlength>{}</D:getcontentlength><D:getlastmodified>{MODIFIED}</D:getlastmodified></D:prop>\
             <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            bytes.len()
        ))
    }

    fn rest(&mut self, r: &Seen, path: &str, query: &str) -> Answer {
        let json = |body: serde_json::Value| Answer::new(200, body.to_string());
        let seg: Vec<String> = path.trim_matches('/').split('/').map(decode).collect();
        let seg: Vec<&str> = seg.iter().map(String::as_str).collect();
        match (r.method.as_str(), seg.as_slice()) {
            ("GET", ["api", "folders"]) => json(serde_json::json!([{"id": "root", "name": "Personal", "path": "/Personal", "parent_id": null}])),
            ("GET", ["api", "folders", id, "resources"]) => {
                assert_eq!(query, "limit=200", "the engine pages by 200");
                let Some(folder) = path_of(id).filter(|f| self.is_folder(f)) else { return Answer::new(404, "{}") };
                let child = |p: &&String| Self::parent_of(p) == folder && !p.is_empty();
                let name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_owned();
                let mut items: Vec<serde_json::Value> = self
                    .folders
                    .iter()
                    .filter(child)
                    .map(|f| serde_json::json!({"resource_type": "folder", "resource": {"id": id_of('d', f), "name": name(f)}}))
                    .collect();
                items.extend(
                    self.files
                        .keys()
                        .filter(child)
                        .map(|f| serde_json::json!({"resource_type": "file", "resource": {"id": id_of('f', f), "name": name(f)}})),
                );
                json(serde_json::json!({"items": items}))
            }
            ("DELETE", ["api", kind @ ("files" | "folders"), id]) => {
                let Some(path) = path_of(id).filter(|p| if *kind == "files" { self.files.contains_key(p) } else { self.folders.contains(p) }) else {
                    return Answer::new(404, "{}");
                };
                let (files, folders) = self.take(&path);
                let id = format!("t{}", self.trash.len() + 1);
                self.trash.push(Trashed { id, path, folder: *kind == "folders", files, folders });
                Answer::new(204, "")
            }
            ("GET", ["api", "trash", "resources"]) => {
                assert_eq!(query, "limit=200", "the engine pages by 200");
                let items: Vec<serde_json::Value> = self
                    .trash
                    .iter()
                    .map(|t| {
                        let kind = if t.folder { "folder" } else { "file" };
                        let name = t.path.rsplit('/').next().unwrap_or(&t.path);
                        serde_json::json!({"resource_type": kind, "resource": {"id": t.id, "name": name, "path": format!("/Personal/{}", t.path)},
                            "trashed_at": "2026-11-16T10:00:00Z", "expires_at": "2026-12-16T10:00:00Z"})
                    })
                    .collect();
                json(serde_json::json!({"items": items}))
            }
            ("POST", ["api", "trash", id, "restore"]) => {
                if let Some(i) = self.trash.iter().position(|t| t.id == *id) {
                    let t = self.trash.remove(i);
                    self.put_back(t.files, t.folders);
                }
                json(serde_json::json!({}))
            }
            ("GET", ["api", "auth", "me"]) => {
                let (role, used, quota) = self.me;
                json(serde_json::json!({"full": {"user": {"id": "u1", "username": "john", "email": "john@example.org", "role": role},
                    "storage_used_bytes": used, "storage_quota_bytes": quota}}))
            }
            ("POST", ["api", "admin", "jobs", _, "trigger"]) => {
                self.me.1 = 0;
                json(serde_json::json!({}))
            }
            _ => Answer::new(404, r#"{"error":"not in the fake"}"#),
        }
    }
}

fn multistatus(entries: &[String]) -> Answer {
    Answer::new(207, format!(r#"<D:multistatus xmlns:D="DAV:">{}</D:multistatus>"#, entries.concat()))
}

/// REST ids: `root`, and `d`/`f` and the path in hex (only letters and digits, as the client's
/// encoding leaves them as they are).
fn id_of(kind: char, path: &str) -> String {
    path.bytes().fold(kind.to_string(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn path_of(id: &str) -> Option<String> {
    if id == "root" {
        return Some(String::new());
    }
    let hex = id.get(1..)?;
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
    String::from_utf8(bytes?).ok()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%').then(|| s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode(path: &str) -> String {
    path.replace(' ', "%20")
}

// --- Running a scenario -----------------------------------------------------------------------

struct Scenario {
    server: FakeServer,
    cloud: Arc<Mutex<Cloud>>,
    _dir: tempfile::TempDir,
    root: PathBuf,
    engine: Engine,
    log: String,
    requests_seen: usize,
    activity_seen: usize,
}

impl Scenario {
    fn new(cloud: Cloud, special: Option<Special>) -> Self {
        let cloud = Arc::new(Mutex::new(cloud));
        let shared = Arc::clone(&cloud);
        let server = FakeServer::start_full(move |r| {
            let mut cloud = shared.lock().unwrap_or_else(PoisonError::into_inner);
            special.as_ref().and_then(|s| s(r, &mut cloud)).unwrap_or_else(|| cloud.answer(r))
        });
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("CraftCloud");
        std::fs::create_dir(&root).unwrap();
        let engine = engine(server.server(), &root, &[]);
        Self { server, cloud, _dir: dir, root, engine, log: String::new(), requests_seen: 0, activity_seen: 0 }
    }

    /// A local file, written long ago.
    fn local(&self, path: &str, bytes: &str) -> PathBuf {
        let file = self.root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, bytes).unwrap();
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(T0)).unwrap();
        file
    }

    /// The local file at `path` as last synced at `etag`.
    fn known(&self, path: &str, etag: &str) -> Known {
        let meta = local_meta(&self.root.join(path)).unwrap();
        Known { etag: format!("\"{etag}\""), size: meta.size, mtime: meta.mtime, hash: None }
    }

    /// A file synced at `etag` that is no longer here.
    fn gone(size: u64, etag: &str) -> Known {
        Known { etag: format!("\"{etag}\""), size, mtime: i64::try_from(T0 * 1000).unwrap(), hash: None }
    }

    fn save(&self, state: State) {
        save_saved(&self.engine.state_path(), &Saved { server: self.engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    }

    fn cycle(&mut self, deletes: Deletes) {
        let _ = writeln!(
            self.log,
            "== cycle ({})",
            match deletes {
                Deletes::Hold => "hold",
                Deletes::Confirm => "confirm deletes",
                Deletes::Discard => "discard deletes",
            }
        );
        self.engine.cycle(deletes);
        self.record();
    }

    /// Logs the requests and activity lines since the last call, and where the sync stands.
    fn record(&mut self) {
        let seen = self.server.seen();
        let lines: Vec<String> = seen[self.requests_seen..].iter().map(|r| request_line(r, &self.server.url)).collect();
        self.requests_seen = seen.len();
        for line in sorted_listings(lines) {
            let _ = writeln!(self.log, "{line}");
        }
        let progress = self.engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
        for e in &progress.activity[self.activity_seen..] {
            let _ = writeln!(self.log, "activity: {:?} {:?} {:?} {}", e.kind, e.path, e.size, mask(&e.detail));
        }
        self.activity_seen = progress.activity.len();
        let state = match &progress.state {
            SyncState::Failed(e) => format!("Failed({})", mask(e)),
            other => format!("{other:?}"),
        };
        let _ = writeln!(self.log, "state: {state}");
        if !progress.pending_deletes.is_empty() {
            let _ = writeln!(self.log, "pending deletes: {:?}", progress.pending_deletes);
        }
        if !progress.no_room.is_empty() {
            let _ = writeln!(self.log, "no room: {:?}", progress.no_room);
        }
        for c in &progress.conflicts {
            let _ = writeln!(self.log, "conflict: {} / {}", mask(&c.original), mask(&c.copy));
        }
        let saved = synced_state(&self.root);
        for (path, k) in &saved.files {
            let _ = writeln!(self.log, "saved: {} {} {}", mask(path), k.etag, k.size);
        }
        let _ = writeln!(self.log, "saved folders: {:?}", saved.folders);
    }

    /// Ends the scenario: the local folder and the cloud as they are now, then the comparison.
    fn finish(mut self, name: &str) {
        let _ = writeln!(self.log, "== end");
        let mut local = Vec::new();
        walk(&self.root, &self.root, &mut local);
        for (path, bytes) in local {
            let _ = writeln!(self.log, "local: {} {bytes:?}", mask(&path));
        }
        let cloud = self.cloud.lock().unwrap_or_else(PoisonError::into_inner);
        for (path, (etag, bytes)) in &cloud.files {
            let _ = writeln!(self.log, "cloud: {} {etag} {bytes:?}", mask(path));
        }
        let _ = writeln!(self.log, "cloud folders: {:?}", cloud.folders.iter().map(|f| mask(f)).collect::<Vec<_>>());
        for t in &cloud.trash {
            let _ = writeln!(self.log, "cloud trash: {} {}", t.id, t.path);
        }
        drop(cloud);
        check_guards(name, &self.log);
        compare(name, &self.log);
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let path = e.path();
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if rel == ".craftcloud" {
            continue;
        }
        if path.is_dir() {
            out.push((format!("{rel}/"), String::new()));
            walk(root, &path, out);
        } else {
            out.push((rel, std::fs::read_to_string(&path).unwrap_or_default()));
        }
    }
}

/// One request as a log line: method, path, the headers in [`HEADERS`], and the body (a hash of
/// it when long).
fn request_line(r: &Seen, server: &str) -> String {
    let mut line = format!("{} {}", r.method, r.path);
    for name in HEADERS {
        if let Some(value) = r.header(name) {
            let _ = write!(line, " | {name}: {}", value.replace(server, "http://SERVER"));
        }
    }
    if !r.body.is_empty() {
        if r.body.len() > 64 {
            use sha2::{Digest, Sha256};
            let digest: String = Sha256::digest(r.body.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
            let _ = write!(line, " | body: {} bytes, sha256 {digest}", r.body.len());
        } else {
            let _ = write!(line, " | body: {:?}", r.body);
        }
    }
    mask(&line)
}

/// Folders listed at the same time are answered in any order: each run of folder listings
/// (`PROPFIND` depth 1, outside `.craftcloud/`) is sorted. Anything else keeps its place.
fn sorted_listings(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut run: Vec<String> = Vec::new();
    for line in lines {
        if line.starts_with("PROPFIND ") && line.contains("| depth: 1") && !line.starts_with("PROPFIND /webdav/.craftcloud/") {
            run.push(line);
            continue;
        }
        run.sort();
        out.append(&mut run);
        out.push(line);
    }
    run.sort();
    out.append(&mut run);
    out
}

/// Masks the times that change from run to run: a version's name (`20261117T101500.123Z`,
/// `20261116T100000Z`) and a conflict copy's (`2026-11-17 10.15`, in a URL `2026-11-17%2010.15`).
fn mask(text: &str) -> String {
    const PATTERNS: [(&str, &str); 4] =
        [("DDDDDDDDTDDDDDD.DDDZ", "<STAMP>"), ("DDDDDDDDTDDDDDDZ", "<STAMP>"), ("DDDD-DD-DD%20DD.DD", "<WHEN>"), ("DDDD-DD-DD DD.DD", "<WHEN>")];
    let matches = |at: &[u8], pattern: &str| {
        pattern.len() <= at.len() && pattern.bytes().zip(at).all(|(p, c)| if p == b'D' { c.is_ascii_digit() } else { p == *c })
    };
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if let Some((pattern, with)) = PATTERNS.iter().find(|(p, _)| matches(&b[i..], p)) {
            out.push_str(with);
            i += pattern.len();
        } else {
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Every write the engine sends carries its guard, today and after any refactor: a `PUT` an
/// `If-Match` or `If-None-Match: *`, a `MOVE` or `COPY` `Overwrite: F`, and a WebDAV `DELETE`
/// (which is for good) only inside `.craftcloud/`.
fn check_guards(name: &str, log: &str) {
    for line in log.lines() {
        let guarded = if line.starts_with("PUT /webdav/") {
            line.contains("| if-match: ") || line.contains("| if-none-match: *")
        } else if line.starts_with("MOVE /webdav/") || line.starts_with("COPY /webdav/") {
            line.contains("| overwrite: F")
        } else if line.starts_with("DELETE /webdav/") {
            line.starts_with("DELETE /webdav/.craftcloud/")
        } else {
            true
        };
        assert!(guarded, "{name}: an unguarded write: {line}");
    }
}

fn compare(name: &str, log: &str) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sync/golden").join(format!("{name}.log"));
    if std::env::var("CRAFTCLOUD_BLESS").as_deref() == Ok("1") {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, log).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e} (CRAFTCLOUD_BLESS=1 writes it)", file.display()));
    if want != log {
        let first = want.lines().zip(log.lines()).position(|(a, b)| a != b).unwrap_or_else(|| want.lines().count().min(log.lines().count()));
        panic!(
            "{name}: the engine no longer does what {} recorded; first difference at line {}:\n  was: {}\n  now: {}\n\nall of it now:\n{log}",
            file.display(),
            first + 1,
            want.lines().nth(first).unwrap_or("(end)"),
            log.lines().nth(first).unwrap_or("(end)"),
        );
    }
}

// --- The scenarios -----------------------------------------------------------------------------

#[test]
fn golden_nothing_to_do() {
    let mut s = Scenario::new(Cloud::new(&[("Docs/a.txt", "e1", "abc"), ("top.txt", "e2", "top")], &[]), None);
    s.local("Docs/a.txt", "abc");
    s.local("top.txt", "top");
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), s.known("Docs/a.txt", "e1"));
    state.files.insert("top.txt".into(), s.known("top.txt", "e2"));
    state.folders.insert("Docs".into());
    s.save(state);
    s.cycle(Deletes::Hold);
    s.finish("nothing_to_do");
}

#[test]
fn golden_first_sync_both_ways() {
    // Downloads (one answered without an ETag: the listing gives it), a cloud folder made here,
    // a local folder made in the cloud, new files uploaded; then a quiet cycle.
    let special: Special =
        Box::new(|r, cloud| (r.method == "GET" && r.path == "/webdav/top.txt").then(|| Answer::new(200, cloud.files["top.txt"].1.clone())));
    let mut s = Scenario::new(
        Cloud::new(&[("top.txt", "e1", "top"), ("Docs/a.txt", "e2", "abc"), ("Docs/Sub/b.txt", "e3", "bee")], &["Empty"]),
        Some(special),
    );
    s.local("mine.txt", "mine");
    s.local("Local/Deep/c.txt", "sea");
    s.cycle(Deletes::Hold);
    s.cycle(Deletes::Hold);
    s.finish("first_sync_both_ways");
}

#[test]
fn golden_a_changed_file_keeps_the_previous_version() {
    // The cloud already keeps six versions: the oldest goes once the new one is kept.
    let versions: Vec<(String, String, String)> =
        (1..=6).map(|n| (format!(".craftcloud/versions/Docs/a.txt/2026111{n}T100000.000Z"), format!("v{n}"), "old".to_owned())).collect();
    let mut files: Vec<(&str, &str, &str)> = vec![("Docs/a.txt", "e1", "abc")];
    files.extend(versions.iter().map(|(p, e, b)| (p.as_str(), e.as_str(), b.as_str())));
    let mut s = Scenario::new(Cloud::new(&files, &[]), None);
    s.local("Docs/a.txt", "abc");
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), s.known("Docs/a.txt", "e1"));
    state.folders.insert("Docs".into());
    s.save(state);
    s.local("Docs/a.txt", "abcd, changed here");
    s.cycle(Deletes::Hold);
    s.finish("a_changed_file_keeps_the_previous_version");
}

#[test]
fn golden_an_overwrite_refused_with_412_becomes_a_conflict_copy() {
    // The listing still shows the version synced, but the cloud has a newer one by the time the
    // upload goes (412): the cloud's comes down, and the local edit goes up as a conflict copy.
    let special: Special = Box::new(|r, cloud| {
        if r.method == "PUT" && r.path == "/webdav/Docs/a.txt" {
            cloud.files.insert("Docs/a.txt".into(), ("e9".into(), "theirs".into()));
            return Some(Answer::new(412, ""));
        }
        None
    });
    let mut s = Scenario::new(Cloud::new(&[("Docs/a.txt", "e1", "abc")], &[]), Some(special));
    s.local("Docs/a.txt", "abc");
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), s.known("Docs/a.txt", "e1"));
    state.folders.insert("Docs".into());
    s.save(state);
    s.local("Docs/a.txt", "mine, changed");
    s.cycle(Deletes::Hold);
    s.finish("an_overwrite_refused_with_412_becomes_a_conflict_copy");
}

#[test]
fn golden_changed_on_both_sides_is_a_conflict_copy() {
    let mut s = Scenario::new(Cloud::new(&[("a.txt", "e2", "theirs, changed")], &[]), None);
    s.local("a.txt", "abc");
    let mut state = State::default();
    state.files.insert("a.txt".into(), s.known("a.txt", "e1"));
    s.save(state);
    s.local("a.txt", "mine, changed");
    s.cycle(Deletes::Hold);
    s.finish("changed_on_both_sides_is_a_conflict_copy");
}

#[test]
fn golden_an_interrupted_download_resumes() {
    let mut s = Scenario::new(Cloud::new(&[("Docs/big.bin", "e1", "0123456789")], &[]), None);
    let part = s.root.join("Docs").join(part_name("big.bin", Some("\"e1\"")));
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, "0123").unwrap();
    s.cycle(Deletes::Hold);
    s.finish("an_interrupted_download_resumes");
}

#[test]
fn golden_no_room_waits_and_an_admin_asks_for_a_recount() {
    let special: Special = Box::new(|r, _| (r.method == "PUT").then(|| Answer::new(507, r#"{"message":"Insufficient storage"}"#)));
    let mut cloud = Cloud::new(&[], &[]);
    cloud.me = ("owner", 100, 101);
    let mut s = Scenario::new(cloud, Some(special));
    s.local("big.txt", "too big for the room left");
    s.cycle(Deletes::Hold);
    s.cycle(Deletes::Hold);
    s.finish("no_room_waits_and_an_admin_asks_for_a_recount");
}

#[test]
fn golden_a_refused_upload_waits_until_the_file_changes() {
    let special: Special = Box::new(|r, _| (r.method == "PUT").then(|| Answer::new(400, r#"{"message":"Invalid file name."}"#)));
    let mut s = Scenario::new(Cloud::new(&[], &[]), Some(special));
    s.local("odd.txt", "odd");
    s.cycle(Deletes::Hold);
    s.cycle(Deletes::Hold);
    s.finish("a_refused_upload_waits_until_the_file_changes");
}

#[test]
fn golden_deletes_go_to_the_cloud_trash() {
    // Two files of one folder (the second finds the folder's id known), a whole folder, and an
    // empty folder, all deleted here.
    let mut s = Scenario::new(
        Cloud::new(
            &[
                ("Docs/a.txt", "e1", "a"),
                ("Docs/b.txt", "e2", "b"),
                ("Docs/keep1.txt", "k1", "1"),
                ("Docs/keep2.txt", "k2", "2"),
                ("Docs/keep3.txt", "k3", "3"),
                ("Docs/keep4.txt", "k4", "4"),
                ("Docs/keep5.txt", "k5", "5"),
                ("Old/x.txt", "e3", "x"),
                ("Old/Sub/y.txt", "e4", "y"),
            ],
            &["Empty"],
        ),
        None,
    );
    let mut state = State::default();
    for n in 1..=5 {
        let path = format!("Docs/keep{n}.txt");
        s.local(&path, &n.to_string());
        state.files.insert(path.clone(), s.known(&path, &format!("k{n}")));
    }
    state.files.insert("Docs/a.txt".into(), Scenario::gone(1, "e1"));
    state.files.insert("Docs/b.txt".into(), Scenario::gone(1, "e2"));
    state.files.insert("Old/x.txt".into(), Scenario::gone(1, "e3"));
    state.files.insert("Old/Sub/y.txt".into(), Scenario::gone(1, "e4"));
    state.folders.extend(["Docs", "Old", "Old/Sub", "Empty"].map(str::to_owned));
    s.save(state);
    s.cycle(Deletes::Hold);
    s.finish("deletes_go_to_the_cloud_trash");
}

#[test]
fn golden_a_mass_delete_waits_and_discarding_brings_the_files_back() {
    // Seven of eight files deleted in the cloud (to its trash): held, then put back from the trash.
    let mut cloud = Cloud::new(&[("Docs/keep.txt", "k1", "k")], &[]);
    for (n, f) in ["a", "b", "c", "d", "e", "f", "g"].iter().enumerate() {
        cloud.trash.push(Trashed {
            id: format!("t{}", n + 1),
            path: format!("Docs/{f}.txt"),
            folder: false,
            files: vec![(format!("Docs/{f}.txt"), (format!("e{}", n + 1), (*f).to_owned()))],
            folders: Vec::new(),
        });
    }
    let mut s = Scenario::new(cloud, None);
    let mut state = State::default();
    for (n, f) in ["a", "b", "c", "d", "e", "f", "g"].iter().enumerate() {
        let path = format!("Docs/{f}.txt");
        s.local(&path, f);
        state.files.insert(path.clone(), s.known(&path, &format!("e{}", n + 1)));
    }
    s.local("Docs/keep.txt", "k");
    state.files.insert("Docs/keep.txt".into(), s.known("Docs/keep.txt", "k1"));
    state.folders.insert("Docs".into());
    s.save(state);
    s.cycle(Deletes::Hold);
    s.cycle(Deletes::Discard);
    s.finish("a_mass_delete_waits_and_discarding_brings_the_files_back");
}

#[test]
fn golden_moves_and_renames() {
    // A file renamed here, a folder moved here into a folder the cloud doesn't have yet (its
    // versions go along), and a file renamed in the cloud.
    let mut s = Scenario::new(
        Cloud::new(
            &[
                ("Docs/a.txt", "e1", "abc"),
                ("Old/x.txt", "e2", "x"),
                ("Pics/renamed there.png", "e3", "png"),
                ("keep1.txt", "k1", "1"),
                ("keep2.txt", "k2", "2"),
                ("keep3.txt", "k3", "3"),
                (".craftcloud/versions/Docs/a.txt/20261110T100000.000Z", "v1", "ab"),
            ],
            &[],
        ),
        None,
    );
    let mut state = State::default();
    for n in 1..=3 {
        let path = format!("keep{n}.txt");
        s.local(&path, &n.to_string());
        state.files.insert(path.clone(), s.known(&path, &format!("k{n}")));
    }
    s.local("Docs/b.txt", "abc");
    state.files.insert("Docs/a.txt".into(), s.known("Docs/b.txt", "e1"));
    s.local("New/Inner/x.txt", "x");
    state.files.insert("Old/x.txt".into(), s.known("New/Inner/x.txt", "e2"));
    s.local("Pics/photo.png", "png");
    state.files.insert("Pics/photo.png".into(), s.known("Pics/photo.png", "e3"));
    state.folders.extend(["Docs", "Old", "Pics"].map(str::to_owned));
    s.save(state);
    s.cycle(Deletes::Hold);
    s.finish("moves_and_renames");
}

#[test]
fn golden_a_failed_listing_ends_the_cycle_with_nothing_done() {
    let special: Special = Box::new(|r, _| (r.method == "PROPFIND" && r.path.starts_with("/webdav/Docs/")).then(|| Answer::new(500, "{}")));
    let mut s = Scenario::new(Cloud::new(&[("Docs/a.txt", "e1", "abc"), ("Other/b.txt", "e2", "b")], &[]), Some(special));
    s.local("Docs/a.txt", "abc");
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), s.known("Docs/a.txt", "e1"));
    state.folders.insert("Docs".into());
    s.save(state);
    std::fs::remove_file(s.root.join("Docs/a.txt")).unwrap();
    s.local("new.txt", "new");
    s.cycle(Deletes::Hold);
    s.finish("a_failed_listing_ends_the_cycle_with_nothing_done");
}

#[test]
fn golden_an_expired_token_is_refreshed_mid_cycle() {
    // The first listing goes through; every request after it needs the refreshed token.
    let special: Special = Box::new(|r, _| {
        if r.is("POST", "/api/auth/refresh") {
            return Some(Answer::new(200, r#"{"access_token":"a2","refresh_token":"r2"}"#));
        }
        (r.bearer() == Some("a1") && r.path != "/webdav/").then(|| Answer::new(401, "{}"))
    });
    let mut s = Scenario::new(Cloud::new(&[("Docs/a.txt", "e1", "abc"), ("More/b.txt", "e2", "b")], &[]), Some(special));
    s.local("up.txt", "up");
    s.cycle(Deletes::Hold);
    s.finish("an_expired_token_is_refreshed_mid_cycle");
}

#[test]
fn golden_a_refused_refresh_signs_out() {
    let special: Special = Box::new(|r, _| {
        if r.is("POST", "/api/auth/refresh") {
            return Some(Answer::new(404, r#"{"error":"Session not found"}"#));
        }
        Some(Answer::new(401, "{}"))
    });
    let mut s = Scenario::new(Cloud::new(&[], &[]), Some(special));
    s.cycle(Deletes::Hold);
    s.finish("a_refused_refresh_signs_out");
}

#[test]
fn golden_guards_against_changes_in_the_cloud() {
    // Straight to the actions, with a cloud that changed after the plan: a delete of a file that
    // changed, of one listed without an ETag, a rename of a file that changed (uploaded under the
    // new name instead), a folder delete of a folder that gained a file, and an empty folder that
    // isn't empty any more.
    use craftcloud_core::reconcile::{Action, Tree};
    let special: Special = Box::new(|r, _| {
        (r.method == "PROPFIND" && r.path == "/webdav/Docs/").then(|| {
            Answer::new(
                207,
                r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/webdav/Docs/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/Docs/plain.txt</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>3</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/Docs/a.txt</D:href><D:propstat><D:prop><D:resourcetype/><D:getetag>&quot;e2&quot;</D:getetag><D:getcontentlength>3</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
            )
        })
    });
    let mut s = Scenario::new(
        Cloud::new(
            &[
                ("Docs/a.txt", "e2", "new"),
                ("Docs/plain.txt", "e5", "abc"),
                ("Old/x.txt", "e3", "x"),
                ("Old/added.txt", "e4", "added"),
                ("Full/late.txt", "e6", "late"),
            ],
            &[],
        ),
        Some(special),
    );
    s.local("Docs/b.txt", "abc");
    let root = s.engine.root();
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), Known { etag: "\"e1\"".into(), size: 3, mtime: 1, hash: None });
    state.files.insert("Docs/plain.txt".into(), Known { etag: "\"e5\"".into(), size: 3, mtime: 1, hash: None });
    let mut folders: BTreeSet<String> = ["Docs", "Old", "Full"].map(str::to_owned).into();
    let actions = [
        Action::DeleteRemote { path: "Docs/a.txt".into(), etag: "\"e1\"".into() },
        Action::DeleteRemote { path: "Docs/plain.txt".into(), etag: "\"e5\"".into() },
        Action::DeleteRemote { path: "Docs/plain.txt".into(), etag: "\"e5\"".into() },
        Action::MoveRemote { from: "Docs/a.txt".into(), to: "Docs/b.txt".into(), etag: "\"e1\"".into() },
        Action::DeleteRemoteFolder { path: "Old".into(), files: vec![("Old/x.txt".into(), "\"e3\"".into())] },
        Action::RemoveRemoteFolder("Full".into()),
    ];
    for action in &actions {
        let done = s.engine.apply(&root, action, &mut state, &mut folders, &Tree::default());
        let _ = writeln!(s.log, "== apply {action:?}");
        let done = match done {
            Ok(Some((kind, detail))) => format!("Ok(Some({kind:?}, {}))", mask(&detail)),
            other => format!("{other:?}"),
        };
        s.record();
        let _ = writeln!(s.log, "done: {done}");
        let _ = writeln!(s.log, "state files: {:?}", state.files.keys().collect::<Vec<_>>());
        let _ = writeln!(s.log, "folders: {folders:?}");
    }
    s.finish("guards_against_changes_in_the_cloud");
}
