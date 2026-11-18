//! The storage layer: what the sync engine asks of a server, and nothing it may not
//! (docs/WEBDAV-STORAGE.md § 7.2).
//!
//! The engine talks to a server only through [`RemoteStorage`]. **There is no unguarded write:**
//! every method that changes the server takes the precondition it must hold, so a forgotten
//! guard is a type error, not a review comment.
//!
//! - [`RemoteStorage::create`] writes only where nothing is (`If-None-Match: *`), and
//!   [`RemoteStorage::replace`] only over the version given (`If-Match`); a 412 is
//!   [`Written::Conflict`], which the engine turns into a conflict copy.
//! - [`RemoteStorage::move_to`] never lands on an existing name, and moves a file only while it
//!   is still at the version given ([`MoveGuard`]).
//! - [`RemoteStorage::trash`] moves into a trash, never deletes for good: a file only while it is
//!   still at its version, a folder only while a fresh listing holds exactly what was planned, or
//!   nothing at all ([`TrashGuard`]).
//! - A permanent delete exists only as [`RemoteStorage::purge_own`], which takes an [`OwnPath`]:
//!   a path inside CraftCloud's own `.craftcloud/` (an [`OwnPath`] can't be made for anything else).
//!
//! What does compile (so the examples below fail for the reason they name, not a wrong path):
//!
//! ```
//! use craftcloud_native::storage::{OwnPath, RemoteStorage, TrashGuard};
//! fn trash_unchanged(storage: &mut dyn RemoteStorage, file: &std::path::Path) {
//!     let _ = storage.replace("Docs/a.txt", "\"e1\"", file);
//!     let _ = storage.trash("Docs/a.txt", TrashGuard::File { etag: "\"e1\"".into() });
//! }
//! assert!(OwnPath::new(".craftcloud/probe/Laptop-1").is_some());
//! assert!(OwnPath::new("Docs/a.txt").is_none());
//! ```
//!
//! ```compile_fail
//! // No such method: a delete goes to a trash, with its guard.
//! fn delete_for_good(storage: &mut impl craftcloud_native::storage::RemoteStorage) {
//!     storage.delete("Docs/a.txt");
//! }
//! ```
//!
//! ```compile_fail
//! // An overwrite always names the version it replaces.
//! fn overwrite(storage: &mut impl craftcloud_native::storage::RemoteStorage, file: &std::path::Path) {
//!     storage.replace("Docs/a.txt", file);
//! }
//! ```
//!
//! ```compile_fail
//! // An `OwnPath` is made only by `OwnPath::new`, which refuses paths outside `.craftcloud/`.
//! let path = craftcloud_native::storage::OwnPath(String::from("Docs/a.txt"));
//! ```
//!
//! The first driver is OxiCloud's ([`oxicloud::OxiCloud`]): the engine's own requests, moved
//! behind the trait unchanged. What a server guarantees is a [`Capabilities`] (pure data in
//! `craftcloud-core`).

pub mod oxicloud;
pub mod probe;
pub mod profile;
pub mod versions;
pub mod webdav;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use craftcloud_core::api::dav;
pub use craftcloud_core::storage::Capabilities;

use crate::http;

/// How many folders a driver lists at the same time when it can.
pub const LISTERS: usize = 4;

/// One entry of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Cloud path, decoded (`Documents/notes.md`).
    pub path: String,
    pub is_folder: bool,
    /// The version tag to send back in `If-Match` (quotes included), where the server listed one.
    pub etag: Option<String>,
    pub size: u64,
    /// As the server wrote it (RFC 1123).
    pub modified: Option<String>,
    /// The server's id for it, where it has one that survives a move (OxiCloud's WebDAV has none).
    pub file_id: Option<String>,
    /// A checksum the server listed (OxiCloud lists none).
    pub checksum: Option<String>,
}

impl From<dav::Entry> for Entry {
    fn from(e: dav::Entry) -> Self {
        Self { path: e.path, is_folder: e.is_folder, etag: e.etag, size: e.size, modified: e.modified, file_id: None, checksum: None }
    }
}

/// What a guarded write did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Written {
    /// Written; the new version tag.
    Saved { etag: String },
    /// Its guard didn't hold (412): something was there, or a newer version. Nothing was written.
    Conflict,
}

/// What restoring an earlier version did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restored {
    /// The version's bytes are the file now.
    Done,
    /// The file changed on the server between the look at it and the write (412): nothing was
    /// restored, and the newer file stays.
    Changed,
}

/// What a move must find before it moves. Neither ever lands on an existing name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveGuard {
    /// A file, only while the cloud still has it at this version.
    File { etag: String },
    /// A folder with everything in it (planned by `reconcile::folder_moves`).
    Folder,
}

/// What a move did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Moved {
    Done,
    /// The file isn't at the version given any more (changed, or gone): not moved.
    Changed,
    /// The file is listed without a version, so it can't be shown to be the one given: not moved.
    Unversioned,
    /// The server refused the move with this status.
    Refused(u16),
}

