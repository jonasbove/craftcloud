//! The trash: list, restore, delete for good, empty. Items are named by the id the file or folder
//! had (`resource.id`). Moving something *to* the trash is [`super::files::trash`] /
//! [`super::folders::trash`] (or [`super::batch::trash`]). OxiCloud keeps trashed items 30 days.

use serde::Deserialize;

use super::ApiError;
use super::resources::{Page, Resource, encode, page_query, parse_page, visible};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// One trashed file or folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashItem {
    /// Its `path` is where it was (`Personal/Documents/b.txt`): restore puts it back there.
    pub resource: Resource,
    /// RFC 3339.
    pub trashed_at: String,
    /// When the server deletes it for good (RFC 3339).
    pub expires_at: String,
}

#[derive(Deserialize)]
struct Extra {
    #[serde(default)]
    trashed_at: String,
    #[serde(default)]
    expires_at: String,
}

/// `GET /api/trash/resources`: one page, soonest to expire first.
pub fn list(server: &ServerUrl, limit: Option<u32>, cursor: Option<&str>) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}{}", server.join("api/trash/resources"), page_query(limit, cursor)))
}

/// Without CraftCloud's own items (old versions land in the trash too); [`parse_list_all`] keeps them.
pub fn parse_list(resp: &HttpResponse) -> Result<Page<TrashItem>, ApiError> {
    parse_list_all(resp).map(|p| visible(p, |t| &t.resource))
}

/// [`parse_list`], including CraftCloud's own items.
pub fn parse_list_all(resp: &HttpResponse) -> Result<Page<TrashItem>, ApiError> {
    parse_page(resp, |resource, e: Extra| TrashItem { resource, trashed_at: e.trashed_at, expires_at: e.expires_at })
}

/// `POST /api/trash/{id}/restore`. Note: OxiCloud 0.10.0 answers 200 even for an id that is not in
/// the trash, so check the listing when it matters.
pub fn restore(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join(&format!("api/trash/{}/restore", encode(id))))
}

/// `DELETE /api/trash/{id}`: deletes one trashed item for good.
pub fn delete(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, server.join(&format!("api/trash/{}", encode(id))))
}

/// `DELETE /api/trash/empty`: deletes everything in the trash for good.
pub fn empty(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Delete, server.join("api/trash/empty"))
}

#[cfg(test)]
mod tests {
    use super::{delete, empty, list, parse_list, restore};
    use crate::ServerUrl;
    use crate::api::fixtures::read;
    use crate::api::resources::Kind;
    use crate::api::{expect_ok, fixtures};
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8095").unwrap()
    }

    #[test]
    fn requests() {
        let s = server();
        assert_eq!(list(&s, None, None).url, "http://127.0.0.1:8095/api/trash/resources");
        assert_eq!(list(&s, Some(1), Some("c")).url, "http://127.0.0.1:8095/api/trash/resources?limit=1&cursor=c");
        let r = restore(&s, "id1");
        assert_eq!((r.method, r.url.as_str()), (Method::Post, "http://127.0.0.1:8095/api/trash/id1/restore"));
        let d = delete(&s, "id1");
        assert_eq!((d.method, d.url.as_str()), (Method::Delete, "http://127.0.0.1:8095/api/trash/id1"));
        let e = empty(&s);
        assert_eq!((e.method, e.url.as_str()), (Method::Delete, "http://127.0.0.1:8095/api/trash/empty"));
    }

    #[test]
    fn lists_files_and_folders() {
        let page = parse_list(&HttpResponse::new(200, read("trash-with-folder.json"))).unwrap();
        let kinds: Vec<(Kind, &str)> = page.items.iter().map(|i| (i.resource.kind(), i.resource.name())).collect();
        assert_eq!(kinds, [(Kind::File, "c.txt"), (Kind::Folder, "Old"), (Kind::Folder, "Older")]);
        assert_eq!(page.items[0].resource.cloud_path().as_deref(), Some("c.txt"));
        assert!(page.items[0].expires_at.starts_with("2026-12-13"));
        let files = parse_list(&HttpResponse::new(200, read("trash.json"))).unwrap();
        assert_eq!(files.items[1].resource.cloud_path().as_deref(), Some("Documents/Q4/a.txt"));
    }

    #[test]
    fn answers() {
        for name in ["trash-restore.json", "trash-delete.json", "trash-empty.json", "trash-file.json", "trash-folder.json"] {
            expect_ok(&HttpResponse::new(200, fixtures::read(name))).unwrap();
        }
    }
}
