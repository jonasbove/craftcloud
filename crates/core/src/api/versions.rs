//! CraftCloud's own file versions: before a file is overwritten, the old bytes are copied to
//! `.craftcloud/versions/<path>/<stamp>` over WebDAV and the newest [`VERSIONS_KEPT`] are kept
//! (OxiCloud has no versions of its own, docs/UPSTREAM.md C3). Keyed by path, since WebDAV has no
//! file id: a rename or move takes the history along ([`move_history`]).
//!
//! The steps, which the desktop client, the command line and the web frontend all follow:
//!
//! - **Snapshot** `path`: skip it when the newest version already holds these bytes
//!   ([`already_kept`], by the content hash in the ETags: a retried overwrite must not keep the same
//!   version twice). Else `MKCOL` each of [`folders_for`] (201, or 405 when it exists), then
//!   [`snapshot`] (a `COPY`; 404 means there was nothing to keep), then list the versions and
//!   delete what [`to_prune`] names.
//! - **Rename or move** `from` to `to` (a file or a folder): `MKCOL` each of [`history_parents`]
//!   of `to`, then [`move_history`] (404: there was no history).
//! - **Restore** version `stamp` of `path`: snapshot the current file first (so a restore can be
//!   undone), then [`restore`] (a `COPY` over the file).

use super::dav::{self, CRAFTCLOUD_DIR, Entry};
pub use super::dav::{VERSIONS_KEPT, version_folders as folders_for, version_path, versions_folder, versions_to_prune as to_prune};
use super::{ApiError, expect_success};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// How a version's stamp is written (a `strftime` pattern, UTC): `20261113T151013.123Z`. Stamps
/// sort by time as text.
pub const STAMP_FORMAT: &str = "%Y%m%dT%H%M%S%.3fZ";

/// One kept version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// Its name, e.g. `20261113T151013.123Z`: when it was replaced.
    pub stamp: String,
    pub size: u64,
    /// Its ETag (`"<content hash>-<time>"`), when the listing had one.
    pub etag: Option<String>,
}

impl Version {
    /// `2026-11-13 15:10:13 UTC`, or the stamp itself when it isn't one CraftCloud wrote.
    pub fn when(&self) -> String {
        display_stamp(&self.stamp).unwrap_or_else(|| self.stamp.clone())
    }

    /// When it was replaced (RFC 3339, UTC: `2026-11-13T15:10:13Z`), from its name.
    pub fn replaced_at(&self) -> Option<String> {
        rfc3339_of(&self.stamp)
    }

    /// When its content was last changed (RFC 3339, UTC), when its name says ([`name_for`]):
    /// versions kept before CraftCloud wrote that (or by the web frontend) don't.
    ///
    /// Never after [`Self::replaced_at`]: content can't have changed after it was replaced. The
    /// two come from different clocks (the server's `Last-Modified`, in whole seconds, and the
    /// replacing computer's), so a server clock that is ahead, or one that rounds up, would
    /// otherwise show a version as changed after it was replaced; then the replacement time is
    /// the best bound known. Both are the same fixed-width UTC text, so they compare as text.
    pub fn modified_at(&self) -> Option<String> {
        let modified = rfc3339_of(self.stamp.split_once('_')?.1)?;
        Some(match self.replaced_at() {
            Some(replaced) if replaced < modified => replaced,
            _ => modified,
        })
    }
}

/// How the content's own time is written in a version's name (a `strftime` pattern, UTC).
pub const MODIFIED_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// A new version's name: `stamp` ([`STAMP_FORMAT`], when it is replaced), then, when known, `_`
/// and when its content was last changed ([`MODIFIED_FORMAT`]): `20261113T151013.123Z_20261113T140205Z`.
/// The server's `COPY` gives the copy the time of the copy, so the content's own time is kept in
/// the name (T-019). Names still sort by when they were replaced.
pub fn name_for(stamp: &str, modified: Option<&str>) -> String {
    match modified {
        Some(m) if !m.is_empty() => format!("{stamp}_{m}"),
        _ => stamp.to_owned(),
    }
}

