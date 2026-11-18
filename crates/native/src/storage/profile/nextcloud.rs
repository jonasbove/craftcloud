//! The Nextcloud profile (docs/WEBDAV-STORAGE.md § 7.4, the Nextcloud column; the requests and
//! parsers are `craftcloud_core::api::nextcloud`'s). It knows where things are:
//!
//! - the WebDAV base is `/remote.php/dav/files/<user id>/`, the user id from `current-user-principal`
//!   (the login name can differ);
//! - the trash bin is `/remote.php/dav/trashbin/<user id>/trash/`, a restore a `MOVE` to
//!   `…/restore/<name>`; the probe's step 16 confirms it with a round trip before `Trash::Server` is
//!   granted, and until then deletes go to CraftCloud's own trash;
//! - versions are `/remote.php/dav/versions/<user id>/versions/<file id>/`, by id, so they follow
//!   renames; step 17 confirms an overwrite leaves a version before `Versions::Server` is granted,
//!   and only then are `keep_version` and `move_versions` no-ops; a version is restored by reading
//!   its bytes and writing them back with the driver's guarded `replace`, never with the server's
//!   own restore `MOVE` (which names no version of the file it replaces);
//! - the quota is the OCS user endpoint's (WebDAV says `-3`);
//! - the server's version string is `status.php`'s.
//!
//! And one thing it asks the driver to do (§ 16, Q13): verify every write by SHA1. Nextcloud 31
//! keeps a file's version tag when it is overwritten within the same second as the write before
//! it, so `If-Match` can't tell two writes a moment apart; the driver sends `OC-Checksum` on every
//! `PUT` and compares the server's `oc:checksums` afterwards, and a mismatch is a conflict copy,
//! never a silent overwrite.
//!
//! Never: bulk upload, chunked upload (Q7), `Depth: infinity` for the sync (Q4).

use craftcloud_core::api::nextcloud as nc;
use craftcloud_core::api::versions::Version;
use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
use craftcloud_core::storage::Checksum;
use craftcloud_core::storage::probe::{Base, ExtraCheck, Learnt, PROPFIND_BODY, Request, Step, parse_listing};
use craftcloud_core::sync::Storage;

use super::ServerProfile;
use crate::storage::probe::Transport;
use crate::storage::{StorageError, TrashItem};

/// A Nextcloud server, by its origin and the signed-in user's id.
#[derive(Clone, Debug)]
pub struct Nextcloud {
    origin: String,
    user: String,
}

impl Nextcloud {
    pub fn new(origin: &str, user: &str) -> Self {
        Self { origin: origin.trim_end_matches('/').to_owned(), user: user.to_owned() }
    }

    /// From the WebDAV base the settings hold (`https://cloud.example.org/remote.php/dav/files/john/`);
    /// `None` for a base that isn't a Nextcloud files base.
    pub fn from_base(base: &Base) -> Option<Self> {
        let (origin, user) = nc::split_files_base(base)?;
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
        nc::files_base(&self.origin, &self.user)
    }

    fn trashbin(&self) -> String {
        nc::trashbin_root(&self.origin, &self.user)
    }

    fn versions(&self) -> String {
        nc::versions_root(&self.origin, &self.user)
    }

    /// A file's id and its modified time in seconds (a depth-0 `PROPFIND`); not found when it
    /// isn't there.
    fn file_id(&self, transport: &mut dyn Transport, base: &Base, path: &str) -> Result<(String, Option<i64>), StorageError> {
        let req = HttpRequest::new(Method::Propfind, craftcloud_core::api::dav::url_under(base.url(), path, false))
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec());
        let resp = transport.call(&req)?;
        if resp.status == 404 {
            return Err(StorageError::NotFound(format!("{path}: no such file")));
        }
        let listed = parse_listing(&resp, base).map_err(|e| format!("listing {path:?}: {e}"))?;
        let entry = listed.into_iter().find(|e| e.path == path.trim_matches('/')).ok_or_else(|| format!("{path}: not in its own listing"))?;
        let id = entry.file_id.ok_or_else(|| format!("{path}: the server lists no file id"))?;
        Ok((id, entry.modified.as_deref().and_then(craftcloud_core::storage::probe::server_seconds)))
    }
}

