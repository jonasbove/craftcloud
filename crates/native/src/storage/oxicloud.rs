//! The OxiCloud driver: the sync engine's requests to CraftCloud's own server, moved behind
//! [`RemoteStorage`] unchanged (docs/WEBDAV-STORAGE.md § 7.4).
//!
//! File bytes and listings go over WebDAV; the trash, the account and the ids the trash needs go
//! over REST. OxiCloud's WebDAV `DELETE` deletes for good, and neither its REST trash nor its
//! `MOVE` takes `If-Match`: so a file is listed just before it is trashed or moved, and goes only
//! while it is still at the version planned (the window is the few requests in between, § 8).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use craftcloud_core::api::dav::{self, Uploaded, Write};
use craftcloud_core::api::resources::Kind;
use craftcloud_core::api::{admin, auth, files, folders, trash};
use craftcloud_core::http::HttpResponse;
use craftcloud_core::storage::Capabilities;

use super::{Download, Entry, LISTERS, MoveGuard, Moved, OwnPath, RemoteStorage, StorageError, TrashGuard, TrashItem, Trashed, Written};
use crate::http;
use crate::session::Session;

/// How often an admin's client asks the cloud to count the storage use again.
const RECOUNT_EVERY: Duration = Duration::from_secs(60);

/// What OxiCloud v0.10.0 guarantees, filled in by hand ([`Capabilities::oxicloud`]).
const CAPABILITIES: Capabilities = Capabilities::oxicloud();

pub struct OxiCloud {
    pub(crate) session: Session,
    /// REST folder ids by cloud path (`""` is the drive's root), learnt this pass: WebDAV has no
    /// ids, and moving to the trash goes by id.
    pub(crate) ids: BTreeMap<String, String>,
    /// When the cloud was last asked to count the storage use again (admins only).
    recounted: Option<Instant>,
}

impl OxiCloud {
    pub fn new(session: Session) -> Self {
        Self { session, ids: BTreeMap::new(), recounted: None }
    }