/// A version named by its time in seconds since 1970 (a server's own versions, as Nextcloud names
/// them): the time, when the name is nothing but digits.
fn seconds_stamp(stamp: &str) -> Option<jiff::Timestamp> {
    if stamp.is_empty() || stamp.len() > 11 || !stamp.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    jiff::Timestamp::from_second(stamp.parse().ok()?).ok()
}

/// `20261113T151013.123Z…` or `20261113T151013Z` → `2026-11-13T15:10:13Z`; a name in seconds too.
fn rfc3339_of(stamp: &str) -> Option<String> {
    if let Some(at) = seconds_stamp(stamp) {
        return Some(at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string());
    }
    let b = stamp.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).filter(|s| s.iter().all(u8::is_ascii_digit)).map(|_| &stamp[r]);
    if b.get(8) != Some(&b'T') {
        return None;
    }
    let (y, mo, d, h, mi, s) = (digits(0..4)?, digits(4..6)?, digits(6..8)?, digits(9..11)?, digits(11..13)?, digits(13..15)?);
    Some(format!("{y}-{mo}-{d}T{h}:{mi}:{s}Z"))
}

/// `PROPFIND` the versions folder of `path`.
pub fn list(server: &ServerUrl, path: &str) -> HttpRequest {
    dav::list(server, &versions_folder(path))
}

/// [`list`] under any WebDAV base (`dav::url_under`): the generic driver keeps versions the same
/// way under its own base.
pub fn list_under(base: &str, path: &str) -> HttpRequest {
    dav::list_under(base, &versions_folder(path))
}

/// The versions, newest first. A file that never had an earlier version has no versions folder
/// (404): that is an empty list, not an error.
pub fn parse_list(resp: &HttpResponse) -> Result<Vec<Version>, ApiError> {
    if resp.status == 404 {
        return Ok(Vec::new());
    }
    let mut out: Vec<Version> = dav::parse_list(resp)?
        .into_iter()
        .filter(|e: &Entry| !e.is_folder)
        .map(|e| Version { stamp: e.name().to_owned(), size: e.size, etag: e.etag })
        .collect();
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    Ok(out)
}

/// `COPY path → .craftcloud/versions/<path>/<stamp>`, never overwriting.
pub fn snapshot(server: &ServerUrl, path: &str, stamp: &str) -> HttpRequest {
    dav::copy(server, path, &version_path(path, stamp))
}

/// [`snapshot`] under any WebDAV base.
pub fn snapshot_under(base: &str, path: &str, stamp: &str) -> HttpRequest {
    dav::copy_under(base, path, &version_path(path, stamp))
}

/// Whether a snapshot answer is fine: copied, or nothing there to keep (404).
pub fn snapshot_done(resp: &HttpResponse) -> Result<(), ApiError> {
    if resp.status == 404 {
        return Ok(());
    }
    expect_success(resp)
}

/// The content part of an OxiCloud file ETag (`"44c77418e27569db-1794615200"` →
/// `44c77418e27569db`): the same bytes have the same one, whatever their time (a `COPY` keeps it).
pub fn content_hash(etag: &str) -> &str {
    let bare = etag.trim_matches('"');
    bare.rsplit_once('-').map_or(bare, |(hash, _)| hash)
}

/// Whether the newest of `versions` (newest first, as [`parse_list`] gives them) already holds the
/// bytes of the file whose ETag is `current`: then a snapshot would only keep them twice.
pub fn already_kept(current: &str, versions: &[Version]) -> bool {
    versions.first().and_then(|v| v.etag.as_deref()).is_some_and(|newest| content_hash(newest) == content_hash(current))
}

/// The folders to `MKCOL` (parent first) before moving a history to `to`'s: all of
/// [`folders_for`] but the last, which the move makes.
pub fn history_parents(to: &str) -> Vec<String> {
    let mut folders = folders_for(to);
    folders.pop();
    folders
}

