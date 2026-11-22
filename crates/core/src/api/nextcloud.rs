//! Nextcloud's own APIs, as pure requests and parsers (docs/WEBDAV-STORAGE.md § 7.4, the Nextcloud
//! column; § 10, Login Flow v2). A profile knows where things are, never what they guarantee: the
//! check on connect still measures every guard, and nothing here grants a capability.
//!
//! Learnt from Nextcloud's public documentation (the Login Flow, the WebDAV trashbin and versions
//! endpoints, the OCS user endpoint) and from a Nextcloud 31 answering, recorded in
//! `crates/core/tests/fixtures/nextcloud-31/`:
//!
//! - **Detection** is `GET /status.php` (`productname`, `versionstring`, `version`). It only
//!   suggests a kind of server.
//! - **Login Flow v2** ([`login_flow_start`]): `POST /index.php/login/v2` answers a `login` URL for
//!   the browser and a `poll` endpoint with a token; the client polls ([`login_flow_poll`]) and gets
//!   404 while the person hasn't finished, then the login name and an app password made for this
//!   device. Signing out deletes that app password ([`app_password_delete`]).
//! - **The user id** can differ from the login name: `current-user-principal` on `/remote.php/dav/`
//!   names it, and the WebDAV base is `/remote.php/dav/files/<user id>/`.
//! - **The trash** is `/remote.php/dav/trashbin/<user id>/trash/`, every item named
//!   `<name>.d<seconds>` with its original path in `nc:trashbin-original-location`; a restore is a
//!   `MOVE` to `/remote.php/dav/trashbin/<user id>/restore/<name>`.
//! - **Versions** are `/remote.php/dav/versions/<user id>/versions/<file id>/`, one entry per
//!   version named by its time in seconds, the current file among them; restoring one is a `MOVE`
//!   to `/remote.php/dav/versions/<user id>/restore/target`, which CraftCloud never sends: that move
//!   names no version of the file it replaces, so it is an unguarded write. A version's bytes are
//!   read with a `GET` ([`versions_download`]) and written back with a guarded `PUT`.
//! - **Quota** is the OCS user endpoint's `quota.free` (WebDAV's `quota-available-bytes` says `-3`).
//! - **Checksums**: the server lists `oc:checksums` only for a file whose last `PUT` carried an
//!   `OC-Checksum` header, and it stores that value without checking it (measured on 31.0.14).

use serde::Deserialize;

use super::ApiError;
use crate::http::{HttpRequest, HttpResponse, Method};
use crate::storage::probe::Base;

/// The header every OCS call needs.
pub const OCS_HEADER: (&str, &str) = ("OCS-APIRequest", "true");

/// What `status.php` says about a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    /// `productname`, e.g. `Nextcloud`.
    pub product: String,
    /// `versionstring`, e.g. `31.0.14` (what a person is shown).
    pub version_string: String,
    /// `version`, e.g. `31.0.14.1` (what the saved check compares).
    pub version: String,
}

impl Detected {
    pub fn is_nextcloud(&self) -> bool {
        self.product.eq_ignore_ascii_case("nextcloud")
    }

    /// ownCloud 10 answers the same `status.php` (`productname` "ownCloud"); its profile is
    /// [`super::owncloud`]'s.
    pub fn is_owncloud(&self) -> bool {
        self.product.eq_ignore_ascii_case("owncloud")
    }

    /// "Nextcloud 31.0.14".
    pub fn words(&self) -> String {
        format!("{} {}", self.product, self.version_string).trim().to_owned()
    }
}

fn under(origin: &str, path: &str) -> String {
    format!("{}/{}", origin.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// `GET /status.php`: no sign-in needed.
pub fn status(origin: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, under(origin, "status.php"))
}

/// What `status.php` answered, or `None` when the server isn't one that has it (any status other
/// than 200, a body that isn't its JSON, no product name).
pub fn parse_status(resp: &HttpResponse) -> Option<Detected> {
    #[derive(Deserialize)]
    struct Status {
        #[serde(default)]
        productname: String,
        #[serde(default)]
        versionstring: String,
        #[serde(default)]
        version: String,
    }
    if resp.status != 200 {
        return None;
    }
    let s: Status = serde_json::from_slice(&resp.body).ok()?;
    if s.productname.trim().is_empty() {
        return None;
    }
    Some(Detected {
        product: s.productname.trim().to_owned(),
        version_string: s.versionstring.trim().to_owned(),
        version: s.version.trim().to_owned(),
    })
}

/// A Login Flow v2 under way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginFlow {
    /// Where the person signs in (opened in the browser).
    pub login_url: String,
    /// Where the client polls.
    pub poll_endpoint: String,
    /// The token the poll carries.
    pub poll_token: String,
}