    /// Whether the cloud file at `path` is still at version `etag` (`None`: it's gone). A file
    /// listed without an ETag can't be shown to be at `etag`: `Some(Err(()))`.
    fn still_at(&mut self, path: &str, etag: &str) -> Result<Option<Result<bool, ()>>, StorageError> {
        let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
        match self.list(parent).map(|entries| entries.into_iter().find(|e| e.path == path)) {
            Ok(Some(e)) => {
                let now = e.etag.as_deref().map(|now| now.trim_matches('"')).filter(|now| !now.is_empty());
                Ok(Some(now.map(|now| now == etag.trim_matches('"')).ok_or(())))
            }
            Ok(None) => Ok(None),
            Err(e) if e.to_string().ends_with(": 404") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Whether the cloud folder `path` holds exactly `files` (path, ETag), nothing more or newer,
    /// listed again now. A file listed without an ETag can't be shown unchanged, so it keeps the
    /// folder.
    fn cloud_holds(&mut self, path: &str, files: &[(String, String)]) -> Result<bool, StorageError> {
        let mut found: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut level = vec![path.to_owned()];
        while !level.is_empty() {
            let mut next = Vec::new();
            for (_, entries) in self.list_many(&level)? {
                for e in entries {
                    if e.is_folder {
                        next.push(e.path);
                    } else {
                        found.insert(e.path, e.etag);
                    }
                }
            }
            level = next;
        }
        Ok(found.len() == files.len()
            && files.iter().all(|(p, etag)| {
                let now = found.get(p).and_then(Option::as_deref).map(|now| now.trim_matches('"')).filter(|now| !now.is_empty());
                now.is_some_and(|now| now == etag.trim_matches('"'))
            }))
    }

    /// Moves a cloud file to the trash by its REST id. One already gone counts as done.
    fn trash_file(&mut self, path: &str) -> Result<(), StorageError> {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let Some(folder) = self.folder_id(parent)? else { return Ok(()) };
        let Some(id) = self.child_id(&folder, name, Kind::File)? else { return Ok(()) };
        let resp = self.session.call(&files::trash(&self.session.server, &id))?;
        if resp.is_success() || resp.status == 404 { Ok(()) } else { Err(trash_refused(&resp)) }
    }

    /// Moves a cloud folder to the trash by its REST id, as one item. One already gone counts as
    /// done.
    fn trash_folder(&mut self, path: &str) -> Result<(), StorageError> {
        let Some(id) = self.folder_id(path)? else { return Ok(()) };
        let resp = self.session.call(&folders::trash(&self.session.server, &id))?;
        self.ids.retain(|p, _| p != path && !p.starts_with(&format!("{path}/")));
        if resp.is_success() || resp.status == 404 { Ok(()) } else { Err(trash_refused(&resp)) }
    }

    /// The REST id of the cloud folder at `path` (`""`: the drive's root), or `None` when there is
    /// no such folder.
    fn folder_id(&mut self, path: &str) -> Result<Option<String>, StorageError> {
        if let Some(id) = self.ids.get(path) {
            return Ok(Some(id.clone()));
        }
        let id = if path.is_empty() {
            let resp = self.session.call(&folders::roots(&self.session.server))?;
            let roots = folders::parse_roots(&resp).map_err(|e| format!("the drive's root folder: {e}"))?;
            roots.into_iter().find(|f| f.parent_id.is_none()).map(|f| f.id)
        } else {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            match self.folder_id(parent)? {
                Some(parent) => self.child_id(&parent, name, Kind::Folder)?,
                None => None,
            }
        };
        if let Some(id) = &id {
            self.ids.insert(path.to_owned(), id.clone());
        }
        Ok(id)
    }

    /// The id of `name` (a file or a folder, as `kind` says) in the folder with id `folder`.
    fn child_id(&mut self, folder: &str, name: &str, kind: Kind) -> Result<Option<String>, StorageError> {
        let mut cursor: Option<String> = None;
        loop {
            let resp = self.session.call(&folders::contents(&self.session.server, folder, Some(200), cursor.as_deref()))?;
            if resp.status == 404 {
                return Ok(None);
            }
            let page = folders::parse_contents_all(&resp).map_err(|e| format!("listing a folder: {e}"))?;
            if let Some(found) = page.items.iter().find(|r| r.kind() == kind && r.name() == name) {
                return Ok(Some(found.id().to_owned()));
            }
            match page.next_cursor {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => return Ok(None),
            }
        }
    }

    /// One guarded `PUT` of `file`.
    fn upload(&mut self, path: &str, file: &Path, write: &Write) -> Result<Written, StorageError> {
        let req = dav::upload(&self.session.server, path, Vec::new(), write);
        let resp = self.session.upload(&req, file)?;
        if resp.status == 507 {
            return Err(StorageError::NoRoom);
        }
        if matches!(resp.status, 400 | 403 | 414 | 415 | 422) {
            // Refused for what it is, not for now.
            let why = serde_json::from_slice::<serde_json::Value>(&resp.body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("the server answered {}", resp.status));
            return Err(StorageError::Refused(why));
        }
        match dav::parse_upload(&resp).map_err(|e| e.to_string())? {
            Uploaded::Conflict => Ok(Written::Conflict),
            Uploaded::Saved { etag } => {
                let etag = match etag {
                    Some(e) => e,
                    None => self.listed_etag(path)?,
                };
                Ok(Written::Saved { etag })
            }
        }
    }

    /// The signed-in account, with its storage use; `None` when it can't be had.
    fn me(&mut self) -> Option<auth::Me> {
        self.session.call(&auth::me(&self.session.server)).ok().and_then(|r| auth::parse_me(&r).ok().flatten())
    }
}

fn trash_refused(resp: &HttpResponse) -> StorageError {
    StorageError::Failed(format!("moving it to the trash: the server answered {}", resp.status))
}

impl RemoteStorage for OxiCloud {
    fn capabilities(&self) -> &Capabilities {
        &CAPABILITIES
    }

    fn begin_pass(&mut self) {
        self.session.offline = None;
        self.ids.clear();
    }

    fn take_offline(&mut self) -> Option<String> {
        self.session.offline.take()
    }

    fn signed_out(&self) -> bool {
        self.session.signed_out
    }

    fn list(&mut self, folder: &str) -> Result<Vec<Entry>, StorageError> {
        let resp = self.session.call(&dav::list(&self.session.server, folder))?;
        if resp.status == 404 {
            return Err(StorageError::NotFound(format!("listing {folder:?}: 404")));
        }
        let mut entries = dav::parse_list(&resp).map_err(|e| format!("listing {folder:?}: {e}"))?;
        let me = folder.trim_matches('/');
        entries.retain(|e| e.path != me);
        Ok(entries.into_iter().map(Entry::from).collect())
    }

    /// [`LISTERS`] folders at a time. A folder gone meanwhile (404) is left out; an expired token
    /// is refreshed once and those folders are listed again.
    fn list_many(&mut self, folders: &[String]) -> Result<Vec<(String, Vec<Entry>)>, StorageError> {
        let mut out = Vec::new();
        for chunk in folders.chunks(LISTERS) {
            let (server, tokens) = (&self.session.server, &self.session.tokens);
            let answers: Vec<(String, Result<HttpResponse, String>)> = std::thread::scope(|scope| {
                let jobs: Vec<_> = chunk
                    .iter()
                    .map(|folder| (folder.clone(), scope.spawn(move || http::execute(&tokens.authorize(dav::list(server, folder))))))
                    .collect();
                jobs.into_iter().map(|(f, job)| (f, job.join().unwrap_or_else(|_| Err("listing thread failed".into())))).collect()
            });
            for (folder, answer) in answers {
                // No answer at all: offline (the pass ends here, and is tried again).
                let answer = answer.inspect_err(|e| self.session.offline = Some(e.clone()));
                let resp = match answer? {
                    r if auth::is_expired(&r) => self.session.call(&dav::list(&self.session.server, &folder))?,
                    r => r,
                };
                if resp.status == 404 {
                    continue;
                }
                let mut entries = dav::parse_list(&resp).map_err(|e| format!("listing {folder:?}: {e}"))?;
                entries.retain(|e| e.path != folder);
                out.push((folder, entries.into_iter().map(Entry::from).collect()));
            }
        }
        Ok(out)
    }

    fn download(&mut self, path: &str, from: Option<u64>) -> Result<Download, StorageError> {
        let mut req = dav::download(&self.session.server, path);
        if let Some(from) = from {
            req = req.header("Range", format!("bytes={from}-"));
        }
        let resp = self.session.open(&req)?;
        Ok(Download::new(resp, Arc::clone(&self.session.cancel)))
    }

    fn create(&mut self, path: &str, file: &Path) -> Result<Written, StorageError> {
        self.upload(path, file, &Write::Create)
    }

    fn replace(&mut self, path: &str, if_match: &str, file: &Path) -> Result<Written, StorageError> {
        self.upload(path, file, &Write::Replace { etag: if_match.to_owned() })
    }

    fn make_folder(&mut self, path: &str) -> Result<(), StorageError> {
        let resp = self.session.call(&dav::create_folder(&self.session.server, path))?;
        if dav::folder_exists_after(&resp) { Ok(()) } else { Err(StorageError::Status(resp.status)) }
    }

    fn move_to(&mut self, from: &str, to: &str, guard: MoveGuard) -> Result<Moved, StorageError> {
        let folder = match guard {
            MoveGuard::File { etag } => {
                // OxiCloud's MOVE takes no If-Match: checked first.
                match self.still_at(from, &etag)? {
                    Some(Ok(true)) => {}
                    Some(Err(())) => return Ok(Moved::Unversioned),
                    Some(Ok(false)) | None => return Ok(Moved::Changed),
                }
                false
            }
            MoveGuard::Folder => true,
        };
        // `Overwrite: F`: never onto an existing name.
        let resp = self.session.call(&dav::move_to(&self.session.server, from, to, folder))?;
        if resp.is_success() { Ok(Moved::Done) } else { Ok(Moved::Refused(resp.status)) }
    }

    fn trash(&mut self, path: &str, guard: TrashGuard) -> Result<Trashed, StorageError> {
        match guard {
            TrashGuard::File { etag } => {
                // Neither REST's delete takes If-Match: checked first.
                match self.still_at(path, &etag)? {
                    None => return Ok(Trashed::Done),
                    Some(Err(())) => return Ok(Trashed::Unversioned),
                    Some(Ok(false)) => return Ok(Trashed::Changed),
                    Some(Ok(true)) => {}
                }
                self.trash_file(path)?;
            }
            TrashGuard::Folder { files } => {
                if !self.cloud_holds(path, &files)? {
                    return Ok(Trashed::Changed);
                }
                self.trash_folder(path)?;
            }
            TrashGuard::EmptyFolder => {
                // A DELETE on a folder is recursive, so the cloud must say it's empty right now. A
                // listing that fails says nothing: keep the folder (T-064).
                match self.list(path) {
                    Ok(entries) if entries.is_empty() => {}
                    _ => return Ok(Trashed::Changed),
                }
                self.trash_folder(path)?;
            }
        }
        Ok(Trashed::Done)
    }

    fn trash_list(&mut self) -> Result<Vec<TrashItem>, StorageError> {
        let mut listed = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let resp = self.session.call(&trash::list(&self.session.server, Some(200), cursor.as_deref()))?;
            let page = trash::parse_list(&resp).map_err(|e| format!("listing the trash: {e}"))?;
            for item in page.items {
                if let Some(path) = item.resource.cloud_path() {
                    listed.push(TrashItem { id: item.resource.id().to_owned(), path, trashed_at: item.trashed_at });
                }
            }
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => return Ok(listed),
            }
        }
    }

