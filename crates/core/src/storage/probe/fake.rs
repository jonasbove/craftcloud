//! A small scripted WebDAV server in memory, for tests of the probe: well behaved by default, with
//! switches for each way a real server has been seen to misbehave (docs/WEBDAV-STORAGE.md § 5).
//! Pure: it answers one request at a time from its state. `crates/core`'s tests drive it directly;
//! `crates/native`'s tests put it behind a loopback HTTP server (`fake_server`).

use std::collections::BTreeMap;

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};

use crate::http::HttpResponse;

/// How the server misbehaves. Everything `false` is a server that does what the sync relies on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// `PUT` with `If-None-Match: *` over an existing file overwrites it (OpenCloud, reva#866).
    pub ignores_if_none_match: bool,
    /// `PUT` with a stale `If-Match` overwrites (rclone's `serve webdav`).
    pub ignores_if_match: bool,
    /// `MOVE` with `Overwrite: F` onto an existing name replaces it.
    pub move_overwrites: bool,
    /// A file's ETag never changes.
    pub fixed_etags: bool,
    /// Files are listed without ETags, and `PUT` answers none.
    pub no_file_etags: bool,
    /// `COPY` answers 405.
    pub no_copy: bool,
    /// `MOVE` answers 405.
    pub no_move: bool,
    /// `DELETE` with a stale `If-Match` deletes anyway.
    pub delete_ignores_if_match: bool,
    /// `PROPFIND` with `Depth: infinity` answers 207 (else 403).
    pub depth_infinity: bool,
    /// `PUT` into a missing folder answers 404 (else 409).
    pub missing_parent_404: bool,
    /// Folder ETags never change.
    pub folder_etags_stay: bool,
    /// Names that differ only in case are one file.
    pub case_insensitive: bool,
    /// No `oc:fileid` and no `oc:checksums` in listings.
    pub no_ids: bool,
    /// A `MOVE` gives the file a new ETag.
    pub move_changes_etag: bool,
    /// A create (`If-None-Match: *`) over an existing file answers 201 with a fresh ETag but keeps
    /// the file already there: a server that passed the probe yet, at runtime, silently drops a
    /// guarded create and leaves someone else's file (the runtime watch, docs/WEBDAV-STORAGE.md
    /// § 7.5).
    pub create_keeps_other: bool,
    /// A replace with a stale `If-Match` answers 204 with a fresh ETag but keeps the newer file
    /// already there: the guard is honoured at probe time yet ignored at runtime (the runtime watch).
    pub replace_keeps_other: bool,
    /// The server's clock, as an RFC 1123 time: the `Date` header on every answer, and the
    /// `getlastmodified` of every folder made from now on.
    pub date: Option<String>,
    /// `quota-available-bytes` listed on the base (none without it; Nextcloud says `-3`).
    pub quota_available: Option<i64>,
    /// A file overwritten within the same second as the write before it keeps its ETag (Nextcloud
    /// 31 and ownCloud 10.15: the tag is kept while the modified time, in whole seconds, is
    /// unchanged; docs/WEBDAV-STORAGE.md § 5 and § 16, Q13). Measured on this computer's clock.
    pub etag_reused_within_second: bool,
    /// `oc:checksums` lists only what the client sent in `OC-Checksum` on its last `PUT` (as
    /// Nextcloud does), nothing for a file written without one; off, every file lists a made-up
    /// SHA1 and MD5 (unless `no_ids`).
    pub checksums_from_client_only: bool,
    /// Right after every `PUT`, another client's write lands on the same file with these bytes
    /// and no checksum (within the same second, so with `etag_reused_within_second` the tag the
    /// `PUT` answered stays): the write the checksum check must catch (Q13).
    pub other_writes_after_put: Option<Vec<u8>>,
    /// Answers like Nextcloud besides its files (docs/WEBDAV-STORAGE.md § 7.4, the Nextcloud
    /// column): `status.php`, the OCS user endpoint, the Login Flow v2, `current-user-principal`
    /// on `/remote.php/dav/`, a trash bin at `/remote.php/dav/trashbin/<user>/` that every `DELETE`
    /// lands in (restored by a `MOVE` to `restore/<name>`), versions at
    /// `/remote.php/dav/versions/<user>/` kept on every overwrite (the current file listed among
    /// them), `oc:checksums` only as the client sent them, and 404 for a `PUT` into a missing
    /// folder. The user is the base path's last segment.
    pub nextcloud: bool,
    /// What `status.php` says in `version` (`31.0.14.1` when unset).
    pub nextcloud_version: Option<String>,
    /// The Login Flow v2 finishes on this poll (1 = the first); 0: never.
    pub login_flow_ready_after: usize,
    /// Answers like ownCloud 10 besides its files (docs/WEBDAV-STORAGE.md § 7.4, the ownCloud
    /// column): `status.php` (`productname` "ownCloud", version `10.15.3.0`),
    /// `current-user-principal` on `/remote.php/dav/`, a trash bin at
    /// `/remote.php/dav/trash-bin/<user>/` whose items are named by file id and that every `DELETE`
    /// lands in (restored by a `MOVE` to any path under the files base, `Overwrite: F` honoured; a
    /// `GET` there answers 403), versions at `/remote.php/dav/meta/<file id>/v/` kept on every
    /// overwrite (the current file **not** among them; restored by a `COPY` to the file), checksums
    /// listed for every file (a made-up SHA1 stands for the server's own when the client sent
    /// none), 412 for `Depth: infinity` and 409 for a `PUT` into a missing folder. The user is the
    /// base path's last segment. Not with `nextcloud`.
    pub owncloud: bool,
}