/// Starts a Login Flow v2. `user_agent` names this client and device: the server lists the app
/// password it makes under that name.
pub fn login_flow_start(origin: &str, user_agent: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, under(origin, "index.php/login/v2")).header("User-Agent", user_agent)
}

pub fn parse_login_flow_start(resp: &HttpResponse) -> Result<LoginFlow, ApiError> {
    #[derive(Deserialize)]
    struct Poll {
        token: String,
        endpoint: String,
    }
    #[derive(Deserialize)]
    struct Start {
        poll: Poll,
        login: String,
    }
    let s: Start = super::json(resp)?;
    if s.poll.token.is_empty() || s.poll.endpoint.is_empty() || s.login.is_empty() {
        return Err(ApiError::Body("the Login Flow answer lacks its poll token, endpoint or login address".into()));
    }
    Ok(LoginFlow { login_url: s.login, poll_endpoint: s.poll.endpoint, poll_token: s.poll.token })
}

/// What a form field's value escapes (`application/x-www-form-urlencoded`): everything but
/// letters, digits, `-`, `_`, `.` and `~`.
const FORM_VALUE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// One poll of a Login Flow v2 (`token=` as a form).
pub fn login_flow_poll(flow: &LoginFlow, user_agent: &str) -> HttpRequest {
    let body = format!("token={}", percent_encoding::utf8_percent_encode(&flow.poll_token, FORM_VALUE));
    HttpRequest::new(Method::Post, flow.poll_endpoint.clone())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", user_agent)
        .body(body.into_bytes())
}

/// The app password a finished Login Flow hands over.
#[derive(Clone, PartialEq, Eq)]
pub struct AppPasswordGrant {
    /// The server, as it names itself.
    pub server: String,
    /// The login name (not always the user id).
    pub login_name: String,
    pub app_password: String,
}

impl std::fmt::Debug for AppPasswordGrant {
    // Never print the password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppPasswordGrant").field("server", &self.server).field("login_name", &self.login_name).finish_non_exhaustive()
    }
}

/// What a poll answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowPoll {
    /// Not finished yet (404).
    Pending,
    Done(AppPasswordGrant),
}

pub fn parse_login_flow_poll(resp: &HttpResponse) -> Result<FlowPoll, ApiError> {
    #[derive(Deserialize)]
    struct Done {
        #[serde(default)]
        server: String,
        #[serde(rename = "loginName")]
        login_name: String,
        #[serde(rename = "appPassword")]
        app_password: String,
    }
    if resp.status == 404 {
        return Ok(FlowPoll::Pending);
    }
    let d: Done = super::json(resp)?;
    if d.login_name.is_empty() || d.app_password.is_empty() {
        return Err(ApiError::Body("the Login Flow answer lacks the login name or the app password".into()));
    }
    Ok(FlowPoll::Done(AppPasswordGrant { server: d.server, login_name: d.login_name, app_password: d.app_password }))
}

/// `DELETE /ocs/v2.php/core/apppassword`: ends the app password the request is signed in with.
pub fn app_password_delete(origin: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, under(origin, "ocs/v2.php/core/apppassword?format=json")).header(OCS_HEADER.0, OCS_HEADER.1)
}

/// `PROPFIND /remote.php/dav/` for `current-user-principal`: the user id the WebDAV paths use.
pub fn current_user_principal(origin: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, under(origin, "remote.php/dav/"))
        .header("Depth", "0")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#.into())
}