    fn restore(&mut self, item: &TrashItem) -> Result<(), StorageError> {
        let resp = self.session.call(&trash::restore(&self.session.server, &item.id))?;
        if resp.is_success() { Ok(()) } else { Err(StorageError::Status(resp.status)) }
    }

    /// Copies the cloud version into `.craftcloud/versions/<path>/`, newest 5 kept
    /// ([`Session::snapshot`]).
    fn keep_version(&mut self, path: &str) -> Result<(), StorageError> {
        Ok(self.session.snapshot(path)?)
    }

    fn move_versions(&mut self, from: &str, to: &str) -> Result<(), StorageError> {
        Ok(self.session.move_versions(from, to)?)
    }

    /// From the account's quota and use. The cloud's count lags (it doesn't drop when the trash
    /// is emptied until its next recount, up to 10 minutes): below `needed`, an admin's client
    /// asks for a recount, at most once every [`RECOUNT_EVERY`].
    fn free_space(&mut self, needed: u64) -> Result<Option<u64>, StorageError> {
        let free = |me: &auth::Me| if me.storage_quota_bytes == 0 { u64::MAX } else { me.storage_quota_bytes.saturating_sub(me.storage_used_bytes) };
        let mut now = self.me();
        if let Some(m) = &now
            && free(m) < needed
            && m.user.is_admin()
            && self.recounted.is_none_or(|at| at.elapsed() >= RECOUNT_EVERY)
        {
            self.recounted = Some(Instant::now());
            let recount = self.session.call(&admin::trigger_job(&self.session.server, admin::USAGE_RECONCILE));
            if recount.is_ok_and(|r| r.is_success()) {
                now = self.me();
            }
        }
        Ok(now.as_ref().map(free))
    }

