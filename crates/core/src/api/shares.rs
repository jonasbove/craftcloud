//! Share links (`/api/shares`): anyone with the link can open the file or folder, optionally
//! only with a password and until a date.
//!
//! OxiCloud 0.10.0 quirks handled here: the list is `{items, pagination}` but a bare array when
//! filtered by item; an update with `null` leaves a field as it is, an empty password removes the
//! password, and an expiry date can't be removed again (create a new link instead).

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::resources::{Kind, encode};
use super::{ApiError, json as read_json};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// `ShareDto`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Share {
    pub id: String,
    pub item_id: String,
    /// What the link was created with; often `null`.
    #[serde(default)]
    pub item_name: Option<String>,
    /// `file` or `folder`.
    pub item_type: String,
    pub token: String,
    /// The link to hand out.
    pub url: String,
    pub has_password: bool,
    /// RFC 3339; `None` = never.
    #[serde(default)]
    pub expires_at: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub access_count: u64,
}

/// `GET /api/shares`: the links the signed-in person made, one page (`page` from 1).
pub fn list(server: &ServerUrl, page: u32, per_page: u32) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}?page={page}&per_page={per_page}", server.join("api/shares")))
}

/// `GET /api/shares?item_id=…&item_type=…`: the links to one file or folder.
pub fn list_for(server: &ServerUrl, kind: Kind, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}?item_id={}&item_type={}", server.join("api/shares"), encode(id), kind.as_str()))
}

/// A page of links and whether another page follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareList {
    pub items: Vec<Share>,
    pub has_next: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ListBody {
    Paged { items: Vec<Share>, pagination: Option<Pagination> },
    Bare(Vec<Share>),
}

#[derive(Deserialize)]
struct Pagination {
    #[serde(default)]
    has_next: bool,
}

/// Reads either list shape.
pub fn parse_list(resp: &HttpResponse) -> Result<ShareList, ApiError> {
    Ok(match read_json::<ListBody>(resp)? {
        ListBody::Paged { items, pagination } => ShareList { items, has_next: pagination.is_some_and(|p| p.has_next) },
        ListBody::Bare(items) => ShareList { items, has_next: false },
    })
}

/// `GET /api/shares/{id}`.
pub fn get(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join(&format!("api/shares/{}", encode(id))))
}

/// Reads one link (the answer of get, create and update).
pub fn parse_share(resp: &HttpResponse) -> Result<Share, ApiError> {
    read_json(resp)
}

/// A new link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewShare {
    pub kind: Kind,
    pub id: String,
    /// Shown on the link's page.
    pub name: Option<String>,
    /// RFC 3339, e.g. `2027-01-01T00:00:00Z`.
    pub expires_at: Option<String>,
    pub password: Option<String>,
}

/// `POST /api/shares` (201).
pub fn create(server: &ServerUrl, new: &NewShare) -> HttpRequest {
    let mut body = json!({"item_id": new.id, "item_type": new.kind.as_str()});
    if let Some(n) = &new.name {
        body["item_name"] = Value::from(n.as_str());
    }
    if let Some(e) = &new.expires_at {
        body["expires_at"] = Value::from(e.as_str());
    }
    if let Some(p) = &new.password {
        body["password"] = Value::from(p.as_str());
    }
    HttpRequest::new(Method::Post, server.join("api/shares")).json(&body)
}

/// What to change on a link; `None` leaves a field as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareChange {
    /// A new expiry (RFC 3339). An expiry can't be removed in OxiCloud 0.10.0.
    pub expires_at: Option<String>,
    /// `Some("")` removes the password.
    pub password: Option<String>,
}

/// `PUT /api/shares/{id}`.
pub fn update(server: &ServerUrl, id: &str, change: &ShareChange) -> HttpRequest {
    let mut body = Map::new();
    if let Some(e) = &change.expires_at {
        body.insert("expires_at".into(), Value::from(e.as_str()));
    }
    if let Some(p) = &change.password {
        body.insert("password".into(), Value::from(p.as_str()));
    }
    HttpRequest::new(Method::Put, server.join(&format!("api/shares/{}", encode(id)))).json(&Value::Object(body))
}

