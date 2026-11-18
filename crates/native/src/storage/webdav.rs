//! The generic WebDAV driver: any server that speaks WebDAV, with CraftCloud's own trash and
//! versions in `.craftcloud/` where the server has none (docs/WEBDAV-STORAGE.md § 7.4, "Other
//! WebDAV"; § 8, every row's "Server lacks it" column).
//!
//! What this driver holds the line on:
//! - **The base URL is the driver's.** Every path hangs under it, and every href a listing names
//!   is read against it (`Base::relative`); one outside it fails the listing, which the engine
//!   takes as unknown, never as empty. A listing is one level (`Depth: 1`); a non-207 or an
//!   unreadable multistatus fails it too.
//! - **No unguarded write.** A create is a `PUT` with `If-None-Match: *`, an overwrite a `PUT`
//!   with `If-Match`, and a 412 is [`Written::Conflict`]. A `MOVE` or `COPY` always carries
//!   `Overwrite: F`. A file is moved or trashed only while a fresh listing shows it at the
//!   version planned (`still_at`, as the OxiCloud driver does), a folder only while a fresh
//!   listing of every level holds exactly the files planned (`cloud_holds`).
//! - **A delete is a `MOVE` into `.craftcloud/trash/<time>-<device>/<path>`**, never a `DELETE`:
//!   the trashed item keeps its cloud path below the entry's folder, so a restore knows where it
//!   goes, and a folder goes as one `MOVE`, whole. A `DELETE` leaves here only through
//!   [`RemoteStorage::purge_own`], for an [`OwnPath`] inside `.craftcloud/` (a pruned version, an
//!   emptied trash entry's folder).
//! - **The bytes an overwrite replaces** are `COPY`d to `.craftcloud/versions/<path>/<time>`, 5
//!   kept, as OxiCloud's driver does ([`super::versions`], one body of code for both).
//! - **What the server guarantees is measured, never assumed**: the [`Capabilities`] come from the
//!   check on connect ([`super::probe`]), saved in `storage-check.json` or run afresh by
//!   [`WebDav::connect`]. The engine takes its mode from them (`mode_for`), so a server that
//!   ignores a guard gets download only, and this driver is then never asked to write.
//!
//! Not in this driver yet (docs/WEBDAV-STORAGE.md § 14, pull request 4b): the trash's retention
//! (entries older than 30 days by the server's clock, through `purge_own`), `craftcloud trash
//! empty`, the daily re-run of the check, and the runtime watch. A trash that is never emptied
//! is safe; it only takes room.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use craftcloud_core::api::dav::{self, Uploaded, Write};
use craftcloud_core::api::expect_ok;
use craftcloud_core::http::{HttpRequest, Method};
use craftcloud_core::names;
use craftcloud_core::storage::Capabilities;
use craftcloud_core::storage::probe::{Base, Listed, PROPFIND_BODY, Profile, StoredCheck, parse_listing};

use super::probe::{self, Limits, load_check, save_check};
use super::{Download, Entry, MoveGuard, Moved, OwnPath, RemoteStorage, StorageError, TrashGuard, TrashItem, Trashed, Written, versions};
use crate::session::AppPasswordSession;

/// How a trash entry's time is written in its folder's name (UTC, whole seconds).
const TRASH_STAMP: &str = "%Y%m%dT%H%M%SZ";

pub struct WebDav {
    session: AppPasswordSession,
    base: Base,
    caps: Capabilities,
    /// This device's name, in the trash entries it makes.
    device: String,
}

impl WebDav {
    /// A driver on `session` with capabilities already known (a saved check).
    pub fn new(session: AppPasswordSession, caps: Capabilities, device: &str) -> Self {
        let base = session.base().clone();
        Self { session, base, caps, device: device.to_owned() }
    }

    /// Connects: the check saved in `settings_dir` for this base, or a fresh probe run (§ 7.5),
    /// saved there. A check that was refused leaves a driver whose capabilities allow nothing
    /// (`Capabilities::unknown`), so the engine refuses to sync and says why.
    pub fn connect(mut session: AppPasswordSession, device: &str, settings_dir: Option<&Path>, limits: Limits) -> (Self, StoredCheck) {
        let base = session.base().clone();
        let saved = settings_dir.and_then(load_check).filter(|c| c.server == base.url());
        let check = match saved {
            Some(check) => check,
            None => {
                let verdict = probe::run(&mut session, &base, &probe::folder_name(), Profile::default(), limits);
                let now = jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
                let check = StoredCheck::from_verdict(&verdict, base.url(), "", &now);
                if let Some(dir) = settings_dir
                    && let Err(e) = save_check(dir, &check)
                {
                    log::warn!("couldn't save the server check: {e}");
                }
                check
            }
        };
        let caps = check.capabilities.clone().unwrap_or_else(Capabilities::unknown);
        (Self::new(session, caps, device), check)
    }