/// `MOVE` the versions of `from` (a file, or a folder with its files' histories) to `to`'s,
/// never over an existing history (412).
pub fn move_history(server: &ServerUrl, from: &str, to: &str) -> HttpRequest {
    dav::move_to(server, &versions_folder(from), &versions_folder(to), true)
}

/// [`move_history`] under any WebDAV base.
pub fn move_history_under(base: &str, from: &str, to: &str) -> HttpRequest {
    dav::move_to_under(base, &versions_folder(from), &versions_folder(to), true)
}

/// Everything that takes `from`'s history along to `to` after a rename or move (a file, or a
/// folder with its files' histories), in order (T-050): `MKCOL` each of [`history_parents`] (201,
/// or 405 when it is there), then [`move_history`] (404: there was none). Send them one after the
/// other once the rename or move itself worked; the desktop client, `craftcloud mv` and the web
/// frontend's Rename and Move all do.
pub fn follow_move(server: &ServerUrl, from: &str, to: &str) -> Vec<HttpRequest> {
    let mut out: Vec<HttpRequest> = history_parents(to).iter().map(|f| HttpRequest::new(Method::Mkcol, dav::url(server, f, true))).collect();
    out.push(move_history(server, from, to));
    out
}

/// `DELETE` (for good) the whole history of `path`: for a file that is gone for good (its trash
/// item deleted, the trash emptied: T-049), or stale history at a path where a new file is made
/// (T-050: the new file must not inherit the versions of the one that was there before). 404:
/// there was none.
pub fn drop_history(server: &ServerUrl, path: &str) -> HttpRequest {
    dav::delete(server, &versions_folder(path), true)
}

/// `DELETE` an old version.
pub fn delete(server: &ServerUrl, path: &str, stamp: &str) -> HttpRequest {
    dav::delete(server, &version_path(path, stamp), false)
}

/// [`delete`] under any WebDAV base.
pub fn delete_under(base: &str, path: &str, stamp: &str) -> HttpRequest {
    dav::delete_under(base, &version_path(path, stamp), false)
}

/// `COPY .craftcloud/versions/<path>/<stamp> → path`, replacing the file.
pub fn restore(server: &ServerUrl, path: &str, stamp: &str) -> HttpRequest {
    dav::copy_over(server, &version_path(path, stamp), path)
}

/// [`restore`] under any WebDAV base (the generic driver).
pub fn restore_under(base: &str, path: &str, stamp: &str) -> HttpRequest {
    dav::copy_over_under(base, &version_path(path, stamp), path)
}

/// `20261113T151013.123Z` → `2026-11-13 15:10:13 UTC`.
pub fn display_stamp(stamp: &str) -> Option<String> {
    if let Some(at) = seconds_stamp(stamp) {
        return Some(at.strftime("%Y-%m-%d %H:%M:%S UTC").to_string());
    }
    let b = stamp.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).filter(|s| s.iter().all(u8::is_ascii_digit)).map(|_| &stamp[r]);
    if b.get(8) != Some(&b'T') || !stamp.ends_with('Z') {
        return None;
    }
    let (y, mo, d, h, mi, s) = (digits(0..4)?, digits(4..6)?, digits(6..8)?, digits(9..11)?, digits(11..13)?, digits(13..15)?);
    Some(format!("{y}-{mo}-{d} {h}:{mi}:{s} UTC"))
}

/// Whether a cloud path is a stored version rather than a file of the person's.
pub fn is_version_path(path: &str) -> bool {
    path.trim_start_matches('/').starts_with(&format!("{CRAFTCLOUD_DIR}/versions/"))
}

/// Requests to make the folders for `path`'s versions, parent first.
pub fn make_folders(server: &ServerUrl, path: &str) -> Vec<HttpRequest> {
    folders_for(path).iter().map(|f| HttpRequest::new(Method::Mkcol, dav::url(server, f, true))).collect()
}