/// `DELETE /api/shares/{id}` (204): the link stops working.
pub fn delete(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, server.join(&format!("api/shares/{}", encode(id))))
}

/// Where a link's item is now: OxiCloud keeps only the name it had when the link was made (often
/// not even that), so look it up by its id, one request per link: `GET /api/files/{id}` or
/// `GET /api/folders/{id}`. Read the answer with [`parse_item_path`].
pub fn item_info(server: &ServerUrl, share: &Share) -> HttpRequest {
    if share.item_type == Kind::Folder.as_str() { super::folders::get(server, &share.item_id) } else { super::files::info(server, &share.item_id) }
}

/// The item's cloud path now (`Documents/report.pdf`), from the answer of [`item_info`]; `None`
/// when it is gone (deleted for good) or the answer can't be read: show [`Share::item_name`]
/// then.
pub fn parse_item_path(share: &Share, resp: &HttpResponse) -> Option<String> {
    if !(200..300).contains(&resp.status) {
        return None;
    }
    let path = if share.item_type == Kind::Folder.as_str() {
        super::folders::parse_folder(resp).ok()?.path
    } else {
        super::files::parse_file(resp).ok()?.path
    };
    let cloud = super::resources::cloud_path(&path);
    (!cloud.is_empty()).then_some(cloud)
}

/// The token of a link URL (`…/s/<token>`), or of a page path (`/s/<token>`).
pub fn token_of(url_or_path: &str) -> Option<&str> {
    let (_, rest) = url_or_path.split_once("/s/")?;
    let token = rest.split(['/', '?', '#']).next()?;
    (!token.is_empty()).then_some(token)
}

/// `GET /api/s/{token}`: what a link opens, for anyone (no sign-in). A link without a password
/// answers at once and sets the `oxi_shares` cookie that lets the visitor download the item
/// (`files::download` by its id); a link with one answers 401 until [`verify`].
pub fn access(server: &ServerUrl, token: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join(&format!("api/s/{}", encode(token))))
}

/// `POST /api/s/{token}/verify`: unlocks a link with its password (and sets the cookie).
pub fn verify(server: &ServerUrl, token: &str, password: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join(&format!("api/s/{}/verify", encode(token)))).json(&json!({"password": password}))
}

/// What a visitor's link shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Open(Box<Share>),
    /// It needs its password (or the one given was wrong).
    NeedsPassword,
    /// Past its expiry.
    Expired,
    /// Deleted, or never was.
    Gone,
}

/// Reads the answer of [`access`] or [`verify`].
pub fn parse_access(resp: &HttpResponse) -> Result<Access, ApiError> {
    match resp.status {
        401 => Ok(Access::NeedsPassword),
        410 => Ok(Access::Expired),
        404 => Ok(Access::Gone),
        _ => read_json(resp).map(|s| Access::Open(Box::new(s))),
    }
}

#[cfg(test)]
mod tests {
    use super::{NewShare, ShareChange, create, delete, get, list, list_for, parse_list, parse_share, update};

