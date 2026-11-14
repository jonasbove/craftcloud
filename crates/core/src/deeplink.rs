//! `craftcloud://` links, which open a synced file from outside the client (a web page, another
//! app, a chat message):
//!
//! - `craftcloud://open?path=<cloud path>` opens the local copy with the system's app for it;
//! - `craftcloud://reveal?path=<cloud path>` shows it in the file manager.
//!
//! The path is the cloud path (`Documents/Q3 report.docx`, percent-encoded; a leading `/` is
//! allowed). It may also follow the action as the link's own path: `craftcloud://open/Documents/
//! Q3%20report.docx` is `craftcloud://open?path=/Documents/Q3%20report.docx`. Parsing refuses anything that could point outside the sync folder; the platform
//! still checks the resolved path is inside it before opening anything.

use url::Url;

/// The URL scheme the desktop client registers.
pub const SCHEME: &str = "craftcloud";

/// What a link asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeepLink {
    /// Open the file with its app.
    Open(String),
    /// Show it in the file manager.
    Reveal(String),
}

impl DeepLink {
    /// The cloud path the link is about (`Documents/notes.md`, no leading slash).
    pub fn path(&self) -> &str {
        match self {
            Self::Open(p) | Self::Reveal(p) => p,
        }
    }

    /// The link for this action, percent-encoded.
    pub fn to_url(&self) -> String {
        let (verb, path) = match self {
            Self::Open(p) => ("open", p),
            Self::Reveal(p) => ("reveal", p),
        };
        let path: String = url::form_urlencoded::byte_serialize(path.as_bytes()).collect();
        format!("{SCHEME}://{verb}?path={path}")
    }
}

/// Why a link was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    NotALink,
    /// Not `open` or `reveal`.
    Action(String),
    /// A path that isn't a plain path inside the sync folder.
    Path,
    /// No path at all.
    NoPath,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotALink => write!(f, "not a {SCHEME}:// link"),
            Self::Action(a) => write!(f, "{SCHEME}://{a} isn't something CraftCloud does (open or reveal)"),
            Self::Path => f.write_str("the link's path isn't a file in the CraftCloud folder"),
            Self::NoPath => write!(f, "this link has no path ({SCHEME}://open?path=Documents/notes.md, or {SCHEME}://open/Documents/notes.md)"),
        }
    }
}

impl std::error::Error for LinkError {}

/// Reads a `craftcloud://` link.
pub fn parse(link: &str) -> Result<DeepLink, LinkError> {
    let url = Url::parse(link.trim()).map_err(|_| LinkError::NotALink)?;
    if url.scheme() != SCHEME {
        return Err(LinkError::NotALink);
    }
    // `craftcloud://open?…` has the action as its host; `craftcloud:open?…` as its path. What
    // follows the action in the link's own path is the cloud path, when `?path=` doesn't say it.
    let (action, rest) = match url.host_str() {
        Some(host) => (host.to_owned(), url.path().to_owned()),
        None => {
            let p = url.path().trim_start_matches('/');
            let (a, r) = p.split_once('/').unwrap_or((p, ""));
            (a.to_owned(), r.to_owned())
        }
    };
    let path = match url.query_pairs().find(|(k, _)| k == "path") {
        Some((_, v)) => v.into_owned(),
        None => {
            // The URL parser resolves `.` and `..` in a link's own path before we see it: look at
            // the link as written, so they are refused here as in `?path=`.
            let raw = link.trim().split(['?', '#']).next().unwrap_or_default();
            if percent_encoding::percent_decode_str(raw).decode_utf8_lossy().split(['/', '\\']).any(|p| p == "." || p == "..") {
                return Err(LinkError::Path);
            }
            percent_encoding::percent_decode_str(&rest).decode_utf8().map_err(|_| LinkError::Path)?.into_owned()
        }
    };
    if path.trim_matches('/').is_empty() {
        return Err(LinkError::NoPath);
    }
    let path = clean(&path).ok_or(LinkError::Path)?;
    match action.to_ascii_lowercase().as_str() {
        "open" => Ok(DeepLink::Open(path)),
        "reveal" => Ok(DeepLink::Reveal(path)),
        other => Err(LinkError::Action(other.to_owned())),
    }
}

