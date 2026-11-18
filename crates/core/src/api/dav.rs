//! File bytes over WebDAV (`/webdav/…`, the user's personal drive).
//!
//! OxiCloud's REST API can't replace a file by id or check a version on upload, but WebDAV can:
//! `PUT` with `If-Match` overwrites only the version the client last saw (else 412), and `COPY`
//! snapshots a file on the server. Paths here are cloud paths like `Documents/notes.md`, without
//! the `/webdav/` prefix and without leading or trailing slashes.

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};

use super::{ApiError, expect_success};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// The folder CraftCloud keeps its own data in, on the server. The desktop client never syncs it.
pub const CRAFTCLOUD_DIR: &str = ".craftcloud";

/// How many earlier versions of a file are kept (docs/ARCHITECTURE.md, "Sync").
pub const VERSIONS_KEPT: usize = 5;

/// Characters escaped in a path segment: controls, space, and what has meaning in a URL.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'\\')
    .add(b'^')
    .add(b'|')
    .add(b'[')
    .add(b']');

/// The URL of a cloud path; folders get a trailing slash.
pub fn url(server: &ServerUrl, path: &str, folder: bool) -> String {
    url_under(&server.join("webdav/"), path, folder)
}

/// The URL of `path` under any WebDAV base (`https://cloud.example.org/remote.php/dav/files/john/`,
/// a trailing slash added where missing), each segment escaped; folders get a trailing slash.
pub fn url_under(base: &str, path: &str, folder: bool) -> String {
    let encoded: Vec<String> = segments(path).map(|s| utf8_percent_encode(s, SEGMENT).to_string()).collect();
    let mut out = base.trim_end_matches('/').to_owned();
    out.push('/');
    out.push_str(&encoded.join("/"));
    if folder && !encoded.is_empty() {
        out.push('/');
    }
    out
}

fn segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty())
}

/// One entry of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Cloud path, decoded (`Documents/notes.md`); the listed folder itself is `""` for the root.
    pub path: String,
    pub is_folder: bool,
    /// The version tag to send back in `If-Match` when overwriting (quotes included).
    pub etag: Option<String>,
    pub size: u64,
    /// As the server wrote it (RFC 1123), e.g. `Fri, 13 Nov 2026 15:10:13 +0000`.
    pub modified: Option<String>,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or("")
    }
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getetag/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

/// `PROPFIND` depth 1: the folder and what is directly in it.
pub fn list(server: &ServerUrl, folder: &str) -> HttpRequest {
    list_under(&server.join("webdav/"), folder)
}

/// [`list`] under any WebDAV base ([`url_under`]).
pub fn list_under(base: &str, folder: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, url_under(base, folder, true))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(PROPFIND_BODY.as_bytes().to_vec())
}

/// Reads a listing. The first entry is the listed folder itself.
pub fn parse_list(resp: &HttpResponse) -> Result<Vec<Entry>, ApiError> {
    if resp.status != 207 {
        expect_success(resp)?;
        return Err(ApiError::Body(format!("expected 207 Multi-Status, got {}", resp.status)));
    }
    let doc = craft_dav::parse_multistatus(&resp.body).map_err(|e| ApiError::Body(e.to_string()))?;
    craft_dav::responses(&doc).map(entry).collect()
}

fn entry(response: &craft_xml::El) -> Result<Entry, ApiError> {
    let href = craft_dav::href(response).ok_or_else(|| ApiError::Body("a response without href".into()))?;
    let path = cloud_path(&href)?;
    let mut e = Entry { path, is_folder: false, etag: None, size: 0, modified: None };
    for prop in craft_dav::properties(response) {
        match prop.name.as_str() {
            "d:resourcetype" => e.is_folder = prop.child("d:collection").is_some(),
            "d:getetag" => e.etag = Some(prop.text().trim().to_owned()).filter(|t| !t.is_empty()),
            "d:getcontentlength" => e.size = prop.text().trim().parse().unwrap_or(0),
            "d:getlastmodified" => e.modified = Some(prop.text().trim().to_owned()).filter(|t| !t.is_empty()),
            _ => {}
        }
    }
    Ok(e)
}

