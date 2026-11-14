//! Favourites ("starred") and recent files, by id and kind.

use serde::Deserialize;

use super::ApiError;
use super::resources::{Kind, Page, Resource, encode, page_query, parse_page, visible};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

fn item(server: &ServerUrl, area: &str, kind: Kind, id: &str) -> String {
    server.join(&format!("api/{area}/{}/{}", kind.as_str(), encode(id)))
}

/// A starred file or folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    /// Its `path` is `Personal/…` (no leading slash).
    pub resource: Resource,
    pub favorited_at: String,
}

#[derive(Deserialize)]
struct FavExtra {
    #[serde(default)]
    favorited_at: String,
}

/// `GET /api/favorites/resources`: one page, by name.
pub fn list(server: &ServerUrl, limit: Option<u32>, cursor: Option<&str>) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}{}", server.join("api/favorites/resources"), page_query(limit, cursor)))
}

/// Without CraftCloud's own items; [`parse_list_all`] keeps them.
pub fn parse_list(resp: &HttpResponse) -> Result<Page<Favorite>, ApiError> {
    parse_list_all(resp).map(|p| visible(p, |f| &f.resource))
}

/// [`parse_list`], including CraftCloud's own items.
pub fn parse_list_all(resp: &HttpResponse) -> Result<Page<Favorite>, ApiError> {
    parse_page(resp, |resource, e: FavExtra| Favorite { resource, favorited_at: e.favorited_at })
}

/// `POST /api/favorites/{kind}/{id}` (201).
pub fn add(server: &ServerUrl, kind: Kind, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, item(server, "favorites", kind, id))
}

/// `DELETE /api/favorites/{kind}/{id}`.
pub fn remove(server: &ServerUrl, kind: Kind, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, item(server, "favorites", kind, id))
}

/// A recently opened file or folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub resource: Resource,
    pub accessed_at: String,
}

#[derive(Deserialize)]
struct RecentExtra {
    #[serde(default)]
    accessed_at: String,
}

/// `GET /api/recent/resources`: one page, newest first.
pub fn recent(server: &ServerUrl, limit: Option<u32>, cursor: Option<&str>) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}{}", server.join("api/recent/resources"), page_query(limit, cursor)))
}

/// Without CraftCloud's own items; [`parse_recent_all`] keeps them.
pub fn parse_recent(resp: &HttpResponse) -> Result<Page<Recent>, ApiError> {
    parse_recent_all(resp).map(|p| visible(p, |r| &r.resource))
}

/// [`parse_recent`], including CraftCloud's own items.
pub fn parse_recent_all(resp: &HttpResponse) -> Result<Page<Recent>, ApiError> {
    parse_page(resp, |resource, e: RecentExtra| Recent { resource, accessed_at: e.accessed_at })
}

/// `POST /api/recent/{kind}/{id}`: records that the person opened it.
pub fn record_recent(server: &ServerUrl, kind: Kind, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, item(server, "recent", kind, id))
}

/// `DELETE /api/recent/{kind}/{id}`.
pub fn forget_recent(server: &ServerUrl, kind: Kind, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, item(server, "recent", kind, id))
}

/// `DELETE /api/recent/clear`.
pub fn clear_recent(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Delete, server.join("api/recent/clear"))
}

#[cfg(test)]
mod tests {
    use super::{add, clear_recent, forget_recent, list, parse_list, parse_recent, recent, record_recent, remove};
    use crate::ServerUrl;
    use crate::api::expect_ok;
    use crate::api::fixtures::read;
    use crate::api::resources::Kind;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8095").unwrap()
    }

    #[test]
    fn requests() {
        let s = server();
        assert_eq!(list(&s, Some(200), None).url, "http://127.0.0.1:8095/api/favorites/resources?limit=200");
        let a = add(&s, Kind::File, "f1");
        assert_eq!((a.method, a.url.as_str()), (Method::Post, "http://127.0.0.1:8095/api/favorites/file/f1"));
        let r = remove(&s, Kind::Folder, "d1");
        assert_eq!((r.method, r.url.as_str()), (Method::Delete, "http://127.0.0.1:8095/api/favorites/folder/d1"));
        assert_eq!(recent(&s, None, None).url, "http://127.0.0.1:8095/api/recent/resources");
        assert_eq!(record_recent(&s, Kind::File, "f1").url, "http://127.0.0.1:8095/api/recent/file/f1");
        assert_eq!(forget_recent(&s, Kind::File, "f1").method, Method::Delete);
        let c = clear_recent(&s);
        assert_eq!((c.method, c.url.as_str()), (Method::Delete, "http://127.0.0.1:8095/api/recent/clear"));
    }

    #[test]
    fn reads_favourites_and_recent() {
        let favs = parse_list(&HttpResponse::new(200, read("favorites.json"))).unwrap();
        let got: Vec<(Kind, Option<String>)> = favs.items.iter().map(|f| (f.resource.kind(), f.resource.cloud_path())).collect();
        assert_eq!(got, [(Kind::Folder, Some("Documents".into())), (Kind::File, Some("Reports/plan v2.txt".into()))]);
        assert!(favs.items.iter().all(|f| f.resource.is_favorite()));

        let recent = parse_recent(&HttpResponse::new(200, read("recent.json"))).unwrap();
        assert_eq!(recent.items[0].resource.name(), "plan v2.txt");
        assert!(recent.items[0].accessed_at > recent.items[1].accessed_at, "newest first");

        for name in ["favorite-add.json", "favorite-remove.json", "recent-record.json", "recent-clear.json"] {
            expect_ok(&HttpResponse::new(200, read(name))).unwrap();
        }
    }
}
