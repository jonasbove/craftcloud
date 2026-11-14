//! The file and folder records OxiCloud's REST API answers with, and its cursor-paged listings.
//!
//! Field names are the server's own. A few answers leave fields blank (a rename or move answers
//! with an empty `content_hash` and an `etag` like `-1794606853`; folder listings send `path: ""`),
//! so those fields default instead of failing the parse.
//!
//! **Paths.** REST paths include the drive's root folder (`/Personal/Documents/notes.md`, or
//! `Personal/Documents/notes.md` in favourites, recent and trash), while WebDAV and CraftCloud's
//! cloud paths start below it (`Documents/notes.md`). [`cloud_path`] turns one into the other.

use serde::Deserialize;
use serde_json::Value;

use super::{ApiError, json};
use crate::http::HttpResponse;

/// File or folder, as the API's `item_type` / `resource_type` and its URLs spell it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    File,
    Folder,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "file" => Some(Self::File),
            "folder" => Some(Self::Folder),
            _ => None,
        }
    }
}

/// `FolderDto`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// `/Personal/Documents` (empty in folder listings).
    #[serde(default)]
    pub path: String,
    /// `None` for a drive's root folder.
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub drive_id: String,
    /// RFC 3339.
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub etag: String,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub is_shared: bool,
}

/// `FileDto`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct File {
    pub id: String,
    pub name: String,
    /// `/Personal/Documents/notes.md` (empty in folder listings).
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub modified_at: String,
    /// A stable machine key ("Image", "Markdown", "Text", …), not display text.
    #[serde(default)]
    pub category: String,
    /// BLAKE3, hex; empty in rename and move answers.
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub etag: String,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub is_shared: bool,
}

/// A file or a folder, from a mixed listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    File(File),
    Folder(Folder),
}

impl Resource {
    pub fn kind(&self) -> Kind {
        match self {
            Self::File(_) => Kind::File,
            Self::Folder(_) => Kind::Folder,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::File(f) => &f.id,
            Self::Folder(f) => &f.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::File(f) => &f.name,
            Self::Folder(f) => &f.name,
        }
    }

    /// The server's path, as sent (may be empty).
    pub fn server_path(&self) -> &str {
        match self {
            Self::File(f) => &f.path,
            Self::Folder(f) => &f.path,
        }
    }

    pub fn modified_at(&self) -> &str {
        match self {
            Self::File(f) => &f.modified_at,
            Self::Folder(f) => &f.modified_at,
        }
    }

    pub fn is_favorite(&self) -> bool {
        match self {
            Self::File(f) => f.is_favorite,
            Self::Folder(f) => f.is_favorite,
        }
    }

    /// Bytes for a file, `None` for a folder.
    pub fn size(&self) -> Option<u64> {
        match self {
            Self::File(f) => Some(f.size),
            Self::Folder(_) => None,
        }
    }

    /// The cloud path (`Documents/notes.md`), when the server sent a path.
    pub fn cloud_path(&self) -> Option<String> {
        let p = self.server_path();
        (!p.is_empty()).then(|| cloud_path(p))
    }

    /// Reads one listed resource; `None` for kinds CraftCloud doesn't show (drives, calendars, …).
    pub(crate) fn from_listing(resource_type: &str, resource: Value) -> Result<Option<Self>, ApiError> {
        let read = |e: serde_json::Error| ApiError::Body(format!("{resource_type}: {e}"));
        Ok(match Kind::parse(resource_type) {
            Some(Kind::File) => Some(Self::File(serde_json::from_value(resource).map_err(read)?)),
            Some(Kind::Folder) => Some(Self::Folder(serde_json::from_value(resource).map_err(read)?)),
            None => None,
        })
    }
}

/// `/Personal/Documents/notes.md` or `Personal/Documents/notes.md` → `Documents/notes.md`; the
/// drive root itself → `""`.
pub fn cloud_path(server_path: &str) -> String {
    let mut parts = server_path.split('/').filter(|s| !s.is_empty());
    parts.next();
    parts.collect::<Vec<_>>().join("/")
}

/// Whether a cloud path is CraftCloud's own bookkeeping (`.craftcloud/…`: versions and the like),
/// which listings, search, recent and starred leave out unless asked. Takes a cloud path
/// (`Documents/a.txt`); for a server path use [`Resource::is_internal`].
pub fn is_internal(cloud_path: &str) -> bool {
    super::dav::is_craftcloud_internal(cloud_path)
}