/// `/webdav/Documents/My%20notes.md` → `Documents/My notes.md`.
fn cloud_path(href: &str) -> Result<String, ApiError> {
    // An absolute URL is possible in WebDAV; keep only its path.
    let path = href.find("://").and_then(|i| href[i + 3..].find('/').map(|j| &href[i + 3 + j..])).unwrap_or(href);
    let rest = path.strip_prefix("/webdav").ok_or_else(|| ApiError::Body(format!("href outside /webdav: {href}")))?;
    let decoded = percent_decode_str(rest).decode_utf8().map_err(|_| ApiError::Body(format!("href is not UTF-8: {href}")))?;
    Ok(decoded.trim_matches('/').to_owned())
}

/// `GET` a file's bytes.
pub fn download(server: &ServerUrl, path: &str) -> HttpRequest {
    download_under(&server.join("webdav/"), path)
}

/// [`download`] under any WebDAV base.
pub fn download_under(base: &str, path: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, url_under(base, path, false))
}

/// How to write a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    /// Only if nothing is there yet (`If-None-Match: *`).
    Create,
    /// Only if the server still has this version (`If-Match`); else 412 and nothing changes.
    Replace { etag: String },
}

pub fn upload(server: &ServerUrl, path: &str, bytes: Vec<u8>, write: &Write) -> HttpRequest {
    upload_under(&server.join("webdav/"), path, bytes, write)
}

/// [`upload`] under any WebDAV base: a `PUT` that always carries its guard.
pub fn upload_under(base: &str, path: &str, bytes: Vec<u8>, write: &Write) -> HttpRequest {
    let req = HttpRequest::new(Method::Put, url_under(base, path, false)).body(bytes);
    match write {
        Write::Create => req.header("If-None-Match", "*"),
        Write::Replace { etag } => req.header("If-Match", etag.clone()),
    }
}

/// What an upload did.
#[derive(Debug, PartialEq, Eq)]
pub enum Uploaded {
    /// Written; the new version tag, when the server sent one.
    Saved { etag: Option<String> },
    /// Someone changed (or created) the file in the meantime: nothing was written. The caller
    /// saves a conflict copy instead (docs/ARCHITECTURE.md, "Sync").
    Conflict,
}

pub fn parse_upload(resp: &HttpResponse) -> Result<Uploaded, ApiError> {
    if resp.status == 412 {
        return Ok(Uploaded::Conflict);
    }
    expect_success(resp)?;
    Ok(Uploaded::Saved { etag: resp.header("ETag").map(str::to_owned) })
}

/// `MKCOL`: one new folder (its parent must exist). 405 means it already exists.
pub fn create_folder(server: &ServerUrl, path: &str) -> HttpRequest {
    create_folder_under(&server.join("webdav/"), path)
}

/// [`create_folder`] under any WebDAV base.
pub fn create_folder_under(base: &str, path: &str) -> HttpRequest {
    HttpRequest::new(Method::Mkcol, url_under(base, path, true))
}

/// Whether a `MKCOL` answer leaves the folder in place (created now, or already there).
pub fn folder_exists_after(resp: &HttpResponse) -> bool {
    resp.status == 201 || resp.status == 405
}

/// `COPY` a file on the server, never overwriting the destination.
pub fn copy(server: &ServerUrl, from: &str, to: &str) -> HttpRequest {
    copy_under(&server.join("webdav/"), from, to)
}

/// [`copy`] under any WebDAV base.
pub fn copy_under(base: &str, from: &str, to: &str) -> HttpRequest {
    HttpRequest::new(Method::Copy, url_under(base, from, false)).header("Destination", url_under(base, to, false)).header("Overwrite", "F")
}

