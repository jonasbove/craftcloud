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
//! The client trash's retention (docs/WEBDAV-STORAGE.md § 16, Q5): [`RemoteStorage::sweep_trash`]
//! removes entries older than 30 days by the **server's** clock (never this computer's), through
//! `purge_own`, at most once a day per client. The engine calls it after a cycle, and re-runs the
//! check and takes fresh capabilities ([`RemoteStorage::recheck`]) when the runtime watch (§ 7.5)
//! sees the server misbehave.
//!
//! **A known server's profile** ([`super::profile::ServerProfile`], § 7.4) plugs in here: it
//! supplies the probe's known-server checks (steps 16 and 17), the server's version string, and
//! the server's own trash, versions and quota endpoint. The driver asks it for the trash only where
//! the check confirmed `Trash::Server` (a `DELETE` then lands in the server's trash, a file's with
//! `If-Match`, a folder's after `cloud_holds`, as § 8 says) and for the versions only where it
//! confirmed `Versions::Server` (`keep_version` and `move_versions` are then no-ops: the server
//! keeps them by file id). **Every write is verified by checksum where the profile asks for it**
//! (§ 16, Q13): the `PUT` carries `OC-Checksum`, a depth-0 listing afterwards must show that same
//! checksum, and anything else is [`Written::Conflict`], never a silent overwrite. On such a profile
//! **a file's version tag is its ETag plus the checksum the server lists** ([`WebDav::version_tag`]):
//! a server that reuses an ETag within one second would otherwise hide another client's write
//! behind the tag this client last saw, and the next overwrite's `If-Match` would pass. With the
//! checksum in the tag, that write is a cloud change like any other (downloaded, or a conflict copy
//! when the local file changed too). The bare ETag goes into `If-Match`; everything the engine sees
//! and saves is the tag. A profile that doesn't verify writes keeps bare ETags.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use craftcloud_core::api::dav::{self, TRASH_RETAIN_SECS, Uploaded, Write};
use craftcloud_core::api::expect_ok;
use craftcloud_core::api::versions::Version;
use craftcloud_core::http::{HttpRequest, Method};
use craftcloud_core::names;
use craftcloud_core::storage::probe::{Base, Listed, PROPFIND_BODY, Profile, StoredCheck, Verdict, older_than, parse_listing, server_seconds};
use craftcloud_core::storage::{Capabilities, Checksum, Trash, Versions};

use super::probe::{self, Limits, Transport, load_check, save_check};
use super::profile::{Generic, ServerProfile};
use super::{
    Download, Entry, MoveGuard, Moved, OwnPath, RemoteStorage, Restored, StorageError, TrashGuard, TrashItem, Trashed, Written, listed_at, versions,
};
use crate::session::AppPasswordSession;

/// How a trash entry's time is written in its folder's name (UTC, whole seconds).
const TRASH_STAMP: &str = "%Y%m%dT%H%M%SZ";

/// What joins a file's ETag and its listed SHA1 in a version tag (`"<etag>";sha1:<hex>`), on a
/// profile that verifies writes by checksum.
const SUM_MARK: &str = ";sha1:";

/// The ETag a version tag was made from: what `If-Match` takes.
fn server_etag(tag: &str) -> &str {
    tag.rsplit_once(SUM_MARK).map_or(tag, |(etag, _)| etag)
}

/// How often one client sweeps the client trash at most (a rate limit on this computer's
/// monotonic clock only; which entries go is decided by the server's clock, never this one).
const SWEEP_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub struct WebDav {
    session: AppPasswordSession,
    base: Base,
    caps: Capabilities,
    /// This device's name, in the trash entries it makes.
    device: String,
    /// Where `storage-check.json` lives, so a re-check after the runtime watch saves there.
    settings_dir: Option<PathBuf>,
    /// How long a probe (the re-check) may take.
    limits: Limits,
    /// When this client last swept the client trash (a rate limit; `None` until the first sweep).
    last_trash_sweep: Option<Instant>,
    /// The server's version string read on connect (the profile's, else its `Server` header, or
    /// nothing), saved with a re-check so the next connect doesn't take the re-check for a version
    /// change.
    server_version: String,
    /// What this kind of server adds (its trash, versions, quota, the probe's known-server checks);
    /// [`Generic`] for a server nobody knows.
    profile: Box<dyn ServerProfile>,
}

impl WebDav {
    /// A driver on `session` with capabilities already known (a saved check). No settings folder,
    /// so a re-check after the runtime watch isn't saved (tests, and callers that pass one through
    /// [`WebDav::connect`]).
    pub fn new(session: AppPasswordSession, caps: Capabilities, device: &str) -> Self {
        let base = session.base().clone();
        Self {
            session,
            base,
            caps,
            device: device.to_owned(),
            settings_dir: None,
            limits: Limits::default(),
            last_trash_sweep: None,
            server_version: String::new(),
            profile: Box::new(Generic),
        }
    }

    /// The same driver with a known server's profile.
    pub fn with_profile(mut self, profile: Box<dyn ServerProfile>) -> Self {
        self.profile = profile;
        self
    }

    /// What this kind of server adds.
    pub fn profile(&self) -> &dyn ServerProfile {
        self.profile.as_ref()
    }

    /// [`WebDav::connect_with`] for a server nobody knows (the generic profile).
    pub fn connect(session: AppPasswordSession, device: &str, settings_dir: Option<&Path>, limits: Limits) -> (Self, StoredCheck) {
        Self::connect_with(session, Box::new(Generic), device, settings_dir, limits)
    }

