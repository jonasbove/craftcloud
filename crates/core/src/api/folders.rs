//! Folders over REST: the drive root, a folder's contents, create, rename, move, trash,
//! breadcrumbs and a zip of a folder.
//!
//! Everything here is by folder id. The drive's root folder (OxiCloud calls it `Personal`) is what
//! WebDAV's `/webdav/` shows; [`roots`] finds its id.

use serde::Deserialize;
use serde_json::json;

use super::resources::{File, Folder, NoExtra, Page, Resource, encode, page_query, parse_page, visible};
use super::{ApiError, json as read_json};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

fn at(server: &ServerUrl, id: &str, rest: &str) -> String {
    server.join(&format!("api/folders/{}{rest}", encode(id)))
}

/// `GET /api/folders`: the signed-in person's root folders (one: their drive's root).
pub fn roots(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join("api/folders"))
}

pub fn parse_roots(resp: &HttpResponse) -> Result<Vec<Folder>, ApiError> {
    read_json(resp)
}

/// `GET /api/folders/{id}`.
pub fn get(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, at(server, id, ""))
}

/// Reads a single folder (the answer of get, create, rename and move).
pub fn parse_folder(resp: &HttpResponse) -> Result<Folder, ApiError> {
    read_json(resp)
}

/// `GET /api/folders/{id}/resources`: one page of what is in a folder, folders first, by name.
/// `limit` is 1–200 (the server's default is 50).
pub fn contents(server: &ServerUrl, id: &str, limit: Option<u32>, cursor: Option<&str>) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}{}", at(server, id, "/resources"), page_query(limit, cursor)))
}

/// The items' `path` is empty here; the caller knows the folder's own path. CraftCloud's own
/// `.craftcloud` folder is left out ([`parse_contents_all`] keeps it).
pub fn parse_contents(resp: &HttpResponse) -> Result<Page<Resource>, ApiError> {
    parse_contents_all(resp).map(|p| visible(p, |r| r))
}

/// [`parse_contents`], including CraftCloud's own items.
pub fn parse_contents_all(resp: &HttpResponse) -> Result<Page<Resource>, ApiError> {
    parse_page(resp, |r, _: NoExtra| r)
}

/// `GET /api/files?folder_id=…`: the files (not folders) of a folder, unpaged, with full paths.
pub fn files_in(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}?folder_id={}", server.join("api/files"), encode(id)))
}

pub fn parse_files_in(resp: &HttpResponse) -> Result<Vec<File>, ApiError> {
    read_json(resp)
}

/// `POST /api/folders`. 409 when the name is taken.
pub fn create(server: &ServerUrl, parent_id: &str, name: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/folders")).json(&json!({"name": name, "parent_id": parent_id}))
}

/// `PUT /api/folders/{id}/rename`.
pub fn rename(server: &ServerUrl, id: &str, name: &str) -> HttpRequest {
    HttpRequest::new(Method::Put, at(server, id, "/rename")).json(&json!({"name": name}))
}

/// `PUT /api/folders/{id}/move`, into another folder.
pub fn move_to(server: &ServerUrl, id: &str, parent_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Put, at(server, id, "/move")).json(&json!({"parent_id": parent_id}))
}

/// `DELETE /api/folders/{id}`: moves the folder, with everything in it, to the trash (204).
/// (WebDAV's `DELETE` deletes for good instead.)
pub fn trash(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, at(server, id, ""))
}

/// One step of a breadcrumb.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Ancestor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// `GET /api/folders/{id}/ancestors`.
pub fn ancestors(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, at(server, id, "/ancestors"))
}

#[derive(Deserialize)]
struct AncestorsBody {
    ancestors: Vec<Ancestor>,
}

/// Root first, the folder itself last (the root is the drive's, e.g. `Personal`).
pub fn parse_ancestors(resp: &HttpResponse) -> Result<Vec<Ancestor>, ApiError> {
    read_json::<AncestorsBody>(resp).map(|b| b.ancestors)
}

/// `GET /api/folders/{id}/download`: the folder as a zip (`application/zip`).
pub fn download_zip(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, at(server, id, "/download"))
}

