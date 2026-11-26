//! OpenCloud's own APIs, as pure requests and parsers (docs/WEBDAV-STORAGE.md § 7.4, the OpenCloud
//! column; § 10; § 16, Q1). A profile knows where things are, never what they guarantee: the check
//! on connect still measures every guard, nothing here grants a capability, and OpenCloud's mode is
//! **download only** whatever it measures (its guards don't hold under a race, Q1).
//!
//! Learnt from OpenCloud's public documentation (the `auth-app`, `proxy` and `idp` services, the
//! Graph drives API) and from OpenCloud 9.1.1 answering, recorded in
//! `crates/core/tests/fixtures/opencloud-9.1.1/`:
//!
//! - **Detection** is `GET /status.php`: `productname` is `OpenCloud`, and the version a person
//!   knows is `productversion` (`9.1.1`); its `version` and `versionstring` say `0.1.0`, so they
//!   aren't used. It only suggests a kind of server.
//! - **Sign-in** is an **app token** the person makes in the web UI (Account › App Tokens › New),
//!   used as the password of HTTP Basic auth with the login name. The `auth-app` service that makes
//!   them is on by default and so is the proxy's app authentication (`PROXY_ENABLE_APP_AUTH`,
//!   default `true`), independent of `PROXY_ENABLE_BASIC_AUTH` (default `false`): measured on
//!   9.1.1 with basic auth off, the token is taken (207, 200) and the account password refused
//!   (401). The API `POST /auth-app/tokens?expiry=…` makes one too, but needs a signed-in request
//!   (the password only where an admin turned basic auth on), so the client never calls it; the
//!   listing ([`app_tokens`]) names the tokens without their secrets, so a sign-out can't revoke
//!   ours: the person does, on the same page. OpenID Connect is OpenCloud's own clients' sign-in:
//!   the identity provider registers only their client ids, has no dynamic registration, and a
//!   new client is an admin's edit of `opencloud.yaml`, so no browser flow is built (§ 10).
//! - **The personal space** is the Graph API's `GET /graph/v1.0/me/drives`, the drive with
//!   `driveType` `personal`; the WebDAV base is `/remote.php/dav/spaces/<drive id>/`, built from
//!   the server's address the client uses (the answer's `webDavUrl` carries the server's public
//!   URL, which may differ). `GET /graph/v1.0/me` names the signed-in user
//!   (`onPremisesSamAccountName` is the login name). The drive id is `<storage id>$<space id>`.
//! - **The trash** is `/remote.php/dav/spaces/trash-bin/<drive id>/`, one item per key, with
//!   `oc:trashbin-original-filename`, `oc:trashbin-original-location` and
//!   `oc:trashbin-delete-timestamp`; a restore is a `MOVE` of the item to its original path under
//!   the space's own URL, **with `Overwrite: F`** (measured: with it a taken name answers 412;
//!   without it the server overwrites the newer file). A restore to another name answers 500.
//! - **Versions** are `/remote.php/dav/meta/<file id>/v/`, one entry per earlier version named
//!   `<node id>.REV.<RFC 3339 time>` (the current file is not among them); a version's bytes are
//!   read with a `GET` ([`versions_download`]) and written back with the driver's guarded `PUT`.
//!   The server's own restore (a `COPY` of the version over the file) carries no precondition on
//!   the file it replaces, so CraftCloud never sends it.
//! - **Quota** is WebDAV's `quota-available-bytes` (`-2` or `-3` for no limit: unknown).
//! - `Depth: infinity` answers 400; `oc:fileid` is `<storage id>$<space id>!<node id>`, the same
//!   under the files and the spaces URL; `oc:checksums` lists SHA1, MD5 and Adler-32.

use serde::Deserialize;

use super::ApiError;
use super::nextcloud::{Detected, Trashed};
use crate::http::{HttpRequest, HttpResponse, Method};
use crate::storage::probe::Base;

