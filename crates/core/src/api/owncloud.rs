//! ownCloud 10's own endpoints, as pure requests and parsers (docs/WEBDAV-STORAGE.md § 7.4, the
//! ownCloud column; § 10, an app password). A profile knows where things are, never what they
//! guarantee: the check on connect still measures every guard, and nothing here grants a capability.
//!
//! Learnt from ownCloud's public documentation (the trash bin and versions WebDAV endpoints) and
//! from an ownCloud 10.15 answering, recorded in `crates/core/tests/fixtures/owncloud-10.15/`:
//!
//! - **Detection** is `GET /status.php` (`productname` "ownCloud"), the same answer Nextcloud
//!   gives, so [`super::nextcloud::parse_status`] reads it. It only suggests a kind of server.
//! - **Sign-in** is an app password the person makes in Settings › Security ("App passwords /
//!   tokens") and pastes in; ownCloud 10 has no browser flow a third-party client can use without
//!   an admin (OAuth2 is an app an admin installs). Signing out forgets the password; the person
//!   revokes it on the server.
//! - **The user id** can differ from the login name: `current-user-principal` on `/remote.php/dav/`
//!   names it ([`super::nextcloud::current_user_principal`]), and the WebDAV base is
//!   `/remote.php/dav/files/<user id>/`, the same shape as Nextcloud's.
//! - **The trash** is `/remote.php/dav/trash-bin/<user id>/` (not Nextcloud's `trashbin/…/trash/`,
//!   which answers 404), every item named by the **file id** it had (a folder with a trailing
//!   slash), with `oc:trashbin-original-filename`, `oc:trashbin-original-location` (the path under
//!   the files base) and `oc:trashbin-delete-timestamp`. A `GET` of an item answers 403: only
//!   listings go there. A restore is a `MOVE` of the item to a URL **under the files base**, and the
//!   server takes any destination there (measured: a move to another name answers 201), so the
//!   client sends the original location, with `Overwrite: F` (honoured: 412 onto a taken name, the
//!   file untouched; without the header the server overwrites). A missing parent answers 409.
//! - **Versions** are `/remote.php/dav/meta/<file id>/v/<seconds>`, by id, so they follow renames;
//!   each is named by the modified time (in seconds) of the file it was, and the current file is
//!   **not** among them. Restoring one is a `COPY` of the version to the file's own URL; the server
//!   keeps the file it replaces as a version itself. `Overwrite: F` answers 412 there, so the copy
//!   says `Overwrite: T`, as the generic driver's own version restore does.
//! - **Quota** is WebDAV's `quota-available-bytes` on the base; `-3` means no limit.
//! - **Checksums**: the server computes `oc:checksums` (SHA1, MD5, ADLER32) for every file and
//!   lists them always; an `OC-Checksum` sent with a `PUT` is checked against the bytes received
//!   (a mismatch answers 400, nothing written).
//! - **Same second**: a file overwritten within the same second as the write before it keeps its
//!   ETag, and the version slot named by that second keeps only the later bytes (docs/UPSTREAM.md,
//!   the ownCloud entry; § 16, Q13 closes the ETag window with the checksum check).

use crate::http::{HttpRequest, HttpResponse, Method};
use crate::storage::probe::Base;

pub use super::nextcloud::{
    Detected, Trashed, current_user_principal, files_base, parse_current_user_principal, parse_status, split_files_base, status,
};