/// The user id the WebDAV paths use (`current-user-principal`), asked with the sign-in.
pub fn user_id(transport: &mut dyn Transport, origin: &str) -> Result<String, String> {
    let resp = transport.call(&nc::current_user_principal(origin))?;
    nc::parse_current_user_principal(&resp)
}

/// Whether the server at `origin` says it is Nextcloud (`status.php`), and which version. Only a
/// suggestion of a kind: nothing here grants a capability (§ 2). No sign-in needed.
pub fn detect(origin: &str) -> Option<nc::Detected> {
    let resp = crate::http::execute(&nc::status(origin)).ok()?;
    nc::parse_status(&resp).filter(nc::Detected::is_nextcloud)
}

/// Signs in through the browser with Login Flow v2 (docs/WEBDAV-STORAGE.md § 10): starts the flow,
/// hands the login address to `open` (the browser, or the terminal), and polls every `poll_every`
/// until the person has finished or `give_up_after` has passed. The app password the server made
/// for this device comes back with the login name; the client never sees the account password.
/// `user_agent` names this client and device, which is how the server lists the app password.
pub fn sign_in(
    origin: &str,
    user_agent: &str,
    open: &mut dyn FnMut(&str),
    poll_every: std::time::Duration,
    give_up_after: std::time::Duration,
) -> Result<nc::AppPasswordGrant, String> {
    let started = crate::http::execute(&nc::login_flow_start(origin, user_agent))?;
    let flow = nc::parse_login_flow_start(&started).map_err(|e| format!("starting the sign-in: {e}"))?;
    open(&flow.login_url);
    let deadline = std::time::Instant::now() + give_up_after;
    loop {
        if std::time::Instant::now() > deadline {
            return Err("the sign-in wasn't finished in the browser in time".into());
        }
        std::thread::sleep(poll_every);
        let resp = crate::http::execute(&nc::login_flow_poll(&flow, user_agent))?;
        match nc::parse_login_flow_poll(&resp).map_err(|e| format!("waiting for the sign-in: {e}"))? {
            nc::FlowPoll::Pending => {}
            nc::FlowPoll::Done(grant) => return Ok(grant),
        }
    }
}

impl ServerProfile for Nextcloud {
    fn kind(&self) -> Storage {
        Storage::Nextcloud
    }

    fn version(&self, transport: &mut dyn Transport) -> Option<String> {
        let resp = transport.call(&nc::status(&self.origin)).ok()?;
        nc::parse_status(&resp).map(|d| d.version)
    }

    fn probe_checks(&self) -> Option<(Box<dyn ExtraCheck>, Box<dyn ExtraCheck>)> {
        Some((Box::new(TrashCheck::new(&self.trashbin())), Box::new(VersionsCheck::new(&self.versions()))))
    }

    fn verify_writes(&self) -> Option<Checksum> {
        Some(Checksum::Sha1)
    }