    pub fn base(&self) -> &Base {
        &self.base
    }

    pub fn session(&mut self) -> &mut AppPasswordSession {
        &mut self.session
    }

    fn propfind(&self, path: &str, folder: bool, depth: &str) -> HttpRequest {
        HttpRequest::new(Method::Propfind, dav::url_under(self.base.url(), path, folder))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec())
    }

    /// One folder's listing, read against the base; the folder itself left out. A 404 is
    /// [`StorageError::NotFound`]; anything else that isn't a readable 207 fails it.
    fn listing(&mut self, folder: &str) -> Result<Vec<Listed>, StorageError> {
        let resp = self.session.call(&self.propfind(folder, true, "1"))?;
        if resp.status == 404 {
            return Err(StorageError::NotFound(format!("listing {folder:?}: 404")));
        }
        let me = folder.trim_matches('/');
        let mut entries = parse_listing(&resp, &self.base).map_err(|e| format!("listing {folder:?}: {e}"))?;
        entries.retain(|e| e.path != me);
        Ok(entries)
    }

    /// Whether the cloud file at `path` is still at version `etag` (`None`: it's gone). A file
    /// listed without an ETag can't be shown to be at `etag`: `Some(Err(()))`.
    fn still_at(&mut self, path: &str, etag: &str) -> Result<Option<Result<bool, ()>>, StorageError> {
        let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
        match self.listing(parent).map(|entries| entries.into_iter().find(|e| e.path == path)) {
            Ok(Some(e)) => {
                let now = e.etag.as_deref().map(|now| now.trim_matches('"')).filter(|now| !now.is_empty());
                Ok(Some(now.map(|now| now == etag.trim_matches('"')).ok_or(())))
            }
            Ok(None) | Err(StorageError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Whether the cloud folder `path` holds exactly `files` (path, ETag), nothing more or newer,
    /// listed again now, every level. A listing that fails is an error: unknown, never empty.
    fn cloud_holds(&mut self, path: &str, files: &[(String, String)]) -> Result<bool, StorageError> {
        let mut found: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut level = vec![path.to_owned()];
        while let Some(folder) = level.pop() {
            for e in self.listing(&folder)? {
                if e.is_folder {
                    level.push(e.path);
                } else {
                    found.insert(e.path, e.etag);
                }
            }
        }
        Ok(found.len() == files.len()
            && files.iter().all(|(p, etag)| {
                let now = found.get(p).and_then(Option::as_deref).map(|now| now.trim_matches('"')).filter(|now| !now.is_empty());
                now.is_some_and(|now| now == etag.trim_matches('"'))
            }))
    }

    /// Makes `folder` and its missing parents (`MKCOL`, parent first); one already there is fine.
    fn make_folders(&mut self, folder: &str) -> Result<(), StorageError> {
        let parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
        for n in 1..=parts.len() {
            let step = parts[..n].join("/");
            let resp = self.session.call(&dav::create_folder_under(self.base.url(), &step))?;
            if !dav::folder_exists_after(&resp) {
                return Err(StorageError::Failed(format!("making the folder {step}: the server answered {}", resp.status)));
            }
        }
        Ok(())
    }

    /// A fresh trash entry's folder (`.craftcloud/trash/<time>-<device>`, made now), with the
    /// parents of `path` made inside it: where `path` goes.
    fn trash_place(&mut self, path: &str) -> Result<String, StorageError> {
        self.make_folders(dav::TRASH_DIR)?;
        let now = jiff::Timestamp::now().strftime(TRASH_STAMP).to_string();
        for n in 1..1000 {
            let entry = format!("{}/{}", dav::TRASH_DIR, dav::trash_entry_name(&now, &self.device, n));
            let resp = self.session.call(&dav::create_folder_under(self.base.url(), &entry))?;
            match resp.status {
                201 => {
                    if let Some((parent, _)) = path.rsplit_once('/') {
                        self.make_folders(&format!("{entry}/{parent}"))?;
                    }
                    return Ok(format!("{entry}/{path}"));
                }
                // Taken (the same second, device and name): the next number.
                405 => {}
                status => return Err(StorageError::Failed(format!("making a trash entry: the server answered {status}"))),
            }
        }
        Err(StorageError::Failed("no free place in the trash".into()))
    }

    /// `MOVE` `path` into a fresh trash entry, never onto an existing name. A 412 (the place was
    /// taken meanwhile) is an error, tried again next pass with a new entry; nothing is forced.
    fn move_to_trash(&mut self, path: &str, folder: bool) -> Result<(), StorageError> {
        let dest = self.trash_place(path)?;
        let resp = self.session.call(&dav::move_to_under(self.base.url(), path, &dest, folder))?;
        if resp.is_success() || resp.status == 404 {
            Ok(())
        } else {
            Err(StorageError::Failed(format!("moving {path} to the trash: the server answered {}", resp.status)))
        }
    }

    /// One guarded `PUT` of `file`.
    fn upload(&mut self, path: &str, file: &Path, write: &Write) -> Result<Written, StorageError> {
        let req = dav::upload_under(self.base.url(), path, Vec::new(), write);
        let resp = self.session.upload(&req, file)?;
        if resp.status == 507 {
            return Err(StorageError::NoRoom);
        }
        if matches!(resp.status, 404 | 409) {
            // Both mean the folder isn't there (§ 7.3, `missing_parent`): the engine makes the
            // folders first, so this is a folder gone meanwhile.
            let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
            return Err(StorageError::NotFound(format!("the folder {parent:?} isn't there (the server answered {})", resp.status)));
        }
        if matches!(resp.status, 400 | 403 | 414 | 415 | 422) {
            // Refused for what it is, not for now.
            return Err(StorageError::Refused(format!("the server answered {}", resp.status)));
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

    /// The trashed items in one trash entry's folder. The entry holds one item at its cloud path,
    /// below the parents made for it, so the walk goes down while a folder holds exactly one
    /// folder: a file is the item, and so is a folder holding nothing, or more than one thing. A
    /// trashed folder whose only content is one folder (or one file) lists as that; a restore
    /// puts the same tree back.
    fn items_in(&mut self, entry: &str, trashed_at: &str) -> Result<Vec<TrashItem>, StorageError> {
        let item = |trash_path: &str| TrashItem {
            id: trash_path.to_owned(),
            path: trash_path.strip_prefix(entry).unwrap_or(trash_path).trim_start_matches('/').to_owned(),
            trashed_at: trashed_at.to_owned(),
        };
        let mut folder = entry.to_owned();
        loop {
            let entries = self.listing(&folder)?;
            match entries.as_slice() {
                [] if folder == entry => return Ok(Vec::new()),
                [one] if one.is_folder => folder = one.path.clone(),
                [one] => return Ok(vec![item(&one.path)]),
                many if folder == entry => return Ok(many.iter().map(|e| item(&e.path)).collect()),
                _ => return Ok(vec![item(&folder)]),
            }
        }
    }

    /// Whether `path` is a folder (a `PROPFIND` of depth 0); not found when nothing is there.
    fn is_folder(&mut self, path: &str) -> Result<bool, StorageError> {
        let resp = self.session.call(&self.propfind(path, false, "0"))?;
        if resp.status == 404 {
            return Err(StorageError::NotFound(format!("{path}: 404")));
        }
        let listed = parse_listing(&resp, &self.base).map_err(|e| format!("listing {path:?}: {e}"))?;
        Ok(listed.iter().find(|e| e.path == path.trim_matches('/')).is_some_and(|e| e.is_folder))
    }

    /// Whether `folder` holds no file at any level (a listing that fails is unknown: `false`).
    fn holds_nothing(&mut self, folder: &str) -> bool {
        let mut level = vec![folder.to_owned()];
        while let Some(f) = level.pop() {
            let Ok(entries) = self.listing(&f) else { return false };
            for e in entries {
                if e.is_folder { level.push(e.path) } else { return false }
            }
        }
        true
    }
}

/// `20261117T143205Z-Laptop` → `2026-11-17T14:32:05Z` (the entry's time, RFC 3339).
fn entry_time(name: &str) -> String {
    let b = name.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).filter(|s| s.iter().all(u8::is_ascii_digit)).map(|_| &name[r]);
    match (digits(0..4), digits(4..6), digits(6..8), b.get(8), digits(9..11), digits(11..13), digits(13..15)) {
        (Some(y), Some(mo), Some(d), Some(b'T'), Some(h), Some(mi), Some(s)) => format!("{y}-{mo}-{d}T{h}:{mi}:{s}Z"),
        _ => String::new(),
    }
}

impl From<Listed> for Entry {
    fn from(e: Listed) -> Self {
        Self {
            path: e.path,
            is_folder: e.is_folder,
            etag: e.etag,
            size: e.size,
            modified: e.modified,
            file_id: e.file_id,
            checksum: e.checksums.into_iter().next(),
        }
    }
}

impl RemoteStorage for WebDav {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn begin_pass(&mut self) {
        self.session.offline = None;
    }

    fn take_offline(&mut self) -> Option<String> {
        self.session.offline.take()
    }

    fn signed_out(&self) -> bool {
        self.session.signed_out
    }

    fn list(&mut self, folder: &str) -> Result<Vec<Entry>, StorageError> {
        Ok(self.listing(folder)?.into_iter().map(Entry::from).collect())
    }

    fn download(&mut self, path: &str, from: Option<u64>) -> Result<Download, StorageError> {
        let mut req = dav::download_under(self.base.url(), path);
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
        let resp = self.session.call(&dav::create_folder_under(self.base.url(), path))?;
        if dav::folder_exists_after(&resp) { Ok(()) } else { Err(StorageError::Status(resp.status)) }
    }

    fn move_to(&mut self, from: &str, to: &str, guard: MoveGuard) -> Result<Moved, StorageError> {
        let folder = match guard {
            MoveGuard::File { etag } => {
                // Checked by a fresh listing first, whatever the server makes of `If-Match` on a
                // `MOVE` (§ 5: not every server honours it).
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
        let resp = self.session.call(&dav::move_to_under(self.base.url(), from, to, folder))?;
        if resp.is_success() { Ok(Moved::Done) } else { Ok(Moved::Refused(resp.status)) }
    }

    fn trash(&mut self, path: &str, guard: TrashGuard) -> Result<Trashed, StorageError> {
        match guard {
            TrashGuard::File { etag } => {
                match self.still_at(path, &etag)? {
                    None => return Ok(Trashed::Done),
                    Some(Err(())) => return Ok(Trashed::Unversioned),
                    Some(Ok(false)) => return Ok(Trashed::Changed),
                    Some(Ok(true)) => {}
                }
                self.move_to_trash(path, false)?;
            }
            TrashGuard::Folder { files } => {
                if !self.cloud_holds(path, &files)? {
                    return Ok(Trashed::Changed);
                }
                self.move_to_trash(path, true)?;
            }
            TrashGuard::EmptyFolder => {
                // The cloud must say it's empty right now; a listing that fails says nothing (T-064).
                match self.listing(path) {
                    Ok(entries) if entries.is_empty() => {}
                    _ => return Ok(Trashed::Changed),
                }
                self.move_to_trash(path, true)?;
            }
        }
        Ok(Trashed::Done)
    }

    fn trash_list(&mut self) -> Result<Vec<TrashItem>, StorageError> {
        let entries = match self.listing(dav::TRASH_DIR) {
            Ok(entries) => entries,
            Err(StorageError::NotFound(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut items = Vec::new();
        for entry in entries.iter().filter(|e| e.is_folder) {
            let at = entry_time(entry.name());
            items.extend(self.items_in(&entry.path, &at)?);
        }
        Ok(items)
    }

    /// `MOVE` back to where it was, never onto an existing name: a name that is taken gets
    /// `names::restored_name`. The entry's folder goes (for good, inside `.craftcloud/`) once it
    /// holds nothing.
    fn restore(&mut self, item: &TrashItem) -> Result<(), StorageError> {
        // The entry's folder, `.craftcloud/trash/<time>-<device>`, from the item's trash path.
        let entry = item
            .id
            .strip_prefix(dav::TRASH_DIR)
            .and_then(|r| r.trim_start_matches('/').split('/').next())
            .filter(|e| !e.is_empty())
            .map(|e| format!("{}/{e}", dav::TRASH_DIR));
        let is_folder = self.is_folder(&item.id)?;
        if let Some((parent, _)) = item.path.rsplit_once('/') {
            self.make_folders(parent)?;
        }
        let mut dest = item.path.clone();
        let mut restored = false;
        for n in 1..100 {
            let resp = self.session.call(&dav::move_to_under(self.base.url(), &item.id, &dest, is_folder))?;
            match resp.status {
                412 => dest = names::restored_name(&item.path, is_folder, n),
                404 => return Err(StorageError::NotFound(format!("{} isn't in the trash", item.path))),
                _ => {
                    expect_ok(&resp).map_err(|e| StorageError::Failed(format!("restoring {}: {e}", item.path)))?;
                    restored = true;
                    break;
                }
            }
        }
        if !restored {
            return Err(StorageError::Failed(format!("restoring {}: every name next to it is taken; it stays in the trash", item.path)));
        }
        if let Some(entry) = entry
            && self.holds_nothing(&entry)
            && let Some(own) = OwnPath::new(&entry)
        {
            let _ = self.purge_own(&own);
        }
        Ok(())
    }

    fn keep_version(&mut self, path: &str) -> Result<(), StorageError> {
        Ok(versions::keep(&mut self.session, &self.base, path)?)
    }

    fn move_versions(&mut self, from: &str, to: &str) -> Result<(), StorageError> {
        Ok(versions::follow_move(&mut self.session, &self.base, from, to)?)
    }

    /// From WebDAV's `quota-available-bytes` on the base, where the server lists it; a negative
    /// number (Nextcloud's `-3`) or none is unknown.
    fn free_space(&mut self, _needed: u64) -> Result<Option<u64>, StorageError> {
        let resp = self.session.call(&self.propfind("", true, "0"))?;
        let listed = parse_listing(&resp, &self.base).map_err(|e| format!("listing the server: {e}"))?;
        Ok(listed.into_iter().find_map(|e| e.quota_available).and_then(|q| u64::try_from(q).ok()))
    }

    fn purge_own(&mut self, path: &OwnPath) -> Result<(), StorageError> {
        let folder = match self.is_folder(path.as_str()) {
            Ok(folder) => folder,
            Err(StorageError::NotFound(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let resp = self.session.call(&dav::delete_under(self.base.url(), path.as_str(), folder))?;
        if resp.is_success() || resp.status == 404 { Ok(()) } else { Err(StorageError::Status(resp.status)) }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use craftcloud_core::storage::probe::fake::{FakeDav, Flags};
    use craftcloud_core::storage::{Capabilities, Mode, mode_for};

    use super::{Base, WebDav, entry_time};
    use crate::fake_server::{Answer, FakeServer, Seen};
    use crate::session::{APP_PASSWORD_REFUSED, AppPasswordSession};
    use crate::storage::probe::Limits;
    use crate::storage::{MoveGuard, Moved, OwnPath, RemoteStorage, StorageError, TrashGuard, Trashed, Written};

    const BASE_PATH: &str = "/remote.php/dav/files/john";

    /// A scripted WebDAV server behind a loopback HTTP server, under a Nextcloud-shaped base.
    pub(crate) fn scripted(flags: Flags) -> (FakeServer, Arc<Mutex<FakeDav>>) {
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
        (server, dav)
    }

    pub(crate) fn base(server: &FakeServer) -> Base {
        Base::parse(&format!("{}{BASE_PATH}/", server.url)).unwrap()
    }

    pub(crate) fn session(server: &FakeServer) -> AppPasswordSession {
        AppPasswordSession::new(base(server), "john", "SECRET-app-pw").unwrap()
    }

    /// A driver with the capabilities a well-behaved server measures (two-way).
    pub(crate) fn driver(server: &FakeServer) -> WebDav {
        WebDav::new(session(server), two_way(), "Laptop")
    }

    /// What the probe measures on the well-behaved scripted server: two-way, client trash and
    /// versions.
    pub(crate) fn two_way() -> Capabilities {
        use craftcloud_core::storage::{DeleteGuard, FolderEtags, Guard, MissingParent, Quota, Trash, Versions};
        Capabilities {
            create_if_absent: Guard::Honoured,
            replace_if_match: Guard::Honoured,
            delete_if_match: DeleteGuard::Honoured,
            move_no_overwrite: true,
            etag_changes_on_write: true,
            etag_kept_by_move: true,
            put_etag_is_listed: true,
            folder_etags: FolderEtags::Follow,
            missing_parent: MissingParent::Conflict,
            depth_infinity: false,
            copy: true,
            file_ids: true,
            checksums: None,
            case_insensitive: false,
            trash: Trash::Client,
            versions: Versions::Client,
            quota: Quota::WebDavProps,
            names: craftcloud_core::names::NameRules::oxicloud(),
        }
    }

    fn file(dir: &std::path::Path, name: &str, bytes: &str) -> std::path::PathBuf {
        let file = dir.join(name);
        std::fs::write(&file, bytes).unwrap();
        file
    }

    /// Every request carries the app password as Basic auth, every write its guard, and a
    /// `DELETE` goes only inside `.craftcloud/` (the request-log check the golden tests make).
    #[test]
    fn every_write_carries_its_guard_and_the_password() {
        let (server, dav) = scripted(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "one");
        let mut s = driver(&server);
        let Written::Saved { etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!("not saved") };
        std::fs::write(&a, "two").unwrap();
        assert!(matches!(s.replace("Docs/a.txt", &etag, &a).unwrap(), Written::Saved { .. }));
        assert_eq!(s.replace("Docs/a.txt", "\"stale\"", &a).unwrap(), Written::Conflict, "a stale version tag is a conflict, not an error");
        assert_eq!(s.create("Docs/a.txt", &a).unwrap(), Written::Conflict, "something is there");
        s.make_folder("Docs/Sub").unwrap();
        assert_eq!(s.move_to("Docs/Sub", "Docs/Moved", MoveGuard::Folder).unwrap(), Moved::Done);
        s.keep_version("Docs/a.txt").unwrap();
        s.purge_own(&OwnPath::new(".craftcloud/versions/Docs/a.txt").unwrap()).unwrap();
        let seen = server.seen();
        let expected = format!("Basic {}", crate::session::base64(b"john:SECRET-app-pw"));
        for r in &seen {
            assert_eq!(r.header("authorization"), Some(expected.as_str()), "{r:?}");
            assert!(r.path.starts_with(BASE_PATH), "under the base: {r:?}");
            match r.method.as_str() {
                "PUT" => assert!(r.header("if-match").is_some() || r.header("if-none-match") == Some("*"), "an unguarded PUT: {r:?}"),
                "MOVE" | "COPY" => assert_eq!(r.header("overwrite"), Some("F"), "{r:?}"),
                "DELETE" => assert!(r.path.starts_with(&format!("{BASE_PATH}/.craftcloud/")), "a DELETE outside .craftcloud/: {r:?}"),
                _ => {}
            }
        }
        assert_eq!(seen.iter().filter(|r| r.method == "PUT").count(), 4);
        assert!(seen.iter().any(|r| r.method == "COPY" && r.header("destination").is_some_and(|d| d.contains("/.craftcloud/versions/Docs/a.txt/"))));
        let dav = dav.lock().unwrap();
        assert!(dav.has("Docs/Moved") && !dav.has("Docs/Sub"));
        assert_eq!(dav.body("Docs/a.txt"), Some(b"two".as_slice()), "the stale overwrite changed nothing");
    }

    /// A listing that can't be trusted fails, never reads as empty: a server error, an answer
    /// that isn't a multistatus, an href outside the base. A missing folder is not found.
    #[test]
    fn a_listing_that_cant_be_trusted_is_an_error_not_empty() {
        let outside = format!(
            r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>{BASE_PATH}/Docs/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response><d:response><d:href>/remote.php/dav/files/jane/secret.txt</d:href><d:propstat><d:prop><d:resourcetype/><d:getetag>"x"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
        );
        for (status, body, what) in
            [(500, String::new(), "a server error"), (200, "<html/>".to_owned(), "not a multistatus"), (207, outside, "an href outside the base")]
        {
            let server = FakeServer::start(move |_| (status, body.clone()));
            let mut s = driver(&server);
            let err = s.list("Docs").unwrap_err();
            assert!(!matches!(err, StorageError::NotFound(_)), "{what}: {err:?}");
            assert!(err.to_string().starts_with("listing \"Docs\""), "{what}: {err}");
        }
        let server = FakeServer::start(|_| (404, String::new()));
        assert!(matches!(driver(&server).list("Gone").unwrap_err(), StorageError::NotFound(_)));
        // A fine listing: entries with ETags and sizes, the folder itself left out.
        let (server, dav) = scripted(Flags::default());
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder("Docs/Sub", None);
            dav.add_file("Docs/a.txt", b"abc");
        }
        let listed = driver(&server).list("Docs").unwrap();
        let names: Vec<(&str, bool, u64)> = listed.iter().map(|e| (e.path.as_str(), e.is_folder, e.size)).collect();
        assert_eq!(names, [("Docs/Sub", true, 0), ("Docs/a.txt", false, 3)]);
        assert!(listed[1].etag.is_some() && listed[1].file_id.is_some());
    }

    /// A file deleted here goes to `.craftcloud/trash/<time>-<device>/<path>` by one `MOVE`, is
    /// listed there, and a restore moves it back (next to a file of its name as `(restored)`);
    /// the entry's folder goes once it is empty. A file that changed in the cloud stays.
    #[test]
    fn trash_and_restore_round_trip() {
        let (server, dav) = scripted(Flags::default());
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder("Docs", None);
            dav.add_file("Docs/a.txt", b"abc");
            dav.add_file("Docs/b.txt", b"b");
        }
        let mut s = driver(&server);
        let etag = s.list("Docs").unwrap().into_iter().find(|e| e.path == "Docs/a.txt").unwrap().etag.unwrap();
        assert_eq!(s.trash("Docs/b.txt", TrashGuard::File { etag: "\"stale\"".into() }).unwrap(), Trashed::Changed);
        assert!(dav.lock().unwrap().has("Docs/b.txt"), "changed in the cloud: stays");
        assert_eq!(s.trash("Docs/a.txt", TrashGuard::File { etag: etag.clone() }).unwrap(), Trashed::Done);
        {
            let dav = dav.lock().unwrap();
            assert!(!dav.has("Docs/a.txt"));
            let trashed: Vec<String> = dav.folders().into_iter().filter(|f| f.starts_with(".craftcloud/trash/")).collect();
            assert_eq!(trashed.len(), 2, "the entry and the parent made in it: {trashed:?}");
            let entry = trashed[0].rsplit('/').next().unwrap().to_owned();
            assert!(entry.ends_with("Z-Laptop") && entry.len() == "20261117T143205Z-Laptop".len(), "<time>-<device>: {entry}");
            assert_eq!(dav.body(&format!("{}/Docs/a.txt", trashed[0])), Some(b"abc".as_slice()));
        }
        assert!(server.seen().iter().all(|r| r.method != "DELETE"), "a trash is never a DELETE: {:?}", server.seen());
        assert_eq!(server.seen().iter().filter(|r| r.method == "MOVE").count(), 1);
        let items = s.trash_list().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "Docs/a.txt");
        assert!(items[0].id.starts_with(".craftcloud/trash/") && items[0].id.ends_with("/Docs/a.txt"));
        assert_eq!(items[0].trashed_at.len(), "2026-11-17T14:32:05Z".len(), "{}", items[0].trashed_at);

        // A file of its name is there now: restored next to it.
        dav.lock().unwrap().add_file("Docs/a.txt", b"new");
        s.restore(&items[0]).unwrap();
        {
            let dav = dav.lock().unwrap();
            assert_eq!(dav.body("Docs/a.txt"), Some(b"new".as_slice()), "never overwritten");
            assert_eq!(dav.body("Docs/a (restored).txt"), Some(b"abc".as_slice()));
            assert!(dav.folders().iter().all(|f| !f.starts_with(".craftcloud/trash/")), "the emptied entry is gone: {:?}", dav.folders());
        }
        assert!(s.trash_list().unwrap().is_empty());
        let deletes: Vec<String> = server.seen().iter().filter(|r| r.method == "DELETE").map(|r| r.path.clone()).collect();
        assert!(deletes.iter().all(|p| p.contains("/.craftcloud/trash/")), "{deletes:?}");
    }

    /// A whole folder deleted here goes to the trash as one `MOVE`, only while a fresh listing
    /// of every level holds exactly the files planned; a restore brings it back whole.
    #[test]
    fn a_folder_goes_to_the_trash_whole() {
        let (server, dav) = scripted(Flags::default());
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder("Old/Sub", None);
            dav.add_file("Old/a.txt", b"a");
            dav.add_file("Old/Sub/b.txt", b"b");
        }
        let mut s = driver(&server);
        let etag = |s: &mut WebDav, folder: &str, name: &str| s.list(folder).unwrap().into_iter().find(|e| e.path == name).unwrap().etag.unwrap();
        let (a, b) = (etag(&mut s, "Old", "Old/a.txt"), etag(&mut s, "Old/Sub", "Old/Sub/b.txt"));
        let planned = vec![("Old/a.txt".to_owned(), a.clone()), ("Old/Sub/b.txt".to_owned(), b.clone())];
        dav.lock().unwrap().add_file("Old/late.txt", b"late");
        assert_eq!(s.trash("Old", TrashGuard::Folder { files: planned.clone() }).unwrap(), Trashed::Changed, "a file added meanwhile keeps it");
        assert!(dav.lock().unwrap().has("Old/late.txt"));
        let planned_late = [planned.clone(), vec![("Old/late.txt".to_owned(), etag(&mut s, "Old", "Old/late.txt"))]].concat();
        assert_eq!(s.trash("Old", TrashGuard::Folder { files: planned_late }).unwrap(), Trashed::Done);
        let moves = server.seen().iter().filter(|r| r.method == "MOVE").count();
        assert_eq!(moves, 1, "one MOVE for the whole folder: {:?}", server.seen());
        assert!(!dav.lock().unwrap().has("Old"));
        let items = s.trash_list().unwrap();
        assert_eq!(items.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["Old"]);
        s.restore(&items[0]).unwrap();
        let dav = dav.lock().unwrap();
        assert_eq!(dav.body("Old/Sub/b.txt"), Some(b"b".as_slice()));
        assert_eq!(dav.body("Old/late.txt"), Some(b"late".as_slice()));

        // An empty folder goes only while a fresh listing says it is empty.
        drop(dav);
        let (server, dav) = scripted(Flags::default());
        dav.lock().unwrap().add_folder("Empty", None);
        let mut s = driver(&server);
        assert_eq!(s.trash("Empty", TrashGuard::EmptyFolder).unwrap(), Trashed::Done);
        assert!(!dav.lock().unwrap().has("Empty"));
        dav.lock().unwrap().add_file("NotEmpty/x.txt", b"x");
        dav.lock().unwrap().add_folder("NotEmpty", None);
        assert_eq!(s.trash("NotEmpty", TrashGuard::EmptyFolder).unwrap(), Trashed::Changed);
    }

    /// The bytes an overwrite replaces are kept as versions by `COPY`, the newest five, and a
    /// rename takes them along.
    #[test]
    fn versions_are_kept_pruned_and_follow_a_move() {
        let (server, dav) = scripted(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "v0");
        let mut s = driver(&server);
        let Written::Saved { mut etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!() };
        for i in 1..=7 {
            s.keep_version("Docs/a.txt").unwrap();
            std::fs::write(&a, format!("v{i}")).unwrap();
            let Written::Saved { etag: new } = s.replace("Docs/a.txt", &etag, &a).unwrap() else { panic!("{i}") };
            etag = new;
            std::thread::sleep(Duration::from_millis(3));
        }
        let kept = |dav: &FakeDav, path: &str| dav.folders().into_iter().filter(|f| f.starts_with(&format!(".craftcloud/versions/{path}"))).count();
        let listed = s.list(".craftcloud/versions/Docs/a.txt").unwrap();
        assert_eq!(listed.len(), 5, "{listed:?}");
        assert_eq!(kept(&dav.lock().unwrap(), "Docs/a.txt"), 1);
        assert_eq!(s.move_to("Docs/a.txt", "Docs/b.txt", MoveGuard::File { etag }).unwrap(), Moved::Done);
        s.move_versions("Docs/a.txt", "Docs/b.txt").unwrap();
        assert_eq!(s.list(".craftcloud/versions/Docs/b.txt").unwrap().len(), 5);
        assert!(matches!(s.list(".craftcloud/versions/Docs/a.txt"), Err(StorageError::NotFound(_))));
        let deletes = server.seen().iter().filter(|r| r.method == "DELETE").count();
        assert_eq!(deletes, 2, "the two oldest versions were pruned: {:?}", server.seen());
    }

    /// Free space comes from `quota-available-bytes` where the server lists it; a negative number
    /// (Nextcloud's `-3`) or none is unknown.
    #[test]
    fn free_space_from_the_quota_props() {
        for (quota, expected) in [(Some(5_000), Some(5_000)), (Some(-3), None), (None, None)] {
            let (server, _) = scripted(Flags { quota_available: quota, ..Flags::default() });
            assert_eq!(driver(&server).free_space(1).unwrap(), expected, "{quota:?}");
        }
    }

    /// A password travels over https, or over plain http to this computer only.
    #[test]
    fn a_password_never_goes_over_plain_http_to_another_computer() {
        let new = |url: &str| AppPasswordSession::new(Base::parse(url).unwrap(), "john", "pw").map(|_| ());
        assert!(new("http://cloud.example.org/dav/").unwrap_err().contains("isn't https"));
        assert!(new("http://192.168.1.10/dav/").is_err());
        for fine in
            ["https://cloud.example.org/dav/", "http://127.0.0.1:8095/remote.php/dav/files/john/", "http://localhost/dav/", "http://[::1]:8080/dav/"]
        {
            assert!(new(fine).is_ok(), "{fine}");
        }
        let shown = format!("{:?}", AppPasswordSession::new(Base::parse("https://cloud.example.org/dav/").unwrap(), "john", "SECRET").unwrap());
        assert!(!shown.contains("SECRET") && !shown.contains("Basic"), "{shown}");
    }

    /// A 401 is "signed out": the app password was revoked on the server, nothing refreshes.
    #[test]
    fn a_refused_password_is_signed_out() {
        let server = FakeServer::start(|_| (401, String::new()));
        let mut s = driver(&server);
        let err = s.list("").unwrap_err();
        assert_eq!(err.to_string(), APP_PASSWORD_REFUSED);
        assert!(s.signed_out() && s.take_offline().is_none());
        assert_eq!(server.seen().len(), 1, "no refresh, no retry");
        let url = server.url.clone();
        drop(server);
        let mut gone =
            WebDav::new(AppPasswordSession::new(Base::parse(&format!("{url}{BASE_PATH}/")).unwrap(), "john", "pw").unwrap(), two_way(), "Laptop");
        assert!(gone.list("").is_err());
        assert!(gone.take_offline().is_some(), "no answer at all is offline");
    }

    /// `connect` runs the probe once and saves it; the next connect reads the saved check and
    /// sends nothing; a check saved for another base doesn't count.
    #[test]
    fn connect_probes_once_and_reuses_the_saved_check() {
        let (server, dav) = scripted(Flags::default());
        let dir = tempfile::tempdir().unwrap();
        let quick = Limits { settle: Duration::ZERO, ..Limits::default() };
        let (driver, check) = WebDav::connect(session(&server), "Laptop", Some(dir.path()), quick);
        assert_eq!((check.mode, check.measured), (Mode::TwoWay, true));
        assert_eq!(mode_for(driver.capabilities()), Mode::TwoWay);
        assert!(server.seen().iter().any(|r| r.path.contains("/.craftcloud/probe/")), "the probe ran");
        assert!(dav.lock().unwrap().has(".craftcloud/probe"));
        let probed = server.seen().len();
        let (again, _) = WebDav::connect(session(&server), "Laptop", Some(dir.path()), quick);
        assert_eq!(server.seen().len(), probed, "the saved check served: nothing sent");
        assert_eq!(mode_for(again.capabilities()), Mode::TwoWay);
        // A server that ignores `If-None-Match` gets download only; a refused one nothing.
        let (server, _) = scripted(Flags { ignores_if_none_match: true, ..Flags::default() });
        let (lowered, check) = WebDav::connect(session(&server), "Laptop", Some(tempfile::tempdir().unwrap().path()), quick);
        assert_eq!((check.mode, mode_for(lowered.capabilities())), (Mode::DownloadOnly, Mode::DownloadOnly));
        let (server, _) = scripted(Flags { no_file_etags: true, ..Flags::default() });
        let (refused, check) = WebDav::connect(session(&server), "Laptop", None, quick);
        assert_eq!((check.mode, mode_for(refused.capabilities())), (Mode::Refused, Mode::Refused));
    }

    #[test]
    fn trash_entry_times_read_as_rfc_3339() {
        assert_eq!(entry_time("20261117T143205Z-Laptop"), "2026-11-17T14:32:05Z");
        assert_eq!(entry_time("20261117T143205Z-Laptop-2"), "2026-11-17T14:32:05Z");
        assert_eq!(entry_time("Laptop"), "");
    }
}