#[cfg(test)]
mod tests {
    use super::{
        ancestors, contents, create, download_zip, files_in, move_to, parse_ancestors, parse_contents, parse_files_in, parse_folder, parse_roots,
        rename, trash,
    };
    use crate::ServerUrl;
    use crate::api::fixtures::read;
    use crate::api::resources::Resource;
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
        assert_eq!((super::roots(&s).method, super::roots(&s).url.as_str()), (Method::Get, "http://127.0.0.1:8095/api/folders"));
        assert_eq!(super::get(&s, "f1").url, "http://127.0.0.1:8095/api/folders/f1");
        assert_eq!(contents(&s, "f1", None, None).url, "http://127.0.0.1:8095/api/folders/f1/resources");
        assert_eq!(contents(&s, "f1", Some(200), Some("eyJ")).url, "http://127.0.0.1:8095/api/folders/f1/resources?limit=200&cursor=eyJ");
        assert_eq!(files_in(&s, "f1").url, "http://127.0.0.1:8095/api/files?folder_id=f1");
        let c = create(&s, "root", "Q3");
        assert_eq!((c.method, c.url.as_str()), (Method::Post, "http://127.0.0.1:8095/api/folders"));
        assert_eq!(body(&c), serde_json::json!({"name": "Q3", "parent_id": "root"}));
        let r = rename(&s, "f1", "Q4");
        assert_eq!(
            (r.method, r.url.as_str(), body(&r)),
            (Method::Put, "http://127.0.0.1:8095/api/folders/f1/rename", serde_json::json!({"name": "Q4"}))
        );
        let m = move_to(&s, "f1", "p2");
        assert_eq!(
            (m.method, m.url.as_str(), body(&m)),
            (Method::Put, "http://127.0.0.1:8095/api/folders/f1/move", serde_json::json!({"parent_id": "p2"}))
        );
        let t = trash(&s, "f1");
        assert_eq!((t.method, t.url.as_str()), (Method::Delete, "http://127.0.0.1:8095/api/folders/f1"));
        assert!(t.body.is_empty());
        assert_eq!(ancestors(&s, "f1").url, "http://127.0.0.1:8095/api/folders/f1/ancestors");
        assert_eq!(download_zip(&s, "f1").url, "http://127.0.0.1:8095/api/folders/f1/download");
    }

    #[test]
    fn the_root_is_the_drive() {
        let roots = parse_roots(&HttpResponse::new(200, read("folders-root.json"))).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!((roots[0].name.as_str(), roots[0].path.as_str(), roots[0].parent_id.as_deref()), ("Personal", "/Personal", None));
    }

    #[test]
    fn reads_a_folder() {
        let f = parse_folder(&HttpResponse::new(200, read("folder-get.json"))).unwrap();
        assert_eq!((f.name.as_str(), f.path.as_str()), ("Documents", "/Personal/Documents"));
        for name in ["folder-create.json", "folder-rename.json", "folder-move.json"] {
            parse_folder(&HttpResponse::new(200, read(name))).unwrap();
        }
        assert_eq!(parse_folder(&HttpResponse::new(200, read("folder-move.json"))).unwrap().path, "/Personal/Documents/Q4");
    }

    #[test]
    fn lists_a_folder() {
        let page = parse_contents(&HttpResponse::new(200, read("folder-resources-documents.json"))).unwrap();
        let names: Vec<&str> = page.items.iter().map(Resource::name).collect();
        assert_eq!(names, ["notes.md", "plan.txt"]);
        assert_eq!(page.next_cursor, None);
        let Resource::File(notes) = &page.items[0] else { panic!("a file") };
        assert_eq!((notes.size, notes.category.as_str(), notes.path.as_str()), (11, "Markdown", ""));

        let paged = parse_contents(&HttpResponse::new(200, read("folder-resources-page.json"))).unwrap();
        assert_eq!(paged.items.len(), 2);
        assert!(paged.next_cursor.is_some(), "more to come");
        assert!(matches!(paged.items[0], Resource::Folder(_)), "folders first");

        let files = parse_files_in(&HttpResponse::new(200, read("files-in-folder.json"))).unwrap();
        assert_eq!(files[1].path, "/Personal/Documents/plan.txt");
    }

    #[test]
    fn breadcrumbs_start_at_the_root() {
        let crumbs = parse_ancestors(&HttpResponse::new(200, read("folder-ancestors.json"))).unwrap();
        let names: Vec<&str> = crumbs.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Personal", "Reports", "Q4"]);
    }

    #[test]
    fn a_taken_name_says_so() {
        let err = parse_folder(&HttpResponse::new(409, read("folder-exists.json"))).unwrap_err();
        assert_eq!(err.status(), Some(409));
        assert_eq!(err.to_string(), "Folder already exists: Documents already exists in parent");
    }
}
