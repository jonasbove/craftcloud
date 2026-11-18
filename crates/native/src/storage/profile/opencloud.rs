//! The OpenCloud profile (docs/WEBDAV-STORAGE.md § 7.4, the OpenCloud column; § 16, Q1; the
//! requests and parsers are `craftcloud_core::api::opencloud`'s). It knows where things are:
//!
//! - the WebDAV base is the personal space, `/remote.php/dav/spaces/<drive id>/`, the drive id from
//!   the Graph API (`me/drives`, the `personal` drive) at sign-in;
//! - the trash is the space's, `/remote.php/dav/spaces/trash-bin/<drive id>/`, a restore a `MOVE`
//!   back to the item's original path under the space's URL with `Overwrite: F`; the probe's step
//!   16 confirms it with a round trip before `Trash::Server` is granted;
//! - versions are `/remote.php/dav/meta/<file id>/v/`, by id, so they follow renames; a version's
//!   bytes are read with a `GET` and the driver writes them back with its guarded `replace`
//!   (`If-Match`), never with OpenCloud's own restore (a `COPY` over the file, unguarded); step
//!   17 confirms an overwrite leaves a version before `Versions::Server` is granted;
//! - the quota is WebDAV's `quota-available-bytes` (the driver's default);
//! - the server's version string is `status.php`'s `productversion`.
//!
//! And one thing it says about the mode, which the check on connect can't measure: **download
//! only, whatever the check measures** ([`Opencloud::lowered`]; § 16, Q1). OpenCloud ignores
//! `If-None-Match: *` on upload (reva#866, which the check does measure), its conditional uploads
//! aren't shown to be atomic (reva#959: two uploads with the same `If-Match` both succeeded), its
//! `MOVE` with `Overwrite: F` lost a race in 1 of 3 runs, and its `MOVE` and `DELETE` ignore a
//! stale `If-Match` (reva#961). A probe can't race itself, so the profile lowers the mode from
//! those measurements; it stays download only until the profile is changed by hand, even on a
//! server that someday passes every step. Under download only nothing is written to the server,
//! so the trash and versions here serve `trash ls` and `versions` (reading) and the day the mode
//! changes.
//!
//! The sign-in is an app token the person makes in the web UI (Account › App Tokens), taken as
//! Basic auth with the login name; the client never makes one itself (§ 10), and no OpenID Connect
//! flow is built (OpenCloud registers only its own clients' ids).

use craftcloud_core::api::nextcloud::Detected;
use craftcloud_core::api::opencloud as oc;
use craftcloud_core::api::versions::Version;
use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
use craftcloud_core::storage::probe::{Base, ExtraCheck, Learnt, PROPFIND_BODY, Request, Step, parse_listing};
use craftcloud_core::storage::{Lowered, Mode};
use craftcloud_core::sync::Storage;

use super::ServerProfile;
use crate::storage::probe::Transport;
use crate::storage::{StorageError, TrashItem};

/// An OpenCloud server, by its origin and the signed-in user's personal drive id.
#[derive(Clone, Debug)]
pub struct Opencloud {
    origin: String,
    drive: String,
}

/// Why OpenCloud gets download only at most (§ 16, Q1), as the check's table says it.
pub const LOWERED_WHY: &str = "OpenCloud's write guards don't hold under a race (two guarded uploads or moves at once can both succeed; reva#959, reva#961), which the check can't measure, so nothing is written to it until that is fixed upstream";

impl Opencloud {
    pub fn new(origin: &str, drive: &str) -> Self {
        Self { origin: origin.trim_end_matches('/').to_owned(), drive: drive.to_owned() }
    }

