//! The ownCloud profile (docs/WEBDAV-STORAGE.md § 7.4, the ownCloud column; the requests and
//! parsers are `craftcloud_core::api::owncloud`'s, measured against ownCloud 10.15). It knows
//! where things are:
//!
//! - the WebDAV base is `/remote.php/dav/files/<user id>/`, the user id from `current-user-principal`
//!   (the login name can differ), as on Nextcloud;
//! - the trash bin is `/remote.php/dav/trash-bin/<user id>/`, its items named by file id; a
//!   restore is a `MOVE` of the item to its original path under the files base, with
//!   `Overwrite: F` (the server would take any destination, so the client names the right one; a
//!   taken name gets `names::restored_name`); the probe's step 16 confirms the round trip before
//!   `Trash::Server` is granted, and until then deletes go to CraftCloud's own trash;
//! - versions are `/remote.php/dav/meta/<file id>/v/<seconds>`, by id, so they follow renames, and
//!   the current file isn't among them; a restore is a `COPY` of the version over the file, which
//!   the server keeps as a version itself; step 17 confirms an overwrite leaves one more version
//!   before `Versions::Server` is granted, and only then are `keep_version` and `move_versions`
//!   no-ops;
//! - the quota is WebDAV's `quota-available-bytes` on the base, `-3` meaning no limit;
//! - the server's version string is `status.php`'s.
//!
//! And one thing it asks the driver to do (§ 16, Q13): verify every write by SHA1. ownCloud 10.15
//! keeps a file's version tag when it is overwritten within the same second as the write before
//! it (and the version slot named by that second keeps only the later bytes), so `If-Match` can't
//! tell two writes a moment apart; the driver sends `OC-Checksum` on every `PUT` (which ownCloud
//! checks against the bytes received) and compares the server's `oc:checksums` afterwards, and a
//! mismatch is a conflict copy, never a silent overwrite.
//!
//! Sign-in is an app password the person makes in Settings › Security (§ 10); there is no browser
//! flow a third-party client can use without an admin. Never: `Depth: infinity` (412 there, Q4).

use craftcloud_core::api::dav;
use craftcloud_core::api::owncloud as oc;
use craftcloud_core::api::versions::Version;
use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
use craftcloud_core::names;
use craftcloud_core::storage::Checksum;
use craftcloud_core::storage::probe::{Base, ExtraCheck, Learnt, PROPFIND_BODY, Request, Step, parse_listing};
use craftcloud_core::sync::Storage;

use super::ServerProfile;
pub use super::nextcloud::user_id;
use crate::storage::probe::Transport;
use crate::storage::{StorageError, TrashItem};

/// An ownCloud server, by its origin and the signed-in user's id.
#[derive(Clone, Debug)]
pub struct Owncloud {
    origin: String,
    user: String,
}

impl Owncloud {
    pub fn new(origin: &str, user: &str) -> Self {
        Self { origin: origin.trim_end_matches('/').to_owned(), user: user.to_owned() }
    }

    /// From the WebDAV base the settings hold (`https://cloud.example.org/remote.php/dav/files/john/`);
    /// `None` for a base that isn't an ownCloud files base.
    pub fn from_base(base: &Base) -> Option<Self> {
        let (origin, user) = oc::split_files_base(base)?;
        Some(Self::new(&origin, &user))
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// The files base: `<origin>/remote.php/dav/files/<user id>/`.
    pub fn files_base(&self) -> String {
        oc::files_base(&self.origin, &self.user)
    }

    fn trash_root(&self) -> String {
        oc::trash_root(&self.origin, &self.user)
    }

    fn meta_root(&self, file_id: &str) -> String {
        oc::meta_root(&self.origin, file_id)
    }

    /// A file's id (a depth-0 `PROPFIND`); not found when it isn't there.
    fn file_id(&self, transport: &mut dyn Transport, base: &Base, path: &str) -> Result<String, StorageError> {
        let req = HttpRequest::new(Method::Propfind, dav::url_under(base.url(), path, false))
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec());
        let resp = transport.call(&req)?;
        if resp.status == 404 {
            return Err(StorageError::NotFound(format!("{path}: no such file")));
        }
        let listed = parse_listing(&resp, base).map_err(|e| format!("listing {path:?}: {e}"))?;
        let entry = listed.into_iter().find(|e| e.path == path.trim_matches('/')).ok_or_else(|| format!("{path}: not in its own listing"))?;
        entry.file_id.ok_or_else(|| StorageError::Failed(format!("{path}: the server lists no file id")))
    }