fn under(origin: &str, path: &str) -> String {
    format!("{}/{}", origin.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// The trash bin's root: `<origin>/remote.php/dav/trash-bin/<user>/` (items right under it, named
/// by file id).
pub fn trash_root(origin: &str, user: &str) -> String {
    crate::api::dav::url_under(&under(origin, "remote.php/dav/trash-bin/"), user, true)
}

/// The versions root of one file: `<origin>/remote.php/dav/meta/<file id>/` (the versions under
/// `v/`, one per earlier version, named by its time in seconds).
pub fn meta_root(origin: &str, file_id: &str) -> String {
    crate::api::dav::url_under(&under(origin, "remote.php/dav/meta/"), file_id, true)
}

/// What a trash listing asks for.
pub const TRASH_PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><oc:trashbin-original-filename/><oc:trashbin-original-location/><oc:trashbin-delete-timestamp/></d:prop></d:propfind>"#;

/// `PROPFIND` of the trash bin, one level.
pub fn trash_list(trash_root: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, crate::api::dav::url_under(trash_root, "", true))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(TRASH_PROPFIND_BODY.as_bytes().to_vec())
}

/// Reads a trash listing against `root` (the trash bin's root, [`trash_root`]): one [`Trashed`]
/// per item, its `name` the item's id. An href outside the root, a status other than 207 or an
/// unreadable body is an error, in words.
pub fn parse_trash(resp: &HttpResponse, root: &Base) -> Result<Vec<Trashed>, String> {
    if resp.status != 207 {
        return Err(format!("the trash listing answered {} instead of 207", resp.status));
    }
    let doc = craft_dav::parse_multistatus(&resp.body).map_err(|e| format!("the trash listing couldn't be read: {e}"))?;
    let mut out = Vec::new();
    for response in craft_dav::responses(&doc) {
        let href = craft_dav::href(response).ok_or_else(|| "a trash entry without a path".to_owned())?;
        let rel = root.relative(&href).ok_or_else(|| format!("the trash listing named a path outside the trash bin: {href}"))?;
        // The root itself, and nothing below an item (a listing of one level has none).
        if rel.is_empty() || rel.contains('/') {
            continue;
        }
        let mut t = Trashed {
            name: rel,
            is_folder: false,
            file_name: String::new(),
            original_location: String::new(),
            deleted_at: None,
            file_id: None,
            size: 0,
        };
        for prop in craft_dav::properties(response) {
            let text = prop.text().trim().to_owned();
            match prop.name.as_str() {
                "d:resourcetype" => t.is_folder = prop.child("d:collection").is_some(),
                "d:getcontentlength" => t.size = text.parse().unwrap_or(0),
                "oc:trashbin-original-filename" => t.file_name = text,
                "oc:trashbin-original-location" => t.original_location = text.trim_matches('/').to_owned(),
                "oc:trashbin-delete-timestamp" => t.deleted_at = text.parse().ok(),
                _ => {}
            }
        }
        // The item is named by the id the file had.
        t.file_id = Some(t.name.clone());
        if t.file_name.is_empty() {
            t.file_name = t.original_location.rsplit('/').next().unwrap_or("").to_owned();
        }
        out.push(t);
    }
    Ok(out)
}

/// `MOVE` a trashed item back: `<trash root>/<id>` to `path` under `files_base`, never onto an
/// existing name (`Overwrite: F`, which ownCloud honours with a 412; the server would take any
/// destination under the files base, so the caller passes the item's original location).
pub fn trash_restore(trash_root: &str, id: &str, is_folder: bool, files_base: &str, path: &str) -> HttpRequest {
    HttpRequest::new(Method::Move, crate::api::dav::url_under(trash_root, id, is_folder))
        .header("Destination", crate::api::dav::url_under(files_base, path, is_folder))
        .header("Overwrite", "F")
}

/// `PROPFIND` of a file's versions, one level (`<meta root>/v/`).
pub fn versions_list(meta_root: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, crate::api::dav::url_under(meta_root, "v", true))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(crate::storage::probe::PROPFIND_BODY.as_bytes().to_vec())
}

/// The earlier versions of a file from its listing against `root` ([`meta_root`]), newest first,
/// as [`super::versions::Version`]s whose stamp is the version's name (its time in seconds). Only
/// entries named by digits count: the `v/` collection itself, and anything else, is left out. A 404
/// is a file without versions.
pub fn parse_versions(resp: &HttpResponse, root: &Base) -> Result<Vec<super::versions::Version>, String> {
    if resp.status == 404 {
        return Ok(Vec::new());
    }
    let listed = crate::storage::probe::parse_listing(resp, root).map_err(|e| format!("the versions listing: {e}"))?;
    let mut out: Vec<super::versions::Version> = listed
        .into_iter()
        .filter(|e| !e.is_folder && !e.name().is_empty() && e.name().bytes().all(|b| b.is_ascii_digit()))
        .map(|e| super::versions::Version { stamp: e.name().to_owned(), size: e.size, etag: e.etag })
        .collect();
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    Ok(out)
}

/// `COPY` version `stamp` (under `meta_root`) over the current file at `path` under `files_base`.
/// `Overwrite: T`: the point is to replace the file, and the server keeps the file it replaces as
/// a version of its own (measured on 10.15; `Overwrite: F` answers 412 there).
pub fn version_restore(meta_root: &str, stamp: &str, files_base: &str, path: &str) -> HttpRequest {
    HttpRequest::new(Method::Copy, crate::api::dav::url_under(meta_root, &format!("v/{stamp}"), false))
        .header("Destination", crate::api::dav::url_under(files_base, path, false))
        .header("Overwrite", "T")
}