#[derive(Clone, Debug)]
struct File {
    /// As written when made (a case-insensitive server keeps the case it was given).
    path: String,
    etag: String,
    body: Vec<u8>,
    id: u64,
    /// What the client's last `PUT` said in `OC-Checksum` (`SHA1:…`), if anything.
    checksum: Option<String>,
    /// When it was last written, for `etag_reused_within_second`.
    written_at: Option<std::time::Instant>,
    /// The server's clock when it was last written (the name of its version, `Flags::nextcloud`).
    written_secs: i64,
}

/// An item in the Nextcloud-shaped (or ownCloud-shaped) trash bin.
#[derive(Clone, Debug)]
struct TrashedItem {
    /// `<name>.d<seconds>` (Nextcloud), or the file id it had (ownCloud).
    name: String,
    /// Where it was.
    original: String,
    is_folder: bool,
    files: Vec<File>,
    folders: Vec<Folder>,
    deleted_at: i64,
}

#[derive(Clone, Debug)]
struct Folder {
    path: String,
    etag: String,
    modified: Option<String>,
}

/// One request the server answered: the method, the cloud path, and the guard headers it carried
/// (`if-match`, `if-none-match`, `overwrite`, `depth`, `destination`).
pub type LoggedRequest = (String, String, Vec<(String, String)>);

/// The server.
#[derive(Clone, Debug)]
pub struct FakeDav {
    /// The base's URL path, e.g. `/webdav` or `/remote.php/dav/files/john`.
    pub base_path: String,
    pub flags: Flags,
    files: BTreeMap<String, File>,
    folders: BTreeMap<String, Folder>,
    next: u64,
    /// Every request answered.
    pub log: Vec<LoggedRequest>,
    /// The server's address (`http://127.0.0.1:1234`), for the absolute URLs the Login Flow
    /// answers; empty until a test sets it.
    pub origin: String,
    /// The server's clock in seconds, moved on by every write (so versions have distinct names).
    clock: i64,
    /// The Nextcloud-shaped trash bin.
    trash: Vec<TrashedItem>,
    /// Earlier versions by file id, oldest first: (when written, the bytes).
    versions: BTreeMap<u64, Vec<(i64, Vec<u8>)>>,
    /// Polls of the Login Flow so far.
    polls: usize,
    /// Whether the app password was deleted (`DELETE /ocs/v2.php/core/apppassword`).
    pub app_password_deleted: bool,
    /// Items purged from the trash bin by a `DELETE` there (never by CraftCloud, § 16, Q8).
    pub trash_purged: usize,
}

const SEGMENT: &AsciiSet =
    &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}').add(b'/').add(b'\\');

impl FakeDav {
    pub fn new(base_path: &str, flags: Flags) -> Self {
        let mut folders = BTreeMap::new();
        folders.insert(String::new(), Folder { path: String::new(), etag: "\"f0\"".into(), modified: None });
        Self {
            base_path: base_path.trim_end_matches('/').to_owned(),
            flags,
            files: BTreeMap::new(),
            folders,
            next: 1,
            log: Vec::new(),
            origin: String::new(),
            clock: 1_795_000_000,
            trash: Vec::new(),
            versions: BTreeMap::new(),
            polls: 0,
            app_password_deleted: false,
            trash_purged: 0,
        }
    }

    /// The names in the Nextcloud-shaped trash bin (`<name>.d<seconds>`; ownCloud: the file id),
    /// with where each was.
    pub fn trashed(&self) -> Vec<(String, String)> {
        self.trash.iter().map(|t| (t.name.clone(), t.original.clone())).collect()
    }

    /// Whether this is a server with its own trash and versions (`Flags::nextcloud` or
    /// `Flags::owncloud`).
    fn known_server(&self) -> bool {
        self.flags.nextcloud || self.flags.owncloud
    }

    /// How many earlier versions the Nextcloud- or ownCloud-shaped server keeps of the file at `path`.
    pub fn versions_of(&self, path: &str) -> usize {
        self.files.get(&self.key(path)).and_then(|f| self.versions.get(&f.id)).map_or(0, Vec::len)
    }

