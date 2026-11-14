//! Search by name: `GET /api/search` (paged, with filters) and `/api/search/suggest` (quick
//! prefix matches for a search box).

use serde::Deserialize;

use super::resources::{Resource, encode};
use super::{ApiError, json};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// The server's category filter (`?type=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeFilter {
    Folder,
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Code,
    Font,
}

impl TypeFilter {
    pub const ALL: [Self; 8] = [Self::Folder, Self::Image, Self::Video, Self::Audio, Self::Document, Self::Archive, Self::Code, Self::Font];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::Archive => "archive",
            Self::Code => "code",
            Self::Font => "font",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// What to search for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub kind: Option<TypeFilter>,
    /// Only inside this folder (and below it).
    pub folder_id: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

impl Query {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_owned(), ..Self::default() }
    }
}

/// `GET /api/search?query=…`.
pub fn search(server: &ServerUrl, q: &Query) -> HttpRequest {
    let mut params = vec![format!("query={}", encode(&q.text))];
    if let Some(k) = q.kind {
        params.push(format!("type={}", k.as_str()));
    }
    if let Some(f) = &q.folder_id {
        params.push(format!("folder_id={}&recursive=true", encode(f)));
    }
    if let Some(l) = q.limit {
        params.push(format!("limit={l}"));
    }
    if let Some(c) = &q.cursor {
        params.push(format!("cursor={}", encode(c)));
    }
    HttpRequest::new(Method::Get, format!("{}?{}", server.join("api/search"), params.join("&")))
}

/// One match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Its `path` is `/Personal/…`.
    pub resource: Resource,
    /// Higher is better.
    pub score: i64,
}

/// A page of matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Results {
    pub items: Vec<Hit>,
    pub next_cursor: Option<String>,
    /// About how many match in all, when the server knows.
    pub total: Option<u64>,
    pub query_time_ms: u64,
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    score: i64,
}

#[derive(Deserialize)]
struct Extra {
    #[serde(default)]
    meta: Option<Meta>,
}

#[derive(Deserialize)]
struct Body {
    items: Vec<super::resources::RawItem<Extra>>,
    #[serde(default)]
    next_cursor: Option<String>,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    query_time_ms: u64,
}

/// The matches, without CraftCloud's own data (search also finds the version folders, which are
/// named like the files they keep); [`parse_search_all`] keeps them.
pub fn parse_search(resp: &HttpResponse) -> Result<Results, ApiError> {
    let mut results = parse_search_all(resp)?;
    results.items.retain(|h| !h.resource.is_internal());
    Ok(results)
}

/// [`parse_search`], including CraftCloud's own items.
pub fn parse_search_all(resp: &HttpResponse) -> Result<Results, ApiError> {
    let body: Body = json(resp)?;
    let mut items = Vec::new();
    for item in body.items {
        if let Some(resource) = Resource::from_listing(&item.resource_type, item.resource)? {
            items.push(Hit { resource, score: item.extra.meta.map_or(0, |m| m.score) });
        }
    }
    Ok(Results { items, next_cursor: body.next_cursor, total: body.total, query_time_ms: body.query_time_ms })
}

/// A quick suggestion.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub name: String,
    /// `file` or `folder`.
    pub item_type: String,
    pub path: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub relevance_score: i64,
}

/// `GET /api/search/suggest?query=…`.
pub fn suggest(server: &ServerUrl, text: &str, limit: Option<u32>) -> HttpRequest {
    let limit = limit.map(|l| format!("&limit={l}")).unwrap_or_default();
    HttpRequest::new(Method::Get, format!("{}?query={}{limit}", server.join("api/search/suggest"), encode(text)))
}

#[derive(Deserialize)]
struct SuggestBody {
    suggestions: Vec<Suggestion>,
}

/// Suggestions, without CraftCloud's own data; [`parse_suggest_all`] keeps it.
pub fn parse_suggest(resp: &HttpResponse) -> Result<Vec<Suggestion>, ApiError> {
    let mut all = parse_suggest_all(resp)?;
    all.retain(|s| !super::resources::is_internal(&super::resources::cloud_path(&s.path)));
    Ok(all)
}

/// [`parse_suggest`], including CraftCloud's own items.
pub fn parse_suggest_all(resp: &HttpResponse) -> Result<Vec<Suggestion>, ApiError> {
    json::<SuggestBody>(resp).map(|b| b.suggestions)
}

