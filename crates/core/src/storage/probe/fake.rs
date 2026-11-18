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
}

#[derive(Clone, Debug)]
struct File {
    /// As written when made (a case-insensitive server keeps the case it was given).
    path: String,
    etag: String,
    body: Vec<u8>,
    id: u64,
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
}

const SEGMENT: &AsciiSet =
    &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}').add(b'/').add(b'\\');

impl FakeDav {
    pub fn new(base_path: &str, flags: Flags) -> Self {
        let mut folders = BTreeMap::new();
        folders.insert(String::new(), Folder { path: String::new(), etag: "\"f0\"".into(), modified: None });
        Self { base_path: base_path.trim_end_matches('/').to_owned(), flags, files: BTreeMap::new(), folders, next: 1, log: Vec::new() }
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
        self.files.insert(self.key(path), File { path: path.trim_matches('/').to_owned(), etag, body: body.to_vec(), id });
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
        let Some(path) = self.relative(url_path) else { return self.done(404, "outside the base") };
        let header = |name: &str| headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_owned());
        let guards: Vec<(String, String)> = ["if-match", "if-none-match", "overwrite", "depth", "destination"]
            .iter()
            .filter_map(|n| header(n).map(|v| ((*n).to_owned(), v)))
            .collect();
        self.log.push((method.to_owned(), path.clone(), guards));
        match method {
            "PROPFIND" => self.propfind(&path, header("Depth").as_deref().unwrap_or("infinity")),
            "MKCOL" => self.mkcol(&path),
            "PUT" => self.put(&path, header("If-None-Match"), header("If-Match"), body),
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
            return self.done(403, "");
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
            format!(
                "<oc:fileid>{}</oc:fileid><oc:checksums><oc:checksum>SHA1:{:040x} MD5:{:032x}</oc:checksum></oc:checksums>",
                file.id, file.id, file.id
            )
        };
        let modified = self.flags.date.as_deref().unwrap_or("Mon, 16 Nov 2026 10:00:00 GMT");
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

    fn put(&mut self, path: &str, if_none_match: Option<String>, if_match: Option<String>, body: &[u8]) -> HttpResponse {
        let key = self.key(path);
        if self.folders.contains_key(&key) {
            return self.done(409, "a folder");
        }
        if !self.parent_exists(path) {
            return self.done(if self.flags.missing_parent_404 { 404 } else { 409 }, "no such folder");
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
        let (etag, id, status) = match existing {
            Some(f) if self.flags.fixed_etags => (f.etag, f.id, 204),
            Some(f) => (self.fresh_etag("e"), f.id, 204),
            None => {
                let etag = self.fresh_etag("e");
                let id = self.next;
                self.next += 1;
                (etag, id, 201)
            }
        };
        self.files.insert(key, File { path: path.trim_matches('/').to_owned(), etag: etag.clone(), body: body.to_vec(), id });
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
            File { path: to.clone(), etag, body: file.body, id }
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
            self.files.insert(self.key(&path), File { path, etag, body: f.body, id });
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
            self.files.remove(&key);
            self.touch_parents(path);
            return self.done(204, "");
        }
        if self.folders.contains_key(&key) {
            let prefix = format!("{key}/");
            self.files.retain(|p, _| !p.starts_with(&prefix));
            self.folders.retain(|p, _| !p.starts_with(&prefix) && *p != key);
            self.touch_parents(path);
            return self.done(204, "");
        }
        self.done(404, "")
    }
}