/// The user id from a `current-user-principal` answer (`/remote.php/dav/principals/users/<id>/`).
pub fn parse_current_user_principal(resp: &HttpResponse) -> Result<String, String> {
    if resp.status != 207 {
        return Err(format!("asking who is signed in answered {} instead of 207", resp.status));
    }
    let doc = craft_dav::parse_multistatus(&resp.body).map_err(|e| format!("the principal answer couldn't be read: {e}"))?;
    for response in craft_dav::responses(&doc) {
        for prop in craft_dav::properties(response) {
            if prop.name == "d:current-user-principal"
                && let Some(href) = prop.child("d:href")
            {
                let href = href.text();
                let decoded = percent_encoding::percent_decode_str(href.trim()).decode_utf8().map_err(|e| e.to_string())?;
                let id = decoded.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_owned();
                if id.is_empty() || !decoded.contains("/principals/users/") {
                    return Err(format!("the principal {decoded} isn't a user"));
                }
                return Ok(id);
            }
        }
    }
    Err("the server didn't say who is signed in (no current-user-principal)".into())
}

/// The WebDAV base of `user`'s files: `<origin>/remote.php/dav/files/<user>/`.
pub fn files_base(origin: &str, user: &str) -> String {
    crate::api::dav::url_under(&under(origin, "remote.php/dav/files/"), user, true)
}

/// The origin and the user id of a Nextcloud files base
/// (`https://cloud.example.org/remote.php/dav/files/john/` → `https://cloud.example.org`, `john`;
/// a server under a path keeps it in the origin); `None` for a base that isn't one.
pub fn split_files_base(base: &Base) -> Option<(String, String)> {
    let path = percent_encoding::percent_decode_str(base.path()).decode_utf8().ok()?;
    let (prefix, user) = path.trim_end_matches('/').rsplit_once('/')?;
    let before = prefix.strip_suffix("/remote.php/dav/files")?;
    if user.is_empty() || user.contains('/') {
        return None;
    }
    Some((format!("{}{before}", base.origin()), user.to_owned()))
}

/// The trash bin's root: `<origin>/remote.php/dav/trashbin/<user>/` (items under `trash/`, a
/// restore is a `MOVE` to `restore/<name>`).
pub fn trashbin_root(origin: &str, user: &str) -> String {
    crate::api::dav::url_under(&under(origin, "remote.php/dav/trashbin/"), user, true)
}

/// The versions root: `<origin>/remote.php/dav/versions/<user>/` (a file's versions under
/// `versions/<file id>/`, each one readable with a `GET`).
pub fn versions_root(origin: &str, user: &str) -> String {
    crate::api::dav::url_under(&under(origin, "remote.php/dav/versions/"), user, true)
}

/// `GET /ocs/v2.php/cloud/user`: the signed-in user, with the quota.
pub fn ocs_user(origin: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, under(origin, "ocs/v2.php/cloud/user?format=json")).header(OCS_HEADER.0, OCS_HEADER.1)
}

/// The free bytes from the OCS user answer: `quota.free` when it is a count, `u64::MAX` when the
/// account has no limit (`-3`), `None` when the server doesn't say (any other negative, or no
/// quota at all).
pub fn parse_quota_free(resp: &HttpResponse) -> Result<Option<u64>, ApiError> {
    let v: serde_json::Value = super::json(resp)?;
    let free = v["ocs"]["data"]["quota"]["free"].as_i64();
    Ok(match free {
        Some(n) if n >= 0 => Some(n.unsigned_abs()),
        Some(-3) => Some(u64::MAX),
        _ => None,
    })
}

/// What a trash listing asks for.
pub const TRASHBIN_PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><oc:fileid/><nc:trashbin-filename/><nc:trashbin-original-location/><nc:trashbin-deletion-time/></d:prop></d:propfind>"#;

/// `PROPFIND` of the trash bin, one level.
pub fn trashbin_list(trashbin_root: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, under(trashbin_root, "trash/"))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(TRASHBIN_PROPFIND_BODY.as_bytes().to_vec())
}

/// One item in the trash bin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    /// Its name in the trash, `<name>.d<seconds>`.
    pub name: String,
    pub is_folder: bool,
    /// The name it had.
    pub file_name: String,
    /// Where it was (a cloud path under the files base, no leading slash).
    pub original_location: String,
    /// When it was deleted, seconds since 1970, where the server says.
    pub deleted_at: Option<i64>,
    pub file_id: Option<String>,
    pub size: u64,
}