fn under(origin: &str, path: &str) -> String {
    format!("{}/{}", origin.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// `GET /status.php`: no sign-in needed (the same request Nextcloud answers).
pub fn status(origin: &str) -> HttpRequest {
    super::nextcloud::status(origin)
}

/// What `status.php` answered when the server is OpenCloud: the product and `productversion`
/// (`9.1.1`, what a person knows and what an update changes). `None` for any other server, any
/// status other than 200, or a body that isn't its JSON.
pub fn parse_status(resp: &HttpResponse) -> Option<Detected> {
    #[derive(Deserialize)]
    struct Status {
        #[serde(default)]
        productname: String,
        #[serde(default)]
        productversion: String,
    }
    if resp.status != 200 {
        return None;
    }
    let s: Status = serde_json::from_slice(&resp.body).ok()?;
    if !s.productname.trim().eq_ignore_ascii_case("opencloud") {
        return None;
    }
    let version = s.productversion.trim().to_owned();
    Some(Detected { product: "OpenCloud".into(), version_string: version.clone(), version })
}

/// `GET /graph/v1.0/me`: who is signed in.
pub fn me(origin: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, under(origin, "graph/v1.0/me"))
}

/// The signed-in user, from the Graph API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Me {
    /// The user's id (a UUID).
    pub id: String,
    /// The login name (`onPremisesSamAccountName`), the one Basic auth takes.
    pub user_name: String,
    pub display_name: String,
}

pub fn parse_me(resp: &HttpResponse) -> Result<Me, ApiError> {
    #[derive(Deserialize)]
    struct Graph {
        #[serde(default)]
        id: String,
        #[serde(rename = "onPremisesSamAccountName", default)]
        user_name: String,
        #[serde(rename = "displayName", default)]
        display_name: String,
    }
    let g: Graph = super::json(resp)?;
    if g.id.is_empty() || g.user_name.is_empty() {
        return Err(ApiError::Body("the Graph answer doesn't name the signed-in user".into()));
    }
    Ok(Me { id: g.id, user_name: g.user_name, display_name: g.display_name })
}

/// `GET /graph/v1.0/me/drives`: the spaces the user can reach.
pub fn drives(origin: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, under(origin, "graph/v1.0/me/drives"))
}

/// The user's personal space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalDrive {
    /// `<storage id>$<space id>`, the segment of its WebDAV URL.
    pub id: String,
    pub name: String,
    /// `quota.remaining`, where the answer says (`i64::MAX` for no limit).
    pub quota_remaining: Option<u64>,
}

/// The personal drive among the drives listed; an error when there is none.
pub fn parse_personal_drive(resp: &HttpResponse) -> Result<PersonalDrive, ApiError> {
    #[derive(Deserialize)]
    struct Quota {
        remaining: Option<i64>,
    }
    #[derive(Deserialize)]
    struct Drive {
        #[serde(default)]
        id: String,
        #[serde(rename = "driveType", default)]
        drive_type: String,
        #[serde(default)]
        name: String,
        quota: Option<Quota>,
    }
    #[derive(Deserialize)]
    struct Drives {
        #[serde(default)]
        value: Vec<Drive>,
    }
    let d: Drives = super::json(resp)?;
    let personal = d
        .value
        .into_iter()
        .find(|d| d.drive_type == "personal" && !d.id.is_empty())
        .ok_or_else(|| ApiError::Body("the Graph answer lists no personal space".into()))?;
    let quota_remaining = personal.quota.and_then(|q| q.remaining).filter(|n| *n >= 0).map(|n| n.unsigned_abs());
    Ok(PersonalDrive { id: personal.id, name: personal.name, quota_remaining })
}

/// `GET /auth-app/tokens`: the user's app tokens, without their secrets.
pub fn app_tokens(origin: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, under(origin, "auth-app/tokens"))
}

/// One app token as listed: its label and when it ends. The secret isn't listed (`token` holds
/// one at creation only), so none is ever read here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppToken {
    pub label: String,
    pub expires_at: String,
}

pub fn parse_app_tokens(resp: &HttpResponse) -> Result<Vec<AppToken>, ApiError> {
    #[derive(Deserialize)]
    struct Listed {
        #[serde(default)]
        label: String,
        #[serde(default)]
        expiration_date: String,
    }
    let list: Vec<Listed> = super::json(resp)?;
    Ok(list.into_iter().map(|t| AppToken { label: t.label, expires_at: t.expiration_date }).collect())
}

/// The WebDAV base of a space: `<origin>/remote.php/dav/spaces/<drive id>/`.
pub fn spaces_base(origin: &str, drive_id: &str) -> String {
    crate::api::dav::url_under(&under(origin, "remote.php/dav/spaces/"), drive_id, true)
}