    /// The trash bin's items, as the server lists them.
    fn trashed(&self, transport: &mut dyn Transport) -> Result<Vec<oc::Trashed>, StorageError> {
        let root = Base::parse(&self.trash_root())?;
        let resp = transport.call(&oc::trash_list(root.url()))?;
        if resp.status == 404 {
            return Ok(Vec::new());
        }
        Ok(oc::parse_trash(&resp, &root)?)
    }

    /// Makes `folder` and its missing parents under the files base (`MKCOL`, parent first).
    fn make_folders(&self, transport: &mut dyn Transport, folder: &str) -> Result<(), StorageError> {
        let base = self.files_base();
        let parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
        for n in 1..=parts.len() {
            let step = parts[..n].join("/");
            let resp = transport.call(&dav::create_folder_under(&base, &step))?;
            if !dav::folder_exists_after(&resp) {
                return Err(StorageError::Failed(format!("making the folder {step}: the server answered {}", resp.status)));
            }
        }
        Ok(())
    }
}

/// Whether the server at `origin` says it is ownCloud (`status.php`), and which version. Only a
/// suggestion of a kind: nothing here grants a capability (§ 2). No sign-in needed.
pub fn detect(origin: &str) -> Option<oc::Detected> {
    let resp = crate::http::execute(&oc::status(origin)).ok()?;
    oc::parse_status(&resp).filter(oc::Detected::is_owncloud)
}

/// Where the person makes (and revokes) the app password the client signs in with.
pub const APP_PASSWORD_WHERE: &str = "Settings › Security › App passwords / tokens on the server";

impl ServerProfile for Owncloud {
    fn kind(&self) -> Storage {
        Storage::Owncloud
    }

    fn version(&self, transport: &mut dyn Transport) -> Option<String> {
        let resp = transport.call(&oc::status(&self.origin)).ok()?;
        oc::parse_status(&resp).map(|d| d.version)
    }

    fn probe_checks(&self) -> Option<(Box<dyn ExtraCheck>, Box<dyn ExtraCheck>)> {
        Some((Box::new(TrashCheck::new(&self.trash_root())), Box::new(VersionsCheck::new(&self.origin))))
    }

    fn verify_writes(&self) -> Option<Checksum> {
        Some(Checksum::Sha1)
    }

    fn trash_list(&self, transport: &mut dyn Transport) -> Result<Vec<TrashItem>, StorageError> {
        Ok(self
            .trashed(transport)?
            .into_iter()
            .map(|t| TrashItem {
                id: t.name,
                path: t.original_location,
                trashed_at: t
                    .deleted_at
                    .and_then(|s| jiff::Timestamp::from_second(s).ok())
                    .map(|at| at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
                    .unwrap_or_default(),
            })
            .collect())
    }

    /// `MOVE` the item to `item.path` under the files base (where it was, or where `--as` says),
    /// never onto an existing name: a taken name gets `names::restored_name`. The parents are made
    /// first (the server answers 409 without them). A fresh listing of the trash bin says whether
    /// the item is a folder and that it is still there.
    fn restore(&self, transport: &mut dyn Transport, item: &TrashItem) -> Result<(), StorageError> {
        let trashed = self.trashed(transport)?;
        let Some(t) = trashed.iter().find(|t| t.name == item.id) else {
            return Err(StorageError::NotFound(format!("{} isn't in the trash", item.path)));
        };
        if let Some((parent, _)) = item.path.rsplit_once('/') {
            self.make_folders(transport, parent)?;
        }
        let (root, files) = (self.trash_root(), self.files_base());
        let mut dest = item.path.clone();
        for n in 1..100 {
            let resp = transport.call(&oc::trash_restore(&root, &t.name, t.is_folder, &files, &dest))?;
            match resp.status {
                412 => dest = names::restored_name(&item.path, t.is_folder, n),
                404 => return Err(StorageError::NotFound(format!("{} isn't in the trash", item.path))),
                _ if resp.is_success() => return Ok(()),
                s => return Err(StorageError::Failed(format!("restoring {}: the server answered {s}", item.path))),
            }
        }
        Err(StorageError::Failed(format!("restoring {}: every name next to it is taken; it stays in the trash", item.path)))
    }

    fn list_versions(&self, transport: &mut dyn Transport, base: &Base, path: &str) -> Result<Vec<Version>, StorageError> {
        let id = match self.file_id(transport, base, path) {
            // A file that isn't there has no versions to list (the server keeps them by id).
            Err(StorageError::NotFound(_)) => return Ok(Vec::new()),
            other => other?,
        };
        let root = Base::parse(&self.meta_root(&id))?;
        let resp = transport.call(&oc::versions_list(root.url()))?;
        Ok(oc::parse_versions(&resp, &root)?)
    }

    /// `COPY` the version over the file; the server keeps the current file as a version itself.
    /// Only a stamp a fresh listing names: the versions collection is never copied by mistake.
    fn restore_version(&self, transport: &mut dyn Transport, base: &Base, path: &str, stamp: &str) -> Result<(), StorageError> {
        let versions = self.list_versions(transport, base, path)?;
        if !versions.iter().any(|v| v.stamp == stamp) {
            return Err(StorageError::NotFound(format!("{path} has no version {stamp}")));
        }
        let id = self.file_id(transport, base, path)?;
        let resp = transport.call(&oc::version_restore(&self.meta_root(&id), stamp, base.url(), path))?;
        if resp.is_success() {
            Ok(())
        } else {
            Err(StorageError::Failed(format!("restoring version {stamp} of {path}: the server answered {}", resp.status)))
        }
    }

    /// WebDAV's `quota-available-bytes` on the base, `-3` read as no limit.
    fn free_space(&self, transport: &mut dyn Transport) -> Option<Result<Option<u64>, StorageError>> {
        let files = self.files_base();
        let req = HttpRequest::new(Method::Propfind, files.clone())
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec());
        let mut read = || -> Result<Option<u64>, StorageError> {
            let base = Base::parse(&files)?;
            let resp = transport.call(&req)?;
            let listed = parse_listing(&resp, &base).map_err(|e| format!("reading the quota: {e}"))?;
            Ok(oc::quota_free(listed.into_iter().find(|e| e.path.is_empty()).and_then(|e| e.quota_available)))
        };
        Some(read())
    }
}