/// Reads a trash listing against `root` (the trash bin's root, [`trashbin_root`]). An href outside
/// it, a status other than 207 or an unreadable body is an error, in words.
pub fn parse_trashbin(resp: &HttpResponse, root: &Base) -> Result<Vec<Trashed>, String> {
    if resp.status != 207 {
        return Err(format!("the trash listing answered {} instead of 207", resp.status));
    }
    let doc = craft_dav::parse_multistatus(&resp.body).map_err(|e| format!("the trash listing couldn't be read: {e}"))?;
    let mut out = Vec::new();
    for response in craft_dav::responses(&doc) {
        let href = craft_dav::href(response).ok_or_else(|| "a trash entry without a path".to_owned())?;
        let rel = root.relative(&href).ok_or_else(|| format!("the trash listing named a path outside the trash bin: {href}"))?;
        let Some(name) = rel.strip_prefix("trash/").filter(|n| !n.is_empty()) else { continue };
        let mut t = Trashed {
            name: name.to_owned(),
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
                "nc:trashbin-filename" => t.file_name = text,
                "nc:trashbin-original-location" => t.original_location = text.trim_matches('/').to_owned(),
                "nc:trashbin-deletion-time" => t.deleted_at = text.parse().ok(),
                "oc:fileid" => t.file_id = Some(text).filter(|s| !s.is_empty()),
                _ => {}
            }
        }
        if t.file_name.is_empty() {
            t.file_name = name.rsplit_once(".d").map_or(name, |(n, _)| n).to_owned();
        }
        out.push(t);
    }
    Ok(out)
}

/// `MOVE` a trashed item back where it was: to `restore/<name>` under the trash bin's root. Without
/// `Overwrite: F`: the restore address is the server's endpoint, not a name in the person's tree,
/// and Nextcloud 31 answers 412 to the header there (measured on 31.0.14).
pub fn trashbin_restore(trashbin_root: &str, name: &str) -> HttpRequest {
    HttpRequest::new(Method::Move, crate::api::dav::url_under(trashbin_root, &format!("trash/{name}"), false))
        .header("Destination", crate::api::dav::url_under(trashbin_root, &format!("restore/{name}"), false))
}

/// `DELETE` one trashed item for good: `trash/<name>` under the trash bin's root. Only ever on the
/// person's own command (`craftcloud trash rm`, `trash empty`), for a name a listing just gave.
pub fn trashbin_delete(trashbin_root: &str, name: &str, is_folder: bool) -> HttpRequest {
    HttpRequest::new(Method::Delete, crate::api::dav::url_under(trashbin_root, &format!("trash/{name}"), is_folder))
}

/// `PROPFIND` of a file's versions, one level.
pub fn versions_list(versions_root: &str, file_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Propfind, crate::api::dav::url_under(versions_root, &format!("versions/{file_id}"), true))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(crate::storage::probe::PROPFIND_BODY.as_bytes().to_vec())
}

/// The versions of a file from its listing against `root` ([`versions_root`]), newest first, as
/// [`super::versions::Version`]s whose stamp is the version's name (its time in seconds). The
/// listing names the current file among them; `current_modified` (the file's `getlastmodified`,
/// in seconds) leaves it out, so the list holds only the earlier versions.
pub fn parse_versions(resp: &HttpResponse, root: &Base, current_modified: Option<i64>) -> Result<Vec<super::versions::Version>, String> {
    if resp.status == 404 {
        return Ok(Vec::new());
    }
    let listed = crate::storage::probe::parse_listing(resp, root).map_err(|e| format!("the versions listing: {e}"))?;
    let mut out: Vec<super::versions::Version> = listed
        .into_iter()
        .filter(|e| !e.is_folder)
        .filter(|e| {
            let name = e.name().to_owned();
            let at: Option<i64> = name.parse().ok();
            at.is_none() || at != current_modified
        })
        .map(|e| super::versions::Version { stamp: e.name().to_owned(), size: e.size, etag: e.etag })
        .collect();
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    Ok(out)
}