/// The origin and the drive id of an OpenCloud spaces base
/// (`https://cloud.example.org/remote.php/dav/spaces/<storage>$<space>/`); `None` for a base that
/// isn't one (the trash bin's URL included).
pub fn split_spaces_base(base: &Base) -> Option<(String, String)> {
    let path = percent_encoding::percent_decode_str(base.path()).decode_utf8().ok()?;
    let (prefix, drive) = path.trim_end_matches('/').rsplit_once('/')?;
    let before = prefix.strip_suffix("/remote.php/dav/spaces")?;
    if drive.is_empty() || drive.contains('/') || drive == "trash-bin" || !drive.contains('$') {
        return None;
    }
    Some((format!("{}{before}", base.origin()), drive.to_owned()))
}

/// The space's trash bin: `<origin>/remote.php/dav/spaces/trash-bin/<drive id>/`.
pub fn trash_root(origin: &str, drive_id: &str) -> String {
    crate::api::dav::url_under(&under(origin, "remote.php/dav/spaces/trash-bin/"), drive_id, true)
}

/// The versions' root: `<origin>/remote.php/dav/meta/` (a file's versions under `<file id>/v/`).
pub fn meta_root(origin: &str) -> String {
    under(origin, "remote.php/dav/meta/")
}

/// What a trash listing asks for.
pub const TRASH_PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:prop><d:resourcetype/><d:getcontentlength/><oc:trashbin-original-filename/><oc:trashbin-original-location/><oc:trashbin-delete-timestamp/><oc:trashbin-delete-datetime/></d:prop></d:propfind>"#;

/// `PROPFIND` of the space's trash bin, one level.
pub fn trash_list(trash_root: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, trash_root.to_owned())
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(TRASH_PROPFIND_BODY.as_bytes().to_vec())
}

/// Reads a trash listing against `root` ([`trash_root`]): one item per key, with where it was.
/// An href outside the root, a status other than 207 or an unreadable body is an error, in words.
pub fn parse_trash(resp: &HttpResponse, root: &Base) -> Result<Vec<Trashed>, String> {
    if resp.status != 207 {
        return Err(format!("the trash listing answered {} instead of 207", resp.status));
    }
    let doc = craft_dav::parse_multistatus(&resp.body).map_err(|e| format!("the trash listing couldn't be read: {e}"))?;
    let mut out = Vec::new();
    for response in craft_dav::responses(&doc) {
        let href = craft_dav::href(response).ok_or_else(|| "a trash entry without a path".to_owned())?;
        let key = root.relative(&href).ok_or_else(|| format!("the trash listing named a path outside the trash bin: {href}"))?;
        if key.is_empty() || key.contains('/') {
            continue;
        }
        let mut t = Trashed {
            name: key,
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
                "oc:trashbin-delete-datetime" if t.deleted_at.is_none() => t.deleted_at = crate::storage::probe::server_seconds(&text),
                _ => {}
            }
        }
        if t.file_name.is_empty() {
            t.file_name = t.original_location.rsplit('/').next().unwrap_or("").to_owned();
        }
        out.push(t);
    }
    Ok(out)
}

/// `MOVE` a trashed item back where it was: from `<trash root>/<key>` to its original location
/// under the space's own URL (`spaces_base`), with `Overwrite: F`. The destination is a real
/// name in the person's tree, so the guard matters: measured on 9.1.1, a taken name answers 412
/// with it and is overwritten without it.
pub fn trash_restore(trash_root: &str, key: &str, spaces_base: &str, original_location: &str, is_folder: bool) -> HttpRequest {
    HttpRequest::new(Method::Move, crate::api::dav::url_under(trash_root, key, is_folder))
        .header("Destination", crate::api::dav::url_under(spaces_base, original_location, is_folder))
        .header("Overwrite", "F")
}

/// `PROPFIND` of a file's versions, one level.
pub fn versions_list(meta_root: &str, file_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, crate::api::dav::url_under(meta_root, &format!("{file_id}/v"), true))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(crate::storage::probe::PROPFIND_BODY.as_bytes().to_vec())
}