    /// Connects (docs/WEBDAV-STORAGE.md § 7.5, "When"): the check saved in `settings_dir` for this
    /// base is reused while it is less than a day old and the server's version string hasn't
    /// changed; otherwise the probe is run afresh (with the profile's known-server checks) and
    /// saved. A check that was refused leaves a driver whose capabilities allow nothing
    /// (`Capabilities::unknown`), so the engine refuses to sync and says why.
    pub fn connect_with(
        mut session: AppPasswordSession,
        profile: Box<dyn ServerProfile>,
        device: &str,
        settings_dir: Option<&Path>,
        limits: Limits,
    ) -> (Self, StoredCheck) {
        let base = session.base().clone();
        let saved = settings_dir.and_then(load_check).filter(|c| c.server == base.url());
        // What the server calls itself now (a known server's own version, else a plain WebDAV
        // server's `Server` header, or nothing). A transport error leaves it unknown, and an
        // unknown version never forces a re-check on its own, so a momentary blip doesn't throw
        // away a good saved check.
        let now = jiff::Timestamp::now();
        let version = profile.version(&mut session).or_else(|| server_version(&mut session, &base));
        let reuse = saved.filter(|c| version.as_ref().is_none_or(|v| *v == c.server_version) && !cache_is_stale(&c.checked_at, now));
        let check = match reuse {
            Some(check) => check,
            None => {
                let verdict = run_probe(&mut session, &base, profile.as_ref(), limits);
                let stamp = now.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
                let check = StoredCheck::from_verdict(&verdict, base.url(), version.as_deref().unwrap_or(""), &stamp);
                if let Some(dir) = settings_dir
                    && let Err(e) = save_check(dir, &check)
                {
                    log::warn!("couldn't save the server check: {e}");
                }
                check
            }
        };
        let caps = check.capabilities.clone().unwrap_or_else(Capabilities::unknown);
        let mut driver = Self::new(session, caps, device).with_profile(profile);
        driver.settings_dir = settings_dir.map(Path::to_path_buf);
        driver.limits = limits;
        driver.server_version = version.unwrap_or_else(|| check.server_version.clone());
        (driver, check)
    }

    /// Whether the check confirmed the server keeps its own versions (then the profile lists them).
    fn server_versions(&self) -> bool {
        self.caps.versions == Versions::Server
    }

    /// Whether the check confirmed the server's own trash (then a `DELETE` lands there).
    fn server_trash(&self) -> bool {
        self.caps.trash == Trash::Server
    }

    /// The earlier versions of `path`, newest first: the server's own where the check confirmed
    /// them (by file id, through the profile), else `.craftcloud/versions/<path>/`, read as the
    /// OxiCloud driver reads them (one body of code, [`super::versions`]).
    pub fn list_versions(&mut self, path: &str) -> Result<Vec<Version>, StorageError> {
        if self.server_versions() {
            return self.profile.list_versions(&mut self.session, &self.base, path);
        }
        Ok(versions::list(&mut self.session, &self.base, path)?)
    }

    /// Puts version `stamp` of `path` back, after keeping the current file as a version of its own
    /// (so the restore can be undone): the server does that itself for its own versions.
    pub fn restore_version(&mut self, path: &str, stamp: &str) -> Result<Restored, StorageError> {
        if self.server_versions() {
            return self.restore_server_version(path, stamp);
        }
        versions::restore(&mut self.session, &self.base, path, stamp)?;
        Ok(Restored::Done)
    }