/// Step 16, the trash round trip: `DELETE` the probe's `d.txt` (step 13's copy of `c.txt`), find
/// it in the trash bin by its original location, `MOVE` it back to exactly that path under the
/// files base (`Overwrite: F`), and `GET` it with its bytes. Confirmed only when every step holds.
struct TrashCheck {
    root: String,
    stage: u8,
    /// The deleted file's id in the trash bin, once found.
    id: Option<String>,
    confirmed: bool,
}

impl TrashCheck {
    fn new(root: &str) -> Self {
        Self { root: root.to_owned(), stage: 0, id: None, confirmed: false }
    }
}

impl ExtraCheck for TrashCheck {
    fn next(&self, learnt: &Learnt) -> Option<Request> {
        if !learnt.d_exists {
            return None;
        }
        let d = format!("{}/d.txt", learnt.folder);
        Some(match self.stage {
            0 => Request::new(Step::ServerTrash, Method::Delete, &d, false),
            1 => Request::under_root(Step::ServerTrash, Method::Propfind, &self.root, "", true)
                .with_header("Depth", "1")
                .with_header("Content-Type", "application/xml; charset=utf-8")
                .with_body(oc::TRASH_PROPFIND_BODY.as_bytes()),
            2 => {
                let id = self.id.as_deref()?;
                let mut req = Request::under_root(Step::ServerTrash, Method::Move, &self.root, id, false).with_header("Overwrite", "F");
                req.destination = Some((d.clone(), false));
                req.destination_under_base = true;
                req.restores = Some(d);
                req
            }
            3 => Request::new(Step::ServerTrash, Method::Get, &d, false),
            _ => return None,
        })
    }

    fn observe(&mut self, resp: &HttpResponse, learnt: &Learnt) {
        let done = |s: &mut Self| s.stage = 9;
        match self.stage {
            0 if resp.is_success() => self.stage = 1,
            1 => {
                let wanted = format!("{}/d.txt", learnt.folder);
                let found = Base::parse(&self.root)
                    .ok()
                    .and_then(|root| oc::parse_trash(resp, &root).ok())
                    .and_then(|items| items.into_iter().find(|t| !t.is_folder && t.original_location == wanted).map(|t| t.name));
                match found {
                    Some(id) => {
                        self.id = Some(id);
                        self.stage = 2;
                    }
                    None => done(self),
                }
            }
            2 if resp.is_success() => self.stage = 3,
            3 => {
                self.confirmed = resp.is_success() && resp.body == learnt.c_bytes;
                done(self);
            }
            _ => done(self),
        }
    }