    #[test]
    fn visitors_open_links() {
        use super::{Access, access, parse_access, token_of, verify};
        let s = server();
        assert_eq!(token_of("https://cloud.example.org/s/ab-12"), Some("ab-12"));
        assert_eq!(token_of("/s/ab-12?x=1"), Some("ab-12"));
        assert_eq!(token_of("/files"), None);
        assert_eq!(access(&s, "ab").url, "http://127.0.0.1:8095/api/s/ab");
        let v = verify(&s, "ab", "pw");
        assert_eq!((v.method, v.url.as_str()), (Method::Post, "http://127.0.0.1:8095/api/s/ab/verify"));
        assert_eq!(body(&v), serde_json::json!({"password": "pw"}));
        let needs = br#"{"error":"Password required","requiresPassword":true}"#.to_vec();
        assert_eq!(parse_access(&HttpResponse::new(401, needs)), Ok(Access::NeedsPassword));
        assert_eq!(parse_access(&HttpResponse::new(410, Vec::new())), Ok(Access::Expired));
        assert_eq!(parse_access(&HttpResponse::new(404, Vec::new())), Ok(Access::Gone));
        let Ok(Access::Open(share)) = parse_access(&HttpResponse::new(200, read("share-create.json"))) else { panic!("open") };
        assert_eq!(share.item_type, "file");
    }
    use crate::ServerUrl;
    use crate::api::fixtures::read;
    use crate::api::resources::Kind;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8095").unwrap()
    }

    fn body(req: &crate::http::HttpRequest) -> serde_json::Value {
        serde_json::from_slice(&req.body).unwrap()
    }

    #[test]
    fn requests() {
        let s = server();
        assert_eq!(list(&s, 1, 100).url, "http://127.0.0.1:8095/api/shares?page=1&per_page=100");
        assert_eq!(list_for(&s, Kind::File, "f1").url, "http://127.0.0.1:8095/api/shares?item_id=f1&item_type=file");
        assert_eq!(get(&s, "s1").url, "http://127.0.0.1:8095/api/shares/s1");
        let new =
            NewShare { kind: Kind::File, id: "f1".into(), name: None, expires_at: Some("2027-01-01T00:00:00Z".into()), password: Some("pw".into()) };
        let c = create(&s, &new);
        assert_eq!((c.method, c.url.as_str()), (Method::Post, "http://127.0.0.1:8095/api/shares"));
        assert_eq!(body(&c), serde_json::json!({"item_id": "f1", "item_type": "file", "expires_at": "2027-01-01T00:00:00Z", "password": "pw"}));
        let u = update(&s, "s1", &ShareChange { expires_at: None, password: Some(String::new()) });
        assert_eq!((u.method, u.url.as_str()), (Method::Put, "http://127.0.0.1:8095/api/shares/s1"));
        assert_eq!(body(&u), serde_json::json!({"password": ""}), "fields left out stay as they are");
        let d = delete(&s, "s1");
        assert_eq!((d.method, d.url.as_str()), (Method::Delete, "http://127.0.0.1:8095/api/shares/s1"));
    }

    #[test]
    fn reads_both_list_shapes() {
        let paged = parse_list(&HttpResponse::new(200, read("shares.json"))).unwrap();
        assert_eq!(paged.items.len(), 1);
        assert!(!paged.has_next);
        assert!(paged.items[0].has_password);
        let bare = parse_list(&HttpResponse::new(200, read("shares-filtered.json"))).unwrap();
        assert_eq!(bare.items[0].item_type, "file");
        assert_eq!(bare.items[0].expires_at.as_deref(), Some("2028-01-01T00:00:00Z"));
    }

    /// T-040: a link shows where its item is now, not the name it had when the link was made.
    #[test]
    fn a_links_item_is_found_by_its_id() {
        use super::{item_info, parse_item_path};
        let s = server();
        let mut share = parse_share(&HttpResponse::new(201, read("share-create.json"))).unwrap();
        share.item_id = "f1".into();
        assert_eq!(item_info(&s, &share).url, "http://127.0.0.1:8095/api/files/f1?metadata=true");
        let file = br#"{"id":"f1","name":"q3.pdf","path":"/Personal/Reports/q3.pdf","size":3}"#.to_vec();
        assert_eq!(parse_item_path(&share, &HttpResponse::new(200, file)).as_deref(), Some("Reports/q3.pdf"));
        assert_eq!(parse_item_path(&share, &HttpResponse::new(404, Vec::new())), None, "gone: the stored name is shown");
        share.item_type = "folder".into();
        assert_eq!(item_info(&s, &share).url, "http://127.0.0.1:8095/api/folders/f1");
        assert_eq!(parse_item_path(&share, &HttpResponse::new(200, read("folder-get.json"))).as_deref(), Some("Documents"));
    }

    #[test]
    fn reads_a_link() {
        let s = parse_share(&HttpResponse::new(201, read("share-create.json"))).unwrap();
        assert!(s.url.ends_with(&format!("/s/{}", s.token)));
        assert_eq!((s.item_name.as_deref(), s.expires_at.as_deref()), (Some("plan v2.txt"), Some("2027-01-01T00:00:00Z")));
        let updated = parse_share(&HttpResponse::new(200, read("share-update.json"))).unwrap();
        assert_eq!(updated.expires_at, s.expires_at, "null left the expiry as it was");
    }
}