/// `COPY` a file over another, replacing it (`Overwrite: T`): restoring a version.
pub fn copy_over(server: &ServerUrl, from: &str, to: &str) -> HttpRequest {
    HttpRequest::new(Method::Copy, url(server, from, false)).header("Destination", url(server, to, false)).header("Overwrite", "T")
}

/// `COPY` a file or folder (a folder with everything in it) to a new path, never overwriting
/// (412 when the destination exists).
pub fn copy_to(server: &ServerUrl, from: &str, to: &str, folder: bool) -> HttpRequest {
    HttpRequest::new(Method::Copy, url(server, from, folder))
        .header("Destination", url(server, to, folder))
        .header("Overwrite", "F")
        .header("Depth", "infinity")
}

/// `MOVE` (rename and/or move) a file or folder to a new path, never overwriting (412 when the
/// destination exists).
pub fn move_to(server: &ServerUrl, from: &str, to: &str, folder: bool) -> HttpRequest {
    move_to_under(&server.join("webdav/"), from, to, folder)
}

/// [`move_to`] under any WebDAV base: a `MOVE` that always carries `Overwrite: F`.
pub fn move_to_under(base: &str, from: &str, to: &str, folder: bool) -> HttpRequest {
    HttpRequest::new(Method::Move, url_under(base, from, folder)).header("Destination", url_under(base, to, folder)).header("Overwrite", "F")
}

/// `PROPFIND` depth 0: one file's (or folder's) own properties, e.g. its current ETag.
pub fn stat(server: &ServerUrl, path: &str) -> HttpRequest {
    stat_under(&server.join("webdav/"), path)
}

/// [`stat`] under any WebDAV base.
pub fn stat_under(base: &str, path: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, url_under(base, path, false))
        .header("Depth", "0")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(PROPFIND_BODY.as_bytes().to_vec())
}

/// Where a snapshot of `path` taken at `stamp` goes: `.craftcloud/versions/<path>/<stamp>`.
/// Keyed by path because OxiCloud's WebDAV has no file id; whoever renames or moves a file (or a
/// folder) moves its versions folder along (`versions::move_history`).
/// `stamp` sorts by time, e.g. `20261113T151013Z`.
pub fn version_path(path: &str, stamp: &str) -> String {
    let clean: Vec<&str> = segments(path).collect();
    format!("{CRAFTCLOUD_DIR}/versions/{}/{stamp}", clean.join("/"))
}

/// The folder that holds the snapshots of `path`.
pub fn versions_folder(path: &str) -> String {
    let clean: Vec<&str> = segments(path).collect();
    format!("{CRAFTCLOUD_DIR}/versions/{}", clean.join("/"))
}

/// The folders to `MKCOL`, parent first, before snapshotting `path`.
pub fn version_folders(path: &str) -> Vec<String> {
    let full = versions_folder(path);
    let parts: Vec<&str> = full.split('/').collect();
    (1..=parts.len()).map(|n| parts[..n].join("/")).collect()
}

/// Which snapshots to delete so that `VERSIONS_KEPT` remain, given the names in a versions
/// folder (oldest names sort first).
pub fn versions_to_prune(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    let extra = names.len().saturating_sub(VERSIONS_KEPT);
    names.truncate(extra);
    names
}

/// `DELETE` a file or folder **for good**: it does not go to the trash (the REST
/// `files::trash` and `folders::trash` do). A folder goes with everything in it.
pub fn delete(server: &ServerUrl, path: &str, folder: bool) -> HttpRequest {
    delete_under(&server.join("webdav/"), path, folder)
}

/// [`delete`] under any WebDAV base: for good. The sync sends it only for a path inside
/// `.craftcloud/` (`craftcloud_native::storage::RemoteStorage::purge_own`).
pub fn delete_under(base: &str, path: &str, folder: bool) -> HttpRequest {
    HttpRequest::new(Method::Delete, url_under(base, path, folder))
}