    fn confirmed(&self) -> bool {
        self.confirmed
    }
}

/// Step 17, the server's versions: list `c.txt`'s versions by file id, overwrite it once more (in
/// a later second), list again; confirmed when the listing grew by one. The current file isn't in
/// ownCloud's listing, so the count before is needed to show this overwrite left a version.
struct VersionsCheck {
    origin: String,
    stage: u8,
    before: usize,
    confirmed: bool,
}

impl VersionsCheck {
    fn new(origin: &str) -> Self {
        Self { origin: origin.to_owned(), stage: 0, before: 0, confirmed: false }
    }

    fn listing(&self, id: &str) -> Request {
        Request::under_root(Step::ServerVersions, Method::Propfind, &oc::meta_root(&self.origin, id), "v", true)
            .with_header("Depth", "1")
            .with_header("Content-Type", "application/xml; charset=utf-8")
            .with_body(PROPFIND_BODY.as_bytes())
    }

    fn count(&self, resp: &HttpResponse, id: &str) -> Option<usize> {
        let root = Base::parse(&oc::meta_root(&self.origin, id)).ok()?;
        oc::parse_versions(resp, &root).ok().map(|v| v.len())
    }
}

impl ExtraCheck for VersionsCheck {
    fn next(&self, learnt: &Learnt) -> Option<Request> {
        if !learnt.c_exists {
            return None;
        }
        let id = learnt.file_id.as_deref()?;
        let etag = learnt.etag.as_deref()?;
        Some(match self.stage {
            0 => self.listing(id),
            1 => Request {
                settles: true,
                ..Request::new(Step::ServerVersions, Method::Put, &format!("{}/c.txt", learnt.folder), false)
                    .with_header("If-Match", etag)
                    .with_body(b"four")
            },
            2 => self.listing(id),
            _ => return None,
        })
    }

    fn observe(&mut self, resp: &HttpResponse, learnt: &Learnt) {
        let id = learnt.file_id.clone().unwrap_or_default();
        match self.stage {
            0 => match self.count(resp, &id) {
                Some(n) => {
                    self.before = n;
                    self.stage = 1;
                }
                None => self.stage = 9,
            },
            1 if resp.is_success() => self.stage = 2,
            2 => {
                self.confirmed = self.count(resp, &id).is_some_and(|after| after > self.before);
                self.stage = 9;
            }
            _ => self.stage = 9,
        }
    }