/// [`make_folders`] under any WebDAV base.
pub fn make_folders_under(base: &str, path: &str) -> Vec<HttpRequest> {
    folders_for(path).iter().map(|f| dav::create_folder_under(base, f)).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Version, already_kept, content_hash, display_stamp, history_parents, is_version_path, list, make_folders, move_history, parse_list, restore,
        snapshot, snapshot_done,
    };
    use crate::ServerUrl;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8095").unwrap()
    }

    const LISTING: &str = r#"<D:multistatus xmlns:D="DAV:">
<D:response><D:href>/webdav/.craftcloud/versions/Documents/notes.md/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/webdav/.craftcloud/versions/Documents/notes.md/20261113T151013.123Z</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>11</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/webdav/.craftcloud/versions/Documents/notes.md/20261114T090000.000Z</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>12</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#;

    #[test]
    fn requests() {
        let s = server();
        let l = list(&s, "Documents/notes.md");
        assert_eq!((l.method, l.url.as_str()), (Method::Propfind, "http://127.0.0.1:8095/webdav/.craftcloud/versions/Documents/notes.md/"));
        let snap = snapshot(&s, "Documents/notes.md", "20261113T151013.123Z");
        assert_eq!((snap.method, snap.url.as_str()), (Method::Copy, "http://127.0.0.1:8095/webdav/Documents/notes.md"));
        assert_eq!(snap.header_value("Overwrite"), Some("F"));
        let r = restore(&s, "Documents/notes.md", "20261113T151013.123Z");
        assert_eq!(r.url, "http://127.0.0.1:8095/webdav/.craftcloud/versions/Documents/notes.md/20261113T151013.123Z");
        assert_eq!(r.header_value("Destination"), Some("http://127.0.0.1:8095/webdav/Documents/notes.md"));
        assert_eq!(r.header_value("Overwrite"), Some("T"));
        let folders: Vec<String> = make_folders(&s, "a.md").into_iter().map(|r| r.url).collect();
        assert_eq!(folders.last().map(String::as_str), Some("http://127.0.0.1:8095/webdav/.craftcloud/versions/a.md/"));
    }

    /// T-050 / T-049: a history follows a rename or move, and goes with a file gone for good.
    #[test]
    fn histories_follow_moves_and_go_for_good() {
        use super::{drop_history, follow_move};
        let s = server();
        let steps: Vec<(Method, String)> = follow_move(&s, "Docs/a.txt", "Archive/2026/b.txt").into_iter().map(|r| (r.method, r.url)).collect();
        let base = "http://127.0.0.1:8095/webdav/.craftcloud/versions";
        assert_eq!(steps.last().map(|(m, u)| (*m, u.as_str())), Some((Method::Move, format!("{base}/Docs/a.txt/").as_str())));
        assert!(steps[..steps.len() - 1].iter().all(|(m, _)| *m == Method::Mkcol));
        assert!(steps.iter().any(|(_, u)| u == &format!("{base}/Archive/2026/")), "{steps:?}");
        let d = drop_history(&s, "Docs/a.txt");
        assert_eq!((d.method, d.url.as_str()), (Method::Delete, format!("{base}/Docs/a.txt/").as_str()));
    }

    /// T-019: a version says when its content was last changed, not only when it was replaced.
    #[test]
    fn a_versions_name_keeps_its_contents_time() {
        use super::name_for;
        let name = name_for("20261113T151013.123Z", Some("20261113T140205Z"));
        assert_eq!(name, "20261113T151013.123Z_20261113T140205Z");
        assert_eq!(name_for("20261113T151013.123Z", None), "20261113T151013.123Z");
        let v = Version { stamp: name, size: 1, etag: None };
        assert_eq!(v.replaced_at().as_deref(), Some("2026-11-13T15:10:13Z"));
        assert_eq!(v.modified_at().as_deref(), Some("2026-11-13T14:02:05Z"));
        assert_eq!(v.when(), "2026-11-13 15:10:13 UTC");
        let old = Version { stamp: "20261113T151013.123Z".into(), size: 1, etag: None };
        assert_eq!((old.replaced_at().as_deref(), old.modified_at()), (Some("2026-11-13T15:10:13Z"), None));
        let mut names = ["20261114T090000.000Z".to_owned(), "20261113T151013.123Z_20261114T080000Z".to_owned()];
        names.sort();
        assert_eq!(names[0], "20261113T151013.123Z_20261114T080000Z", "still sorted by when they were replaced");
    }

    /// The content's time is the server's (whole seconds), the replacement time the replacing
    /// computer's: a server clock a second (or an hour) ahead must not show a version's content
    /// as changed after it was replaced.
    #[test]
    fn a_versions_content_is_never_newer_than_its_replacement() {
        for name in ["20261113T151013.123Z_20261113T151014Z", "20261113T151013.123Z_20261113T161013Z"] {
            let v = Version { stamp: name.into(), size: 1, etag: None };
            assert_eq!(v.modified_at().as_deref(), Some("2026-11-13T15:10:13Z"), "{name}");
            assert_eq!(v.replaced_at().as_deref(), Some("2026-11-13T15:10:13Z"));
        }
        let same = Version { stamp: "20261113T151013.123Z_20261113T151013Z".into(), size: 1, etag: None };
        assert_eq!(same.modified_at().as_deref(), Some("2026-11-13T15:10:13Z"));
    }

    #[test]
    fn newest_first_and_none_is_empty() {
        let v = parse_list(&HttpResponse::new(207, LISTING.as_bytes().to_vec())).unwrap();
        assert_eq!(
            v,
            [
                Version { stamp: "20261114T090000.000Z".into(), size: 12, etag: None },
                Version { stamp: "20261113T151013.123Z".into(), size: 11, etag: None }
            ]
        );
        assert_eq!(parse_list(&HttpResponse::new(404, Vec::new())), Ok(Vec::new()));
        assert!(snapshot_done(&HttpResponse::new(404, Vec::new())).is_ok());
        assert!(snapshot_done(&HttpResponse::new(201, Vec::new())).is_ok());
        assert!(snapshot_done(&HttpResponse::new(507, Vec::new())).is_err());
    }

    #[test]
    fn the_same_bytes_are_not_kept_twice() {
        assert_eq!(content_hash("\"8e4c7c1b99dbfd50-1794615202\""), "8e4c7c1b99dbfd50");
        let kept =
            |etag: &str| vec![Version { stamp: "2".into(), size: 6, etag: Some(etag.into()) }, Version { stamp: "1".into(), size: 3, etag: None }];
        // A COPY keeps the hash and changes the time.
        assert!(already_kept("\"8e4c7c1b99dbfd50-1794615201\"", &kept("\"8e4c7c1b99dbfd50-1794615202\"")));
        assert!(!already_kept("\"aaaa-1\"", &kept("\"8e4c7c1b99dbfd50-1794615202\"")));
        assert!(!already_kept("\"aaaa-1\"", &[]));
    }

    #[test]
    fn a_history_moves_with_its_file() {
        let m = move_history(&server(), "Docs/a.md", "Archive/2026/b.md");
        assert_eq!((m.method, m.url.as_str()), (Method::Move, "http://127.0.0.1:8095/webdav/.craftcloud/versions/Docs/a.md/"));
        assert_eq!(m.header_value("Destination"), Some("http://127.0.0.1:8095/webdav/.craftcloud/versions/Archive/2026/b.md/"));
        assert_eq!(m.header_value("Overwrite"), Some("F"));
        assert_eq!(
            history_parents("Archive/2026/b.md"),
            [".craftcloud", ".craftcloud/versions", ".craftcloud/versions/Archive", ".craftcloud/versions/Archive/2026"]
        );
    }

    #[test]
    fn stamps_read_as_times() {
        assert_eq!(display_stamp("20261113T151013.123Z").as_deref(), Some("2026-11-13 15:10:13 UTC"));
        assert_eq!(display_stamp("20261113T151013Z").as_deref(), Some("2026-11-13 15:10:13 UTC"));
        assert_eq!(display_stamp("notes-old.md"), None);
        assert!(is_version_path(".craftcloud/versions/a.md/1"));
        assert!(!is_version_path("Documents/a.md"));
    }
}