    /// From the WebDAV base the settings hold (`https://cloud.example.org/remote.php/dav/spaces/<drive id>/`);
    /// `None` for a base that isn't a space.
    pub fn from_base(base: &Base) -> Option<Self> {
        let (origin, drive) = oc::split_spaces_base(base)?;
        Some(Self::new(&origin, &drive))
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn drive(&self) -> &str {
        &self.drive
    }

    /// The files base: the personal space, `<origin>/remote.php/dav/spaces/<drive id>/`.
    pub fn files_base(&self) -> String {
        oc::spaces_base(&self.origin, &self.drive)
    }

    fn trash_root(&self) -> String {
        oc::trash_root(&self.origin, &self.drive)
    }

    fn meta_root(&self) -> String {
        oc::meta_root(&self.origin)
    }

    /// The root every spaces URL hangs under (`<origin>/remote.php/dav/spaces/`): the trash bin
    /// and the space itself are both below it, which is what the probe's restore `MOVE` needs.
    fn spaces_root(&self) -> String {
        format!("{}/remote.php/dav/spaces/", self.origin)
    }

    /// A file's id (a depth-0 `PROPFIND`); not found when it isn't there.
    fn file_id(&self, transport: &mut dyn Transport, base: &Base, path: &str) -> Result<String, StorageError> {
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
        entry.file_id.ok_or_else(|| StorageError::Failed(format!("{path}: the server lists no file id")))
    }

    /// The trash item with `id` as the server lists it now (its kind and where it was).
    fn trashed(&self, transport: &mut dyn Transport, id: &str) -> Result<craftcloud_core::api::nextcloud::Trashed, StorageError> {
        let root = Base::parse(&self.trash_root())?;
        let resp = transport.call(&oc::trash_list(root.url()))?;
        let items = oc::parse_trash(&resp, &root)?;
        items.into_iter().find(|t| t.name == id).ok_or_else(|| StorageError::NotFound(format!("{id} isn't in the trash")))
    }
}

/// Whether the server at `origin` says it is OpenCloud (`status.php`), and which version. Only a
/// suggestion of a kind: nothing here grants a capability (§ 2). No sign-in needed.
pub fn detect(origin: &str) -> Option<Detected> {
    let resp = crate::http::execute(&oc::status(origin)).ok()?;
    oc::parse_status(&resp)
}

/// Who is signed in and their personal space, from the Graph API (asked with the sign-in, which
/// also shows the app token works). `Err` in words when the server doesn't answer like OpenCloud.
pub fn personal_space(transport: &mut dyn Transport, origin: &str) -> Result<(oc::Me, oc::PersonalDrive), String> {
    let me = oc::parse_me(&transport.call(&oc::me(origin))?).map_err(|e| format!("asking who is signed in: {e}"))?;
    let drive = oc::parse_personal_drive(&transport.call(&oc::drives(origin))?).map_err(|e| format!("asking for the personal space: {e}"))?;
    Ok((me, drive))
}

impl ServerProfile for Opencloud {
    fn kind(&self) -> Storage {
        Storage::Opencloud
    }

    fn version(&self, transport: &mut dyn Transport) -> Option<String> {
        let resp = transport.call(&oc::status(&self.origin)).ok()?;
        oc::parse_status(&resp).map(|d| d.version)
    }

    fn probe_checks(&self) -> Option<(Box<dyn ExtraCheck>, Box<dyn ExtraCheck>)> {
        Some((Box::new(TrashCheck::new(&self.spaces_root(), &self.drive)), Box::new(VersionsCheck::new(&self.meta_root(), &self.files_base()))))
    }

    /// Download only, whatever the check measures (§ 16, Q1).
    fn lowered(&self) -> Option<Lowered> {
        Some(Lowered { to: Mode::DownloadOnly, why: LOWERED_WHY.to_owned() })
    }