    fn purge_own(&mut self, path: &OwnPath) -> Result<(), StorageError> {
        let resp = self.session.call(&dav::delete(&self.session.server, path.as_str(), false))?;
        if resp.is_success() || resp.status == 404 { Ok(()) } else { Err(StorageError::Status(resp.status)) }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use craftcloud_core::api::auth::Tokens;

    use super::OxiCloud;
    use crate::fake_server::{Answer, FakeServer};
    use crate::session::Session;
    use crate::storage::{MoveGuard, Moved, OwnPath, RemoteStorage, Written};

    fn driver(server: &FakeServer) -> OxiCloud {
        let tokens = Tokens { access_token: "a1".into(), refresh_token: "r1".into() };
        OxiCloud::new(Session::new(server.server(), tokens, Box::new(|_| {})))
    }

    #[test]
    fn every_write_carries_its_guard() {
        let server = FakeServer::start_full(|r| match r.method.as_str() {
            "PUT" => Answer::new(201, "").header("ETag", "\"n1\""),
            _ => Answer::new(201, ""),
        });
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "abc").unwrap();
        let mut s = driver(&server);
        assert_eq!(s.create("Docs/a.txt", &file).unwrap(), Written::Saved { etag: "\"n1\"".into() });
        assert_eq!(s.replace("Docs/a.txt", "\"e1\"", &file).unwrap(), Written::Saved { etag: "\"n1\"".into() });
        assert_eq!(s.move_to("Old", "New", MoveGuard::Folder).unwrap(), Moved::Done);
        s.purge_own(&OwnPath::new(".craftcloud/versions/Docs/a.txt/1").unwrap()).unwrap();
        let seen = server.seen();
        assert_eq!(seen[0].header("if-none-match"), Some("*"), "a create only where nothing is");
        assert_eq!(seen[1].header("if-match"), Some("\"e1\""), "an overwrite only of the version given");
        assert_eq!(seen[2].header("overwrite"), Some("F"), "a move never onto an existing name");
        assert!(seen[3].is("DELETE", "/webdav/.craftcloud/"), "for good only inside .craftcloud/");
    }

    #[test]
    fn a_412_is_a_conflict_not_an_error() {
        let server = FakeServer::start(|_| (412, String::new()));
        let mut s = driver(&server);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "abc").unwrap();
        assert_eq!(s.create("a.txt", &file).unwrap(), Written::Conflict);
        assert_eq!(s.replace("a.txt", "\"e1\"", Path::new(&file)).unwrap(), Written::Conflict);
    }
}