/// CraftCloud's own trash on a server without one (docs/WEBDAV-STORAGE.md § 8), and the local
/// folder's trash under download only (§ 9): `.craftcloud/trash`.
pub const TRASH_DIR: &str = ".craftcloud/trash";

/// The name of one trash entry's folder, `<time>-<device>` (`20261117T143205Z-Laptop`), or
/// `<time>-<device>-<n>` for `n` 2 and on when that name is taken; the trashed item keeps its
/// cloud path below it, so a restore knows where it goes. `/` and `\` in a device name become `-`.
pub fn trash_entry_name(time: &str, device: &str, n: u32) -> String {
    let device: String = device.chars().map(|c| if matches!(c, '/' | '\\') { '-' } else { c }).collect();
    if n <= 1 { format!("{time}-{device}") } else { format!("{time}-{device}-{n}") }
}

/// Whether a path is CraftCloud's own (never shown in file lists, never synced).
pub fn is_craftcloud_internal(path: &str) -> bool {
    segments(path).next() == Some(CRAFTCLOUD_DIR)
}

#[cfg(test)]
mod tests {
    use super::{
        Uploaded, Write, copy, is_craftcloud_internal, list, parse_list, parse_upload, upload, url, version_folders, version_path, versions_to_prune,
    };
    use crate::ServerUrl;
    use crate::api::fixtures::read;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8093").unwrap()
    }

    #[test]
    fn urls_escape_each_segment() {
        assert_eq!(url(&server(), "", true), "http://127.0.0.1:8093/webdav/");
        assert_eq!(url(&server(), "/Documents/", true), "http://127.0.0.1:8093/webdav/Documents/");
        assert_eq!(url(&server(), "Documents/Q3 report #2.docx", false), "http://127.0.0.1:8093/webdav/Documents/Q3%20report%20%232.docx");
        assert_eq!(url(&server(), "Billeder/Æble?.png", false), "http://127.0.0.1:8093/webdav/Billeder/%C3%86ble%3F.png");
    }

    #[test]
    fn lists_a_folder() {
        let req = list(&server(), "Documents");
        assert_eq!((req.method, req.header_value("Depth")), (Method::Propfind, Some("1")));

        let entries = parse_list(&HttpResponse::new(207, read("propfind-documents.xml"))).unwrap();
        let names: Vec<(&str, bool)> = entries.iter().map(|e| (e.path.as_str(), e.is_folder)).collect();
        assert!(names.contains(&("Documents", true)), "{names:?}");
        assert!(names.contains(&("Documents/notes.md", false)), "{names:?}");
        assert!(names.contains(&("Documents/Reports", true)), "{names:?}");
        let notes = entries.iter().find(|e| e.name() == "notes.md").unwrap();
        assert_eq!(notes.size, "version one".len() as u64);
        assert!(notes.etag.as_deref().is_some_and(|e| e.starts_with('"')), "{:?}", notes.etag);

        let root = parse_list(&HttpResponse::new(207, read("propfind-root.xml"))).unwrap();
        assert_eq!(root[0].path, "");
        assert!(root[0].is_folder);
    }

    #[test]
    fn a_listing_must_be_multistatus() {
        assert!(parse_list(&HttpResponse::new(401, b"no".to_vec())).is_err());
        assert!(parse_list(&HttpResponse::new(200, b"<html/>".to_vec())).is_err());
    }

    #[test]
    fn uploads_check_the_version() {
        let create = upload(&server(), "a.md", b"x".to_vec(), &Write::Create);
        assert_eq!(create.header_value("If-None-Match"), Some("*"));
        let replace = upload(&server(), "a.md", b"y".to_vec(), &Write::Replace { etag: "\"v1\"".into() });
        assert_eq!(replace.header_value("If-Match"), Some("\"v1\""));

        assert_eq!(parse_upload(&HttpResponse::new(412, Vec::new())), Ok(Uploaded::Conflict));
        assert_eq!(
            parse_upload(&HttpResponse::new(204, Vec::new()).with_header("ETag", "\"v2\"")),
            Ok(Uploaded::Saved { etag: Some("\"v2\"".into()) })
        );
        assert!(parse_upload(&HttpResponse::new(507, b"quota".to_vec())).is_err());
    }

    #[test]
    fn versions_live_under_craftcloud_by_path() {
        assert_eq!(version_path("/Documents/notes.md", "20261113T151013Z"), ".craftcloud/versions/Documents/notes.md/20261113T151013Z");
        assert_eq!(
            version_folders("Documents/notes.md"),
            [".craftcloud", ".craftcloud/versions", ".craftcloud/versions/Documents", ".craftcloud/versions/Documents/notes.md"]
        );
        let snap = copy(&server(), "Documents/notes.md", &version_path("Documents/notes.md", "1"));
        assert_eq!(snap.header_value("Destination"), Some("http://127.0.0.1:8093/webdav/.craftcloud/versions/Documents/notes.md/1"));
        assert_eq!(snap.header_value("Overwrite"), Some("F"));
        assert!(is_craftcloud_internal(".craftcloud/versions/x"));
        assert!(!is_craftcloud_internal("Documents/.craftcloud"));
    }

    /// The `_under` builders give the same requests for OxiCloud's base as the server ones (the
    /// generic driver runs the same code under its own base).
    #[test]
    fn requests_under_oxiclouds_base_are_the_servers() {
        use super::{copy_under, create_folder_under, delete_under, list_under, move_to, move_to_under, stat, stat_under, upload_under};
        let s = server();
        let base = s.join("webdav/");
        let same = |a: crate::http::HttpRequest, b: crate::http::HttpRequest| {
            assert_eq!((a.method, a.url, a.headers, a.body), (b.method, b.url, b.headers, b.body));
        };
        same(list(&s, "Docs"), list_under(&base, "Docs"));
        same(stat(&s, "Docs/a.txt"), stat_under(&base, "Docs/a.txt"));
        same(upload(&s, "a.md", b"x".to_vec(), &Write::Create), upload_under(&base, "a.md", b"x".to_vec(), &Write::Create));
        same(super::create_folder(&s, "Docs"), create_folder_under(&base, "Docs"));
        same(copy(&s, "a", "b"), copy_under(&base, "a", "b"));
        same(move_to(&s, "a", "b", true), move_to_under(&base, "a", "b", true));
        same(super::delete(&s, ".craftcloud/x", false), delete_under(&base, ".craftcloud/x", false));
        assert_eq!(
            list_under("https://cloud.example.org/remote.php/dav/files/john/", "Docs").url,
            "https://cloud.example.org/remote.php/dav/files/john/Docs/"
        );
    }

    /// The client trash's entry names (docs/WEBDAV-STORAGE.md § 8): the time, the device, and a
    /// count only when the first name is taken.
    #[test]
    fn trash_entries_are_named_by_time_and_device() {
        use super::{TRASH_DIR, trash_entry_name};
        assert_eq!(trash_entry_name("20261117T143205Z", "Laptop", 1), "20261117T143205Z-Laptop");
        assert_eq!(trash_entry_name("20261117T143205Z", "Laptop", 2), "20261117T143205Z-Laptop-2");
        assert_eq!(trash_entry_name("20261117T143205Z", "a/b\\c", 1), "20261117T143205Z-a-b-c");
        assert!(is_craftcloud_internal(TRASH_DIR));
    }

    #[test]
    fn keeps_the_newest_five_versions() {
        let names: Vec<String> = ["20261107", "20261101", "20261113", "20261109", "20261103", "20261111", "20261105"].map(String::from).into();
        assert_eq!(versions_to_prune(names), ["20261101", "20261103"]);
        assert!(versions_to_prune(vec!["a".into(), "b".into()]).is_empty());
    }
}