    fn trash_list(&self, transport: &mut dyn Transport) -> Result<Vec<TrashItem>, StorageError> {
        let root = Base::parse(&self.trash_root())?;
        let resp = transport.call(&oc::trash_list(root.url()))?;
        if resp.status == 404 {
            return Ok(Vec::new());
        }
        let items = oc::parse_trash(&resp, &root)?;
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

    /// Back where it was, never onto a taken name (`Overwrite: F`; measured: without it the server
    /// overwrites), and nowhere else (the server restores only to the original path).
    fn restore(&self, transport: &mut dyn Transport, item: &TrashItem) -> Result<(), StorageError> {
        let listed = self.trashed(transport, &item.id)?;
        if listed.original_location != item.path.trim_matches('/') {
            return Err(StorageError::Failed(format!(
                "this server puts {} back only where it was ({}); move it afterwards",
                item.path, listed.original_location
            )));
        }
        let resp =
            transport.call(&oc::trash_restore(&self.trash_root(), &item.id, &self.files_base(), &listed.original_location, listed.is_folder))?;
        match resp.status {
            404 => Err(StorageError::NotFound(format!("{} isn't in the trash", item.path))),
            412 => Err(StorageError::Failed(format!("{} is taken: move or rename what is there first, then restore again", item.path))),
            _ if resp.is_success() => Ok(()),
            s => Err(StorageError::Failed(format!("restoring {}: the server answered {s}", item.path))),
        }
    }

    fn list_versions(&self, transport: &mut dyn Transport, base: &Base, path: &str) -> Result<Vec<Version>, StorageError> {
        let id = match self.file_id(transport, base, path) {
            // A file that isn't there has no versions to list (the server keeps them by id).
            Err(StorageError::NotFound(_)) => return Ok(Vec::new()),
            other => other?,
        };
        let root = Base::parse(&self.meta_root())?;
        let resp = transport.call(&oc::versions_list(root.url(), &id))?;
        Ok(oc::parse_versions(&resp, &root)?)
    }

    /// Where version `stamp`'s bytes are read; the driver writes them back with its guarded
    /// `replace`. OpenCloud's own restore (a `COPY` over the file) is never sent.
    fn version_download(&self, transport: &mut dyn Transport, base: &Base, path: &str, stamp: &str) -> Result<HttpRequest, StorageError> {
        let versions = self.list_versions(transport, base, path)?;
        if !versions.iter().any(|v| v.stamp == stamp) {
            return Err(StorageError::NotFound(format!("{path} has no version {stamp}")));
        }
        let id = self.file_id(transport, base, path)?;
        Ok(oc::versions_download(&self.meta_root(), &id, stamp))
    }
}

/// Step 16, the trash round trip: `DELETE` the probe's `d.txt` (step 13's copy of `c.txt`), find
/// it in the space's trash bin by its original location, `MOVE` it back to that path under the
/// space's URL (`Overwrite: F`), and `GET` it back with its bytes. Confirmed only when every step
/// holds. Every request on the spaces root is one the runner allows there: a listing, and the
/// restore of the probe's own item to where it was.
struct TrashCheck {
    /// `<origin>/remote.php/dav/spaces/`.
    root: String,
    drive: String,
    stage: u8,
    /// The deleted file's key in the trash bin, once found.
    key: Option<String>,
    confirmed: bool,
}

impl TrashCheck {
    fn new(spaces_root: &str, drive: &str) -> Self {
        Self { root: spaces_root.to_owned(), drive: drive.to_owned(), stage: 0, key: None, confirmed: false }
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
            1 => Request::under_root(Step::ServerTrash, Method::Propfind, &self.root, &format!("trash-bin/{}", self.drive), true)
                .with_header("Depth", "1")
                .with_header("Content-Type", "application/xml; charset=utf-8")
                .with_body(oc::TRASH_PROPFIND_BODY.as_bytes()),
            2 => {
                let key = self.key.as_deref()?;
                let mut req = Request::under_root(Step::ServerTrash, Method::Move, &self.root, &format!("trash-bin/{}/{key}", self.drive), false)
                    .with_header("Overwrite", "F");
                req.destination = Some((format!("{}/{d}", self.drive), false));
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
                let found = Base::parse(&format!("{}trash-bin/{}/", self.root, self.drive))
                    .ok()
                    .and_then(|root| oc::parse_trash(resp, &root).ok())
                    .and_then(|items| items.into_iter().find(|t| !t.is_folder && t.original_location == wanted).map(|t| t.name));
                match found {
                    Some(key) => {
                        self.key = Some(key);
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

/// Step 17, the server's versions, on `d.txt` (step 15's `DELETE` with a stale tag deletes `c.txt`
/// on OpenCloud, reva#961, so the file the Nextcloud check uses is gone there): read its id and
/// version tag, overwrite it once more (in a later second, `If-Match`), then list its versions by
/// id; confirmed when the listing holds an earlier version.
struct VersionsCheck {
    /// `<origin>/remote.php/dav/meta/`.
    root: String,
    /// The files base, to read the depth-0 listing against.
    base: String,
    stage: u8,
    file_id: Option<String>,
    etag: Option<String>,
    confirmed: bool,
}

impl VersionsCheck {
    fn new(meta_root: &str, files_base: &str) -> Self {
        Self { root: meta_root.to_owned(), base: files_base.to_owned(), stage: 0, file_id: None, etag: None, confirmed: false }
    }
}

impl ExtraCheck for VersionsCheck {
    fn next(&self, learnt: &Learnt) -> Option<Request> {
        if !learnt.d_exists {
            return None;
        }
        let d = format!("{}/d.txt", learnt.folder);
        Some(match self.stage {
            0 => Request::new(Step::ServerVersions, Method::Propfind, &d, false)
                .with_header("Depth", "0")
                .with_header("Content-Type", "application/xml; charset=utf-8")
                .with_body(PROPFIND_BODY.as_bytes()),
            1 => Request {
                settles: true,
                ..Request::new(Step::ServerVersions, Method::Put, &d, false).with_header("If-Match", self.etag.as_deref()?).with_body(b"four")
            },
            2 => Request::under_root(Step::ServerVersions, Method::Propfind, &self.root, &format!("{}/v", self.file_id.as_deref()?), true)
                .with_header("Depth", "1")
                .with_header("Content-Type", "application/xml; charset=utf-8")
                .with_body(PROPFIND_BODY.as_bytes()),
            _ => return None,
        })
    }

    fn observe(&mut self, resp: &HttpResponse, learnt: &Learnt) {
        match self.stage {
            0 => {
                let d = format!("{}/d.txt", learnt.folder);
                let entry =
                    Base::parse(&self.base).ok().and_then(|base| parse_listing(resp, &base).ok()).and_then(|l| l.into_iter().find(|e| e.path == d));
                match entry.and_then(|e| Some((e.file_id?, e.etag?))) {
                    Some((id, etag)) => {
                        self.file_id = Some(id);
                        self.etag = Some(etag);
                        self.stage = 1;
                    }
                    None => self.stage = 9,
                }
            }
            1 if resp.is_success() => self.stage = 2,
            2 => {
                let versions = Base::parse(&self.root).ok().and_then(|root| oc::parse_versions(resp, &root).ok()).unwrap_or_default();
                // The current file isn't listed: one entry means an earlier version was kept.
                self.confirmed = !versions.is_empty();
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

    const DRIVE: &str = "5d90215b-14f0-4b05-9f8d-8971b9810172$4ed81edc-3290-4003-a070-9f24bb826ff8";

    #[test]
    fn the_profile_is_read_from_the_spaces_base() {
        let p = Opencloud::from_base(&Base::parse(&format!("https://cloud.example.org/remote.php/dav/spaces/{DRIVE}/")).unwrap()).unwrap();
        assert_eq!((p.origin(), p.drive()), ("https://cloud.example.org", DRIVE));
        assert_eq!(p.files_base(), format!("https://cloud.example.org/remote.php/dav/spaces/{DRIVE}/"));
        assert_eq!(p.trash_root(), format!("https://cloud.example.org/remote.php/dav/spaces/trash-bin/{DRIVE}/"));
        assert_eq!(p.meta_root(), "https://cloud.example.org/remote.php/dav/meta/");
        assert_eq!(p.spaces_root(), "https://cloud.example.org/remote.php/dav/spaces/");
        let p = Opencloud::from_base(&Base::parse(&format!("https://example.org/cloud/remote.php/dav/spaces/{DRIVE}")).unwrap()).unwrap();
        assert_eq!(p.origin(), "https://example.org/cloud");
        assert!(Opencloud::from_base(&Base::parse("https://cloud.example.org/remote.php/dav/files/john/").unwrap()).is_none());
        assert_eq!(p.kind(), Storage::Opencloud);
        assert_eq!(p.verify_writes(), None, "Q9: the SHA1 is recorded, not used");
        assert_eq!(p.lowered(), Some(Lowered { to: Mode::DownloadOnly, why: LOWERED_WHY.to_owned() }));
    }

    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use craftcloud_core::storage::probe::fake::{FakeDav, Flags};
    use craftcloud_core::storage::{Guard, Trash, Versions};

    use crate::fake_server::{Answer, FakeServer, Seen};
    use crate::session::AppPasswordSession;
    use crate::storage::probe::Limits;
    use crate::storage::webdav::WebDav;
    use crate::storage::webdav::tests::two_way;
    use crate::storage::{RemoteStorage, Written};

    /// The scripted server's drive id (`FakeDav`'s base is the space).
    const FAKE_DRIVE: &str = "5d90215b$4ed81edc";

    fn base_path() -> String {
        format!("/remote.php/dav/spaces/{FAKE_DRIVE}")
    }

    /// An OpenCloud-shaped scripted server behind a loopback HTTP server, with the flags given on top.
    fn opencloud(flags: Flags) -> (FakeServer, Arc<Mutex<FakeDav>>) {
        let dav = Arc::new(Mutex::new(FakeDav::new(&base_path(), Flags { opencloud: true, ..flags })));
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

    /// As OpenCloud 9.1.1 measures (docs/WEBDAV-STORAGE.md § 5): `If-None-Match: *` and a
    /// `DELETE`'s `If-Match` ignored.
    fn as_measured() -> Flags {
        Flags { ignores_if_none_match: true, delete_ignores_if_match: true, ..Flags::default() }
    }

    fn session(server: &FakeServer) -> AppPasswordSession {
        AppPasswordSession::new(Base::parse(&format!("{}{}/", server.url, base_path())).unwrap(), "john", "SECRET-app-token").unwrap()
    }

    fn profile(server: &FakeServer) -> Box<dyn ServerProfile> {
        Box::new(Opencloud::new(&server.url, FAKE_DRIVE))
    }

    fn quick() -> Limits {
        Limits { settle: Duration::ZERO, ..Limits::default() }
    }

    /// Capabilities as the check records them on an OpenCloud whose round trips confirmed its
    /// trash and versions, with the profile's ceiling.
    fn opencloud_caps() -> craftcloud_core::storage::Capabilities {
        craftcloud_core::storage::Capabilities {
            trash: Trash::Server,
            versions: Versions::Server,
            lowered: Some(Lowered { to: Mode::DownloadOnly, why: LOWERED_WHY.to_owned() }),
            ..two_way()
        }
    }

    fn driver(server: &FakeServer) -> WebDav {
        WebDav::new(session(server), opencloud_caps(), "Laptop").with_profile(profile(server))
    }

    /// The check on connect with the profile, against the server as measured: download only,
    /// with `If-None-Match: *` ignored measured (reva#866) and the profile's ceiling recorded in
    /// the capabilities; step 16 finds the deleted file in the space's trash bin and restores it
    /// to where it was (a `MOVE` with `Overwrite: F`, never a `DELETE` there, § 16 Q8), step 17
    /// overwrites the restored file and finds an earlier version by id. The version string is
    /// `status.php`'s `productversion`.
    #[test]
    fn the_check_measures_download_only_and_grants_the_servers_trash_and_versions() {
        let (server, dav) = opencloud(as_measured());
        let dir = tempfile::tempdir().unwrap();
        let (connected, check) = WebDav::connect_with(session(&server), profile(&server), "Laptop", Some(dir.path()), quick());
        assert_eq!(check.mode, Mode::DownloadOnly, "{check:#?}");
        assert_eq!(check.refused_because, None, "download only is not a refusal");
        let caps = check.capabilities.clone().unwrap();
        assert_eq!(caps.create_if_absent, Guard::Ignored, "measured, reva#866");
        assert_eq!((caps.trash, caps.versions), (Trash::Server, Versions::Server), "{:#?}", check.steps);
        assert_eq!(caps.lowered.as_ref().map(|l| l.to), Some(Mode::DownloadOnly));
        assert!(caps.file_ids && caps.checksums == Some(craftcloud_core::storage::Checksum::Sha1));
        assert_eq!(check.server_version, "9.1.1");
        assert_eq!(connected.profile().kind(), Storage::Opencloud);
        assert_eq!(craftcloud_core::storage::mode_for(connected.capabilities()), Mode::DownloadOnly, "the engine reads the same mode");
        let trash_step = check.steps.iter().find(|s| s.number == 16).unwrap();
        assert!(trash_step.result.contains("came back whole"), "{trash_step:?}");
        let versions_step = check.steps.iter().find(|s| s.number == 17).unwrap();
        assert!(versions_step.result.contains("one more version"), "{versions_step:?}");
        let known = check.checks.iter().find(|c| c.check == "Known about this kind of server").unwrap();
        assert!(known.result.starts_with("download only at most: OpenCloud's write guards"), "{known:?}");
        let seen = server.seen();
        let on_trash: Vec<String> =
            seen.iter().filter(|r| r.path.contains("/spaces/trash-bin/")).map(|r| format!("{} {}", r.method, r.path)).collect();
        assert_eq!(on_trash.len(), 2, "one listing and one restore: {on_trash:?}");
        assert!(on_trash[0].starts_with("PROPFIND ") && on_trash[1].starts_with("MOVE "), "{on_trash:?}");
        let restore = seen.iter().find(|r| r.method == "MOVE" && r.path.contains("/spaces/trash-bin/")).unwrap();
        assert_eq!(restore.header("overwrite"), Some("F"), "a restore never lands on a taken name: {restore:?}");
        assert!(restore.header("destination").is_some_and(|d| d.contains(&format!("{}/.craftcloud/probe/", base_path()))), "{restore:?}");
        assert!(seen.iter().all(|r| !(r.method == "DELETE" && !r.path.starts_with(&base_path()))), "a DELETE outside the space: {seen:?}");
        assert!(seen.iter().any(|r| r.method == "PROPFIND" && r.path.contains("/remote.php/dav/meta/") && r.path.ends_with("/v/")), "{seen:?}");
        let dav = dav.lock().unwrap();
        assert_eq!(dav.trash_purged, 0, "the person's trash is never purged");
        assert!(
            dav.trashed().iter().any(|(_, from)| from.starts_with(".craftcloud/probe/")),
            "the probe folder is in the trash: {:?}",
            dav.trashed()
        );
        drop(dav);

        // Without the profile the same server is download only too (reva#866 measured), with
        // CraftCloud's own trash and versions and no ceiling.
        let (generic, check) = WebDav::connect(session(&server), "Laptop", None, quick());
        assert_eq!(check.mode, Mode::DownloadOnly);
        let caps = check.capabilities.unwrap();
        assert_eq!((caps.trash, caps.versions, caps.lowered), (Trash::Client, Versions::Client, None));
        assert_eq!(generic.profile().kind(), Storage::Webdav);
    }

    /// Q1's "until": an OpenCloud that someday honours every guard still gets download only
    /// with the profile (the ceiling, recorded in the saved check), while the same server without
    /// the profile measures two-way. The saved check reused on the next connect carries it too.
    #[test]
    fn a_server_that_passes_every_step_is_still_download_only_with_the_profile() {
        let (server, _) = opencloud(Flags::default());
        let dir = tempfile::tempdir().unwrap();
        let (driver, check) = WebDav::connect_with(session(&server), profile(&server), "Laptop", Some(dir.path()), quick());
        assert_eq!(check.mode, Mode::DownloadOnly, "{check:#?}");
        let caps = check.capabilities.clone().unwrap();
        assert_eq!((caps.create_if_absent, caps.replace_if_match, caps.move_no_overwrite), (Guard::Honoured, Guard::Honoured, true), "measured fine");
        assert_eq!(craftcloud_core::storage::mode_for(driver.capabilities()), Mode::DownloadOnly);
        let saved = crate::storage::probe::load_check(dir.path()).unwrap();
        assert_eq!(saved.capabilities.as_ref().and_then(|c| c.lowered.as_ref()).map(|l| l.to), Some(Mode::DownloadOnly));
        let before = server.seen().len();
        let (again, check) = WebDav::connect_with(session(&server), profile(&server), "Laptop", Some(dir.path()), quick());
        assert_eq!(check.mode, Mode::DownloadOnly);
        assert_eq!(craftcloud_core::storage::mode_for(again.capabilities()), Mode::DownloadOnly, "from the saved check");
        assert_eq!(server.seen().len(), before + 1, "only the version string was asked");
        // Without the profile: two-way.
        let (_, check) = WebDav::connect(session(&server), "Laptop", None, quick());
        assert_eq!(check.mode, Mode::TwoWay);
    }

    /// The space's trash lists items by key with where they were and when; a restore is a `MOVE`
    /// back with `Overwrite: F` (a taken name is an error, never an overwrite), a folder goes back
    /// whole, and an item can't be put anywhere but where it was. Nothing is ever purged there.
    #[test]
    fn the_trash_lists_and_restores_items_and_never_purges() {
        let (server, dav) = opencloud(Flags::default());
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder("Docs/Sub", None);
            dav.add_file("Docs/a.txt", b"abc");
            dav.add_file("Docs/Sub/b.txt", b"b");
        }
        let mut s = driver(&server);
        assert!(s.trash_list().unwrap().is_empty());
        // The person (or another client) deletes on the server: the items land in the space's trash.
        {
            let mut dav = dav.lock().unwrap();
            let delete = |dav: &mut FakeDav, path: &str| dav.answer("DELETE", &format!("{}/{path}", base_path()), &[], b"");
            assert_eq!(delete(&mut dav, "Docs/a.txt").status, 204);
            assert_eq!(delete(&mut dav, "Docs/Sub").status, 204);
        }
        let items = s.trash_list().unwrap();
        assert_eq!(items.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["Docs/a.txt", "Docs/Sub"]);
        assert!(items[0].trashed_at.starts_with("2026-"), "{:?}", items[0]);
        // A taken name: refused, nothing overwritten.
        dav.lock().unwrap().add_file("Docs/a.txt", b"newer");
        let err = s.restore(&items[0]).unwrap_err();
        assert!(matches!(&err, StorageError::Failed(why) if why.contains("is taken")), "{err:?}");
        assert_eq!(dav.lock().unwrap().body("Docs/a.txt"), Some(b"newer".as_slice()), "never overwritten");
        // Elsewhere: not on this server.
        let elsewhere = TrashItem { path: "Docs/old.txt".into(), ..items[0].clone() };
        assert!(matches!(s.restore(&elsewhere).unwrap_err(), StorageError::Failed(why) if why.contains("only where it was")));
        // Back where it was, once the place is free; the folder whole.
        dav.lock().unwrap().answer("DELETE", &format!("{}/Docs/a.txt", base_path()), &[], b"");
        let items = s.trash_list().unwrap();
        let old = items.iter().find(|i| i.path == "Docs/a.txt" && i.trashed_at <= items[0].trashed_at).unwrap().clone();
        s.restore(&old).unwrap();
        s.restore(items.iter().find(|i| i.path == "Docs/Sub").unwrap()).unwrap();
        {
            let dav = dav.lock().unwrap();
            assert!(dav.body("Docs/a.txt").is_some());
            assert_eq!(dav.body("Docs/Sub/b.txt"), Some(b"b".as_slice()));
            assert_eq!(dav.trash_purged, 0);
        }
        assert!(matches!(s.restore(&old).unwrap_err(), StorageError::NotFound(_)), "gone from the trash");
        let on_trash: Vec<String> = server.seen().iter().filter(|r| r.path.contains("/spaces/trash-bin/")).map(|r| r.method.clone()).collect();
        assert!(on_trash.iter().all(|m| m == "PROPFIND" || m == "MOVE"), "never a DELETE on the trash bin: {on_trash:?}");
        assert!(server.seen().iter().filter(|r| r.method == "MOVE").all(|r| r.header("overwrite") == Some("F")));
        let before = server.seen().len();
        s.sweep_trash().unwrap();
        assert_eq!(server.seen().len(), before, "the server keeps its own trash: nothing swept");
    }

    /// The server's versions are listed by file id without the current file, newest first, follow
    /// a rename, show as times, and one is restored by a guarded write: its bytes read with a
    /// `GET`, then a `PUT` with `If-Match` (the server keeps the replaced file), never the server's
    /// own `COPY` over the file. Where the check didn't confirm them, the client's
    /// `.craftcloud/versions/` is used.
    #[test]
    fn versions_are_the_servers_by_id_and_restored_by_a_guarded_write() {
        let (server, dav) = opencloud(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "v0").unwrap();
        // The writes are another client's (the sync never writes to OpenCloud, Q1): the driver
        // with two-way capabilities stands in for it.
        let mut other = WebDav::new(session(&server), two_way(), "Studio");
        let Written::Saved { mut etag } = other.create("Docs/a.txt", &a).unwrap() else { panic!() };
        for i in 1..=2 {
            std::fs::write(&a, format!("v{i}")).unwrap();
            let Written::Saved { etag: new } = other.replace("Docs/a.txt", &etag, &a).unwrap() else { panic!("{i}") };
            etag = new;
        }
        let mut s = driver(&server);
        let versions = s.list_versions("Docs/a.txt").unwrap();
        assert_eq!(versions.len(), 2, "the current file isn't listed: {versions:#?}");
        assert!(oc::rev_time(&versions[0].stamp) > oc::rev_time(&versions[1].stamp), "newest first");
        assert!(versions[0].when().ends_with(" UTC") && !versions[0].when().contains(".REV."), "{}", versions[0].when());
        assert!(s.list_versions("Docs/none.txt").unwrap().is_empty());
        s.keep_version("Docs/a.txt").unwrap();
        assert!(server.seen().iter().all(|r| r.method != "COPY"), "no client versions: {:?}", server.seen());
        dav.lock().unwrap().answer(
            "MOVE",
            &format!("{}/Docs/a.txt", base_path()),
            &[("Destination".into(), format!("{}{}/Docs/b.txt", server.url, base_path())), ("Overwrite".into(), "F".into())],
            b"",
        );
        assert_eq!(s.list_versions("Docs/b.txt").unwrap().len(), 2, "the history followed the rename");
        let before = server.seen().len();
        assert_eq!(s.restore_version("Docs/b.txt", &versions[1].stamp).unwrap(), crate::storage::Restored::Done);
        assert_eq!(dav.lock().unwrap().body("Docs/b.txt"), Some(b"v0".as_slice()));
        assert_eq!(s.list_versions("Docs/b.txt").unwrap().len(), 3, "the replaced file was kept by the server");
        assert!(matches!(s.restore_version("Docs/b.txt", "nope").unwrap_err(), StorageError::NotFound(_)));
        let restore = server.seen()[before..].to_vec();
        let writes: Vec<_> = restore.iter().filter(|r| !matches!(r.method.as_str(), "PROPFIND" | "GET")).collect();
        assert_eq!(writes.len(), 1, "one write, the guarded PUT: {writes:?}");
        assert!(writes[0].method == "PUT" && writes[0].header("if-match").is_some(), "{:?}", writes[0]);
        assert!(
            restore.iter().any(|r| r.method == "GET" && r.path.contains("/remote.php/dav/meta/") && r.path.contains("/v/")),
            "the version's bytes are read"
        );
        assert!(server.seen().iter().all(|r| r.method != "COPY"), "never the server's own COPY restore");
        // Unconfirmed (client versions): the same server, the history is CraftCloud's.
        let mut client = WebDav::new(session(&server), two_way(), "Laptop").with_profile(profile(&server));
        client.keep_version("Docs/b.txt").unwrap();
        assert!(server.seen().iter().any(|r| r.method == "COPY" && r.path.contains("/Docs/b.txt")));
    }

    /// Detection and the Graph API: `status.php` says OpenCloud and its product version; `me` and
    /// `me/drives` name the user and the personal space, from which the base is built.
    #[test]
    fn detection_and_the_personal_space() {
        let (server, _) = opencloud(Flags::default());
        let d = detect(&server.url).unwrap();
        assert_eq!((d.product.as_str(), d.version.as_str()), ("OpenCloud", "9.1.1"));
        assert!(detect(&crate::storage::webdav::tests::scripted(Flags::default()).0.url).is_none(), "a plain WebDAV server isn't OpenCloud");
        assert!(
            detect(&crate::storage::webdav::tests::scripted(Flags { nextcloud: true, ..Flags::default() }).0.url).is_none(),
            "nor is a Nextcloud"
        );
        let mut session = AppPasswordSession::new(Base::parse(&format!("{}/", server.url)).unwrap(), "john", "SECRET-app-token").unwrap();
        let (me, drive) = personal_space(&mut session, &server.url).unwrap();
        assert_eq!(me.user_name, "john");
        assert_eq!(drive.id, FAKE_DRIVE);
        assert_eq!(oc::spaces_base(&server.url, &drive.id), format!("{}{}/", server.url, base_path()));
        assert!(personal_space(&mut session, &crate::storage::webdav::tests::scripted(Flags::default()).0.url).is_err());
    }
}
