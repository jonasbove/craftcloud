//! What a storage server guarantees the sync, as plain data (docs/WEBDAV-STORAGE.md § 7.3).
//!
//! The sync engine relies on a server for a handful of things: a create that refuses to overwrite,
//! an overwrite that refuses a version it didn't start from, a move that never lands on an
//! existing name, ETags that change with every write, a trash and earlier versions. Which of them
//! a server has is a [`Capabilities`]. For OxiCloud they are filled in by hand
//! ([`Capabilities::oxicloud`]) from what the engine relies on today; the check on connect that
//! measures them on other servers, and the sync modes that follow from them, come later (§ 14).
//!
//! No I/O here: the drivers that talk to a server are in `craftcloud-native` (`storage`).

/// Whether a server keeps a write guard it was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Guard {
    /// A write it doesn't allow is refused (412) and nothing changes.
    Honoured,
    /// It writes anyway.
    Ignored,
}

/// Whether a delete with `If-Match` is refused when the file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeleteGuard {
    Honoured,
    Ignored,
    /// Deletes don't go over WebDAV (OxiCloud: its REST trash, which takes no guard; the engine
    /// lists the file just before, the window docs/WEBDAV-STORAGE.md § 8 describes).
    NotUsed,
}

/// Whether a folder's ETag changes when something in it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FolderEtags {
    Follow,
    DontFollow,
    /// Seen to miss changes: never used to skip a folder (every folder is listed every cycle).
    NotTrusted,
}

/// What a `PUT` into a folder that isn't there answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissingParent {
    /// 404.
    NotFound,
    /// 409.
    Conflict,
    Other,
}

/// A checksum a server lists for its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Checksum {
    Sha1,
    Md5,
    Adler32,
}

/// Where a delete goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trash {
    /// The server's own trash (restored the way its profile says).
    Server,
    /// CraftCloud's own, `.craftcloud/trash/` on the server.
    Client,
}

/// Where the bytes a save replaces are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Versions {
    /// The server keeps them itself (by file id).
    Server,
    /// CraftCloud copies them to `.craftcloud/versions/<path>/<time>` (5 kept).
    Client,
    None,
}

/// Where the free space comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quota {
    /// OxiCloud's `GET /api/auth/me`.
    OxiCloudRest,
    /// Nextcloud's OCS user endpoint.
    Ocs,
    /// WebDAV's quota properties.
    WebDavProps,
    Unknown,
}

/// What a server guarantees the sync (docs/WEBDAV-STORAGE.md § 7.3).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Capabilities {
    /// `PUT` with `If-None-Match: *` over an existing file.
    pub create_if_absent: Guard,
    /// `PUT` with a stale `If-Match`.
    pub replace_if_match: Guard,
    /// `DELETE` with a stale `If-Match`.
    pub delete_if_match: DeleteGuard,
    /// `MOVE` with `Overwrite: F` onto an existing name is refused.
    pub move_no_overwrite: bool,
    /// A file's ETag changes on every write.
    pub etag_changes_on_write: bool,
    /// A `MOVE` keeps a file's ETag (moves made elsewhere are paired by it).
    pub etag_kept_by_move: bool,
    /// The ETag a `PUT` answers is the one a listing shows.
    pub put_etag_is_listed: bool,
    pub folder_etags: FolderEtags,
    pub missing_parent: MissingParent,
    /// `PROPFIND` with `Depth: infinity` answers.
    pub depth_infinity: bool,
    /// `COPY` works (client versions need it).
    pub copy: bool,
    /// Files have ids that survive a `MOVE` (`oc:fileid`).
    pub file_ids: bool,
    pub checksums: Option<Checksum>,
    /// Names that differ only in case are one file.
    pub case_insensitive: bool,
    pub trash: Trash,
    pub versions: Versions,
    pub quota: Quota,
}

impl Capabilities {
    /// OxiCloud v0.10.0, CraftCloud's own server, as the sync engine relies on it today (checked
    /// against the server in docs/WEBDAV-STORAGE.md § 5 and `crates/native/tests/live.rs`).
    pub const fn oxicloud() -> Self {
        Self {
            // `dav::Write::Create`: a 412 is a conflict copy.
            create_if_absent: Guard::Honoured,
            // `dav::Write::Replace`: a 412 is a conflict copy.
            replace_if_match: Guard::Honoured,
            // Deletes go to the REST trash, after a fresh listing shows the version planned.
            delete_if_match: DeleteGuard::NotUsed,
            // `dav::move_to` sends `Overwrite: F`.
            move_no_overwrite: true,
            etag_changes_on_write: true,
            // `reconcile` pairs a cloud rename by its ETag.
            etag_kept_by_move: true,
            put_etag_is_listed: true,
            // UPSTREAM.md C8: every folder is listed every cycle.
            folder_etags: FolderEtags::NotTrusted,
            missing_parent: MissingParent::Conflict,
            // 403: listed one level at a time.
            depth_infinity: false,
            // Client versions are `COPY`s.
            copy: true,
            // `oc:fileid` answers 404: versions are keyed by path.
            file_ids: false,
            checksums: None,
            // The engine treats the cloud's names as case-sensitive (only a Windows or macOS
            // computer's own clash rule applies, `names::clashes`).
            case_insensitive: false,
            // `files::trash` and `folders::trash` over REST; restored by id.
            trash: Trash::Server,
            // `.craftcloud/versions/<path>/<time>`, newest 5.
            versions: Versions::Client,
            quota: Quota::OxiCloudRest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Capabilities, DeleteGuard, FolderEtags, Guard, MissingParent, Quota, Trash, Versions};

    #[test]
    fn oxicloud_is_what_the_engine_relies_on() {
        let c = Capabilities::oxicloud();
        assert_eq!((c.create_if_absent, c.replace_if_match), (Guard::Honoured, Guard::Honoured), "both writes are guarded");
        assert_eq!(c.delete_if_match, DeleteGuard::NotUsed, "deletes go to the REST trash");
        assert!(c.move_no_overwrite && c.etag_changes_on_write && c.etag_kept_by_move && c.put_etag_is_listed && c.copy);
        assert_eq!(c.folder_etags, FolderEtags::NotTrusted, "UPSTREAM.md C8");
        assert_eq!(c.missing_parent, MissingParent::Conflict);
        assert!(!c.depth_infinity && !c.file_ids && !c.case_insensitive && c.checksums.is_none());
        assert_eq!((c.trash, c.versions, c.quota), (Trash::Server, Versions::Client, Quota::OxiCloudRest));
    }
}