#[cfg(test)]
mod tests {
    use super::{Query, TypeFilter, parse_search, parse_suggest, search, suggest};
    use crate::ServerUrl;
    use crate::api::fixtures::read;
    use crate::api::resources::Kind;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8095").unwrap()
    }

    /// Recorded: searching "Notes" with one stored version finds its version folder and the
    /// version itself, which no list shows (T-011); the `_all` parsers keep them for `--all`.
    #[test]
    fn craftcloud_data_is_left_out() {
        let resp = HttpResponse::new(200, read("search-with-versions.json"));
        let shown: Vec<String> = parse_search(&resp).unwrap().items.iter().filter_map(|h| h.resource.cloud_path()).collect();
        assert_eq!(shown, ["Documents/Notes.md"]);
        assert_eq!(super::parse_search_all(&resp).unwrap().items.len(), 3);
        let suggested = br#"{"suggestions":[
            {"id":"1","name":"Notes.md","item_type":"folder","path":"/Personal/.craftcloud/versions/Documents/Notes.md"},
            {"id":"2","name":"Notes.md","item_type":"file","path":"/Personal/Documents/Notes.md"}]}"#;
        let resp = HttpResponse::new(200, suggested.to_vec());
        assert_eq!(parse_suggest(&resp).unwrap().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["2"]);
        assert_eq!(super::parse_suggest_all(&resp).unwrap().len(), 2);
    }

    #[test]
    fn listings_leave_it_out_too() {
        use crate::api::{favorites, folders, trash};
        let item = |path: &str| format!(r#"{{"resource_type":"file","resource":{{"id":"{path}","name":"a","path":"{path}"}}}}"#);
        let page = format!(r#"{{"items":[{},{}]}}"#, item("Personal/.craftcloud/versions/a/x"), item("Personal/a"));
        let resp = HttpResponse::new(200, page.into_bytes());
        assert_eq!(favorites::parse_recent(&resp).unwrap().items.len(), 1);
        assert_eq!(favorites::parse_recent_all(&resp).unwrap().items.len(), 2);
        assert_eq!(favorites::parse_list(&resp).unwrap().items.len(), 1);
        assert_eq!(trash::parse_list(&resp).unwrap().items.len(), 1);
        assert_eq!(trash::parse_list_all(&resp).unwrap().items.len(), 2);
        let root = br#"{"items":[{"resource_type":"folder","resource":{"id":"c","name":".craftcloud"}},{"resource_type":"folder","resource":{"id":"d","name":"Documents"}}]}"#;
        let resp = HttpResponse::new(200, root.to_vec());
        assert_eq!(folders::parse_contents(&resp).unwrap().items.len(), 1);
        assert_eq!(folders::parse_contents_all(&resp).unwrap().items.len(), 2);
    }

    #[test]
    fn requests() {
        let s = server();
        let r = search(&s, &Query::new("plan v2"));
        assert_eq!((r.method, r.url.as_str()), (Method::Get, "http://127.0.0.1:8095/api/search?query=plan%20v2"));
        let q =
            Query { kind: Some(TypeFilter::Folder), folder_id: Some("d1".into()), limit: Some(10), cursor: Some("c".into()), ..Query::new("Doc") };
        assert_eq!(search(&s, &q).url, "http://127.0.0.1:8095/api/search?query=Doc&type=folder&folder_id=d1&recursive=true&limit=10&cursor=c");
        assert_eq!(suggest(&s, "pl", Some(5)).url, "http://127.0.0.1:8095/api/search/suggest?query=pl&limit=5");
        assert_eq!(TypeFilter::parse("image"), Some(TypeFilter::Image));
        assert_eq!(TypeFilter::parse("pdf"), None);
    }

    #[test]
    fn reads_matches() {
        let found = parse_search(&HttpResponse::new(200, read("search.json"))).unwrap();
        assert_eq!(found.total, Some(1));
        assert_eq!(found.items[0].resource.cloud_path().as_deref(), Some("Reports/plan v2.txt"));
        assert_eq!(found.items[0].score, 80);
        let folders = parse_search(&HttpResponse::new(200, read("search-folders.json"))).unwrap();
        assert_eq!(folders.items[0].resource.kind(), Kind::Folder);
        assert!(parse_suggest(&HttpResponse::new(200, read("search-suggest.json"))).unwrap().is_empty());
    }
}