/// What must hold for something to go to the trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashGuard {
    /// A file, only while the cloud still has it at this version.
    File { etag: String },
    /// A folder, only while a fresh listing of every level holds exactly these files (path,
    /// ETag), nothing more or newer. A listing that fails is an error: unknown, never empty.
    Folder { files: Vec<(String, String)> },
    /// A folder, only while a fresh listing says it is empty. A listing that fails keeps it.
    EmptyFolder,
}

/// What a trash did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trashed {
    /// In the trash now (or gone already).
    Done,
    /// The guard didn't hold: it stays.
    Changed,
    /// A file listed without a version: it can't be shown unchanged, so it stays.
    Unversioned,
}

/// One trashed file or folder, as a trash listing gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashItem {
    /// The server's handle for it (OxiCloud: the id it had).
    pub id: String,
    /// Where it was: the cloud path a restore puts it back to.
    pub path: String,
    /// When it was trashed (RFC 3339).
    pub trashed_at: String,
}

/// Why a storage call failed. Its text is what the engine says (the sync's activity list and
/// errors keep their wording).
///
/// Whether a failure meant the server didn't answer or the sign-in is over is not here: it is
/// read from [`RemoteStorage::take_offline`] and [`RemoteStorage::signed_out`] once a pass ends,
/// so a failure the engine shrugged off (a free-space lookup) still counts, as it always has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    /// The server answered with a status this call can't take.
    Status(u16),
    /// What was asked for isn't there (404), in words.
    NotFound(String),
    /// No room (507): tried again once there is.
    NoRoom,
    /// Refused for what it is (a 4xx for the name or the file), in the server's words: tried
    /// again once the file changes.
    Refused(String),
    /// Anything else, in words: no answer, an answer that couldn't be read, a missing folder.
    Failed(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(status) => write!(f, "the server answered {status}"),
            Self::NoRoom => f.write_str("no room in the cloud"),
            Self::Refused(why) => write!(f, "the cloud refused it: {why}"),
            Self::NotFound(why) | Self::Failed(why) => f.write_str(why),
        }
    }
}