/// The earlier versions of a file from its listing against `root` ([`meta_root`]), newest first,
/// each stamped with its name (`<node id>.REV.<RFC 3339 time>`, which [`super::versions::Version`]
/// shows as a time). The listing doesn't name the current file. A 404 is a file without versions.
pub fn parse_versions(resp: &HttpResponse, root: &Base) -> Result<Vec<super::versions::Version>, String> {
    if resp.status == 404 {
        return Ok(Vec::new());
    }
    let listed = crate::storage::probe::parse_listing(resp, root).map_err(|e| format!("the versions listing: {e}"))?;
    let mut out: Vec<super::versions::Version> = listed
        .into_iter()
        .filter(|e| !e.is_folder && e.name().contains(".REV."))
        .map(|e| super::versions::Version { stamp: e.name().to_owned(), size: e.size, etag: e.etag })
        .collect();
    out.sort_by(|a, b| rev_time(&b.stamp).cmp(rev_time(&a.stamp)));
    Ok(out)
}

/// The time part of a version's name (`<node id>.REV.<time>` → `<time>`), which sorts as text.
pub fn rev_time(stamp: &str) -> &str {
    stamp.rsplit_once(".REV.").map_or(stamp, |(_, t)| t)
}

/// `GET` the bytes of version `stamp` of file `file_id`. A restore writes them back over the file
/// with the driver's guarded `PUT` (`If-Match`), never with the server's own restore (a `COPY` of
/// the version over the file, which carries no precondition on the file it replaces).
pub fn versions_download(meta_root: &str, file_id: &str, stamp: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, crate::api::dav::url_under(meta_root, &format!("{file_id}/v/{stamp}"), false))
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::read_from;
    use super::*;

    const FIXTURES: &str = "opencloud-9.1.1";
    const ORIGIN: &str = "https://cloud.example.org";
    const DRIVE: &str = "5d90215b-14f0-4b05-9f8d-8971b9810172$4ed81edc-3290-4003-a070-9f24bb826ff8";

    fn fixture(name: &str) -> Vec<u8> {
        read_from(FIXTURES, name)
    }

    #[test]
    fn status_php_says_opencloud_and_its_product_version() {
        let d = parse_status(&HttpResponse::new(200, fixture("status.json"))).unwrap();
        assert_eq!(
            d,
            Detected { product: "OpenCloud".into(), version_string: "9.1.1".into(), version: "9.1.1".into() },
            "productversion, not version (0.1.0.0)"
        );
        assert_eq!(d.words(), "OpenCloud 9.1.1");
        assert!(!d.is_nextcloud());
        // Nextcloud's status.php isn't OpenCloud, nor is anything else.
        assert_eq!(parse_status(&HttpResponse::new(200, read_from("nextcloud-31", "status.json"))), None);
        assert_eq!(parse_status(&HttpResponse::new(404, Vec::new())), None);
        assert_eq!(parse_status(&HttpResponse::new(200, b"<html/>".to_vec())), None);
        assert_eq!(status(ORIGIN).url, "https://cloud.example.org/status.php");
    }

    #[test]
    fn the_graph_api_names_the_user_and_the_personal_space() {
        assert_eq!((me(ORIGIN).method, me(ORIGIN).url.as_str()), (Method::Get, "https://cloud.example.org/graph/v1.0/me"));
        let who = parse_me(&HttpResponse::new(200, fixture("graph-me.json"))).unwrap();
        assert_eq!((who.user_name.as_str(), who.display_name.as_str()), ("admin", "Admin"));
        assert_eq!(who.id, "c2c3fbb3-c7d4-4be3-872c-6b7eeb431f42");
        assert!(parse_me(&HttpResponse::new(200, br#"{"displayName":"x"}"#.to_vec())).is_err());
        assert!(parse_me(&HttpResponse::new(401, Vec::new())).is_err());
        assert_eq!(drives(ORIGIN).url, "https://cloud.example.org/graph/v1.0/me/drives");
        let drive = parse_personal_drive(&HttpResponse::new(200, fixture("graph-me-drives.json"))).unwrap();
        assert_eq!((drive.id.as_str(), drive.name.as_str()), (DRIVE, "Admin"), "the personal drive, not the Shares one");
        assert_eq!(drive.quota_remaining, Some(i64::MAX.unsigned_abs()), "no limit");
        assert!(parse_personal_drive(&HttpResponse::new(200, br#"{"value":[{"id":"x$y","driveType":"virtual"}]}"#.to_vec())).is_err());
        assert_eq!(spaces_base(ORIGIN, DRIVE), format!("https://cloud.example.org/remote.php/dav/spaces/{DRIVE}/"));
        assert_eq!(split_spaces_base(&Base::parse(&spaces_base(ORIGIN, DRIVE)).unwrap()), Some((ORIGIN.into(), DRIVE.into())));
        assert_eq!(
            split_spaces_base(&Base::parse(&format!("https://example.org/cloud/remote.php/dav/spaces/{DRIVE}")).unwrap()),
            Some(("https://example.org/cloud".into(), DRIVE.into()))
        );
        for not_a_space in [
            "https://cloud.example.org/remote.php/dav/files/john/",
            "https://cloud.example.org/remote.php/dav/spaces/",
            &trash_root(ORIGIN, DRIVE),
            "https://cloud.example.org/remote.php/dav/spaces/not-a-drive-id/",
        ] {
            assert_eq!(split_spaces_base(&Base::parse(not_a_space).unwrap()), None, "{not_a_space}");
        }
        assert_eq!(trash_root(ORIGIN, DRIVE), format!("https://cloud.example.org/remote.php/dav/spaces/trash-bin/{DRIVE}/"));
        assert_eq!(meta_root(ORIGIN), "https://cloud.example.org/remote.php/dav/meta/");
        // The space's hrefs, with their literal `$`, read against the base.
        let base = Base::parse(&spaces_base(ORIGIN, DRIVE)).unwrap();
        let root = crate::storage::probe::parse_listing(&HttpResponse::new(207, fixture("space-root.xml")), &base).unwrap();
        assert_eq!(root.len(), 1);
        assert_eq!((root[0].path.as_str(), root[0].is_folder, root[0].quota_available), ("", true, Some(-2)), "no limit reads as unknown");
        assert_eq!(root[0].file_id.as_deref(), Some(format!("{DRIVE}!4ed81edc-3290-4003-a070-9f24bb826ff8").as_str()));
    }

    #[test]
    fn the_app_tokens_are_listed_without_their_secrets() {
        assert_eq!(app_tokens(ORIGIN).url, "https://cloud.example.org/auth-app/tokens");
        let tokens = parse_app_tokens(&HttpResponse::new(200, fixture("auth-app-tokens.json"))).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!((tokens[0].label.as_str(), tokens[0].expires_at.as_str()), ("Generated via API", "2026-11-21T13:00:35.518363424Z"));
        assert!(!format!("{tokens:?}").contains("fixture-app-token"), "the secret is never read");
        assert!(parse_app_tokens(&HttpResponse::new(401, Vec::new())).is_err());
    }

    #[test]
    fn a_folder_listing_has_ids_and_checksums() {
        let base = Base::parse(&spaces_base(ORIGIN, DRIVE)).unwrap();
        let listed = crate::storage::probe::parse_listing(&HttpResponse::new(207, fixture("folder-listing.xml")), &base).unwrap();
        let paths: Vec<&str> = listed.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["Docs", "Docs/Sub", "Docs/b.txt", "Docs/a.txt"]);
        let a = listed.iter().find(|e| e.path == "Docs/a.txt").unwrap();
        assert_eq!(a.file_id.as_deref(), Some(format!("{DRIVE}!7f42db93-cff4-46ac-9f87-9ec04f76d22c").as_str()));
        assert!(a.checksums.iter().any(|c| c.starts_with("SHA1:") && c.contains(" MD5:") && c.contains(" ADLER32:")), "{:?}", a.checksums);
        assert_eq!((a.size, a.etag.as_deref()), (4, Some("\"59b93c2f099802c82ee4320b76ed406d\"")));
        // Depth: infinity answers 400, which fails a listing.
        assert!(crate::storage::probe::parse_listing(&HttpResponse::new(400, fixture("depth-infinity.xml")), &base).is_err());
    }

    #[test]
    fn the_trash_lists_items_by_key_with_where_they_were_and_restores_by_a_guarded_move() {
        let root = Base::parse(&trash_root(ORIGIN, DRIVE)).unwrap();
        let req = trash_list(root.url());
        assert_eq!((req.url.as_str(), req.header_value("Depth")), (root.url(), Some("1")));
        let items = parse_trash(&HttpResponse::new(207, fixture("trash-bin.xml")), &root).unwrap();
        assert_eq!(items.len(), 2, "{items:#?}");
        let file = items.iter().find(|t| !t.is_folder).unwrap();
        assert_eq!(file.name, "9e7e117a-7b8d-40ba-8587-fb86b3df82e7");
        assert_eq!((file.file_name.as_str(), file.original_location.as_str(), file.size), ("b.txt", "Docs/b.txt", 3));
        assert_eq!(file.deleted_at, Some(1_795_006_875));
        let folder = items.iter().find(|t| t.is_folder).unwrap();
        assert_eq!(
            (folder.name.as_str(), folder.file_name.as_str(), folder.original_location.as_str()),
            ("77835667-b4ab-4f87-a965-9ab4a74a3b8c", "Sub", "Docs/Sub")
        );
        // Another space's listing, or not a listing, is an error.
        assert!(parse_trash(&HttpResponse::new(207, fixture("trash-bin.xml")), &Base::parse(&trash_root(ORIGIN, "a$b")).unwrap()).is_err());
        assert!(parse_trash(&HttpResponse::new(500, Vec::new()), &root).is_err());
        let spaces = spaces_base(ORIGIN, DRIVE);
        let restore = trash_restore(root.url(), &file.name, &spaces, &file.original_location, false);
        assert_eq!(restore.method, Method::Move);
        assert_eq!(restore.url, format!("{}{}", root.url(), file.name));
        assert_eq!(restore.header_value("Destination"), Some(format!("{spaces}Docs/b.txt").as_str()));
        assert_eq!(restore.header_value("Overwrite"), Some("F"), "a taken name is never overwritten (measured: without it the server does)");
        let restore = trash_restore(root.url(), &folder.name, &spaces, &folder.original_location, true);
        assert_eq!(restore.url, format!("{}{}/", root.url(), folder.name));
        assert_eq!(restore.header_value("Destination"), Some(format!("{spaces}Docs/Sub/").as_str()));
    }

    #[test]
    fn versions_are_listed_by_file_id_newest_first_and_read_by_get() {
        let root = Base::parse(&meta_root(ORIGIN)).unwrap();
        let id = format!("{DRIVE}!7f42db93-cff4-46ac-9f87-9ec04f76d22c");
        let req = versions_list(root.url(), &id);
        assert_eq!(
            (req.url.as_str(), req.header_value("Depth")),
            (format!("https://cloud.example.org/remote.php/dav/meta/{id}/v/").as_str(), Some("1"))
        );
        // Recorded: a file created ("v1"), overwritten ("v2!") and overwritten again: two earlier versions, the current one not among them.
        let versions = parse_versions(&HttpResponse::new(207, fixture("versions.xml")), &root).unwrap();
        assert_eq!(versions.len(), 2, "{versions:#?}");
        assert_eq!(versions[0].stamp, "7f42db93-cff4-46ac-9f87-9ec04f76d22c.REV.2026-11-18T13:01:15.077736661Z");
        assert_eq!(versions[1].stamp, "7f42db93-cff4-46ac-9f87-9ec04f76d22c.REV.2026-11-18T13:01:13.845168626Z");
        assert_eq!((versions[0].size, versions[1].size), (3, 2), "newest first");
        assert_eq!(versions[0].etag.as_deref(), Some("\"d2913bb8fcbccc2e8147a66f1b86b709\""));
        assert_eq!(versions[0].when(), "2026-11-18 13:01:15 UTC");
        assert_eq!(versions[1].replaced_at().as_deref(), Some("2026-11-18T13:01:13Z"));
        assert!(parse_versions(&HttpResponse::new(404, Vec::new()), &root).unwrap().is_empty(), "no versions yet");
        assert!(parse_versions(&HttpResponse::new(500, Vec::new()), &root).is_err());
        let read = versions_download(root.url(), &id, &versions[1].stamp);
        assert_eq!(read.method, Method::Get, "a version is read, never copied over the file");
        assert_eq!(read.url, format!("https://cloud.example.org/remote.php/dav/meta/{id}/v/{}", versions[1].stamp));
        assert!(read.headers.is_empty());
    }
}