    fn confirmed(&self) -> bool {
        self.confirmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_read_from_the_files_base() {
        let p = Owncloud::from_base(&Base::parse("https://cloud.example.org/remote.php/dav/files/john/").unwrap()).unwrap();
        assert_eq!((p.origin(), p.user()), ("https://cloud.example.org", "john"));
        assert_eq!(p.files_base(), "https://cloud.example.org/remote.php/dav/files/john/");
        assert_eq!(p.trash_root(), "https://cloud.example.org/remote.php/dav/trash-bin/john/");
        assert_eq!(p.meta_root("1660"), "https://cloud.example.org/remote.php/dav/meta/1660/");
        let p = Owncloud::from_base(&Base::parse("https://example.org/cloud/remote.php/dav/files/jane%20doe/").unwrap()).unwrap();
        assert_eq!((p.origin(), p.user()), ("https://example.org/cloud", "jane doe"));
        assert_eq!(p.trash_root(), "https://example.org/cloud/remote.php/dav/trash-bin/jane%20doe/");
        assert!(Owncloud::from_base(&Base::parse("https://nas.example.org/dav/").unwrap()).is_none());
        assert_eq!(p.kind(), Storage::Owncloud);
        assert_eq!(p.verify_writes(), Some(Checksum::Sha1));
    }

    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use craftcloud_core::storage::probe::fake::{FakeDav, Flags};
    use craftcloud_core::storage::{MissingParent, Mode, Trash, Versions};

    use crate::fake_server::{Answer, FakeServer, Seen};
    use crate::session::AppPasswordSession;
    use crate::storage::probe::Limits;
    use crate::storage::webdav::WebDav;
    use crate::storage::webdav::tests::two_way;
    use crate::storage::{MoveGuard, Moved, RemoteStorage, TrashGuard, Trashed, Written};

    const BASE_PATH: &str = "/remote.php/dav/files/john";

    /// An ownCloud-shaped scripted server behind a loopback HTTP server, with the flags given on top.
    fn owncloud(flags: Flags) -> (FakeServer, Arc<Mutex<FakeDav>>) {
        scripted(Flags { owncloud: true, ..flags })
    }

    /// A scripted server under a Nextcloud-shaped base, as the flags say.
    fn scripted(flags: Flags) -> (FakeServer, Arc<Mutex<FakeDav>>) {
        let dav = Arc::new(Mutex::new(FakeDav::new(BASE_PATH, flags)));
        let shared = Arc::clone(&dav);
        let server = FakeServer::start_full(move |seen: &Seen| {
            let mut dav = shared.lock().unwrap_or_else(PoisonError::into_inner);
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer
        });
        dav.lock().unwrap().origin = server.url.clone();
        (server, dav)
    }

    fn session(server: &FakeServer) -> AppPasswordSession {
        AppPasswordSession::new(Base::parse(&format!("{}{BASE_PATH}/", server.url)).unwrap(), "john", "SECRET-app-pw").unwrap()
    }

    fn profile(server: &FakeServer) -> Box<dyn ServerProfile> {
        Box::new(Owncloud::new(&server.url, "john"))
    }

    /// Capabilities as the check measures them on an ownCloud: the server's trash and versions.
    fn owncloud_caps() -> craftcloud_core::storage::Capabilities {
        craftcloud_core::storage::Capabilities { trash: Trash::Server, versions: Versions::Server, ..two_way() }
    }

    fn driver(server: &FakeServer) -> WebDav {
        WebDav::new(session(server), owncloud_caps(), "Laptop").with_profile(profile(server))
    }

    fn quick() -> Limits {
        Limits { settle: Duration::ZERO, ..Limits::default() }
    }

    fn file(dir: &std::path::Path, name: &str, bytes: &str) -> std::path::PathBuf {
        let file = dir.join(name);
        std::fs::write(&file, bytes).unwrap();
        file
    }

    /// Steps 16 and 17 with the profile: the probe deletes its `d.txt`, finds it in the trash bin
    /// by its original location, moves it back to exactly that path (under the files base, with
    /// `Overwrite: F`) and reads it back; lists `c.txt`'s versions, overwrites it and finds one
    /// more. Only then are `Trash::Server` and `Versions::Server` granted; nothing is ever deleted
    /// on the trash bin's root (§ 16, Q8), and the probe folder ends in the server's trash. The
    /// version string is `status.php`'s, a missing parent answers 409, `Depth: infinity` 412.
    #[test]
    fn the_probe_grants_the_servers_trash_and_versions_only_after_the_round_trips() {
        let (server, dav) = owncloud(Flags::default());
        let dir = tempfile::tempdir().unwrap();
        let (connected, check) = WebDav::connect_with(session(&server), profile(&server), "Laptop", Some(dir.path()), quick());
        assert_eq!(check.mode, Mode::TwoWay, "{check:#?}");
        let caps = check.capabilities.clone().unwrap();
        assert_eq!((caps.trash, caps.versions), (Trash::Server, Versions::Server), "{:#?}", check.steps);
        assert_eq!(caps.missing_parent, MissingParent::Conflict);
        assert!(caps.file_ids && !caps.depth_infinity);
        assert_eq!(caps.checksums, Some(Checksum::Sha1), "listed for every file");
        assert_eq!(check.server_version, "10.15.3.0");
        assert_eq!(connected.profile().kind(), Storage::Owncloud);
        let trash_step = check.steps.iter().find(|s| s.number == 16).unwrap();
        assert!(trash_step.result.contains("came back whole"), "{trash_step:?}");
        let versions_step = check.steps.iter().find(|s| s.number == 17).unwrap();
        assert!(versions_step.result.contains("one more version"), "{versions_step:?}");
        let seen = server.seen();
        let on_trash: Vec<&Seen> = seen.iter().filter(|r| r.path.contains("/trash-bin/")).collect();
        assert_eq!(on_trash.len(), 2, "one listing and one restore: {on_trash:?}");
        assert_eq!((on_trash[0].method.as_str(), on_trash[1].method.as_str()), ("PROPFIND", "MOVE"));
        let dest = on_trash[1].header("destination").unwrap();
        assert!(dest.contains(&format!("{BASE_PATH}/.craftcloud/probe/")) && dest.ends_with("/d.txt"), "back to where it was: {dest}");
        assert_eq!(on_trash[1].header("overwrite"), Some("F"));
        assert!(seen.iter().all(|r| !(r.method == "DELETE" && !r.path.starts_with(BASE_PATH))), "a DELETE outside the files base: {seen:?}");
        assert_eq!(seen.iter().filter(|r| r.method == "PROPFIND" && r.path.contains("/remote.php/dav/meta/")).count(), 2, "before and after");
        let dav = dav.lock().unwrap();
        assert_eq!(dav.trash_purged, 0, "the person's trash is never purged");
        assert!(dav.trashed().iter().any(|(_, from)| from.starts_with(".craftcloud/probe/")), "the probe folder is in the server's trash");
        drop(dav);

        // Without the profile the same server gets CraftCloud's own trash and versions.
        let (generic, check) = WebDav::connect(session(&server), "Laptop", None, quick());
        assert_eq!((check.capabilities.as_ref().unwrap().trash, check.capabilities.as_ref().unwrap().versions), (Trash::Client, Versions::Client));
        assert_eq!(generic.profile().kind(), Storage::Webdav);

        // The profile on a server that has no trash bin: the round trip fails, nothing is granted.
        let (plain, _) = scripted(Flags::default());
        let (_, check) = WebDav::connect_with(session(&plain), profile(&plain), "Laptop", None, quick());
        assert_eq!(check.mode, Mode::TwoWay);
        let caps = check.capabilities.unwrap();
        assert_eq!((caps.trash, caps.versions), (Trash::Client, Versions::Client), "{:#?}", check.steps);
    }

    /// Q13: every `PUT` carries `OC-Checksum`, and the listing afterwards must show that checksum.
    /// ownCloud lists a checksum for every file (its own, computed), so another client's write
    /// right behind ours within the second shows up as a checksum that isn't ours: without the
    /// check the write is taken as ours (and later overwritten); with it, a conflict.
    #[test]
    fn a_write_landing_under_our_tag_within_the_second_is_a_conflict_not_a_silent_overwrite() {
        let flags = Flags { etag_reused_within_second: true, other_writes_after_put: Some(b"theirs".to_vec()), ..Flags::default() };
        let (server, dav) = owncloud(flags.clone());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "mine");
        // Without the check (the generic profile): the write "succeeds", the server holds theirs.
        let mut unverified = WebDav::new(session(&server), owncloud_caps(), "Laptop");
        assert!(matches!(unverified.create("Docs/a.txt", &a).unwrap(), Written::Saved { .. }), "the loss the check prevents");
        assert_eq!(dav.lock().unwrap().body("Docs/a.txt"), Some(b"theirs".as_slice()));
        // With it: a conflict, and the next write too.
        let mut verified = driver(&server);
        dav.lock().unwrap().add_file("Docs/b.txt", b"old");
        let etag = verified.list("Docs").unwrap().into_iter().find(|e| e.path == "Docs/b.txt").unwrap().etag.unwrap();
        assert_eq!(verified.replace("Docs/b.txt", &etag, &a).unwrap(), Written::Conflict);
        assert_eq!(verified.create("Docs/c.txt", &a).unwrap(), Written::Conflict);
        let sha1 = "SHA1:8034c43d15bb7ef42e2cc24255217acfc5f4b2f4";
        let checksums: Vec<Option<String>> =
            server.seen().iter().filter(|r| r.method == "PUT").map(|r| r.header("oc-checksum").map(str::to_owned)).collect();
        assert_eq!(checksums, [None, Some(sha1.to_owned()), Some(sha1.to_owned())], "the unverified driver sends none, the verified one SHA1");
        // On a well-behaved server the same writes are saved: the server lists our checksum.
        let (calm, _) = owncloud(Flags::default());
        let mut s = WebDav::new(session(&calm), owncloud_caps(), "Laptop").with_profile(profile(&calm));
        s.make_folder("Docs").unwrap();
        let Written::Saved { etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!("not saved") };
        std::fs::write(&a, "mine too").unwrap();
        assert!(matches!(s.replace("Docs/a.txt", &etag, &a).unwrap(), Written::Saved { .. }));
        assert_eq!(calm.seen().iter().filter(|r| r.method == "PROPFIND" && r.path.ends_with("/Docs/a.txt")).count(), 2, "one read-back per write");
    }

