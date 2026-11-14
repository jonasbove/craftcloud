//! Files over REST: info, download, rename, move, trash and thumbnails, by file id.
//!
//! **Uploads go over WebDAV** ([`super::dav::upload`]), not `POST /api/files/upload`: a WebDAV
//! `PUT` streams the bytes as they are (no multipart framing), creates only when nothing is there
//! (`If-None-Match: *`), overwrites only the version the client last saw (`If-Match`, else 412),
//! and addresses the file by path, which is what the sync folder and the command line have. The
//! REST upload answers 409 for an existing name and can't check a version. To get the uploaded
//! file's record afterwards, list its folder ([`super::folders::files_in`]).

use serde_json::json;

use super::resources::{File, encode};
use super::{ApiError, json as read_json};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

fn at(server: &ServerUrl, id: &str, rest: &str) -> String {
    server.join(&format!("api/files/{}{rest}", encode(id)))
}

/// `GET /api/files/{id}?metadata=true`: the file's record. (`/api/files/{id}/metadata` is only
/// its EXIF data.)
pub fn info(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}?metadata=true", at(server, id, "")))
}

/// Reads a file record (the answer of info, rename and move).
pub fn parse_file(resp: &HttpResponse) -> Result<File, ApiError> {
    read_json(resp)
}

/// `GET /api/files/{id}`: the bytes.
pub fn download(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, at(server, id, ""))
}

/// `PUT /api/files/{id}/rename`.
pub fn rename(server: &ServerUrl, id: &str, name: &str) -> HttpRequest {
    HttpRequest::new(Method::Put, at(server, id, "/rename")).json(&json!({"name": name}))
}

/// `PUT /api/files/{id}/move`, into another folder.
pub fn move_to(server: &ServerUrl, id: &str, folder_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Put, at(server, id, "/move")).json(&json!({"folder_id": folder_id}))
}

/// `DELETE /api/files/{id}`: moves the file to the trash (204). (WebDAV's `DELETE` deletes for
/// good instead.)
pub fn trash(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, at(server, id, ""))
}

/// Thumbnail sizes the server makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbnailSize {
    Icon,
    Preview,
    Large,
}

impl ThumbnailSize {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Preview => "preview",
            Self::Large => "large",
        }
    }
}

/// `GET /api/files/{id}/thumbnail/{size}`.
pub fn thumbnail(server: &ServerUrl, id: &str, size: ThumbnailSize) -> HttpRequest {
    HttpRequest::new(Method::Get, at(server, id, &format!("/thumbnail/{}", size.as_str())))
}

/// The image bytes (WebP or JPEG, see `Content-Type`), or `None` (204) when this kind of file has
/// no thumbnail.
pub fn parse_thumbnail(resp: &HttpResponse) -> Result<Option<Vec<u8>>, ApiError> {
    super::expect_success(resp)?;
    Ok((resp.status != 204 && !resp.body.is_empty()).then(|| resp.body.clone()))
}

#[cfg(test)]
mod tests {
    use super::{ThumbnailSize, download, info, move_to, parse_file, parse_thumbnail, rename, thumbnail, trash};
    use crate::ServerUrl;
    use crate::api::fixtures::read;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8095").unwrap()
    }

    #[test]
    fn requests() {
        let s = server();
        assert_eq!((info(&s, "f1").method, info(&s, "f1").url.as_str()), (Method::Get, "http://127.0.0.1:8095/api/files/f1?metadata=true"));
        assert_eq!(download(&s, "f1").url, "http://127.0.0.1:8095/api/files/f1");
        let r = rename(&s, "f1", "plan v2.txt");
        assert_eq!((r.method, r.url.as_str()), (Method::Put, "http://127.0.0.1:8095/api/files/f1/rename"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&r.body).unwrap(), serde_json::json!({"name": "plan v2.txt"}));
        let m = move_to(&s, "f1", "d2");
        assert_eq!((m.method, m.url.as_str()), (Method::Put, "http://127.0.0.1:8095/api/files/f1/move"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&m.body).unwrap(), serde_json::json!({"folder_id": "d2"}));
        let t = trash(&s, "f1");
        assert_eq!((t.method, t.url.as_str()), (Method::Delete, "http://127.0.0.1:8095/api/files/f1"));
        assert_eq!(thumbnail(&s, "f1", ThumbnailSize::Preview).url, "http://127.0.0.1:8095/api/files/f1/thumbnail/preview");
    }

    #[test]
    fn reads_file_records() {
        let up = parse_file(&HttpResponse::new(201, read("file-upload.json"))).unwrap();
        assert_eq!(
            (up.name.as_str(), up.path.as_str(), up.size, up.mime_type.as_str()),
            ("plan.txt", "/Personal/Documents/plan.txt", 8, "text/plain")
        );
        assert_eq!(up.content_hash.len(), 64);
        let renamed = parse_file(&HttpResponse::new(200, read("file-rename.json"))).unwrap();
        assert_eq!(renamed.name, "plan v2.txt");
        assert_eq!(renamed.content_hash, "", "OxiCloud leaves it blank after a rename");
        let moved = parse_file(&HttpResponse::new(200, read("file-move.json"))).unwrap();
        assert_eq!(moved.path, "/Personal/Reports/plan v2.txt");
    }

    #[test]
    fn some_files_have_no_thumbnail() {
        assert_eq!(parse_thumbnail(&HttpResponse::new(204, Vec::new())), Ok(None));
        assert_eq!(parse_thumbnail(&HttpResponse::new(200, b"RIFF".to_vec())), Ok(Some(b"RIFF".to_vec())));
        assert!(parse_thumbnail(&HttpResponse::new(404, Vec::new())).is_err());
    }
}