impl Resource {
    /// Whether this is CraftCloud's own (see [`is_internal`]); a resource without a path (in a
    /// folder listing) counts by its name only when it is the internal folder itself.
    pub fn is_internal(&self) -> bool {
        match self.cloud_path() {
            Some(p) => is_internal(&p),
            None => is_internal(self.name()),
        }
    }
}

/// `page` without CraftCloud's own items (see [`is_internal`]).
pub(crate) fn visible<T>(mut page: Page<T>, resource: impl Fn(&T) -> &Resource) -> Page<T> {
    page.items.retain(|i| !resource(i).is_internal());
    page
}

/// One page of a cursor-paged listing. `next_cursor` is `None` on the last page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// A listed item before its resource is read: `{resource_type, resource, …extra fields}`.
#[derive(Deserialize)]
pub(crate) struct RawItem<X> {
    pub resource_type: String,
    pub resource: Value,
    #[serde(flatten)]
    pub extra: X,
}

#[derive(Deserialize)]
struct RawPage<X> {
    items: Vec<RawItem<X>>,
    #[serde(default)]
    next_cursor: Option<String>,
}

/// Reads a cursor page, building each item from its resource and its extra fields; resources of
/// other kinds are skipped.
pub(crate) fn parse_page<X, T>(resp: &HttpResponse, build: impl Fn(Resource, X) -> T) -> Result<Page<T>, ApiError>
where
    X: serde::de::DeserializeOwned,
{
    let raw: RawPage<X> = json(resp)?;
    let mut items = Vec::with_capacity(raw.items.len());
    for item in raw.items {
        if let Some(resource) = Resource::from_listing(&item.resource_type, item.resource)? {
            items.push(build(resource, item.extra));
        }
    }
    Ok(Page { items, next_cursor: raw.next_cursor })
}

/// No extra fields.
#[derive(Deserialize)]
pub(crate) struct NoExtra {}

/// `?limit=…&cursor=…` (with a leading `?`, or empty).
pub(crate) fn page_query(limit: Option<u32>, cursor: Option<&str>) -> String {
    let mut q = Vec::new();
    if let Some(l) = limit {
        q.push(format!("limit={l}"));
    }
    if let Some(c) = cursor {
        q.push(format!("cursor={}", encode(c)));
    }
    if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) }
}

/// Percent-encodes a query value or a path segment.
pub(crate) fn encode(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[cfg(test)]
mod tests {
    use super::{Kind, cloud_path, page_query};

    #[test]
    fn server_paths_become_cloud_paths() {
        assert_eq!(cloud_path("/Personal/Documents/plan v2.txt"), "Documents/plan v2.txt");
        assert_eq!(cloud_path("Personal/Documents"), "Documents");
        assert_eq!(cloud_path("/Personal"), "");
        assert_eq!(cloud_path(""), "");
    }

    #[test]
    fn craftcloud_bookkeeping_is_internal() {
        use super::is_internal;
        assert!(is_internal(".craftcloud"));
        assert!(is_internal(".craftcloud/versions/T011/zebra.txt/20261113T000000.000Z"));
        assert!(!is_internal("T011/zebra.txt"));
        assert!(!is_internal("Documents/.craftcloud"), "only CraftCloud's own folder at the top");
        let found = crate::api::search::parse_search_all(&crate::http::HttpResponse::new(
            200,
            br#"{"items":[{"resource_type":"folder","resource":{"id":"1","name":"zebra.txt","path":"/Personal/.craftcloud/versions/T011/zebra.txt"},"meta":{"score":80}},
                         {"resource_type":"file","resource":{"id":"2","name":"zebra.txt","path":"/Personal/T011/zebra.txt"},"meta":{"score":80}}],"query_time_ms":1}"#
                .to_vec(),
        ))
        .unwrap();
        let shown: Vec<bool> = found.items.iter().map(|h| h.resource.is_internal()).collect();
        assert_eq!(shown, [true, false], "a version folder is named like the file it keeps");
    }

    #[test]
    fn kinds_and_paging() {
        assert_eq!(Kind::parse("folder"), Some(Kind::Folder));
        assert_eq!(Kind::parse("drive"), None);
        assert_eq!(Kind::File.as_str(), "file");
        assert_eq!(page_query(None, None), "");
        assert_eq!(page_query(Some(50), Some("ab=c")), "?limit=50&cursor=ab%3Dc");
    }
}