impl From<String> for StorageError {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

impl From<StorageError> for String {
    fn from(e: StorageError) -> Self {
        e.to_string()
    }
}

/// A path inside CraftCloud's own `.craftcloud/` on the server: the only kind of path
/// [`RemoteStorage::purge_own`] deletes for good.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnPath(String);

impl OwnPath {
    /// `Some` only for a path below `.craftcloud/` (never the folder itself), with no empty, `.`
    /// or `..` part and no backslash, so it can't lead out of it.
    pub fn new(path: &str) -> Option<Self> {
        let path = path.trim_matches('/');
        let mut parts = path.split('/');
        if parts.next() != Some(dav::CRAFTCLOUD_DIR) {
            return None;
        }
        let rest: Vec<&str> = parts.collect();
        let clean = |p: &&str| !p.is_empty() && *p != "." && *p != ".." && !p.contains('\\');
        (!rest.is_empty() && rest.iter().all(clean)).then(|| Self(path.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A download the server has started answering: its status and headers, and its body to stream.
pub struct Download {
    resp: ureq::http::Response<ureq::Body>,
    cancel: Arc<AtomicBool>,
}

impl Download {
    pub(crate) fn new(resp: ureq::http::Response<ureq::Body>, cancel: Arc<AtomicBool>) -> Self {
        Self { resp, cancel }
    }

    pub fn status(&self) -> u16 {
        self.resp.status().as_u16()
    }

    /// A response header, when it is there and readable.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.resp.headers().get(name).and_then(|v| v.to_str().ok())
    }

    /// Writes the body to `out`; stops when the transfer is cancelled (Pause, Quit).
    pub fn stream_to(&mut self, out: &mut impl std::io::Write) -> Result<u64, String> {
        http::stream_body(&mut self.resp, out, &self.cancel)
    }
}

/// What the sync engine asks of a server (docs/WEBDAV-STORAGE.md § 7.2). Paths are cloud paths
/// (`Documents/notes.md`), without leading or trailing slashes.
pub trait RemoteStorage: Send {
    /// What this server guarantees.
    fn capabilities(&self) -> &Capabilities;

    /// A new sync pass starts: forget what was learnt in the last one (ids, an offline error).
    fn begin_pass(&mut self);
    /// The last time the server didn't answer at all during this pass, if it didn't (taken).
    fn take_offline(&mut self) -> Option<String>;
    /// The sign-in is over: only a new sign-in helps.
    fn signed_out(&self) -> bool;

    /// One folder, one level, without the folder itself. An error is "unknown": the caller holds
    /// that place, never takes it as empty.
    fn list(&mut self, folder: &str) -> Result<Vec<Entry>, StorageError>;
    /// Several folders, one level each (at the same time where the driver can). A folder gone
    /// meanwhile is left out; any other failure is an error.
    fn list_many(&mut self, folders: &[String]) -> Result<Vec<(String, Vec<Entry>)>, StorageError> {
        let mut out = Vec::new();
        for folder in folders {
            match self.list(folder) {
                Ok(entries) => out.push((folder.clone(), entries)),
                Err(StorageError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
    /// The ETag the cloud lists for the file at `path` (for a write or a download whose answer
    /// carried none).
    fn listed_etag(&mut self, path: &str) -> Result<String, StorageError> {
        let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
        self.list(parent)?
            .into_iter()
            .find(|e| e.path == path)
            .and_then(|e| e.etag)
            .ok_or_else(|| StorageError::Failed(format!("{path}: no ETag after upload")))
    }

    /// Starts a download of `path`, from byte `from` on when given (the caller checks the answer
    /// is that version from that byte on before using it).
    fn download(&mut self, path: &str, from: Option<u64>) -> Result<Download, StorageError>;

    /// Writes `file` to `path` only if nothing is there. `Written::Conflict` when something is.
    fn create(&mut self, path: &str, file: &Path) -> Result<Written, StorageError>;
    /// Writes `file` over `path` only if the server still has version `if_match`.
    /// `Written::Conflict` otherwise.
    fn replace(&mut self, path: &str, if_match: &str, file: &Path) -> Result<Written, StorageError>;
    /// Makes one folder (its parent must be there). One that is there already is fine.
    fn make_folder(&mut self, path: &str) -> Result<(), StorageError>;
    /// Moves `from` to `to`, never onto an existing name, only while `guard` holds.
    fn move_to(&mut self, from: &str, to: &str, guard: MoveGuard) -> Result<Moved, StorageError>;
    /// Moves `path` into a trash, never deletes it for good, only while `guard` holds.
    fn trash(&mut self, path: &str, guard: TrashGuard) -> Result<Trashed, StorageError>;
    /// What is in the trash.
    fn trash_list(&mut self) -> Result<Vec<TrashItem>, StorageError>;
    /// Puts `item` back where it was.
    fn restore(&mut self, item: &TrashItem) -> Result<(), StorageError>;

    /// Keeps the cloud version of `path` that is about to be replaced (nothing to keep is fine).
    fn keep_version(&mut self, path: &str) -> Result<(), StorageError>;
    /// Takes the earlier versions of `from` (a file, or a folder's files) along to `to`.
    fn move_versions(&mut self, from: &str, to: &str) -> Result<(), StorageError>;
    /// The free space in bytes (`u64::MAX` without a quota), `None` when unknown. `needed` is
    /// the least a waiting upload needs: a driver may ask the server to count again below it.
    fn free_space(&mut self, needed: u64) -> Result<Option<u64>, StorageError>;

    /// Deletes for good, only inside CraftCloud's own `.craftcloud/` (probe files, old client
    /// trash, pruned versions). Gone already is fine.
    fn purge_own(&mut self, path: &OwnPath) -> Result<(), StorageError>;

    /// Removes client-trash entries past their retention, where the driver keeps the trash itself
    /// (the generic WebDAV driver; docs/WEBDAV-STORAGE.md § 16, Q5: 30 days, by the server's clock,
    /// through [`RemoteStorage::purge_own`], at most once a day). A server with its own trash keeps
    /// to its own policy: this is a no-op. Called after a cycle; a failure never fails the cycle.
    fn sweep_trash(&mut self) -> Result<(), StorageError> {
        Ok(())
    }

    /// Re-runs the check on connect after the runtime watch saw this server misbehave
    /// (docs/WEBDAV-STORAGE.md § 7.5), saves it, and takes the fresh capabilities. The new mode is
    /// for the record; the engine has already dropped two-way sync for the rest of the run. A
    /// driver whose capabilities are known rather than measured (OxiCloud) never probes: `None`.
    fn recheck(&mut self) -> Option<craftcloud_core::storage::Mode> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{OwnPath, StorageError};

    #[test]
    fn own_paths_are_only_inside_craftcloud() {
        for inside in [".craftcloud/versions/Docs/a.txt/20261113T151013Z", ".craftcloud/probe/Laptop-1", "/.craftcloud/trash/x/"] {
            assert!(OwnPath::new(inside).is_some(), "{inside}");
        }
        for outside in [
            "Docs/a.txt",
            "",
            ".craftcloud",
            ".craftcloud/",
            ".craftcloud/../Docs",
            ".craftcloud/versions/../../Docs",
            ".craftcloud/./x",
            ".craftcloud//x",
            ".craftcloud/a\\..\\..\\Docs",
            ".craftcloudx/a",
            "Docs/.craftcloud/a",
        ] {
            assert!(OwnPath::new(outside).is_none(), "{outside:?}");
        }
        assert_eq!(OwnPath::new("/.craftcloud/a/").map(|p| p.as_str().to_owned()).as_deref(), Some(".craftcloud/a"));
    }

    #[test]
    fn errors_say_what_the_engine_always_said() {
        assert_eq!(String::from(StorageError::Status(409)), "the server answered 409");
        assert_eq!(String::from(StorageError::Failed("listing \"Docs\": 404".into())), "listing \"Docs\": 404");
    }
}