    /// Restores one of the server's own versions as a **guarded write**, the same on every kind of
    /// server (docs/WEBDAV-STORAGE.md § 7.2, "no unguarded write"): the file's version tag is read
    /// first, the version's bytes are read where the profile says ([`ServerProfile::version_download`]),
    /// and they are written back with `If-Match` on that tag (and the checksum check of Q13 where
    /// the profile asks for it). A change that lands in between answers 412: nothing is restored,
    /// the newer file stays ([`Restored::Changed`]). The server keeps the bytes the `PUT` replaces
    /// as a version itself. Never the server's own restore call: it names no version of the file
    /// it replaces. The bytes pass through memory (the HTTP layer reads at most 64 MB).
    fn restore_server_version(&mut self, path: &str, stamp: &str) -> Result<Restored, StorageError> {
        let resp = self.session.call(&self.propfind(path, false, "0"))?;
        if resp.status == 404 {
            return Err(StorageError::NotFound(format!("{path} isn't there any more; nothing restored")));
        }
        let listed = parse_listing(&resp, &self.base).map_err(|e| format!("listing {path:?}: {e}"))?;
        let etag = listed.into_iter().find(|e| e.path == path.trim_matches('/')).and_then(|e| e.etag).ok_or_else(|| {
            StorageError::Failed(format!("{path} is listed without a version tag, so a restore can't be guarded; nothing restored"))
        })?;
        let read = self.profile.version_download(&mut self.session, &self.base, path, stamp)?;
        let version = self.session.call(&read)?;
        if !version.is_success() {
            return Err(StorageError::Failed(format!("reading version {stamp} of {path}: the server answered {}", version.status)));
        }
        match self.upload_bytes(path, version.body, &Write::Replace { etag })? {
            Written::Saved { .. } => Ok(Restored::Done),
            Written::Conflict => Ok(Restored::Changed),
        }
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

    /// A file's version tag: its ETag, and on a profile that verifies writes by checksum (§ 16,
    /// Q13) the SHA1 the server lists for it too. Nextcloud reuses a file's ETag when it is
    /// overwritten within the same second as the write before it, but it lists the checksum only
    /// of a `PUT` that carried one, and clears it on any other write: so the pair changes when
    /// another client's write lands under an unchanged ETag, whether that write carried a checksum
    /// (another one) or none (the web page). Without a listed checksum, or on any other profile,
    /// the tag is the bare ETag.
    fn version_tag(&self, etag: &str, checksums: &[String]) -> String {
        if self.profile.verify_writes() != Some(Checksum::Sha1) {
            return etag.to_owned();
        }
        let sha1 = checksums.iter().flat_map(|c| c.split_whitespace()).find_map(|c| {
            let (kind, hex) = c.split_once(':')?;
            (kind.eq_ignore_ascii_case("SHA1") && !hex.is_empty()).then(|| hex.to_ascii_lowercase())
        });
        match sha1 {
            Some(hex) => format!("{etag}{SUM_MARK}{hex}"),
            None => etag.to_owned(),
        }
    }

    /// One folder's listing, read against the base; the folder itself left out, every file's
    /// `etag` its version tag ([`Self::version_tag`]). A 404 is [`StorageError::NotFound`];
    /// anything else that isn't a readable 207 fails it.
    fn listing(&mut self, folder: &str) -> Result<Vec<Listed>, StorageError> {
        let resp = self.session.call(&self.propfind(folder, true, "1"))?;
        if resp.status == 404 {
            return Err(StorageError::NotFound(format!("listing {folder:?}: 404")));
        }
        let me = folder.trim_matches('/');
        let mut entries = parse_listing(&resp, &self.base).map_err(|e| format!("listing {folder:?}: {e}"))?;
        entries.retain(|e| e.path != me);
        for e in &mut entries {
            if let Some(etag) = e.etag.take() {
                e.etag = Some(self.version_tag(&etag, &e.checksums));
            }
        }
        Ok(entries)
    }

    /// Whether the cloud file at `path` is still at version `etag` (`None`: it's gone). A file
    /// listed without an ETag can't be shown to be at `etag`: `Some(Err(()))`.
    fn still_at(&mut self, path: &str, etag: &str) -> Result<Option<Result<bool, ()>>, StorageError> {
        let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
        match self.listing(parent).map(|entries| entries.into_iter().find(|e| e.path == path)) {
            Ok(Some(e)) => Ok(Some(listed_at(e.etag.as_deref(), etag).ok_or(()))),
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
        Ok(found.len() == files.len() && files.iter().all(|(p, etag)| listed_at(found.get(p).and_then(Option::as_deref), etag) == Some(true)))
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
    fn trash_place(&mut self, path: &str) -> Result<(String, String), StorageError> {
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
                    let dest = format!("{entry}/{path}");
                    return Ok((entry, dest));
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
    /// A 404 (gone meanwhile) is done, and the entry just made for it, which holds nothing, is
    /// removed again (see [`Self::purge_empty_entry`]).
    fn move_to_trash(&mut self, path: &str, folder: bool) -> Result<(), StorageError> {
        let (entry, dest) = self.trash_place(path)?;
        let resp = self.session.call(&dav::move_to_under(self.base.url(), path, &dest, folder))?;
        match resp.status {
            404 => {
                self.purge_empty_entry(&entry);
                Ok(())
            }
            _ if resp.is_success() => Ok(()),
            status => Err(StorageError::Failed(format!("moving {path} to the trash: the server answered {status}"))),
        }
    }

    /// Removes a trash entry's folder (`.craftcloud/trash/<time>-<device>`) once it holds nothing:
    /// after a restore emptied it, or when the `MOVE` into a fresh one found the file gone. It is
    /// deleted for good, so only through `purge_own` (an `OwnPath`, inside `.craftcloud/`), and only
    /// after a fresh listing of every level answered and showed no file: a listing that failed is
    /// unknown, and the entry stays (a stray empty folder costs nothing; a wrong delete can't be
    /// undone). A failed purge is left for the retention sweep.
    fn purge_empty_entry(&mut self, entry: &str) {
        if self.holds_nothing(entry)
            && let Some(own) = OwnPath::new(entry)
        {
            let _ = self.purge_own(&own);
        }
    }

    /// One guarded `PUT` of `file`. Where the profile asks for it (§ 16, Q13), the `PUT` carries
    /// the file's checksum and the result is read back: a server that reuses a version tag within
    /// one second can't tell two writes apart, so a listed checksum that isn't ours means the
    /// server holds someone else's write under our tag, and that is a conflict, never a silent
    /// overwrite.
    fn upload(&mut self, path: &str, file: &Path, write: &Write) -> Result<Written, StorageError> {
        let checksum = self.write_checksum(|| sha1_hex(file))?;
        let mut req = dav::upload_under(self.base.url(), path, Vec::new(), &bare(write));
        if let Some(sum) = &checksum {
            req = req.header("OC-Checksum", sum.clone());
        }
        let resp = self.session.upload(&req, file)?;
        self.uploaded(path, &resp, checksum.as_deref())
    }

    /// [`Self::upload`] for bytes in memory (a version being restored): the same guard, the same
    /// checksum check.
    fn upload_bytes(&mut self, path: &str, bytes: Vec<u8>, write: &Write) -> Result<Written, StorageError> {
        let checksum = self.write_checksum(|| {
            use sha1::Digest;
            Ok(sha1::Sha1::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
        })?;
        let mut req = dav::upload_under(self.base.url(), path, bytes, &bare(write));
        if let Some(sum) = &checksum {
            req = req.header("OC-Checksum", sum.clone());
        }
        let resp = self.session.call(&req)?;
        self.uploaded(path, &resp, checksum.as_deref())
    }

    /// The checksum a write carries where the profile asks for one (`SHA1:<sha1>`, from `sha1`), the
    /// same for both kinds of upload. A kind this client can't make is said in the log, and the
    /// write goes unverified.
    fn write_checksum(&self, sha1: impl FnOnce() -> Result<String, StorageError>) -> Result<Option<String>, StorageError> {
        match self.profile.verify_writes() {
            Some(Checksum::Sha1) => Ok(Some(format!("SHA1:{}", sha1()?))),
            Some(other) => {
                log::warn!("verifying writes by {other:?} isn't supported; writes to {} go unverified", self.base.url());
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// What a guarded `PUT`'s answer means, with the checksum read back where one was sent.
    fn uploaded(&mut self, path: &str, resp: &craftcloud_core::http::HttpResponse, checksum: Option<&str>) -> Result<Written, StorageError> {
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
        match dav::parse_upload(resp).map_err(|e| e.to_string())? {
            Uploaded::Conflict => Ok(Written::Conflict),
            Uploaded::Saved { etag } => {
                if let Some(sum) = checksum
                    && !self.holds_checksum(path, sum)?
                {
                    log::warn!(
                        "{path}: the server lists another checksum than the one just uploaded (a write landed within the same second); kept as a conflict"
                    );
                    return Ok(Written::Conflict);
                }
                // The tag of what was just written: the answer's ETag with our checksum (which
                // the read-back above showed the server lists), or the listing's tag.
                let etag = match (etag, checksum) {
                    (Some(e), Some(sum)) => self.version_tag(&e, &[sum.to_owned()]),
                    (Some(e), None) => e,
                    (None, _) => self.listed_etag(path)?,
                };
                Ok(Written::Saved { etag })
            }
        }
    }

    /// Whether a depth-0 listing of `path` names `checksum` (`SHA1:…`, compared without regard to
    /// case) among its `oc:checksums`. A listing that fails is an error: unknown, never "yes".
    fn holds_checksum(&mut self, path: &str, checksum: &str) -> Result<bool, StorageError> {
        let resp = self.session.call(&self.propfind(path, false, "0"))?;
        let listed = parse_listing(&resp, &self.base).map_err(|e| format!("reading {path} back after the upload: {e}"))?;
        let entry = listed.into_iter().find(|e| e.path == path.trim_matches('/'));
        Ok(entry.is_some_and(|e| e.checksums.iter().flat_map(|c| c.split_whitespace()).any(|c| c.eq_ignore_ascii_case(checksum))))
    }

    /// `DELETE` into the server's own trash (only where the check on connect confirmed it lands
    /// there): a file's with `If-Match` (412: changed meanwhile, it stays), a folder's after the
    /// fresh listing the caller made.
    fn delete_to_server_trash(&mut self, path: &str, folder: bool, if_match: Option<&str>) -> Result<Trashed, StorageError> {
        let mut req = dav::delete_under(self.base.url(), path, folder);
        if let Some(etag) = if_match {
            req = req.header("If-Match", server_etag(etag));
        }
        let resp = self.session.call(&req)?;
        match resp.status {
            412 => Ok(Trashed::Changed),
            // Gone meanwhile (deleted or moved elsewhere): there is nothing left to trash, and the
            // state then forgets it. A folder moved elsewhere comes down again under its new name
            // from the next listing, so nothing is lost by taking this as done.
            404 => Ok(Trashed::Done),
            _ if resp.is_success() => Ok(Trashed::Done),
            s => Err(StorageError::Failed(format!("deleting {path} into the server's trash: the server answered {s}"))),
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
        // Apache's mod_dav answers 301 to a folder's URL without its trailing slash, pointing at
        // the slash form. The session never follows a redirect (`http::execute_direct`), so the
        // answer is read here: a redirect to `<path>/` says "a folder". Any other 3xx is unknown.
        if (300..400).contains(&resp.status) {
            // Exactly this path with its slash, on this server: an absolute `Location` must start
            // with the base's origin, a relative one with `/`; the path must be the base's path plus
            // the folder's. Anything else (another host, another path) is an error, not a folder.
            let expected = format!("{}{}", self.base.path().trim_end_matches('/'), dav::url_under("", path, true));
            let to_slash_form = resp.header("Location").is_some_and(|l| {
                let l = l.split('?').next().unwrap_or(l);
                let path_part = l.strip_prefix(&self.base.origin()).or_else(|| l.starts_with('/').then_some(l));
                path_part == Some(expected.as_str())
            });
            return if to_slash_form { Ok(true) } else { Err(StorageError::Status(resp.status)) };
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

/// The same write with the bare ETag in its guard (a version tag's checksum part isn't the
/// server's).
fn bare(write: &Write) -> Write {
    match write {
        Write::Create => Write::Create,
        Write::Replace { etag } => Write::Replace { etag: server_etag(etag).to_owned() },
    }
}

/// The check on connect run now, with the profile's known-server checks (steps 16 and 17) where it
/// has them, and the server's version string as the profile (else the `Server` header) reads it:
/// what `craftcloud server check` does on another WebDAV server.
pub fn probe_with(transport: &mut AppPasswordSession, base: &Base, profile: &dyn ServerProfile, limits: Limits) -> (Verdict, Option<String>) {
    let verdict = run_probe(transport, base, profile, limits);
    let version = profile.version(transport).or_else(|| server_version(transport, base));
    (verdict, version)
}

/// Runs the check on connect with the profile's known-server checks (steps 16 and 17), where it
/// has them.
fn run_probe(transport: &mut dyn Transport, base: &Base, profile: &dyn ServerProfile, limits: Limits) -> Verdict {
    match profile.probe_checks() {
        Some((mut trash, mut versions)) => {
            probe::run(transport, base, &probe::folder_name(), Profile { trash: Some(trash.as_mut()), versions: Some(versions.as_mut()) }, limits)
        }
        None => probe::run(transport, base, &probe::folder_name(), Profile::default(), limits),
    }
}

/// SHA1 of a file's bytes (hex), streamed.
fn sha1_hex(file: &Path) -> Result<String, StorageError> {
    use sha1::Digest;
    let mut reader = std::fs::File::open(file).map_err(|e| StorageError::Failed(format!("can't read {}: {e}", file.display())))?;
    let mut hasher = sha1::Sha1::new();
    std::io::copy(&mut reader, &mut hasher).map_err(|e| StorageError::Failed(format!("can't read {}: {e}", file.display())))?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// What the server calls itself now, for the check's re-run schedule (§ 7.5): a plain WebDAV
/// server's `Server` header, or `None` when it sends none or doesn't answer. A known server's
/// profile reads a richer version first (`status.php`); this is the fallback.
fn server_version(session: &mut AppPasswordSession, base: &Base) -> Option<String> {
    let req = HttpRequest::new(Method::Propfind, dav::url_under(base.url(), "", true))
        .header("Depth", "0")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(PROPFIND_BODY.as_bytes().to_vec());
    // Only a successful answer says what the server is: a 401, a proxy's error page or a 5xx
    // carries no version worth comparing, and must never throw a good saved check away.
    session.call(&req).ok().filter(|resp| resp.is_success()).map(|resp| resp.header("Server").unwrap_or_default().to_owned())
}

/// Whether a saved check's `checked_at` (RFC 3339, this computer's clock when it was written) is
/// more than a day before `now`, so the check is re-run (§ 7.5). An unreadable stamp is stale.
/// This is cache freshness on the local clock, never a sync-safety decision about the server.
fn cache_is_stale(checked_at: &str, now: jiff::Timestamp) -> bool {
    match checked_at.parse::<jiff::Timestamp>() {
        Ok(then) => now.as_second().saturating_sub(then.as_second()) > 24 * 60 * 60,
        Err(_) => true,
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

    /// One depth-0 `PROPFIND` of the path (a file: no trailing slash). A 404 is nothing there; a
    /// redirect or anything else that isn't a readable 207 is an error (unknown).
    fn stat(&mut self, path: &str) -> Result<Option<Entry>, StorageError> {
        let resp = self.session.call(&self.propfind(path, false, "0"))?;
        if resp.status == 404 {
            return Ok(None);
        }
        if (300..400).contains(&resp.status) {
            return Err(StorageError::Status(resp.status));
        }
        let listed = parse_listing(&resp, &self.base).map_err(|e| format!("reading {path:?}: {e}"))?;
        Ok(listed.into_iter().find(|e| e.path == path.trim_matches('/')).map(Entry::from))
    }

    fn download(&mut self, path: &str, from: Option<u64>) -> Result<Download, StorageError> {
        let mut req = dav::download_under(self.base.url(), path);
        if let Some(from) = from {
            req = req.header("Range", format!("bytes={from}-"));
        }
        let resp = self.session.open(&req)?;
        // The tag of the bytes coming down, from the same answer (never a second look, which could
        // show a later write): its ETag and, where writes are verified, its `OC-Checksum`.
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let tag = header("etag").map(|etag| self.version_tag(&etag, &header("oc-checksum").into_iter().collect::<Vec<_>>()));
        let download = Download::new(resp, Arc::clone(&self.session.cancel));
        Ok(match tag {
            Some(tag) => download.with_etag(tag),
            None => download,
        })
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
                if self.server_trash() {
                    return self.delete_to_server_trash(path, false, Some(&etag));
                }
                self.move_to_trash(path, false)?;
            }
            TrashGuard::Folder { files } => {
                if !self.cloud_holds(path, &files)? {
                    return Ok(Trashed::Changed);
                }
                if self.server_trash() {
                    return self.delete_to_server_trash(path, true, None);
                }
                self.move_to_trash(path, true)?;
            }
            TrashGuard::EmptyFolder => {
                // The cloud must say it's empty right now; a listing that fails says nothing (T-064).
                match self.listing(path) {
                    Ok(entries) if entries.is_empty() => {}
                    _ => return Ok(Trashed::Changed),
                }
                if self.server_trash() {
                    return self.delete_to_server_trash(path, true, None);
                }
                self.move_to_trash(path, true)?;
            }
        }
        Ok(Trashed::Done)
    }

    /// The server's own trash where the check confirmed it, else CraftCloud's `.craftcloud/trash/`.
    fn trash_list(&mut self) -> Result<Vec<TrashItem>, StorageError> {
        if self.server_trash() {
            return self.profile.trash_list(&mut self.session);
        }
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
        // An item of the server's own trash (its id is the server's name for it, never a path in
        // CraftCloud's trash) goes back the way the profile knows.
        if self.server_trash() && OwnPath::new(&item.id).is_none() {
            return self.profile.restore(&mut self.session, item);
        }
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
        // The restored item left its entry's folder (and the parents made in it) behind. It is
        // CraftCloud's own, inside `.craftcloud/trash/`, and holds nothing now, so it goes for
        // good; `purge_empty_entry` lists it first and keeps it if the listing fails or shows a file.
        if let Some(entry) = entry {
            self.purge_empty_entry(&entry);
        }
        Ok(())
    }

    /// A no-op where the check confirmed the server keeps its own versions (§ 8: they follow
    /// renames by file id); else a `COPY` into `.craftcloud/versions/`.
    fn keep_version(&mut self, path: &str) -> Result<(), StorageError> {
        if self.server_versions() {
            return Ok(());
        }
        Ok(versions::keep(&mut self.session, &self.base, path)?)
    }

    fn move_versions(&mut self, from: &str, to: &str) -> Result<(), StorageError> {
        if self.server_versions() {
            return Ok(());
        }
        Ok(versions::follow_move(&mut self.session, &self.base, from, to)?)
    }

    /// From the profile's endpoint where it has one (Nextcloud's OCS user endpoint), else WebDAV's
    /// `quota-available-bytes` on the base, where the server lists it; a negative number
    /// (Nextcloud's `-3`) or none is unknown.
    fn free_space(&mut self, _needed: u64) -> Result<Option<u64>, StorageError> {
        if let Some(answer) = self.profile.free_space(&mut self.session) {
            return answer;
        }
        let resp = self.session.call(&self.propfind("", true, "0"))?;
        let listed = parse_listing(&resp, &self.base).map_err(|e| format!("listing the server: {e}"))?;
        Ok(listed.into_iter().find_map(|e| e.quota_available).and_then(|q| u64::try_from(q).ok()))
    }

    /// A path that isn't there any more (404 on the listing or the `DELETE`) counts as purged:
    /// what this removes is CraftCloud's own leftovers (a pruned version, an emptied trash
    /// entry's folder, a probe folder), and "already gone" is the state wanted; nothing of the
    /// person's is ever named here (`OwnPath` can't name it).
    fn purge_own(&mut self, path: &OwnPath) -> Result<(), StorageError> {
        let folder = match self.is_folder(path.as_str()) {
            Ok(folder) => folder,
            Err(StorageError::NotFound(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let resp = self.session.call(&dav::delete_under(self.base.url(), path.as_str(), folder))?;
        if resp.is_success() || resp.status == 404 { Ok(()) } else { Err(StorageError::Status(resp.status)) }
    }

    /// Removes client-trash entries older than [`TRASH_RETAIN_SECS`] (30 days) **by the server's
    /// clock**: each entry folder's `getlastmodified` against the `Date` of the same listing (both
    /// in seconds), never this computer's clock, so a device whose clock is off can't take a fresh
    /// entry for an old one. A listing without a `Date`, or an entry without a time, removes
    /// nothing. Only inside `.craftcloud/trash/`, through [`RemoteStorage::purge_own`]; at most
    /// once a day per client.
    fn sweep_trash(&mut self) -> Result<(), StorageError> {
        // A server with its own trash keeps to its own policy.
        if self.server_trash() || self.last_trash_sweep.is_some_and(|at| at.elapsed() < SWEEP_EVERY) {
            return Ok(());
        }
        self.last_trash_sweep = Some(Instant::now());
        let resp = self.session.call(&self.propfind(dav::TRASH_DIR, true, "1"))?;
        if resp.status == 404 {
            return Ok(());
        }
        let server_now = resp.header("Date").and_then(server_seconds);
        let me = dav::TRASH_DIR.trim_matches('/');
        let entries = parse_listing(&resp, &self.base).map_err(|e| format!("listing the client trash: {e}"))?;
        let aged: Vec<(String, Option<i64>)> =
            entries.into_iter().filter(|e| e.is_folder && e.path != me).map(|e| (e.path, e.modified.as_deref().and_then(server_seconds))).collect();
        for path in older_than(aged.iter().map(|(p, m)| (p.as_str(), *m)), server_now, TRASH_RETAIN_SECS) {
            if let Some(own) = OwnPath::new(&path) {
                self.purge_own(&own)?;
            }
        }
        Ok(())
    }

    fn measured(&self) -> bool {
        // Its capabilities come from the check on connect, never from knowing the kind of server.
        true
    }

    /// The one write allowed after the watch has tripped: the probe, inside `.craftcloud/probe/`
    /// only, bounded by `Limits` (30 s a request, 180 s in all).
    fn recheck(&mut self) -> Option<craftcloud_core::storage::Mode> {
        let verdict = run_probe(&mut self.session, &self.base, self.profile.as_ref(), self.limits);
        let now = jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
        let check = StoredCheck::from_verdict(&verdict, self.base.url(), &self.server_version, &now);
        if let Some(dir) = &self.settings_dir
            && let Err(e) = save_check(dir, &check)
        {
            log::warn!("couldn't save the server re-check: {e}");
        }
        self.caps = check.capabilities.clone().unwrap_or_else(Capabilities::unknown);
        Some(verdict.mode)
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
    use crate::storage::probe::{Limits, load_check, save_check};
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
        let deletes: Vec<String> = server.seen().iter().filter(|r| r.method == "DELETE").map(|r| r.path.clone()).collect();
        assert_eq!(deletes.len(), 2, "the two oldest versions were pruned: {:?}", server.seen());
        let versions_of_a = format!("{BASE_PATH}/.craftcloud/versions/Docs/a.txt/2026");
        assert!(deletes.iter().all(|p| p.starts_with(&versions_of_a)), "a prune deletes only inside the file's own versions folder: {deletes:?}");
    }

    /// The ETag a version tag was made from (for tests of a profile whose tags carry a checksum).
    pub(crate) fn bare_etag(tag: &str) -> &str {
        super::server_etag(tag)
    }

    /// The generic profile keeps bare ETags, even where the server lists checksums (the scripted
    /// server does by default): nothing but a profile that verifies writes gets the longer tag, so
    /// a plain WebDAV server's saved state and its requests are what they were.
    #[test]
    fn the_generic_profile_keeps_bare_etags() {
        let (server, dav) = scripted(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "one");
        let mut s = driver(&server);
        let Written::Saved { etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!("not saved") };
        let listed = s.list("Docs").unwrap();
        assert!(listed[0].checksum.is_some(), "the server lists a checksum");
        assert_eq!(listed[0].etag.as_deref(), Some(etag.as_str()));
        assert!(!etag.contains("sha1") && etag.starts_with('"') && etag.ends_with('"'), "{etag}");
        assert_eq!(s.download("Docs/a.txt", None).unwrap().header("etag"), Some(etag.as_str()));
        assert!(server.seen().iter().all(|r| r.header("oc-checksum").is_none()), "and sends none");
        assert_eq!(super::server_etag("\"e1\";sha1:abc"), "\"e1\"");
        assert_eq!(super::server_etag("\"e1\""), "\"e1\"");
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
        let after = server.seen();
        assert!(
            after[probed..].iter().all(|r| !r.path.contains("/.craftcloud/probe/")),
            "the saved check served: only the version check, no re-probe: {:?}",
            &after[probed..]
        );
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

    /// The client trash keeps entries 30 days by the **server's** clock: a day-old entry stays, a
    /// 31-day-old one goes, nothing outside `.craftcloud/trash/` is touched, and a listing without
    /// a `Date` sweeps nothing. The server dates are months from this computer's clock (Jun 2026),
    /// so a sweep by this computer's clock would delete the day-old one too: it doesn't.
    #[test]
    fn the_client_trash_keeps_thirty_days_by_the_servers_clock() {
        const NOW: &str = "Mon, 15 Jun 2026 12:00:00 GMT";
        const DAY_OLD: &str = "Sun, 14 Jun 2026 12:00:00 GMT";
        const OVER_30_DAYS: &str = "Wed, 13 May 2026 11:00:00 GMT";
        let (server, dav) = scripted(Flags { date: Some(NOW.into()), ..Flags::default() });
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder(".craftcloud/trash/20260614T120000Z-Laptop", Some(DAY_OLD));
            dav.add_file(".craftcloud/trash/20260614T120000Z-Laptop/Docs/recent.txt", b"r");
            dav.add_folder(".craftcloud/trash/20260513T110000Z-Studio", Some(OVER_30_DAYS));
            dav.add_file(".craftcloud/trash/20260513T110000Z-Studio/Docs/old.txt", b"o");
            dav.add_folder("Docs", None);
            dav.add_file("Docs/keep.txt", b"k");
        }
        let mut s = driver(&server);
        s.sweep_trash().unwrap();
        {
            let dav = dav.lock().unwrap();
            assert!(dav.has(".craftcloud/trash/20260614T120000Z-Laptop"), "a day-old entry stays");
            assert!(!dav.has(".craftcloud/trash/20260513T110000Z-Studio"), "a 31-day-old entry goes");
            assert!(dav.has("Docs/keep.txt"), "nothing outside the trash is touched");
        }
        let deletes: Vec<String> = server.seen().iter().filter(|r| r.method == "DELETE").map(|r| r.path.clone()).collect();
        assert!(!deletes.is_empty() && deletes.iter().all(|p| p.contains("/.craftcloud/trash/")), "only inside the trash: {deletes:?}");
        // A second sweep the same day does nothing (at most once a day per client).
        let before = server.seen().len();
        s.sweep_trash().unwrap();
        assert_eq!(server.seen().len(), before, "at most once a day");

        // No `Date` header: the server's clock is unknown, so nothing is swept.
        let (server, dav) = scripted(Flags::default());
        dav.lock().unwrap().add_folder(".craftcloud/trash/20200101T000000Z-Laptop", Some("Wed, 01 Jan 2020 00:00:00 GMT"));
        driver(&server).sweep_trash().unwrap();
        assert!(dav.lock().unwrap().has(".craftcloud/trash/20200101T000000Z-Laptop"), "no Date: nothing swept");
    }

    /// Emptying the client trash (the `trash empty` path through the trait) deletes for good only
    /// inside `.craftcloud/trash/`, by `purge_own`: the person's files are untouched.
    #[test]
    fn emptying_the_client_trash_purges_only_inside_craftcloud() {
        let (server, dav) = scripted(Flags::default());
        {
            let mut d = dav.lock().unwrap();
            d.add_folder("Docs", None);
            d.add_file("Docs/a.txt", b"a");
            d.add_file("Docs/b.txt", b"b");
            d.add_file("Docs/keep.txt", b"k");
        }
        let mut s = driver(&server);
        for name in ["a", "b"] {
            let etag = s.list("Docs").unwrap().into_iter().find(|e| e.path == format!("Docs/{name}.txt")).unwrap().etag.unwrap();
            assert_eq!(s.trash(&format!("Docs/{name}.txt"), TrashGuard::File { etag }).unwrap(), Trashed::Done);
        }
        assert_eq!(s.trash_list().unwrap().len(), 2);
        for e in s.list(".craftcloud/trash").unwrap().into_iter().filter(|e| e.is_folder) {
            s.purge_own(&OwnPath::new(&e.path).unwrap()).unwrap();
        }
        assert!(s.trash_list().unwrap().is_empty(), "the trash is empty");
        assert_eq!(dav.lock().unwrap().body("Docs/keep.txt"), Some(b"k".as_slice()), "the person's file is untouched");
        let deletes: Vec<String> = server.seen().iter().filter(|r| r.method == "DELETE").map(|r| r.path.clone()).collect();
        assert!(!deletes.is_empty() && deletes.iter().all(|p| p.contains("/.craftcloud/trash/")), "{deletes:?}");
    }

    /// The kept versions of a file are listed and a chosen one restored on the generic driver, the
    /// same `.craftcloud/versions/<path>/` layout and code as OxiCloud.
    #[test]
    fn versions_are_listed_and_restored_on_the_generic_driver() {
        let (server, dav) = scripted(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let dir = tempfile::tempdir().unwrap();
        let a = file(dir.path(), "a.txt", "v0");
        let mut s = driver(&server);
        let Written::Saved { mut etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!() };
        for i in 1..=3 {
            s.keep_version("Docs/a.txt").unwrap();
            std::fs::write(&a, format!("v{i}")).unwrap();
            let Written::Saved { etag: new } = s.replace("Docs/a.txt", &etag, &a).unwrap() else { panic!("{i}") };
            etag = new;
            std::thread::sleep(Duration::from_millis(3));
        }
        let versions = s.list_versions("Docs/a.txt").unwrap();
        assert_eq!(versions.len(), 3, "{versions:?}");
        let oldest = versions.last().unwrap().stamp.clone();
        s.restore_version("Docs/a.txt", &oldest).unwrap();
        assert_eq!(dav.lock().unwrap().body("Docs/a.txt"), Some(b"v0".as_slice()), "the version is back");
        assert_eq!(s.list_versions("Docs/a.txt").unwrap().len(), 4, "the replaced file is kept as a new version, so the restore can be undone");
        assert!(s.restore_version("Docs/a.txt", "20200101T000000.000Z").unwrap_err().to_string().contains("no version"));
        // The restore is a guarded write: a `PUT` with `If-Match`, never a `COPY` onto the file (the
        // only `COPY`s are `keep_version`'s, from the file into `.craftcloud/versions/`).
        for r in &server.seen() {
            match r.method.as_str() {
                "PUT" => assert!(r.header("if-match").is_some() || r.header("if-none-match") == Some("*"), "{r:?}"),
                "COPY" => assert!(r.header("destination").is_some_and(|d| d.contains("/.craftcloud/versions/")), "a COPY onto a file: {r:?}"),
                _ => {}
            }
            assert_ne!(r.header("overwrite"), Some("T"), "never Overwrite: T: {r:?}");
        }

        // A change landing between the version's read and the write: the guard holds (412),
        // nothing is restored, and the change stays, with its own version kept.
        let (plain, dav2) = scripted(Flags::default());
        dav2.lock().unwrap().add_folder("Docs", None);
        let racing = Arc::clone(&dav2);
        let server2 = FakeServer::start_full(move |seen: &Seen| {
            let mut dav = racing.lock().unwrap_or_else(PoisonError::into_inner);
            if seen.method == "GET" && seen.path.contains("/.craftcloud/versions/") {
                dav.add_file("Docs/a.txt", b"changed meanwhile");
            }
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer
        });
        drop(plain);
        let mut s = driver(&server2);
        let Written::Saved { etag } = s.create("Docs/a.txt", &a).unwrap() else { panic!() };
        s.keep_version("Docs/a.txt").unwrap();
        std::fs::write(&a, "v9").unwrap();
        assert!(matches!(s.replace("Docs/a.txt", &etag, &a).unwrap(), Written::Saved { .. }));
        let stamp = s.list_versions("Docs/a.txt").unwrap().last().unwrap().stamp.clone();
        let err = s.restore_version("Docs/a.txt", &stamp).unwrap_err().to_string();
        assert!(err.contains("changed on the server meanwhile"), "{err}");
        assert_eq!(dav2.lock().unwrap().body("Docs/a.txt"), Some(b"changed meanwhile".as_slice()), "never silently overwritten");
    }

    /// A `MOVE` into a fresh trash entry that finds the file gone (404) is done, and the entry just
    /// made (holding nothing) is removed again through `purge_own`, after a listing that worked
    /// showed it empty. When that listing fails, the entry stays: unknown is never deleted.
    #[test]
    fn a_trash_entry_whose_move_found_the_file_gone_is_removed_unless_its_listing_fails() {
        let (server, dav) = scripted(Flags::default());
        dav.lock().unwrap().add_folder("Docs", None);
        let mut s = driver(&server);
        s.move_to_trash("Docs/gone.txt", false).unwrap();
        {
            let dav = dav.lock().unwrap();
            assert!(dav.folders().iter().all(|f| !f.starts_with(".craftcloud/trash/")), "the fresh entry is gone again: {:?}", dav.folders());
            assert!(dav.has(".craftcloud/trash"), "the trash folder itself stays");
        }
        let deletes: Vec<String> = server.seen().iter().filter(|r| r.method == "DELETE").map(|r| r.path.clone()).collect();
        assert_eq!(deletes.len(), 1, "{deletes:?}");
        assert!(deletes[0].contains("/.craftcloud/trash/") && deletes[0].ends_with('/'), "{deletes:?}");
        let listed_first = server.seen().iter().position(|r| r.method == "PROPFIND" && r.path.contains("/.craftcloud/trash/"));
        let deleted_at = server.seen().iter().position(|r| r.method == "DELETE");
        assert!(listed_first < deleted_at, "listed before purged: {:?}", server.seen());

        // The entry's listing fails: unknown, so the entry stays and nothing is deleted.
        let (ok_server, ok_dav) = scripted(Flags::default());
        ok_dav.lock().unwrap().add_folder("Docs", None);
        let inner = Arc::clone(&ok_dav);
        let failing = FakeServer::start_full(move |seen: &Seen| {
            if seen.method == "PROPFIND" && seen.path.contains("/.craftcloud/trash/") {
                return Answer::new(500, "");
            }
            let mut dav = inner.lock().unwrap_or_else(PoisonError::into_inner);
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer
        });
        drop(ok_server);
        let mut s = driver(&failing);
        s.move_to_trash("Docs/gone.txt", false).unwrap();
        let dav = ok_dav.lock().unwrap();
        let entries =
            dav.folders().into_iter().filter(|f| f.starts_with(".craftcloud/trash/") && !f[".craftcloud/trash/".len()..].contains('/')).count();
        assert_eq!(entries, 1, "the entry stays (with the parent made in it): {:?}", dav.folders());
        assert!(failing.seen().iter().all(|r| r.method != "DELETE"), "nothing deleted on a failed listing: {:?}", failing.seen());
    }

    /// Apache's mod_dav answers 301 to a folder URL without its trailing slash. The driver never
    /// follows a redirect (followed, it became a `GET` without the password, a 401 and a wrong
    /// "signed out"): it reads the 301 as "a folder" and sends the folder's URL with its slash, so
    /// a trashed folder is still restored and an entry's folder still purged, and no request ever
    /// leaves without the `Authorization` header.
    #[test]
    fn a_folder_url_without_its_slash_is_redirected_and_never_followed() {
        let (server, dav) = scripted(Flags { folders_need_slash: true, ..Flags::default() });
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder("Old/Sub", None);
            dav.add_file("Old/a.txt", b"a");
            dav.add_file("Old/Sub/b.txt", b"b");
        }
        let mut s = driver(&server);
        let etag = |s: &mut WebDav, folder: &str, name: &str| s.list(folder).unwrap().into_iter().find(|e| e.path == name).unwrap().etag.unwrap();
        let planned =
            vec![("Old/a.txt".to_owned(), etag(&mut s, "Old", "Old/a.txt")), ("Old/Sub/b.txt".to_owned(), etag(&mut s, "Old/Sub", "Old/Sub/b.txt"))];
        assert_eq!(s.trash("Old", TrashGuard::Folder { files: planned }).unwrap(), Trashed::Done);
        let items = s.trash_list().unwrap();
        assert_eq!(items.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["Old"]);
        s.restore(&items[0]).unwrap();
        {
            let dav = dav.lock().unwrap();
            assert_eq!(dav.body("Old/Sub/b.txt"), Some(b"b".as_slice()), "the folder is back whole");
            assert!(dav.folders().iter().all(|f| !f.starts_with(".craftcloud/trash/")), "the emptied entry is purged: {:?}", dav.folders());
        }
        assert!(!s.signed_out(), "a redirect is never 'signed out'");
        let seen = server.seen();
        assert!(seen.iter().any(|r| r.method == "PROPFIND" && !r.path.ends_with('/')), "the slashless PROPFIND was sent: {seen:?}");
        for r in &seen {
            assert!(r.header("authorization").is_some(), "a request without the password (a followed redirect): {r:?}");
            assert_ne!(r.method, "GET", "a redirect was followed as a GET: {r:?}");
            if r.method == "DELETE" {
                assert!(r.path.ends_with('/'), "a folder DELETE without its slash: {r:?}");
            }
        }

        // A 3xx the driver can't read as "a folder" is an error, not a listing and not a sign-out.
        let server = FakeServer::start_full(|_| Answer::new(302, "").header("Location", "https://elsewhere.example.org/"));
        let mut s = driver(&server);
        assert!(matches!(s.list("Docs").unwrap_err(), StorageError::Failed(_)));
        assert!(!s.signed_out() && s.take_offline().is_none());
        assert_eq!(server.seen().len(), 1, "never followed: {:?}", server.seen());
    }

    /// The check's re-run schedule (docs/WEBDAV-STORAGE.md § 7.5): a fresh saved check of the same
    /// server version is reused (no re-probe); a changed version or a day-old cache re-probes.
    #[test]
    fn the_recheck_schedule_reprobes_when_stale_or_the_version_changed() {
        let quick = Limits { settle: Duration::ZERO, ..Limits::default() };
        let header = Arc::new(Mutex::new("Caddy/2.0".to_owned()));
        let sh = Arc::clone(&header);
        let dav = Arc::new(Mutex::new(FakeDav::new(BASE_PATH, Flags::default())));
        let shared = Arc::clone(&dav);
        let server = FakeServer::start_full(move |seen: &Seen| {
            let mut dav = shared.lock().unwrap_or_else(PoisonError::into_inner);
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer.header("Server", sh.lock().unwrap_or_else(PoisonError::into_inner).clone())
        });
        let dir = tempfile::tempdir().unwrap();
        let probed = |from: usize| server.seen()[from..].iter().any(|r| r.path.contains("/.craftcloud/probe/"));

        // First connect: probes and saves, recording the server's version.
        WebDav::connect(session(&server), "Laptop", Some(dir.path()), quick);
        assert_eq!(load_check(dir.path()).unwrap().server_version, "Caddy/2.0");

        // A fresh check of the same version: reused, no re-probe.
        let before = server.seen().len();
        WebDav::connect(session(&server), "Laptop", Some(dir.path()), quick);
        assert!(!probed(before), "a fresh check of the same version is reused");

        // The version changed: re-probed, though the cache is fresh.
        *header.lock().unwrap() = "Caddy/3.0".to_owned();
        let before = server.seen().len();
        WebDav::connect(session(&server), "Laptop", Some(dir.path()), quick);
        assert!(probed(before), "a changed version re-probes");
        assert_eq!(load_check(dir.path()).unwrap().server_version, "Caddy/3.0");

        // A day-old cache (same version now): re-probed on age alone.
        let mut c = load_check(dir.path()).unwrap();
        c.checked_at = "2020-01-01T00:00:00Z".to_owned();
        save_check(dir.path(), &c).unwrap();
        let before = server.seen().len();
        WebDav::connect(session(&server), "Laptop", Some(dir.path()), quick);
        assert!(probed(before), "a day-old check is re-probed");
    }
}