/// A cloud path made plain: `/`-separated parts, no empty, `.` or `..` parts, no backslashes or
/// control characters, never CraftCloud's own `.craftcloud` folder.
fn clean(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    // (A drive letter or other `:` name is refused on Windows when the path is resolved.)
    let bad = |p: &&str| *p == "." || *p == ".." || p.contains('\\') || p.chars().any(char::is_control);
    if parts.is_empty() || parts.iter().any(bad) {
        return None;
    }
    let clean = parts.join("/");
    if crate::api::dav::is_craftcloud_internal(&clean) {
        return None;
    }
    Some(clean)
}

#[cfg(test)]
mod tests {
    use super::{DeepLink, LinkError, parse};

    /// T-026: the path may follow the action; no path at all is said as such.
    #[test]
    fn the_path_may_follow_the_action() {
        assert_eq!(parse("craftcloud://open/t16/after.txt"), Ok(DeepLink::Open("t16/after.txt".into())));
        assert_eq!(parse("craftcloud://reveal/Documents/Q3%20report.docx"), Ok(DeepLink::Reveal("Documents/Q3 report.docx".into())));
        assert_eq!(parse("craftcloud:open/t16/after.txt"), Ok(DeepLink::Open("t16/after.txt".into())));
        assert_eq!(parse("craftcloud://open/a/%2E%2E/b"), Err(LinkError::Path), "checked the same way");
        assert_eq!(parse("craftcloud://open/.craftcloud/versions/a"), Err(LinkError::Path));
        assert_eq!(parse("craftcloud://open"), Err(LinkError::NoPath));
        assert_eq!(parse("craftcloud://open/"), Err(LinkError::NoPath));
        assert!(LinkError::NoPath.to_string().starts_with("this link has no path"));
    }

    #[test]
    fn open_and_reveal_links_are_read() {
        assert_eq!(parse("craftcloud://open?path=Documents%2FQ3%20report.docx"), Ok(DeepLink::Open("Documents/Q3 report.docx".into())));
        assert_eq!(parse("craftcloud://open?path=/Documents/Q3+report.docx"), Ok(DeepLink::Open("Documents/Q3 report.docx".into())));
        assert_eq!(parse("craftcloud://reveal?path=%C3%89t%C3%A9/fichier.txt"), Ok(DeepLink::Reveal("Été/fichier.txt".into())));
        assert_eq!(parse("craftcloud:open?path=a.md"), Ok(DeepLink::Open("a.md".into())));
        assert_eq!(parse("  CRAFTCLOUD://OPEN?path=a.md "), Ok(DeepLink::Open("a.md".into())));
    }

    #[test]
    fn links_that_could_leave_the_folder_are_refused() {
        for bad in [
            "craftcloud://open?path=../etc/passwd",
            "craftcloud://open?path=Documents/../../x",
            "craftcloud://open?path=%2E%2E/x",
            "craftcloud://open?path=a%5C..%5Cb",
            "craftcloud://open?path=a%00b",
            "craftcloud://open?path=.craftcloud/state.json",
        ] {
            assert_eq!(parse(bad), Err(LinkError::Path), "{bad}");
        }
        for empty in ["craftcloud://open?path=", "craftcloud://open?path=/", "craftcloud://open"] {
            assert_eq!(parse(empty), Err(LinkError::NoPath), "{empty}");
        }
        assert_eq!(parse("craftcloud://delete?path=a.md"), Err(LinkError::Action("delete".into())));
        assert_eq!(parse("https://cloud.example.org/?path=a.md"), Err(LinkError::NotALink));
        assert_eq!(parse("not a link"), Err(LinkError::NotALink));
    }

    #[test]
    fn links_round_trip() {
        for link in [DeepLink::Open("Documents/Q3 report.docx".into()), DeepLink::Reveal("Été/a&b=c?.txt".into())] {
            assert_eq!(parse(&link.to_url()), Ok(link.clone()));
        }
    }
}