    fn trash_list(&self, transport: &mut dyn Transport) -> Result<Vec<TrashItem>, StorageError> {
        let root = Base::parse(&self.trashbin())?;
        let resp = transport.call(&nc::trashbin_list(root.url()))?;
        if resp.status == 404 {
            return Ok(Vec::new());
        }
        let items = nc::parse_trashbin(&resp, &root)?;
        Ok(items
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

    fn restore(&self, transport: &mut dyn Transport, item: &TrashItem) -> Result<(), StorageError> {
        let resp = transport.call(&nc::trashbin_restore(&self.trashbin(), &item.id))?;
        match resp.status {
            404 => Err(StorageError::NotFound(format!("{} isn't in the trash", item.path))),
            _ if resp.is_success() => Ok(()),
            s => Err(StorageError::Failed(format!("restoring {}: the server answered {s}", item.path))),
        }
    }

    fn list_versions(&self, transport: &mut dyn Transport, base: &Base, path: &str) -> Result<Vec<Version>, StorageError> {
        let (id, modified) = match self.file_id(transport, base, path) {
            // A file that isn't there has no versions to list (the server keeps them by id).
            Err(StorageError::NotFound(_)) => return Ok(Vec::new()),
            other => other?,
        };
        let root = Base::parse(&self.versions())?;
        let resp = transport.call(&nc::versions_list(root.url(), &id))?;
        Ok(nc::parse_versions(&resp, &root, modified)?)
    }

    fn version_download(&self, transport: &mut dyn Transport, base: &Base, path: &str, stamp: &str) -> Result<HttpRequest, StorageError> {
        let versions = self.list_versions(transport, base, path)?;
        if !versions.iter().any(|v| v.stamp == stamp) {
            return Err(StorageError::NotFound(format!("{path} has no version {stamp}")));
        }
        let (id, _) = self.file_id(transport, base, path)?;
        Ok(nc::versions_download(&self.versions(), &id, stamp))
    }

    fn free_space(&self, transport: &mut dyn Transport) -> Option<Result<Option<u64>, StorageError>> {
        Some(match transport.call(&nc::ocs_user(&self.origin)) {
            Ok(resp) => nc::parse_quota_free(&resp).map_err(|e| StorageError::Failed(format!("reading the quota: {e}"))),
            Err(e) => Err(StorageError::Failed(e)),
        })
    }
}

/// Step 16, the trash round trip: `DELETE` the probe's `d.txt` (step 13's copy of `c.txt`), find
/// it in the trash bin by its original location, `MOVE` it to `restore/<name>`, and `GET` it back
/// with its bytes. Confirmed only when every step holds.
struct TrashCheck {
    root: String,
    stage: u8,
    /// The deleted file's name in the trash bin, once found.
    name: Option<String>,
    confirmed: bool,
}

impl TrashCheck {
    fn new(root: &str) -> Self {
        Self { root: root.to_owned(), stage: 0, name: None, confirmed: false }
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
            1 => Request::under_root(Step::ServerTrash, Method::Propfind, &self.root, "trash", true)
                .with_header("Depth", "1")
                .with_header("Content-Type", "application/xml; charset=utf-8")
                .with_body(nc::TRASHBIN_PROPFIND_BODY.as_bytes()),
            2 => {
                let name = self.name.as_deref()?;
                let mut req = Request::under_root(Step::ServerTrash, Method::Move, &self.root, &format!("trash/{name}"), false);
                req.destination = Some((format!("restore/{name}"), false));
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
                    .and_then(|root| nc::parse_trashbin(resp, &root).ok())
                    .and_then(|items| items.into_iter().find(|t| !t.is_folder && t.original_location == wanted).map(|t| t.name));
                match found {
                    Some(name) => {
                        self.name = Some(name);
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

/// Step 17, the server's versions: overwrite `c.txt` once more (in a later second), then list its
/// versions by file id; confirmed when the listing holds an earlier version besides the current
/// file.
struct VersionsCheck {
    root: String,
    stage: u8,
    confirmed: bool,
}

impl VersionsCheck {
    fn new(root: &str) -> Self {
        Self { root: root.to_owned(), stage: 0, confirmed: false }
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
            0 => Request {
                settles: true,
                ..Request::new(Step::ServerVersions, Method::Put, &format!("{}/c.txt", learnt.folder), false)
                    .with_header("If-Match", etag)
                    .with_body(b"four")
            },
            1 => Request::under_root(Step::ServerVersions, Method::Propfind, &self.root, &format!("versions/{id}"), true)
                .with_header("Depth", "1")
                .with_header("Content-Type", "application/xml; charset=utf-8")
                .with_body(PROPFIND_BODY.as_bytes()),
            _ => return None,
        })
    }

    fn observe(&mut self, resp: &HttpResponse, _: &Learnt) {
        match self.stage {
            0 if resp.is_success() => self.stage = 1,
            1 => {
                let versions = Base::parse(&self.root).ok().and_then(|root| nc::parse_versions(resp, &root, None).ok()).unwrap_or_default();
                // The current file is listed among them: one more means an earlier version was kept.
                self.confirmed = versions.len() >= 2;
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
        let p = Nextcloud::from_base(&Base::parse("https://cloud.example.org/remote.php/dav/files/john/").unwrap()).unwrap();
        assert_eq!((p.origin(), p.user()), ("https://cloud.example.org", "john"));
        assert_eq!(p.files_base(), "https://cloud.example.org/remote.php/dav/files/john/");
        assert_eq!(p.trashbin(), "https://cloud.example.org/remote.php/dav/trashbin/john/");
        assert_eq!(p.versions(), "https://cloud.example.org/remote.php/dav/versions/john/");
        // A server under a path, and a user id with a space.
        let p = Nextcloud::from_base(&Base::parse("https://example.org/cloud/remote.php/dav/files/jane%20doe/").unwrap()).unwrap();
        assert_eq!((p.origin(), p.user()), ("https://example.org/cloud", "jane doe"));
        assert_eq!(p.trashbin(), "https://example.org/cloud/remote.php/dav/trashbin/jane%20doe/");
        assert!(Nextcloud::from_base(&Base::parse("https://nas.example.org/dav/").unwrap()).is_none());
        assert!(Nextcloud::from_base(&Base::parse("https://cloud.example.org/remote.php/dav/files/").unwrap()).is_none());
        assert_eq!(p.kind(), Storage::Nextcloud);
        assert_eq!(p.verify_writes(), Some(Checksum::Sha1));
    }

    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use craftcloud_core::storage::probe::fake::{FakeDav, Flags};
    use craftcloud_core::storage::{Mode, Trash, Versions};

    use crate::fake_server::{Answer, FakeServer, Seen};
    use crate::session::AppPasswordSession;
    use crate::storage::probe::Limits;
    use crate::storage::webdav::WebDav;
    use crate::storage::webdav::tests::two_way;
    use crate::storage::{MoveGuard, Moved, RemoteStorage, Restored, TrashGuard, Trashed, Written};

    const BASE_PATH: &str = "/remote.php/dav/files/john";

    /// A Nextcloud-shaped scripted server behind a loopback HTTP server, with the flags given on top.
    fn nextcloud(flags: Flags) -> (FakeServer, Arc<Mutex<FakeDav>>) {
        let dav = Arc::new(Mutex::new(FakeDav::new(BASE_PATH, Flags { nextcloud: true, ..flags })));
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
        Box::new(Nextcloud::new(&server.url, "john"))
    }

    /// Capabilities as the check measures them on a Nextcloud: the server's trash and versions.
    fn nextcloud_caps() -> craftcloud_core::storage::Capabilities {
        craftcloud_core::storage::Capabilities { trash: Trash::Server, versions: Versions::Server, ..two_way() }
    }

    fn driver(server: &FakeServer) -> WebDav {
        WebDav::new(session(server), nextcloud_caps(), "Laptop").with_profile(profile(server))
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
    /// by its original location, restores it and reads it back; overwrites `c.txt` and finds an
    /// earlier version by file id. Only then are `Trash::Server` and `Versions::Server` granted;
    /// nothing is ever deleted on the trash bin's root (§ 16, Q8), and the probe folder ends in
    /// the server's trash. The version string is `status.php`'s, and a missing parent answers 404.
    #[test]
    fn the_probe_grants_the_servers_trash_and_versions_only_after_the_round_trips() {
        let (server, dav) = nextcloud(Flags::default());
        let dir = tempfile::tempdir().unwrap();
        let (connected, check) = WebDav::connect_with(session(&server), profile(&server), "Laptop", Some(dir.path()), quick());
        assert_eq!(check.mode, Mode::TwoWay, "{check:#?}");
        let caps = check.capabilities.clone().unwrap();
        assert_eq!((caps.trash, caps.versions), (Trash::Server, Versions::Server), "{:#?}", check.steps);
        assert_eq!(caps.missing_parent, craftcloud_core::storage::MissingParent::NotFound);
        assert!(caps.file_ids);
        assert_eq!(check.server_version, "31.0.14.1");
        assert_eq!(connected.profile().kind(), Storage::Nextcloud);
        let trash_step = check.steps.iter().find(|s| s.number == 16).unwrap();
        assert!(trash_step.result.contains("came back whole"), "{trash_step:?}");
        let versions_step = check.steps.iter().find(|s| s.number == 17).unwrap();
        assert!(versions_step.result.contains("one more version"), "{versions_step:?}");
        let seen = server.seen();
        let on_trashbin: Vec<String> = seen.iter().filter(|r| r.path.contains("/trashbin/")).map(|r| format!("{} {}", r.method, r.path)).collect();
        assert_eq!(on_trashbin.len(), 2, "one listing and one restore: {on_trashbin:?}");
        assert!(on_trashbin[0].starts_with("PROPFIND ") && on_trashbin[1].starts_with("MOVE "), "{on_trashbin:?}");
        assert!(seen.iter().all(|r| !(r.method == "DELETE" && !r.path.starts_with(BASE_PATH))), "a DELETE outside the files base: {seen:?}");
        assert!(seen.iter().any(|r| r.method == "PROPFIND" && r.path.contains("/versions/john/versions/")));
        let dav = dav.lock().unwrap();
        assert_eq!(dav.trash_purged, 0, "the person's trash is never purged");
        assert!(
            dav.trashed().iter().any(|(_, from)| from.starts_with(".craftcloud/probe/")),
            "the probe folder is in the server's trash: {:?}",
            dav.trashed()
        );
        drop(dav);

        // Without the profile the same server gets CraftCloud's own trash and versions.
        let (generic, check) = WebDav::connect(session(&server), "Laptop", None, quick());
        assert_eq!((check.capabilities.as_ref().unwrap().trash, check.capabilities.as_ref().unwrap().versions), (Trash::Client, Versions::Client));
        assert_eq!(generic.profile().kind(), Storage::Webdav);

        // The profile on a server that has no trash bin: the round trip fails, nothing is granted.
        let (plain, _) = crate::storage::webdav::tests::scripted(Flags::default());
        let (_, check) = WebDav::connect_with(session(&plain), profile(&plain), "Laptop", None, quick());
        assert_eq!(check.mode, Mode::TwoWay);
        let caps = check.capabilities.unwrap();
        assert_eq!((caps.trash, caps.versions), (Trash::Client, Versions::Client), "{:#?}", check.steps);
    }

    /// Q13: every `PUT` carries `OC-Checksum`, and the listing afterwards must show that checksum.
    /// On a server that reuses a version tag within one second, another client's write right
    /// behind ours lands under our tag: without the check it is taken as ours (and later
    /// overwritten); with it the write is a conflict, so the engine keeps both.
    #[test]
    fn a_write_landing_under_our_tag_within_the_second_is_a_conflict_not_a_silent_overwrite() {
        let flags = Flags { etag_reused_within_second: true, other_writes_after_put: Some(b"theirs".to_vec()), ..Flags::default() };
        let (server, dav) = nextcloud(flags.clone());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "mine");
        // Without the check (the generic profile): the write "succeeds", the server holds theirs.
        let mut unverified = WebDav::new(session(&server), nextcloud_caps(), "Laptop");
        assert!(matches!(unverified.create("Docs/a.txt", &a).unwrap(), Written::Saved { .. }), "the loss the check prevents");
        assert_eq!(dav.lock().unwrap().body("Docs/a.txt"), Some(b"theirs".as_slice()));
        // With it: a conflict, and the next write too.
        let mut verified = driver(&server);
        dav.lock().unwrap().add_file("Docs/b.txt", b"old");
        let etag = verified.list("Docs").unwrap().into_iter().find(|e| e.path == "Docs/b.txt").unwrap().etag.unwrap();
        assert_eq!(verified.replace("Docs/b.txt", &etag, &a).unwrap(), Written::Conflict);
        assert_eq!(verified.create("Docs/c.txt", &a).unwrap(), Written::Conflict);
        // `printf mine | sha1sum`.
        let sha1 = "SHA1:8034c43d15bb7ef42e2cc24255217acfc5f4b2f4";
        let seen = server.seen();
        let checksums: Vec<Option<String>> = seen.iter().filter(|r| r.method == "PUT").map(|r| r.header("oc-checksum").map(str::to_owned)).collect();
        assert_eq!(checksums, [None, Some(sha1.to_owned()), Some(sha1.to_owned())], "the unverified driver sends none, the verified one SHA1");
        // On a well-behaved server the same writes are saved, and the server lists our checksum.
        let (calm, dav) = nextcloud(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let mut s = WebDav::new(session(&calm), nextcloud_caps(), "Laptop").with_profile(profile(&calm));
        let Written::Saved { etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!("not saved") };
        std::fs::write(&a, "mine too").unwrap();
        assert!(matches!(s.replace("Docs/a.txt", &etag, &a).unwrap(), Written::Saved { .. }));
        assert!(dav.lock().unwrap().checksum("Docs/a.txt").is_some_and(|c| c.starts_with("SHA1:")));
        assert_eq!(calm.seen().iter().filter(|r| r.method == "PROPFIND" && r.path.ends_with("/Docs/a.txt")).count(), 2, "one read-back per write");
    }

    /// On this profile a file's version tag is its ETag plus the SHA1 the server lists: what a
    /// write answers, what a listing shows and what a download carries are the same tag; the bare
    /// ETag goes into `If-Match`; and a write by someone else under the same ETag (within the same
    /// second, without a checksum) changes the listed tag, so guards on the old tag don't hold.
    #[test]
    fn a_version_tag_is_the_etag_and_the_listed_checksum() {
        let (server, dav) = nextcloud(Flags { etag_reused_within_second: true, ..Flags::default() });
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "mine");
        let mut s = driver(&server);
        let Written::Saved { etag: tag } = s.create("Docs/a.txt", &a).unwrap() else { panic!("not saved") };
        assert!(tag.ends_with(";sha1:8034c43d15bb7ef42e2cc24255217acfc5f4b2f4"), "{tag}");
        let listed = |s: &mut WebDav| s.list("Docs").unwrap().into_iter().find(|e| e.path == "Docs/a.txt").unwrap().etag.unwrap();
        assert_eq!(listed(&mut s), tag, "the listing shows the tag the write answered");
        assert_eq!(s.download("Docs/a.txt", None).unwrap().header("ETag"), Some(tag.as_str()), "and so does a download");
        // Another client's write within the same second: the same ETag, no checksum.
        let theirs = dav.lock().unwrap().answer("PUT", &format!("{BASE_PATH}/Docs/a.txt"), &[], b"theirs");
        let bare = tag.split(";sha1:").next().unwrap().to_owned();
        assert_eq!(theirs.header("ETag"), Some(bare.as_str()), "the ETag was reused");
        assert_eq!(listed(&mut s), bare, "the listed tag changed all the same");
        assert_eq!(s.download("Docs/a.txt", None).unwrap().header("ETag"), Some(bare.as_str()));
        assert_eq!(s.trash("Docs/a.txt", TrashGuard::File { etag: tag.clone() }).unwrap(), Trashed::Changed, "a guard on the old tag doesn't hold");
        assert_eq!(s.move_to("Docs/a.txt", "Docs/b.txt", MoveGuard::File { etag: tag.clone() }).unwrap(), Moved::Changed);
        assert_eq!(dav.lock().unwrap().body("Docs/a.txt"), Some(b"theirs".as_slice()));
        // An overwrite from the listed tag sends the bare ETag in `If-Match`.
        std::thread::sleep(Duration::from_millis(1100));
        let now = listed(&mut s);
        let Written::Saved { etag: next } = s.replace("Docs/a.txt", &now, &a).unwrap() else { panic!("not saved") };
        assert!(next.contains(";sha1:") && next != tag, "{next}");
        let seen = server.seen();
        let put = seen.iter().rev().find(|r| r.method == "PUT").unwrap();
        assert_eq!(put.header("if-match"), Some(bare.as_str()), "{put:?}");
    }

    /// A delete goes into the server's trash by `DELETE` (a file's with `If-Match`; a stale tag
    /// keeps it), the trash lists it by its original path, a restore brings it back, a folder goes
    /// whole; the client trash's sweep sends nothing.
    #[test]
    fn deletes_land_in_the_servers_trash_and_come_back_from_it() {
        let (server, dav) = nextcloud(Flags::default());
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
        assert!(items[0].id.starts_with("a.txt.d") && items[0].trashed_at.starts_with("2026-"), "{:?}", items[0]);
        s.restore(&items[0]).unwrap();
        s.restore(&items[1]).unwrap();
        let dav_now = dav.lock().unwrap();
        assert_eq!(dav_now.body("Docs/a.txt"), Some(b"abc".as_slice()));
        assert_eq!(dav_now.body("Docs/Sub/b.txt"), Some(b"b".as_slice()));
        assert!(dav_now.trashed().is_empty());
        drop(dav_now);
        let before = server.seen().len();
        s.sweep_trash().unwrap();
        assert_eq!(server.seen().len(), before, "the server keeps its own trash: nothing swept");
        let on_trashbin = server.seen().iter().filter(|r| r.path.contains("/trashbin/")).map(|r| r.method.clone()).collect::<Vec<_>>();
        assert_eq!(on_trashbin, ["PROPFIND", "MOVE", "MOVE"], "never a DELETE on the trash bin");
    }

    /// With the server's versions confirmed, `keep_version` and `move_versions` send nothing; the
    /// versions are listed by file id without the current one, follow a rename, and one is
    /// restored by a guarded write: its bytes read with a `GET`, then a `PUT` with `If-Match` and
    /// the checksum, never the server's restore `MOVE`. With the client's versions (unconfirmed)
    /// a rename moves the history.
    #[test]
    fn versions_are_the_servers_and_follow_renames_by_id() {
        let (server, dav) = nextcloud(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "v0");
        let mut s = driver(&server);
        let Written::Saved { mut etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!() };
        for i in 1..=2 {
            s.keep_version("Docs/a.txt").unwrap();
            std::fs::write(&a, format!("v{i}")).unwrap();
            let Written::Saved { etag: new } = s.replace("Docs/a.txt", &etag, &a).unwrap() else { panic!("{i}") };
            etag = new;
        }
        assert!(server.seen().iter().all(|r| r.method != "COPY"), "no client versions: {:?}", server.seen());
        let versions = s.list_versions("Docs/a.txt").unwrap();
        assert_eq!(versions.len(), 2, "the current file left out: {versions:#?}");
        assert!(versions[0].stamp > versions[1].stamp, "newest first");
        assert_eq!(s.move_to("Docs/a.txt", "Docs/b.txt", MoveGuard::File { etag }).unwrap(), Moved::Done);
        let before = server.seen().len();
        s.move_versions("Docs/a.txt", "Docs/b.txt").unwrap();
        assert_eq!(server.seen().len(), before, "a no-op: the server keeps them by id");
        assert_eq!(s.list_versions("Docs/b.txt").unwrap().len(), 2, "the history followed the rename");
        let before = server.seen().len();
        assert_eq!(s.restore_version("Docs/b.txt", &versions[1].stamp).unwrap(), Restored::Done);
        assert_eq!(dav.lock().unwrap().body("Docs/b.txt"), Some(b"v0".as_slice()));
        assert_eq!(s.list_versions("Docs/b.txt").unwrap().len(), 3, "the replaced file was kept by the server");
        assert!(matches!(s.restore_version("Docs/b.txt", "1").unwrap_err(), crate::storage::StorageError::NotFound(_)));
        let restore: Vec<Seen> = server.seen()[before..].to_vec();
        let writes: Vec<&Seen> = restore.iter().filter(|r| !matches!(r.method.as_str(), "PROPFIND" | "GET")).collect();
        assert_eq!(writes.len(), 1, "one write, the guarded PUT: {writes:?}");
        assert_eq!(writes[0].method, "PUT");
        assert!(writes[0].header("if-match").is_some() && writes[0].header("oc-checksum").is_some_and(|c| c.starts_with("SHA1:")), "{:?}", writes[0]);
        assert!(restore.iter().any(|r| r.method == "GET" && r.path.contains("/versions/john/versions/")), "the version's bytes are read");
        // Unconfirmed (client versions): the same server, the history is CraftCloud's and moves by MOVE.
        let mut client = WebDav::new(session(&server), two_way(), "Laptop").with_profile(profile(&server));
        client.keep_version("Docs/b.txt").unwrap();
        assert!(server.seen().iter().any(|r| r.method == "COPY"));
        client.move_versions("Docs/b.txt", "Docs/c.txt").unwrap();
        assert!(server.seen().iter().any(|r| r.method == "MOVE" && r.path.contains("/.craftcloud/versions/Docs/b.txt")));
    }

    /// A version restore is a guarded write: a change landing between the look at the file and the
    /// write is never overwritten. The newer file stays, nothing is restored, and the driver says
    /// so ([`Restored::Changed`], exit code 5 on the command line). The server's own restore `MOVE`
    /// (no precondition on the file it replaces) would have overwritten it.
    #[test]
    fn a_change_landing_during_a_version_restore_is_never_overwritten() {
        let dav = Arc::new(Mutex::new(FakeDav::new(BASE_PATH, Flags { nextcloud: true, ..Flags::default() })));
        let racing = Arc::clone(&dav);
        let server = FakeServer::start_full(move |seen: &Seen| {
            let mut dav = racing.lock().unwrap_or_else(PoisonError::into_inner);
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            // Another client writes the file right after the version's bytes were read.
            if seen.method == "GET" && seen.path.contains("/versions/john/versions/") {
                let theirs = dav.answer("PUT", &format!("{BASE_PATH}/Docs/a.txt"), &[], b"changed meanwhile");
                assert!(theirs.is_success());
            }
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer
        });
        dav.lock().unwrap().origin = server.url.clone();
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "v0");
        let mut s = driver(&server);
        let Written::Saved { etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!() };
        std::fs::write(&a, "v1").unwrap();
        assert!(matches!(s.replace("Docs/a.txt", &etag, &a).unwrap(), Written::Saved { .. }));
        let versions = s.list_versions("Docs/a.txt").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(s.restore_version("Docs/a.txt", &versions[0].stamp).unwrap(), Restored::Changed);
        assert_eq!(dav.lock().unwrap().body("Docs/a.txt"), Some(b"changed meanwhile".as_slice()), "the newer file stays");
        let seen = server.seen();
        assert!(seen.iter().all(|r| !(r.method == "MOVE" && r.path.contains("/versions/"))), "never the server's restore MOVE: {seen:?}");
        let last_put = seen.iter().rev().find(|r| r.method == "PUT").unwrap();
        assert!(last_put.header("if-match").is_some(), "the restore's PUT carried its guard: {last_put:?}");
    }

    /// Free space comes from the OCS user endpoint: a count, or no limit (`-3`).
    #[test]
    fn free_space_comes_from_the_ocs_user_endpoint() {
        for (quota, expected) in [(Some(5_000), Some(5_000)), (None, Some(u64::MAX)), (Some(-1), None)] {
            let (server, _) = nextcloud(Flags { quota_available: quota, ..Flags::default() });
            assert_eq!(driver(&server).free_space(1).unwrap(), expected, "{quota:?}");
            assert!(server.seen().iter().all(|r| r.path.starts_with("/ocs/v2.php/cloud/user")), "{:?}", server.seen());
        }
    }

    /// Login Flow v2: the start names the browser address and the poll endpoint, the client polls
    /// until the person has finished (404 meanwhile), and gets the login name and app password;
    /// it gives up after its deadline; a server error on the poll is an error.
    #[test]
    fn the_login_flow_polls_until_the_browser_is_done() {
        let (server, dav) = nextcloud(Flags { login_flow_ready_after: 3, ..Flags::default() });
        let mut opened = Vec::new();
        let grant =
            sign_in(&server.url, "CraftCloud on Laptop", &mut |url| opened.push(url.to_owned()), Duration::from_millis(10), Duration::from_secs(5))
                .unwrap();
        assert_eq!(opened, [format!("{}/login/v2/flow/fake-flow-token", server.url)]);
        assert_eq!((grant.login_name.as_str(), grant.app_password.as_str()), ("john", "fake-app-password"));
        let seen = server.seen();
        assert_eq!(seen[0].header("user-agent"), Some("CraftCloud on Laptop"), "the app password is listed under this name");
        assert_eq!(seen.iter().filter(|r| r.path == "/login/v2/poll").count(), 3);
        assert!(!dav.lock().unwrap().app_password_deleted);
        // Never finished: gives up.
        let (never, _) = nextcloud(Flags::default());
        let err = sign_in(&never.url, "CraftCloud on Laptop", &mut |_| {}, Duration::from_millis(10), Duration::from_millis(60)).unwrap_err();
        assert!(err.contains("in time"), "{err}");
        // The poll fails: an error, not a sign-in.
        let broken = FakeServer::start(|r: &Seen| {
            if r.path == "/index.php/login/v2" {
                (200, format!(r#"{{"poll":{{"token":"t","endpoint":"{}/login/v2/poll"}},"login":"x"}}"#, "http://127.0.0.1:1"))
            } else {
                (500, String::new())
            }
        });
        assert!(sign_in(&broken.url, "CraftCloud", &mut |_| {}, Duration::from_millis(1), Duration::from_secs(1)).is_err());
        // Detection and the user id.
        let (server, _) = nextcloud(Flags::default());
        let d = detect(&server.url).unwrap();
        assert_eq!((d.product.as_str(), d.version.as_str()), ("Nextcloud", "31.0.14.1"));
        assert!(detect(&crate::storage::webdav::tests::scripted(Flags::default()).0.url).is_none(), "a plain WebDAV server isn't Nextcloud");
        assert_eq!(user_id(&mut session(&server), &server.url).unwrap(), "john");
    }
}
