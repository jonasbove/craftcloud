//! Many files and folders at once: move, copy, and move to the trash.
//!
//! A batch answers 200 when everything worked, 206 when some items failed, and 4xx/507 when all of
//! them failed (for one reason) — always with the same `{successful, failed, stats}` body, so
//! [`parse`] reads that body whatever the status.

use serde::Deserialize;
use serde_json::{Value, json};

use super::ApiError;
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// `POST /api/batch/files/move`.
pub fn move_files(server: &ServerUrl, file_ids: &[String], target_folder_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/batch/files/move")).json(&json!({"file_ids": file_ids, "target_folder_id": target_folder_id}))
}

/// `POST /api/batch/files/copy`.
pub fn copy_files(server: &ServerUrl, file_ids: &[String], target_folder_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/batch/files/copy")).json(&json!({"file_ids": file_ids, "target_folder_id": target_folder_id}))
}

/// `POST /api/batch/folders/move`.
pub fn move_folders(server: &ServerUrl, folder_ids: &[String], target_folder_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/batch/folders/move"))
        .json(&json!({"folder_ids": folder_ids, "target_folder_id": target_folder_id, "recursive": true}))
}

/// `POST /api/batch/folders/copy`: whole folder trees.
pub fn copy_folders(server: &ServerUrl, folder_ids: &[String], target_folder_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/batch/folders/copy"))
        .json(&json!({"folder_ids": folder_ids, "target_folder_id": target_folder_id, "recursive": true}))
}

/// `POST /api/batch/trash`: files and folders to the trash.
pub fn trash(server: &ServerUrl, file_ids: &[String], folder_ids: &[String]) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/batch/trash")).json(&json!({"file_ids": file_ids, "folder_ids": folder_ids}))
}

/// `GET /api/batch/download?file_ids=…&folder_ids=…`: files and folders as one zip
/// (`Content-Disposition: attachment`). A GET, so a browser can download it by following the link.
pub fn download(server: &ServerUrl, file_ids: &[String], folder_ids: &[String]) -> HttpRequest {
    let list = |ids: &[String]| ids.iter().map(|i| super::resources::encode(i)).collect::<Vec<_>>().join(",");
    HttpRequest::new(Method::Get, format!("{}?file_ids={}&folder_ids={}", server.join("api/batch/download"), list(file_ids), list(folder_ids)))
}

/// One item that failed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Failure {
    pub id: String,
    pub error: String,
    #[serde(default)]
    pub error_type: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Stats {
    pub total: u64,
    pub successful: u64,
    pub failed: u64,
}

/// What a batch did. `successful` differs per operation: file records for file moves and copies,
/// ids for the trash, `{new_root_folder_id, folders_copied, files_copied}` for folder copies.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct BatchResult {
    #[serde(default)]
    pub successful: Vec<Value>,
    #[serde(default)]
    pub failed: Vec<Failure>,
    #[serde(default)]
    pub stats: Stats,
}

impl BatchResult {
    pub fn all_done(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Reads a batch answer, whatever its status; only an answer without a batch body is an error.
pub fn parse(resp: &HttpResponse) -> Result<BatchResult, ApiError> {
    match serde_json::from_slice::<BatchResult>(&resp.body) {
        Ok(result) if resp.is_success() || !result.failed.is_empty() => Ok(result),
        _ => {
            super::expect_success(resp)?;
            Err(ApiError::Body("not a batch answer".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{copy_files, copy_folders, move_files, move_folders, parse, trash};
    use crate::ServerUrl;
    use crate::api::fixtures::read;
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
        let ids = vec!["a".to_owned(), "b".to_owned()];
        let d = super::download(&s, &ids, &["c".to_owned()]);
        assert_eq!((d.method, d.url.as_str()), (Method::Get, "http://127.0.0.1:8095/api/batch/download?file_ids=a,b&folder_ids=c"));
        let m = move_files(&s, &ids, "t");
        assert_eq!((m.method, m.url.as_str()), (Method::Post, "http://127.0.0.1:8095/api/batch/files/move"));
        assert_eq!(body(&m), serde_json::json!({"file_ids": ["a", "b"], "target_folder_id": "t"}));
        assert_eq!(copy_files(&s, &ids, "t").url, "http://127.0.0.1:8095/api/batch/files/copy");
        assert_eq!(body(&move_folders(&s, &ids, "t"))["folder_ids"], serde_json::json!(["a", "b"]));
        assert_eq!(copy_folders(&s, &ids, "t").url, "http://127.0.0.1:8095/api/batch/folders/copy");
        let t = trash(&s, &ids, &[]);
        assert_eq!(
            (t.url.as_str(), body(&t)),
            ("http://127.0.0.1:8095/api/batch/trash", serde_json::json!({"file_ids": ["a", "b"], "folder_ids": []}))
        );
    }

    #[test]
    fn reads_every_kind_of_answer() {
        let copied = parse(&HttpResponse::new(200, read("batch-copy.json"))).unwrap();
        assert!(copied.all_done());
        assert_eq!(copied.stats.successful, 2);
        assert_eq!(copied.successful[0]["path"], "/Personal/Reports/a.txt");
        assert_eq!(parse(&HttpResponse::new(200, read("batch-trash.json"))).unwrap().successful.len(), 1);
        parse(&HttpResponse::new(200, read("batch-move.json"))).unwrap();
        assert_eq!(parse(&HttpResponse::new(200, read("batch-folders-copy.json"))).unwrap().successful[0]["folders_copied"], 1);

        let failed = parse(&HttpResponse::new(404, read("batch-failed.json"))).unwrap();
        assert!(!failed.all_done());
        assert_eq!(failed.failed[0].error_type, "Not Found");
        assert!(parse(&HttpResponse::new(401, br#"{"error":"Invalid token"}"#.to_vec())).is_err());
    }
}