    /// A delete goes into the server's trash by `DELETE` (a file's with `If-Match`; a stale tag
    /// keeps it), the trash lists it by its original path with the file id as its handle, a
    /// restore is a `MOVE` back to that path under the files base (`Overwrite: F`; a taken name
    /// gets "(restored)"), a folder goes and comes back whole; the client trash's sweep sends
    /// nothing.
    #[test]
    fn deletes_land_in_the_servers_trash_and_come_back_from_it() {
        let (server, dav) = owncloud(Flags::default());
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder("Docs/Sub", None);
            dav.add_file("Docs/a.txt", b"abc");
            dav.add_file("Docs/Sub/b.txt", b"b");
        }
        let mut s = driver(&server);
        let etag = s.list("Docs").unwrap().into_iter().find(|e| e.path == "Docs/a.txt").unwrap().etag.unwrap();
        assert_eq!(s.trash("Docs/a.txt", TrashGuard::File { etag: "\"stale\"".into() }).unwrap(), Trashed::Changed);
        assert!(dav.lock().unwrap().has("Docs/a.txt"), "a stale tag keeps it");
        assert_eq!(s.trash("Docs/a.txt", TrashGuard::File { etag }).unwrap(), Trashed::Done);
        let b = s.list("Docs/Sub").unwrap().into_iter().next().unwrap().etag.unwrap();
        assert_eq!(s.trash("Docs/Sub", TrashGuard::Folder { files: vec![("Docs/Sub/b.txt".into(), b)] }).unwrap(), Trashed::Done);
        {
            let dav = dav.lock().unwrap();
            assert!(!dav.has("Docs/a.txt") && !dav.has("Docs/Sub"));
            assert_eq!(dav.trashed().iter().map(|(_, from)| from.as_str()).collect::<Vec<_>>(), ["Docs/a.txt", "Docs/Sub"]);
            assert!(dav.folders().iter().all(|f| !f.starts_with(".craftcloud/trash")), "nothing in CraftCloud's own trash");
        }
        let deletes: Vec<String> = server.seen().iter().filter(|r| r.method == "DELETE").map(|r| r.path.clone()).collect();
        assert_eq!(deletes.len(), 2, "the stale one never reached the server: {deletes:?}");
        assert!(server.seen().iter().filter(|r| r.method == "DELETE" && r.path.ends_with("/Docs/a.txt")).all(|r| r.header("if-match").is_some()));
        let items = s.trash_list().unwrap();
        assert_eq!(items.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["Docs/a.txt", "Docs/Sub"]);
        assert!(items[0].id.bytes().all(|b| b.is_ascii_digit()) && items[0].trashed_at.starts_with("2026-"), "{:?}", items[0]);
        // The place is taken meanwhile: the restored file goes next to it, nothing overwritten.
        dav.lock().unwrap().add_file("Docs/a.txt", b"newer");
        s.restore(&items[0]).unwrap();
        s.restore(&items[1]).unwrap();
        {
            let dav_now = dav.lock().unwrap();
            assert_eq!(dav_now.body("Docs/a.txt"), Some(b"newer".as_slice()), "the newer file stays");
            assert_eq!(dav_now.body("Docs/a (restored).txt"), Some(b"abc".as_slice()), "{:?}", dav_now.folders());
            assert_eq!(dav_now.body("Docs/Sub/b.txt"), Some(b"b".as_slice()));
            assert!(dav_now.trashed().is_empty());
        }
        assert!(matches!(s.restore(&items[0]).unwrap_err(), StorageError::NotFound(_)), "gone from the trash");
        let before = server.seen().len();
        s.sweep_trash().unwrap();
        assert_eq!(server.seen().len(), before, "the server keeps its own trash: nothing swept");
        let on_trash: Vec<String> = server.seen().iter().filter(|r| r.path.contains("/trash-bin/")).map(|r| r.method.clone()).collect();
        assert!(on_trash.iter().all(|m| m == "PROPFIND" || m == "MOVE"), "never a DELETE on the trash bin: {on_trash:?}");
        let moves: Vec<Seen> = server.seen().into_iter().filter(|r| r.method == "MOVE").collect();
        assert!(
            moves.iter().all(|r| r.header("overwrite") == Some("F") && r.header("destination").is_some_and(|d| d.contains(BASE_PATH))),
            "{moves:?}"
        );
        assert_eq!(moves.len(), 3, "the taken name once, then the free one, then the folder: {moves:?}");
    }

    /// With the server's versions confirmed, `keep_version` and `move_versions` send nothing; the
    /// versions are listed by file id (the current file isn't among them), follow a rename, and one
    /// is restored by a `COPY` over the file, which the server keeps as a version itself. With the
    /// client's versions (unconfirmed) a rename moves the history.
    #[test]
    fn versions_are_the_servers_and_follow_renames_by_id() {
        let (server, dav) = owncloud(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "v0");
        let mut s = driver(&server);
        let Written::Saved { mut etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!() };
        assert!(s.list_versions("Docs/a.txt").unwrap().is_empty(), "a new file has none");
        for i in 1..=2 {
            s.keep_version("Docs/a.txt").unwrap();
            std::fs::write(&a, format!("v{i}")).unwrap();
            let Written::Saved { etag: new } = s.replace("Docs/a.txt", &etag, &a).unwrap() else { panic!("{i}") };
            etag = new;
        }
        assert!(server.seen().iter().all(|r| r.method != "COPY"), "no client versions: {:?}", server.seen());
        let versions = s.list_versions("Docs/a.txt").unwrap();
        assert_eq!(versions.len(), 2, "{versions:#?}");
        assert!(versions[0].stamp > versions[1].stamp, "newest first");
        assert_eq!(s.move_to("Docs/a.txt", "Docs/b.txt", MoveGuard::File { etag }).unwrap(), Moved::Done);
        let before = server.seen().len();
        s.move_versions("Docs/a.txt", "Docs/b.txt").unwrap();
        assert_eq!(server.seen().len(), before, "a no-op: the server keeps them by id");
        assert_eq!(s.list_versions("Docs/b.txt").unwrap().len(), 2, "the history followed the rename");
        s.restore_version("Docs/b.txt", &versions[1].stamp).unwrap();
        assert_eq!(dav.lock().unwrap().body("Docs/b.txt"), Some(b"v0".as_slice()));
        assert_eq!(s.list_versions("Docs/b.txt").unwrap().len(), 3, "the replaced file was kept by the server");
        let copy = server.seen().into_iter().find(|r| r.method == "COPY").unwrap();
        assert!(copy.path.contains("/remote.php/dav/meta/") && copy.header("destination").is_some_and(|d| d.ends_with("/Docs/b.txt")), "{copy:?}");
        assert!(matches!(s.restore_version("Docs/b.txt", "1").unwrap_err(), StorageError::NotFound(_)), "never a stamp the listing doesn't name");
        // Unconfirmed (client versions): the same server, the history is CraftCloud's and moves by MOVE.
        let mut client = WebDav::new(session(&server), two_way(), "Laptop").with_profile(profile(&server));
        client.keep_version("Docs/b.txt").unwrap();
        assert!(server.seen().iter().any(|r| r.method == "COPY" && r.path.contains(BASE_PATH)));
        client.move_versions("Docs/b.txt", "Docs/c.txt").unwrap();
        assert!(server.seen().iter().any(|r| r.method == "MOVE" && r.path.contains("/.craftcloud/versions/Docs/b.txt")));
    }

    /// Free space is WebDAV's `quota-available-bytes` on the base: a count, or no limit (`-3`).
    #[test]
    fn free_space_is_the_quota_property_with_no_limit_read_right() {
        for (quota, expected) in [(Some(5_000), Some(5_000)), (Some(-3), Some(u64::MAX)), (None, None), (Some(-1), None)] {
            let (server, _) = owncloud(Flags { quota_available: quota, ..Flags::default() });
            assert_eq!(driver(&server).free_space(1).unwrap(), expected, "{quota:?}");
            assert!(server.seen().iter().all(|r| r.method == "PROPFIND" && r.path == format!("{BASE_PATH}/")), "{:?}", server.seen());
        }
    }

    /// Detection by `status.php`, and the user id by `current-user-principal`.
    #[test]
    fn detection_and_the_user_id() {
        let (server, _) = owncloud(Flags::default());
        let d = detect(&server.url).unwrap();
        assert_eq!((d.product.as_str(), d.version.as_str(), d.version_string.as_str()), ("ownCloud", "10.15.3.0", "10.15.3"));
        assert!(detect(&scripted(Flags::default()).0.url).is_none(), "a plain WebDAV server isn't ownCloud");
        let (nc, _) = scripted(Flags { nextcloud: true, ..Flags::default() });
        assert!(detect(&nc.url).is_none(), "a Nextcloud isn't ownCloud");
        assert_eq!(user_id(&mut session(&server), &server.url).unwrap(), "john");
    }
}