    /// Adds a folder (and its parents) with this `getlastmodified`.
    pub fn add_folder(&mut self, path: &str, modified: Option<&str>) {
        let mut so_far = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            so_far = if so_far.is_empty() { part.to_owned() } else { format!("{so_far}/{part}") };
            let etag = self.fresh_etag("f");
            self.folders.entry(self.key(&so_far)).or_insert(Folder { path: so_far.clone(), etag, modified: None });
        }
        if let Some(f) = self.folders.get_mut(&self.key(path)) {
            f.modified = modified.map(str::to_owned);
        }
    }

    pub fn add_file(&mut self, path: &str, body: &[u8]) {
        let etag = self.fresh_etag("e");
        let id = self.next;
        self.next += 1;
        let written_secs = self.tick();
        self.files.insert(
            self.key(path),
            File { path: path.trim_matches('/').to_owned(), etag, body: body.to_vec(), id, checksum: None, written_at: None, written_secs },
        );
    }

    /// The server's clock, one second on.
    fn tick(&mut self) -> i64 {
        self.clock += 1;
        self.clock
    }

    /// The user the base path names (`/remote.php/dav/files/john` → `john`).
    fn user(&self) -> &str {
        self.base_path.rsplit('/').next().unwrap_or("")
    }

    /// The checksum a file's last `PUT` carried, if any.
    pub fn checksum(&self, path: &str) -> Option<&str> {
        self.files.get(&self.key(path)).and_then(|f| f.checksum.as_deref())
    }

    pub fn has(&self, path: &str) -> bool {
        let key = self.key(path);
        self.files.contains_key(&key) || self.folders.contains_key(&key)
    }

    pub fn body(&self, path: &str) -> Option<&[u8]> {
        self.files.get(&self.key(path)).map(|f| f.body.as_slice())
    }

    /// Every folder path, sorted.
    pub fn folders(&self) -> Vec<String> {
        self.folders.keys().filter(|k| !k.is_empty()).cloned().collect()
    }

    fn key(&self, path: &str) -> String {
        let clean = path.trim_matches('/');
        if self.flags.case_insensitive { clean.to_lowercase() } else { clean.to_owned() }
    }

    fn fresh_etag(&mut self, prefix: &str) -> String {
        let n = self.next;
        self.next += 1;
        format!("\"{prefix}{n}\"")
    }

    /// The cloud path a request URL path names, or `None` outside the base.
    fn relative(&self, url_path: &str) -> Option<String> {
        let path = url_path.find("://").and_then(|i| url_path[i + 3..].find('/').map(|j| &url_path[i + 3 + j..])).unwrap_or(url_path);
        let path = path.split('?').next().unwrap_or(path);
        let decoded = percent_decode_str(path).decode_utf8().ok()?;
        let rest = decoded.strip_prefix(self.base_path.as_str())?;
        if !rest.is_empty() && !rest.starts_with('/') {
            return None;
        }
        Some(rest.trim_matches('/').to_owned())
    }

    fn href(&self, path: &str, folder: bool) -> String {
        let encoded: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(|s| utf8_percent_encode(s, SEGMENT).to_string()).collect();
        let mut out = format!("{}/{}", self.base_path, encoded.join("/"));
        if folder && !encoded.is_empty() {
            out.push('/');
        }
        out
    }

    fn parent_exists(&self, path: &str) -> bool {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        self.folders.contains_key(&self.key(parent))
    }

    /// Bumps the ETag of every folder on the way to `path` (a change in it).
    fn touch_parents(&mut self, path: &str) {
        if self.flags.folder_etags_stay {
            return;
        }
        let mut parent = path.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
        loop {
            let etag = self.fresh_etag("f");
            if let Some(f) = self.folders.get_mut(&self.key(&parent)) {
                f.etag = etag;
            }
            if parent.is_empty() {
                return;
            }
            parent = parent.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
        }
    }

    fn done(&self, status: u16, body: impl Into<Vec<u8>>) -> HttpResponse {
        let mut resp = HttpResponse::new(status, body);
        if let Some(date) = &self.flags.date {
            resp = resp.with_header("Date", date);
        }
        resp
    }

    /// Answers one request. `url_path` is the request's URL or its path (percent-encoded);
    /// header names in any case.
    pub fn answer(&mut self, method: &str, url_path: &str, headers: &[(String, String)], body: &[u8]) -> HttpResponse {
        let header = |name: &str| headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_owned());
        if self.known_server()
            && let Some(resp) = self.beside_the_files(method, url_path, header("Destination").as_deref(), header("Overwrite").as_deref(), body)
        {
            return resp;
        }
        let Some(path) = self.relative(url_path) else { return self.done(404, "outside the base") };
        let guards: Vec<(String, String)> = ["if-match", "if-none-match", "overwrite", "depth", "destination", "oc-checksum"]
            .iter()
            .filter_map(|n| header(n).map(|v| ((*n).to_owned(), v)))
            .collect();
        self.log.push((method.to_owned(), path.clone(), guards));
        match method {
            "PROPFIND" => self.propfind(&path, header("Depth").as_deref().unwrap_or("infinity")),
            "MKCOL" => self.mkcol(&path),
            "PUT" => {
                let resp = self.put(&path, header("If-None-Match"), header("If-Match"), header("OC-Checksum"), body);
                if resp.is_success()
                    && let Some(theirs) = self.flags.other_writes_after_put.clone()
                {
                    // Someone else's write, right behind ours, without a checksum.
                    let key = self.key(&path);
                    let reused = self.flags.etag_reused_within_second;
                    let fresh = self.fresh_etag("e");
                    if let Some(f) = self.files.get_mut(&key) {
                        f.body = theirs;
                        f.checksum = None;
                        if !reused {
                            f.etag = fresh;
                        }
                    }
                }
                resp
            }
            "GET" => self.get(&path),
            "MOVE" | "COPY" => self.relocate(method == "MOVE", &path, header("Destination").as_deref(), header("Overwrite").as_deref()),
            "DELETE" => self.delete(&path, header("If-Match")),
            _ => self.done(405, ""),
        }
    }

    fn propfind(&self, path: &str, depth: &str) -> HttpResponse {
        let key = self.key(path);
        let is_folder = self.folders.contains_key(&key);
        if !is_folder && !self.files.contains_key(&key) {
            return self.done(404, "");
        }
        if depth == "infinity" && !self.flags.depth_infinity {
            // ownCloud 10 answers 412 ("Depth infinity not supported"), most others 403.
            return self.done(if self.flags.owncloud { 412 } else { 403 }, "");
        }
        let mut xml = String::from(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">"#);
        xml.push_str(&self.entry(&key));
        if is_folder && depth != "0" {
            let prefix = if key.is_empty() { String::new() } else { format!("{key}/") };
            let direct = |p: &str| depth == "infinity" || !p[prefix.len()..].contains('/');
            for p in self.folders.keys().chain(self.files.keys()) {
                if p != &key && p.starts_with(&prefix) && direct(p) {
                    xml.push_str(&self.entry(p));
                }
            }
        }
        xml.push_str("</d:multistatus>");
        self.done(207, xml)
    }

    fn entry(&self, key: &str) -> String {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
        if let Some(folder) = self.folders.get(key) {
            let modified = folder.modified.as_deref().map(|m| format!("<d:getlastmodified>{}</d:getlastmodified>", esc(m))).unwrap_or_default();
            let quota = match self.flags.quota_available {
                Some(q) if key.is_empty() => format!("<d:quota-available-bytes>{q}</d:quota-available-bytes>"),
                _ => String::new(),
            };
            return format!(
                "<d:response><d:href>{}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getetag>{}</d:getetag>{modified}{quota}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
                self.href(&folder.path, true),
                esc(&folder.etag)
            );
        }
        let Some(file) = self.files.get(key) else { return String::new() };
        let etag = if self.flags.no_file_etags { String::new() } else { format!("<d:getetag>{}</d:getetag>", esc(&file.etag)) };
        let ids = if self.flags.no_ids {
            String::new()
        } else {
            let checksums = match (&file.checksum, self.flags.checksums_from_client_only || self.flags.nextcloud) {
                (Some(sent), _) => format!("<oc:checksums><oc:checksum>{}</oc:checksum></oc:checksums>", esc(sent)),
                (None, true) => String::new(),
                (None, false) => format!("<oc:checksums><oc:checksum>SHA1:{:040x} MD5:{:032x}</oc:checksum></oc:checksums>", file.id, file.id),
            };
            format!("<oc:fileid>{}</oc:fileid>{checksums}", file.id)
        };
        // A Nextcloud- or ownCloud-shaped server names a file's versions by its modified time, so
        // the listing says when it was written (by the server's clock).
        let written = jiff::Timestamp::from_second(file.written_secs).ok().map(|t| t.strftime("%a, %d %b %Y %H:%M:%S GMT").to_string());
        let modified = match (&self.flags.date, self.known_server()) {
            (_, true) => written.unwrap_or_default(),
            (Some(date), false) => date.clone(),
            (None, false) => "Mon, 16 Nov 2026 10:00:00 GMT".to_owned(),
        };
        let modified = modified.as_str();
        format!(
            "<d:response><d:href>{}</d:href><d:propstat><d:prop><d:resourcetype/>{etag}{ids}<d:getcontentlength>{}</d:getcontentlength><d:getlastmodified>{}</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
            self.href(&file.path, false),
            file.body.len(),
            esc(modified)
        )
    }

    fn mkcol(&mut self, path: &str) -> HttpResponse {
        if self.has(path) {
            return self.done(405, "");
        }
        if !self.parent_exists(path) {
            return self.done(409, "");
        }
        let etag = self.fresh_etag("f");
        self.folders.insert(self.key(path), Folder { path: path.trim_matches('/').to_owned(), etag, modified: self.flags.date.clone() });
        self.touch_parents(path);
        self.done(201, "")
    }

    fn put(&mut self, path: &str, if_none_match: Option<String>, if_match: Option<String>, checksum: Option<String>, body: &[u8]) -> HttpResponse {
        let key = self.key(path);
        if self.folders.contains_key(&key) {
            return self.done(409, "a folder");
        }
        if !self.parent_exists(path) {
            return self.done(if self.flags.missing_parent_404 || self.flags.nextcloud { 404 } else { 409 }, "no such folder");
        }
        let existing = self.files.get(&key).cloned();
        // The runtime-watch misbehaviours: the server answers success but keeps the file already
        // there (someone else's), so the next listing shows neither our ETag nor our bytes.
        if if_none_match.as_deref() == Some("*") && existing.is_some() && self.flags.create_keeps_other {
            let etag = self.fresh_etag("e");
            return self.done(201, "").with_header("ETag", &etag);
        }
        if let Some(wanted) = &if_match
            && self.flags.replace_keeps_other
            && existing.as_ref().is_some_and(|f| f.etag.trim_matches('"') != wanted.trim_matches('"'))
        {
            let etag = self.fresh_etag("e");
            return self.done(204, "").with_header("ETag", &etag);
        }
        if if_none_match.as_deref() == Some("*") && existing.is_some() && !self.flags.ignores_if_none_match {
            return self.done(412, "");
        }
        if let Some(wanted) = &if_match
            && !self.flags.ignores_if_match
            && existing.as_ref().is_none_or(|f| f.etag.trim_matches('"') != wanted.trim_matches('"'))
        {
            return self.done(412, "");
        }
        let now = std::time::Instant::now();
        let same_second = |f: &File| f.written_at.is_some_and(|at| now.duration_since(at) < std::time::Duration::from_secs(1));
        let (etag, id, status) = match existing {
            Some(f) if self.flags.fixed_etags => (f.etag, f.id, 204),
            Some(f) if self.flags.etag_reused_within_second && same_second(&f) => (f.etag, f.id, 204),
            Some(f) => (self.fresh_etag("e"), f.id, 204),
            None => {
                let etag = self.fresh_etag("e");
                let id = self.next;
                self.next += 1;
                (etag, id, 201)
            }
        };
        let checksum = checksum.filter(|c| !c.is_empty());
        // Nextcloud and ownCloud keep the bytes an overwrite replaces as a version, by file id.
        if self.known_server()
            && let Some(old) = self.files.get(&key)
        {
            let kept = (old.written_secs, old.body.clone());
            self.versions.entry(id).or_default().push(kept);
        }
        let written_secs = self.tick();
        self.files.insert(
            key,
            File {
                path: path.trim_matches('/').to_owned(),
                etag: etag.clone(),
                body: body.to_vec(),
                id,
                checksum,
                written_at: Some(now),
                written_secs,
            },
        );
        self.touch_parents(path);
        let resp = self.done(status, "");
        if self.flags.no_file_etags { resp } else { resp.with_header("ETag", &etag) }
    }

    fn get(&self, path: &str) -> HttpResponse {
        match self.files.get(&self.key(path)) {
            Some(f) => {
                let resp = self.done(200, f.body.clone());
                if self.flags.no_file_etags { resp } else { resp.with_header("ETag", &f.etag) }
            }
            None => self.done(404, ""),
        }
    }

    fn relocate(&mut self, is_move: bool, from: &str, destination: Option<&str>, overwrite: Option<&str>) -> HttpResponse {
        if (is_move && self.flags.no_move) || (!is_move && self.flags.no_copy) {
            return self.done(405, "");
        }
        let Some(to) = destination.and_then(|d| self.relative(d)) else { return self.done(400, "no destination") };
        let exists = self.has(&to);
        let no_overwrite = overwrite.is_some_and(|o| o.eq_ignore_ascii_case("F"));
        if self.folders.contains_key(&self.key(from)) && !from.is_empty() {
            return self.relocate_folder(is_move, from, &to, exists && no_overwrite);
        }
        let Some(file) = self.files.get(&self.key(from)).cloned() else { return self.done(404, "") };
        if exists && no_overwrite && !(is_move && self.flags.move_overwrites) {
            return self.done(412, "");
        }
        if !self.parent_exists(&to) {
            return self.done(409, "");
        }
        let moved = if is_move {
            self.files.remove(&self.key(from));
            let etag = if self.flags.move_changes_etag { self.fresh_etag("e") } else { file.etag };
            File { path: to.clone(), etag, ..file }
        } else {
            let etag = self.fresh_etag("e");
            let id = self.next;
            self.next += 1;
            File {
                path: to.clone(),
                etag,
                body: file.body,
                id,
                checksum: file.checksum,
                written_at: file.written_at,
                written_secs: file.written_secs,
            }
        };
        self.files.insert(self.key(&to), moved);
        self.touch_parents(from);
        self.touch_parents(&to);
        self.done(if exists { 204 } else { 201 }, "")
    }

    /// `MOVE` or `COPY` of a whole folder (its files and folders keep their places below it).
    fn relocate_folder(&mut self, is_move: bool, from: &str, to: &str, refused: bool) -> HttpResponse {
        if refused {
            return self.done(412, "");
        }
        if !self.parent_exists(to) {
            return self.done(409, "");
        }
        let from_key = self.key(from);
        let prefix = format!("{from_key}/");
        let below = |k: &str| k == from_key || k.starts_with(&prefix);
        let moved_path = |p: &str| format!("{to}{}", &p[from.trim_matches('/').len()..]);
        let folders: Vec<Folder> = self.folders.iter().filter(|(k, _)| below(k)).map(|(_, f)| f.clone()).collect();
        let files: Vec<File> = self.files.iter().filter(|(k, _)| below(k)).map(|(_, f)| f.clone()).collect();
        if is_move {
            self.folders.retain(|k, _| !below(k));
            self.files.retain(|k, _| !below(k));
        }
        for f in folders {
            let path = moved_path(&f.path);
            let etag = if is_move { f.etag } else { self.fresh_etag("f") };
            self.folders.insert(self.key(&path), Folder { path, etag, modified: f.modified });
        }
        for f in files {
            let path = moved_path(&f.path);
            let (etag, id) = if is_move {
                (f.etag, f.id)
            } else {
                let etag = self.fresh_etag("e");
                let id = self.next;
                self.next += 1;
                (etag, id)
            };
            self.files.insert(
                self.key(&path),
                File { path, etag, body: f.body, id, checksum: f.checksum, written_at: f.written_at, written_secs: f.written_secs },
            );
        }
        self.touch_parents(from);
        self.touch_parents(to);
        self.done(201, "")
    }

    fn delete(&mut self, path: &str, if_match: Option<String>) -> HttpResponse {
        let key = self.key(path);
        if let Some(file) = self.files.get(&key) {
            if let Some(wanted) = if_match
                && !self.flags.delete_ignores_if_match
                && file.etag.trim_matches('"') != wanted.trim_matches('"')
            {
                return self.done(412, "");
            }
            let file = self.files.remove(&key);
            if self.known_server()
                && let Some(file) = file
            {
                self.put_in_trash(path, false, vec![file], Vec::new());
            }
            self.touch_parents(path);
            return self.done(204, "");
        }
        if self.folders.contains_key(&key) {
            let prefix = format!("{key}/");
            let below = |k: &str| k == key || k.starts_with(&prefix);
            let files: Vec<File> = self.files.iter().filter(|(k, _)| below(k)).map(|(_, f)| f.clone()).collect();
            let folders: Vec<Folder> = self.folders.iter().filter(|(k, _)| below(k)).map(|(_, f)| f.clone()).collect();
            self.files.retain(|p, _| !p.starts_with(&prefix));
            self.folders.retain(|p, _| !p.starts_with(&prefix) && *p != key);
            if self.known_server() {
                self.put_in_trash(path, true, files, folders);
            }
            self.touch_parents(path);
            return self.done(204, "");
        }
        self.done(404, "")
    }

    // --- Nextcloud's and ownCloud's own endpoints (`Flags::nextcloud`, `Flags::owncloud`) ---------

    fn put_in_trash(&mut self, path: &str, is_folder: bool, files: Vec<File>, folders: Vec<Folder>) {
        let deleted_at = self.tick();
        // Nextcloud names a trashed item `<name>.d<seconds>`, ownCloud by the file id it had (a
        // folder gets a fresh one here).
        let name = if self.flags.owncloud {
            match files.first().filter(|_| !is_folder) {
                Some(f) => f.id.to_string(),
                None => {
                    let id = self.next;
                    self.next += 1;
                    id.to_string()
                }
            }
        } else {
            format!("{}.d{deleted_at}", path.rsplit('/').next().unwrap_or(path))
        };
        self.trash.push(TrashedItem { name, original: path.trim_matches('/').to_owned(), is_folder, files, folders, deleted_at });
    }

    /// Puts a trashed item's files and folders back, at `to` (its original place, or elsewhere).
    fn restore_trashed(&mut self, t: TrashedItem, to: &str) {
        let moved = |p: &str| format!("{to}{}", p.strip_prefix(t.original.as_str()).unwrap_or(""));
        let parent = to.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
        self.add_folder(&parent, None);
        for f in t.folders {
            let path = moved(&f.path);
            self.folders.insert(self.key(&path), Folder { path, ..f });
        }
        for f in t.files {
            let path = moved(&f.path);
            self.files.insert(self.key(&path), File { path, ..f });
        }
        self.touch_parents(to);
    }

    /// The URL path of a request (its query dropped), decoded.
    fn url_path(url: &str) -> Option<String> {
        let path = url.find("://").and_then(|i| url[i + 3..].find('/').map(|j| &url[i + 3 + j..])).unwrap_or(url);
        let path = path.split('?').next().unwrap_or(path);
        percent_decode_str(path).decode_utf8().ok().map(|p| p.into_owned())
    }

    /// Answers what a Nextcloud or an ownCloud answers besides its files; `None` for a request on
    /// the files base.
    fn beside_the_files(&mut self, method: &str, url: &str, destination: Option<&str>, overwrite: Option<&str>, body: &[u8]) -> Option<HttpResponse> {
        let path = Self::url_path(url)?;
        let user = self.user().to_owned();
        let json = |status: u16, text: String| HttpResponse::new(status, text).with_header("Content-Type", "application/json");
        if path.starts_with(&format!("{}/", self.base_path)) || path == self.base_path {
            return None;
        }
        self.log.push((method.to_owned(), path.clone(), destination.map(|d| vec![("destination".to_owned(), d.to_owned())]).unwrap_or_default()));
        if self.flags.owncloud {
            return Some(self.owncloud(method, &path, destination, overwrite));
        }
        match (method, path.as_str()) {
            ("GET", "/status.php") => {
                let version = self.flags.nextcloud_version.clone().unwrap_or_else(|| "31.0.14.1".to_owned());
                let short = version.rsplit_once('.').map_or(version.as_str(), |(s, _)| s).to_owned();
                Some(json(200, format!(r#"{{"installed":true,"maintenance":false,"version":"{version}","versionstring":"{short}","edition":"","productname":"Nextcloud"}}"#)))
            }
            ("GET", "/ocs/v2.php/cloud/user") => {
                let free = self.flags.quota_available.unwrap_or(-3);
                Some(json(200, format!(r#"{{"ocs":{{"meta":{{"status":"ok","statuscode":200}},"data":{{"id":"{user}","quota":{{"free":{free},"used":747,"total":{free},"quota":{free}}}}}}}}}"#)))
            }
            ("DELETE", "/ocs/v2.php/core/apppassword") => {
                self.app_password_deleted = true;
                Some(json(200, r#"{"ocs":{"meta":{"status":"ok","statuscode":200}}}"#.into()))
            }
            ("POST", "/index.php/login/v2") => Some(json(
                200,
                format!(r#"{{"poll":{{"token":"fake-poll-token","endpoint":"{0}/login/v2/poll"}},"login":"{0}/login/v2/flow/fake-flow-token"}}"#, self.origin),
            )),
            ("POST", "/login/v2/poll") => {
                if !String::from_utf8_lossy(body).contains("token=fake-poll-token") {
                    return Some(json(404, "[]".into()));
                }
                self.polls += 1;
                if self.flags.login_flow_ready_after > 0 && self.polls >= self.flags.login_flow_ready_after {
                    Some(json(200, format!(r#"{{"server":"{}","loginName":"{user}","appPassword":"fake-app-password"}}"#, self.origin)))
                } else {
                    Some(json(404, "[]".into()))
                }
            }
            ("PROPFIND", "/remote.php/dav/" | "/remote.php/dav") => Some(self.done(
                207,
                format!(
                    r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/remote.php/dav/</d:href><d:propstat><d:prop><d:current-user-principal><d:href>/remote.php/dav/principals/users/{user}/</d:href></d:current-user-principal></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
                ),
            )),
            _ => {
                let trashbin = format!("/remote.php/dav/trashbin/{user}/");
                let versions = format!("/remote.php/dav/versions/{user}/");
                if let Some(rest) = path.strip_prefix(&trashbin) {
                    return Some(self.trashbin(method, rest, destination.and_then(Self::url_path).as_deref()));
                }
                if let Some(rest) = path.strip_prefix(&versions) {
                    return Some(self.versions_endpoint(method, rest));
                }
                Some(self.done(404, "not found"))
            }
        }
    }

    /// What an ownCloud 10 answers besides its files (`Flags::owncloud`): `status.php`, the
    /// principal, the trash bin at `trash-bin/<user>/` and the versions at `meta/<file id>/v/`.
    fn owncloud(&mut self, method: &str, path: &str, destination: Option<&str>, overwrite: Option<&str>) -> HttpResponse {
        let user = self.user().to_owned();
        match (method, path) {
            ("GET", "/status.php") => HttpResponse::new(
                200,
                r#"{"installed":true,"maintenance":false,"needsDbUpgrade":false,"version":"10.15.3.0","versionstring":"10.15.3","edition":"Community","productname":"ownCloud","product":"ownCloud"}"#,
            )
            .with_header("Content-Type", "application/json"),
            ("PROPFIND", "/remote.php/dav/" | "/remote.php/dav") => self.done(
                207,
                format!(
                    r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/remote.php/dav/</d:href><d:propstat><d:prop><d:current-user-principal><d:href>/remote.php/dav/principals/users/{user}/</d:href></d:current-user-principal></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
                ),
            ),
            _ => {
                let trash = format!("/remote.php/dav/trash-bin/{user}/");
                if let Some(rest) = path.strip_prefix(&trash) {
                    return self.owncloud_trash(method, rest, destination, overwrite);
                }
                if let Some(rest) = path.strip_prefix("/remote.php/dav/meta/") {
                    return self.owncloud_meta(method, rest, destination, overwrite);
                }
                self.done(404, "not found")
            }
        }
    }

    /// ownCloud's trash bin: items named by file id right under the root; a `GET` is 403; a
    /// `MOVE` to any path under the files base restores (`Overwrite: F` honoured, 409 for a
    /// missing parent, 409 for Nextcloud's `restore/` shape); a `DELETE` purges.
    fn owncloud_trash(&mut self, method: &str, rest: &str, destination: Option<&str>, overwrite: Option<&str>) -> HttpResponse {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
        let user = self.user().to_owned();
        let root = format!("/remote.php/dav/trash-bin/{user}/");
        let id = rest.trim_end_matches('/');
        match method {
            "PROPFIND" if id.is_empty() => {
                let mut xml = format!(
                    r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:response><d:href>{root}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#
                );
                for t in &self.trash {
                    let href = format!("{root}{}{}", t.name, if t.is_folder { "/" } else { "" });
                    let kind = if t.is_folder { "<d:collection/>" } else { "" };
                    let size = if t.is_folder {
                        String::new()
                    } else {
                        format!("<d:getcontentlength>{}</d:getcontentlength>", t.files.first().map_or(0, |f| f.body.len()))
                    };
                    let file_name = t.original.rsplit('/').next().unwrap_or(&t.original);
                    let when = jiff::Timestamp::from_second(t.deleted_at)
                        .ok()
                        .map(|w| w.strftime("%a, %d %b %Y %H:%M:%S GMT").to_string())
                        .unwrap_or_default();
                    xml.push_str(&format!(
                        "<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:resourcetype>{kind}</d:resourcetype>{size}<d:getlastmodified>{when}</d:getlastmodified><oc:trashbin-original-filename>{}</oc:trashbin-original-filename><oc:trashbin-original-location>{}</oc:trashbin-original-location><oc:trashbin-delete-timestamp>{}</oc:trashbin-delete-timestamp></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
                        esc(file_name),
                        esc(&t.original),
                        t.deleted_at
                    ));
                }
                xml.push_str("</d:multistatus>");
                self.done(207, xml)
            }
            "PROPFIND" => match self.trash.iter().find(|t| t.name == id) {
                Some(t) => {
                    let href = format!("{root}{}{}", t.name, if t.is_folder { "/" } else { "" });
                    self.done(207, format!(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>{href}</d:href><d:propstat><d:prop><d:resourcetype/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#))
                }
                None => self.done(404, ""),
            },
            "GET" => self.done(403, "Permission denied to read this file"),
            "MOVE" => {
                let Some(at) = self.trash.iter().position(|t| t.name == id) else { return self.done(404, "") };
                // Only a destination under the files base; Nextcloud's `restore/` shape is a
                // destination that isn't there (409, measured).
                let Some(to) = destination.and_then(|d| self.relative(d)) else { return self.done(409, "the destination node is not found") };
                if self.has(&to) && overwrite.is_some_and(|o| o.eq_ignore_ascii_case("F")) {
                    return self.done(412, "");
                }
                if !self.parent_exists(&to) {
                    return self.done(409, "");
                }
                let t = self.trash.remove(at);
                self.restore_trashed(t, &to);
                self.done(201, "")
            }
            "DELETE" => {
                let Some(at) = self.trash.iter().position(|t| t.name == id) else { return self.done(404, "") };
                self.trash.remove(at);
                self.trash_purged += 1;
                self.done(204, "")
            }
            _ => self.done(405, ""),
        }
    }

    /// ownCloud's versions: `meta/<file id>/v/` lists the earlier versions (not the current file),
    /// `GET` of one gives its bytes, a `COPY` of one to the file's URL restores it (`Overwrite: F`
    /// answers 412 since the file is there; the replaced file is kept as a version).
    fn owncloud_meta(&mut self, method: &str, rest: &str, destination: Option<&str>, overwrite: Option<&str>) -> HttpResponse {
        let root = "/remote.php/dav/meta/";
        let (id_text, inside) = rest.split_once('/').unwrap_or((rest, ""));
        let Ok(id) = id_text.parse::<u64>() else { return self.done(404, "") };
        let Some(file) = self.files.values().find(|f| f.id == id).cloned() else { return self.done(404, "") };
        let stamp = inside.strip_prefix("v/").map_or("", |s| s.trim_end_matches('/'));
        if !inside.starts_with("v") {
            return self.done(404, "");
        }
        let earlier = self.versions.get(&id).cloned().unwrap_or_default();
        match method {
            "PROPFIND" if stamp.is_empty() => {
                let mut xml = format!(
                    r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:response><d:href>{root}{id}/v/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#
                );
                for (at, bytes) in &earlier {
                    let when =
                        jiff::Timestamp::from_second(*at).ok().map(|w| w.strftime("%a, %d %b %Y %H:%M:%S GMT").to_string()).unwrap_or_default();
                    xml.push_str(&format!(
                        "<d:response><d:href>{root}{id}/v/{at}</d:href><d:propstat><d:prop><d:resourcetype/><d:getetag>&quot;v{at}&quot;</d:getetag><d:getcontentlength>{}</d:getcontentlength><d:getlastmodified>{when}</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
                        bytes.len()
                    ));
                }
                xml.push_str("</d:multistatus>");
                self.done(207, xml)
            }
            "GET" => match earlier.iter().find(|(at, _)| at.to_string() == stamp) {
                Some((_, bytes)) => self.done(200, bytes.clone()),
                None => self.done(404, ""),
            },
            "COPY" => {
                let Some(version) = earlier.iter().find(|(at, _)| at.to_string() == stamp).cloned() else { return self.done(404, "") };
                let Some(to) = destination.and_then(|d| self.relative(d)) else { return self.done(409, "") };
                if self.key(&to) != self.key(&file.path) {
                    return self.done(409, "a version is copied over its own file here");
                }
                if overwrite.is_some_and(|o| o.eq_ignore_ascii_case("F")) {
                    return self.done(412, "The destination node already exists, and the overwrite header is set to false");
                }
                let key = self.key(&file.path);
                let etag = self.fresh_etag("e");
                let now = self.tick();
                self.versions.entry(id).or_default().push((file.written_secs, file.body.clone()));
                if let Some(f) = self.files.get_mut(&key) {
                    f.body = version.1;
                    f.etag = etag;
                    f.written_secs = now;
                    f.checksum = None;
                }
                self.done(204, "")
            }
            _ => self.done(405, ""),
        }
    }

    fn trashbin(&mut self, method: &str, rest: &str, destination: Option<&str>) -> HttpResponse {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
        let user = self.user().to_owned();
        let root = format!("/remote.php/dav/trashbin/{user}/");
        match (method, rest.trim_end_matches('/')) {
            ("PROPFIND", "trash") => {
                let mut xml = format!(
                    r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns"><d:response><d:href>{root}trash/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#
                );
                for t in &self.trash {
                    let href = format!("{root}trash/{}{}", utf8_percent_encode(&t.name, SEGMENT), if t.is_folder { "/" } else { "" });
                    let kind = if t.is_folder { "<d:collection/>" } else { "" };
                    let size = t.files.first().filter(|_| !t.is_folder).map_or(0, |f| f.body.len());
                    let id = t.files.first().map_or(0, |f| f.id);
                    let file_name = t.name.rsplit_once(".d").map_or(t.name.as_str(), |(n, _)| n);
                    xml.push_str(&format!(
                        "<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:resourcetype>{kind}</d:resourcetype><d:getcontentlength>{size}</d:getcontentlength><oc:fileid>{id}</oc:fileid><nc:trashbin-filename>{}</nc:trashbin-filename><nc:trashbin-original-location>{}</nc:trashbin-original-location><nc:trashbin-deletion-time>{}</nc:trashbin-deletion-time></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
                        esc(file_name),
                        esc(&t.original),
                        t.deleted_at
                    ));
                }
                xml.push_str("</d:multistatus>");
                self.done(207, xml)
            }
            ("MOVE", item) => {
                let Some(name) = item.strip_prefix("trash/") else { return self.done(404, "") };
                let Some(at) = self.trash.iter().position(|t| t.name == name) else { return self.done(404, "") };
                if destination != Some(format!("{root}restore/{name}").as_str()) {
                    return self.done(400, "a trashed item moves only to its restore address here");
                }
                let t = self.trash.remove(at);
                let original = t.original.clone();
                self.restore_trashed(t, &original);
                self.done(201, "")
            }
            ("DELETE", item) => {
                let Some(name) = item.strip_prefix("trash/") else { return self.done(404, "") };
                let Some(at) = self.trash.iter().position(|t| t.name == name) else { return self.done(404, "") };
                self.trash.remove(at);
                self.trash_purged += 1;
                self.done(204, "")
            }
            _ => self.done(405, ""),
        }
    }

    fn versions_endpoint(&mut self, method: &str, rest: &str) -> HttpResponse {
        let user = self.user().to_owned();
        let root = format!("/remote.php/dav/versions/{user}/");
        let Some(rest) = rest.strip_prefix("versions/") else { return self.done(404, "") };
        let (id_text, stamp) = rest.trim_end_matches('/').split_once('/').map_or((rest.trim_end_matches('/'), ""), |(i, s)| (i, s));
        let Ok(id) = id_text.parse::<u64>() else { return self.done(404, "") };
        let Some(file) = self.files.values().find(|f| f.id == id).cloned() else { return self.done(404, "") };
        let earlier = self.versions.get(&id).cloned().unwrap_or_default();
        match method {
            "PROPFIND" => {
                let mut xml = format!(
                    r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns"><d:response><d:href>{root}versions/{id}/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#
                );
                // The earlier versions and the current file, each named by its time.
                let mut all: Vec<(i64, usize)> = earlier.iter().map(|(at, b)| (*at, b.len())).collect();
                all.push((file.written_secs, file.body.len()));
                for (at, size) in all {
                    xml.push_str(&format!(
                        "<d:response><d:href>{root}versions/{id}/{at}</d:href><d:propstat><d:prop><d:resourcetype/><d:getetag>{at}</d:getetag><d:getcontentlength>{size}</d:getcontentlength></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
                    ));
                }
                xml.push_str("</d:multistatus>");
                self.done(207, xml)
            }
            // A version's bytes. (The real server also restores by a `MOVE` to `restore/target`;
            // CraftCloud never sends that unguarded write, so the scripted server answers 405.)
            "GET" => match earlier.iter().find(|(at, _)| at.to_string() == stamp) {
                Some((_, bytes)) => self.done(200, bytes.clone()),
                None if file.written_secs.to_string() == stamp => self.done(200, file.body.clone()),
                None => self.done(404, ""),
            },
            _ => self.done(405, ""),
        }
    }
}