/// The free bytes from WebDAV's `quota-available-bytes` as ownCloud lists it: a count, `u64::MAX`
/// for no limit (`-3`), `None` when the server doesn't say (any other negative, or none).
pub fn quota_free(quota_available: Option<i64>) -> Option<u64> {
    match quota_available {
        Some(n) if n >= 0 => Some(n.unsigned_abs()),
        Some(-3) => Some(u64::MAX),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::read_from;
    use super::*;
    use crate::storage::probe::parse_listing;

    const FIXTURES: &str = "owncloud-10.15";
    const ORIGIN: &str = "https://cloud.example.org";

    fn fixture(name: &str) -> Vec<u8> {
        read_from(FIXTURES, name)
    }

    #[test]
    fn status_php_says_owncloud_and_its_version() {
        let d = parse_status(&HttpResponse::new(200, fixture("status.json"))).unwrap();
        assert_eq!(d, Detected { product: "ownCloud".into(), version_string: "10.15.3".into(), version: "10.15.3.0".into() });
        assert!(d.is_owncloud() && !d.is_nextcloud());
        assert_eq!(d.words(), "ownCloud 10.15.3");
        let nc = parse_status(&HttpResponse::new(200, read_from("nextcloud-31", "status.json"))).unwrap();
        assert!(nc.is_nextcloud() && !nc.is_owncloud());
    }

    #[test]
    fn the_user_id_and_the_bases_are_nextclouds_shapes() {
        assert_eq!(parse_current_user_principal(&HttpResponse::new(207, fixture("current-user-principal.xml"))).unwrap(), "john");
        assert_eq!(files_base(ORIGIN, "john"), "https://cloud.example.org/remote.php/dav/files/john/");
        assert_eq!(trash_root(ORIGIN, "john"), "https://cloud.example.org/remote.php/dav/trash-bin/john/");
        assert_eq!(trash_root("https://example.org/cloud/", "jane doe"), "https://example.org/cloud/remote.php/dav/trash-bin/jane%20doe/");
        assert_eq!(meta_root(ORIGIN, "1660"), "https://cloud.example.org/remote.php/dav/meta/1660/");
    }

    #[test]
    fn the_quota_is_the_webdav_property_with_minus_three_as_no_limit() {
        let base = Base::parse(&files_base(ORIGIN, "john")).unwrap();
        let listed = parse_listing(&HttpResponse::new(207, fixture("base-quota.xml")), &base).unwrap();
        assert_eq!(listed[0].quota_available, Some(-3), "the recorded account has no limit");
        assert_eq!(quota_free(listed[0].quota_available), Some(u64::MAX));
        assert_eq!(quota_free(Some(5_000)), Some(5_000));
        assert_eq!(quota_free(Some(0)), Some(0));
        assert_eq!(quota_free(Some(-1)), None);
        assert_eq!(quota_free(None), None);
    }

    /// ownCloud computes and lists checksums for every file, also one written without
    /// `OC-Checksum` (`b.txt` in the recording); a wrong `OC-Checksum` is refused (400).
    #[test]
    fn checksums_are_listed_for_every_file() {
        let base = Base::parse(&files_base(ORIGIN, "john")).unwrap();
        let listed = parse_listing(&HttpResponse::new(207, fixture("folder-listing.xml")), &base).unwrap();
        let files: Vec<&crate::storage::probe::Listed> = listed.iter().filter(|e| !e.is_folder).collect();
        assert_eq!(files.len(), 2, "{listed:#?}");
        for f in &files {
            assert!(f.file_id.is_some(), "{f:?}");
            assert!(f.checksums.iter().any(|c| c.contains("SHA1:") && c.contains("MD5:") && c.contains("ADLER32:")), "{f:?}");
        }
        // `printf one | sha1sum`, sent with the PUT and listed back as sent.
        assert!(files.iter().any(|f| f.name() == "a.txt" && f.checksums[0].starts_with("SHA1:fe05bcdcdc4928012781a5f1a2a77cbb5398e106")));
        let refused = HttpResponse::new(400, fixture("checksum-mismatch.xml"));
        assert!(String::from_utf8_lossy(&refused.body).contains("checksum does not match"));
        assert_eq!(HttpResponse::new(412, fixture("depth-infinity.xml")).status, 412, "Depth: infinity is refused with 412");
    }

    #[test]
    fn the_trash_bin_lists_items_by_file_id_with_where_they_were() {
        let root = Base::parse(&trash_root(ORIGIN, "john")).unwrap();
        let req = trash_list(root.url());
        assert_eq!((req.url.as_str(), req.header_value("Depth")), ("https://cloud.example.org/remote.php/dav/trash-bin/john/", Some("1")));
        let items = parse_trash(&HttpResponse::new(207, fixture("trash-bin.xml")), &root).unwrap();
        assert_eq!(items.len(), 2, "the root itself is left out: {items:#?}");
        let file = items.iter().find(|t| !t.is_folder).unwrap();
        assert_eq!((file.name.as_str(), file.file_name.as_str(), file.size), ("1668", "a.txt", 5));
        assert!(file.original_location.ends_with("/a.txt") && !file.original_location.starts_with('/'), "{}", file.original_location);
        assert_eq!(file.deleted_at, Some(1_795_006_711));
        assert_eq!(file.file_id.as_deref(), Some("1668"));
        let folder = items.iter().find(|t| t.is_folder).unwrap();
        assert_eq!((folder.name.as_str(), folder.file_name.as_str()), ("1669", "sub"));
        assert!(folder.original_location.ends_with("/sub"));
        // A listing from another root, or not a listing, is an error.
        assert!(parse_trash(&HttpResponse::new(207, fixture("trash-bin.xml")), &Base::parse(&trash_root(ORIGIN, "jane")).unwrap()).is_err());
        assert!(parse_trash(&HttpResponse::new(500, Vec::new()), &root).is_err());
        // The restore: a MOVE to the original location under the files base, never onto a taken name.
        let files = files_base(ORIGIN, "john");
        let restore = trash_restore(root.url(), &file.name, false, &files, &file.original_location);
        assert_eq!(restore.method, Method::Move);
        assert_eq!(restore.url, "https://cloud.example.org/remote.php/dav/trash-bin/john/1668");
        assert_eq!(restore.header_value("Destination"), Some(format!("{files}{}", file.original_location).as_str()));
        assert_eq!(restore.header_value("Overwrite"), Some("F"));
        let restore = trash_restore(root.url(), &folder.name, true, &files, "Docs/Sub");
        assert_eq!(restore.url, "https://cloud.example.org/remote.php/dav/trash-bin/john/1669/");
        assert_eq!(restore.header_value("Destination"), Some("https://cloud.example.org/remote.php/dav/files/john/Docs/Sub/"));
    }

    #[test]
    fn versions_are_listed_by_file_id_without_the_current_one() {
        let root = Base::parse(&meta_root(ORIGIN, "1660")).unwrap();
        let req = versions_list(root.url());
        assert_eq!((req.url.as_str(), req.header_value("Depth")), ("https://cloud.example.org/remote.php/dav/meta/1660/v/", Some("1")));
        // Recorded: a file written, then overwritten three times, two of them within one second:
        // two versions (the slot of the shared second holds only the later bytes), the current
        // file not among them.
        let versions = parse_versions(&HttpResponse::new(207, fixture("versions.xml")), &root).unwrap();
        assert_eq!(versions.len(), 2, "{versions:#?}");
        assert_eq!((versions[0].stamp.as_str(), versions[1].stamp.as_str()), ("1795006696", "1795006694"), "newest first");
        assert_eq!((versions[0].size, versions[1].size), (5, 3));
        assert!(versions.iter().all(|v| v.etag.is_some()));
        assert!(parse_versions(&HttpResponse::new(404, Vec::new()), &root).unwrap().is_empty(), "no versions yet");
        assert!(parse_versions(&HttpResponse::new(500, Vec::new()), &root).is_err());
        // The collection itself, listed as a file by a confused server, never counts as a version.
        let odd = br#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/remote.php/dav/meta/1660/v/</d:href><d:propstat><d:prop><d:resourcetype/><d:getetag>"x"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        assert!(parse_versions(&HttpResponse::new(207, odd.to_vec()), &root).unwrap().is_empty());
        let restore = version_restore(root.url(), &versions[1].stamp, &files_base(ORIGIN, "john"), "Docs/a.txt");
        assert_eq!(restore.method, Method::Copy);
        assert_eq!(restore.url, "https://cloud.example.org/remote.php/dav/meta/1660/v/1795006694");
        assert_eq!(restore.header_value("Destination"), Some("https://cloud.example.org/remote.php/dav/files/john/Docs/a.txt"));
        assert_eq!(restore.header_value("Overwrite"), Some("T"));
    }
}