/// `GET` the bytes of version `stamp` of file `file_id`. A restore writes them back over the file
/// with a guarded `PUT` (`If-Match`), never with the server's own restore `MOVE`, which carries no
/// precondition on the file it replaces.
pub fn versions_download(versions_root: &str, file_id: &str, stamp: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, crate::api::dav::url_under(versions_root, &format!("versions/{file_id}/{stamp}"), false))
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::read_from;
    use super::*;

    const FIXTURES: &str = "nextcloud-31";
    const ORIGIN: &str = "https://cloud.example.org";

    fn fixture(name: &str) -> Vec<u8> {
        read_from(FIXTURES, name)
    }

    #[test]
    fn status_php_says_nextcloud_and_its_version() {
        let d = parse_status(&HttpResponse::new(200, fixture("status.json"))).unwrap();
        assert_eq!(d, Detected { product: "Nextcloud".into(), version_string: "31.0.14".into(), version: "31.0.14.1".into() });
        assert!(d.is_nextcloud());
        assert_eq!(d.words(), "Nextcloud 31.0.14");
        // Anything else is not a detection: a 404 (OxiCloud, a NAS), HTML, JSON without a product.
        assert_eq!(parse_status(&HttpResponse::new(404, b"not found".to_vec())), None);
        assert_eq!(parse_status(&HttpResponse::new(200, b"<html/>".to_vec())), None);
        assert_eq!(parse_status(&HttpResponse::new(200, br#"{"installed":true}"#.to_vec())), None);
        assert_eq!(status(ORIGIN).url, "https://cloud.example.org/status.php");
        assert_eq!(status("https://cloud.example.org/").url, "https://cloud.example.org/status.php");
    }

    #[test]
    fn the_login_flow_starts_polls_and_ends_with_an_app_password() {
        let start = login_flow_start(ORIGIN, "CraftCloud on Laptop");
        assert_eq!((start.method, start.url.as_str()), (Method::Post, "https://cloud.example.org/index.php/login/v2"));
        assert_eq!(start.header_value("User-Agent"), Some("CraftCloud on Laptop"));
        let flow = parse_login_flow_start(&HttpResponse::new(200, fixture("login-v2-start.json"))).unwrap();
        assert_eq!(flow.login_url, "https://cloud.example.org/login/v2/flow/fixture-flow-token");
        assert_eq!(flow.poll_endpoint, "https://cloud.example.org/login/v2/poll");
        assert_eq!(flow.poll_token, "fixture-poll-token");
        let poll = login_flow_poll(&flow, "CraftCloud on Laptop");
        assert_eq!((poll.method, poll.url.as_str()), (Method::Post, "https://cloud.example.org/login/v2/poll"));
        assert_eq!(poll.body, b"token=fixture-poll-token");
        let odd = LoginFlow { poll_token: "a b+c/=".into(), ..flow.clone() };
        assert_eq!(login_flow_poll(&odd, "x").body, b"token=a%20b%2Bc%2F%3D");
        assert_eq!(poll.header_value("Content-Type"), Some("application/x-www-form-urlencoded"));
        // Not finished: 404 (as recorded: an empty JSON list).
        assert_eq!(parse_login_flow_poll(&HttpResponse::new(404, fixture("login-v2-poll-pending.json"))).unwrap(), FlowPoll::Pending);
        // Finished (the shape the public Login Flow documentation gives; not recorded).
        let done = parse_login_flow_poll(&HttpResponse::new(200, fixture("login-v2-poll-done.json"))).unwrap();
        let FlowPoll::Done(grant) = done else { panic!("{done:?}") };
        assert_eq!((grant.server.as_str(), grant.login_name.as_str(), grant.app_password.as_str()), (ORIGIN, "john", "fixture-app-password"));
        assert!(!format!("{grant:?}").contains("fixture-app-password"), "no password in Debug");
        // Anything else is an error, never a sign-in.
        assert!(parse_login_flow_poll(&HttpResponse::new(500, Vec::new())).is_err());
        assert!(parse_login_flow_poll(&HttpResponse::new(200, br#"{"server":"x"}"#.to_vec())).is_err());
        assert!(parse_login_flow_start(&HttpResponse::new(200, br#"{"poll":{"token":"","endpoint":""},"login":""}"#.to_vec())).is_err());
        let del = app_password_delete(ORIGIN);
        assert_eq!((del.method, del.url.as_str()), (Method::Delete, "https://cloud.example.org/ocs/v2.php/core/apppassword?format=json"));
        assert_eq!(del.header_value("OCS-APIRequest"), Some("true"));
    }

    #[test]
    fn the_user_id_comes_from_the_current_user_principal() {
        let req = current_user_principal(ORIGIN);
        assert_eq!(
            (req.method, req.url.as_str(), req.header_value("Depth")),
            (Method::Propfind, "https://cloud.example.org/remote.php/dav/", Some("0"))
        );
        assert_eq!(parse_current_user_principal(&HttpResponse::new(207, fixture("current-user-principal.xml"))).unwrap(), "john");
        assert!(parse_current_user_principal(&HttpResponse::new(401, Vec::new())).is_err());
        let other = br#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/remote.php/dav/</d:href><d:propstat><d:prop><d:getetag>x</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        assert!(parse_current_user_principal(&HttpResponse::new(207, other.to_vec())).is_err());
        assert_eq!(files_base(ORIGIN, "john"), "https://cloud.example.org/remote.php/dav/files/john/");
        assert_eq!(files_base("https://cloud.example.org/", "jane doe"), "https://cloud.example.org/remote.php/dav/files/jane%20doe/");
        assert_eq!(
            split_files_base(&Base::parse("https://cloud.example.org/remote.php/dav/files/john/").unwrap()),
            Some((ORIGIN.into(), "john".into()))
        );
        assert_eq!(
            split_files_base(&Base::parse("https://example.org/cloud/remote.php/dav/files/jane%20doe").unwrap()),
            Some(("https://example.org/cloud".into(), "jane doe".into()))
        );
        assert_eq!(split_files_base(&Base::parse("https://nas.example.org/dav/").unwrap()), None);
        assert_eq!(split_files_base(&Base::parse("https://cloud.example.org/remote.php/dav/files/").unwrap()), None);
        assert_eq!(trashbin_root(ORIGIN, "john"), "https://cloud.example.org/remote.php/dav/trashbin/john/");
        assert_eq!(versions_root(ORIGIN, "john"), "https://cloud.example.org/remote.php/dav/versions/john/");
    }

    #[test]
    fn the_quota_comes_from_the_ocs_user_endpoint() {
        let req = ocs_user(ORIGIN);
        assert_eq!(req.url, "https://cloud.example.org/ocs/v2.php/cloud/user?format=json");
        assert_eq!(req.header_value("OCS-APIRequest"), Some("true"));
        // The recorded account has no limit: -3.
        assert_eq!(parse_quota_free(&HttpResponse::new(200, fixture("ocs-user.json"))).unwrap(), Some(u64::MAX));
        let with = |free: i64| HttpResponse::new(200, format!(r#"{{"ocs":{{"data":{{"quota":{{"free":{free}}}}}}}}}"#).into_bytes());
        assert_eq!(parse_quota_free(&with(5_000)).unwrap(), Some(5_000));
        assert_eq!(parse_quota_free(&with(0)).unwrap(), Some(0));
        assert_eq!(parse_quota_free(&with(-1)).unwrap(), None, "unknown");
        assert_eq!(parse_quota_free(&HttpResponse::new(200, br#"{"ocs":{"data":{}}}"#.to_vec())).unwrap(), None);
        assert!(parse_quota_free(&HttpResponse::new(401, Vec::new())).is_err());
    }

    #[test]
    fn the_trash_bin_lists_items_with_where_they_were_and_restores_by_move() {
        let root = Base::parse(&trashbin_root(ORIGIN, "john")).unwrap();
        let req = trashbin_list(root.url());
        assert_eq!((req.url.as_str(), req.header_value("Depth")), ("https://cloud.example.org/remote.php/dav/trashbin/john/trash/", Some("1")));
        let items = parse_trashbin(&HttpResponse::new(207, fixture("trashbin.xml")), &root).unwrap();
        assert_eq!(items.len(), 2, "{items:#?}");
        let file = items.iter().find(|t| !t.is_folder).unwrap();
        assert!(file.name.starts_with("notes.md.d"), "{}", file.name);
        assert_eq!((file.file_name.as_str(), file.original_location.as_str(), file.size), ("notes.md", "Documents/notes.md", 4));
        assert!(file.deleted_at.is_some() && file.file_id.is_some());
        let folder = items.iter().find(|t| t.is_folder).unwrap();
        assert_eq!((folder.file_name.as_str(), folder.original_location.as_str()), ("Sub", "Documents/Sub"));
        // A listing from another root, or not a listing, is an error.
        assert!(parse_trashbin(&HttpResponse::new(207, fixture("trashbin.xml")), &Base::parse(&trashbin_root(ORIGIN, "jane")).unwrap()).is_err());
        assert!(parse_trashbin(&HttpResponse::new(500, Vec::new()), &root).is_err());
        let restore = trashbin_restore(root.url(), &file.name);
        assert_eq!(restore.method, Method::Move);
        assert_eq!(restore.url, format!("https://cloud.example.org/remote.php/dav/trashbin/john/trash/{}", file.name));
        assert_eq!(
            restore.header_value("Destination"),
            Some(format!("https://cloud.example.org/remote.php/dav/trashbin/john/restore/{}", file.name).as_str())
        );
        // No `Overwrite: F`: the restore address isn't a name in the person's tree, and Nextcloud 31
        // answers 412 to the header there (measured); the server puts the item back where it was.
        assert_eq!(restore.header_value("Overwrite"), None);
        // Deleting one for good (the person's `trash rm`): only ever the item under `trash/`.
        let purge = trashbin_delete(root.url(), &file.name, false);
        assert_eq!(purge.method, Method::Delete);
        assert_eq!(purge.url, format!("https://cloud.example.org/remote.php/dav/trashbin/john/trash/{}", file.name));
    }

    #[test]
    fn versions_are_listed_by_file_id_without_the_current_one() {
        let root = Base::parse(&versions_root(ORIGIN, "john")).unwrap();
        let req = versions_list(root.url(), "2517");
        assert_eq!(
            (req.url.as_str(), req.header_value("Depth")),
            ("https://cloud.example.org/remote.php/dav/versions/john/versions/2517/", Some("1"))
        );
        let resp = HttpResponse::new(207, fixture("versions.xml"));
        // Recorded: a file created, then overwritten once; the listing names both writes.
        let all = parse_versions(&resp, &root, None).unwrap();
        assert_eq!(all.len(), 2, "{all:#?}");
        assert!(all[0].stamp > all[1].stamp, "newest first");
        assert!(all.iter().all(|v| v.stamp.chars().all(|c| c.is_ascii_digit()) && v.size > 0));
        let current: i64 = all[0].stamp.parse().unwrap();
        let earlier = parse_versions(&resp, &root, Some(current)).unwrap();
        assert_eq!(earlier.len(), 1, "the current file left out: {earlier:#?}");
        assert_eq!(earlier[0].stamp, all[1].stamp);
        assert!(parse_versions(&HttpResponse::new(404, Vec::new()), &root, None).unwrap().is_empty(), "no versions yet");
        assert!(parse_versions(&HttpResponse::new(500, Vec::new()), &root, None).is_err());
        let read = versions_download(root.url(), "2517", &earlier[0].stamp);
        assert_eq!(read.method, Method::Get, "a version is read, never moved over the file");
        assert_eq!(read.url, format!("https://cloud.example.org/remote.php/dav/versions/john/versions/2517/{}", earlier[0].stamp));
        assert!(read.headers.is_empty());
    }

    /// A version named by seconds shows as a date and time.
    #[test]
    fn a_version_named_by_seconds_shows_as_a_time() {
        let v = super::super::versions::Version { stamp: "1795001742".into(), size: 3, etag: None };
        assert_eq!(v.when(), "2026-11-18 11:35:42 UTC");
        assert_eq!(v.replaced_at().as_deref(), Some("2026-11-18T11:35:42Z"));
    }
}
