//! The rows the file browser shows, from whichever listing they came (a folder, recent, starred,
//! search, trash), and how they sort.

use craftcloud_core::api::resources::{Kind, Resource};

use super::apps::{self, App};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub id: String,
    pub name: String,
    /// Bytes; `None` for folders.
    pub size: Option<u64>,
    /// RFC 3339.
    pub modified: String,
    pub mime: String,
    /// The server's category ("Image", "Text", "Markdown", …).
    pub category: String,
    pub starred: bool,
    pub shared: bool,
    /// Cloud path (`Documents/notes.md`), when known.
    pub path: String,
    /// The version tag WebDAV expects in `If-Match` (without the quotes).
    pub etag: String,
    /// The folder it is in, when known.
    pub parent: Option<String>,
    /// When the trash deletes it for good (trash view only).
    pub expires: Option<String>,
    /// When it went to the trash (trash view only), RFC 3339.
    pub trashed: Option<String>,
    /// When it was last opened (Recent only), RFC 3339.
    pub opened: Option<String>,
}

impl Item {
    /// From a listing; `path` is the cloud path when the listing didn't send one.
    pub fn from_resource(r: Resource, path_if_missing: impl FnOnce(&str) -> String) -> Self {
        let path = r.cloud_path().unwrap_or_else(|| path_if_missing(r.name()));
        match r {
            Resource::File(f) => Self {
                kind: Kind::File,
                id: f.id,
                name: f.name,
                size: Some(f.size),
                modified: f.modified_at,
                mime: f.mime_type,
                category: f.category,
                starred: f.is_favorite,
                shared: f.is_shared,
                path,
                etag: f.etag,
                parent: f.folder_id,
                expires: None,
                trashed: None,
                opened: None,
            },
            Resource::Folder(f) => Self {
                kind: Kind::Folder,
                id: f.id,
                name: f.name,
                size: None,
                modified: f.modified_at,
                mime: String::new(),
                category: "Folder".into(),
                starred: f.is_favorite,
                shared: f.is_shared,
                path,
                etag: f.etag,
                parent: f.parent_id,
                expires: None,
                trashed: None,
                opened: None,
            },
        }
    }

    pub fn is_folder(&self) -> bool {
        self.kind == Kind::Folder
    }

    /// The Craft app that opens it (files only).
    pub fn app(&self) -> Option<&'static App> {
        if self.is_folder() { None } else { apps::for_name(&self.name) }
    }

    /// Whether the server makes thumbnails of it.
    pub fn has_thumbnail(&self) -> bool {
        !self.is_folder() && (self.category == "Image" || self.mime.starts_with("image/")) && !self.mime.contains("svg")
    }

    /// Whether a text preview makes sense.
    #[allow(dead_code)] // For the details and previews that follow.
    pub fn is_text(&self) -> bool {
        !self.is_folder()
            && (self.mime.starts_with("text/")
                || matches!(self.category.as_str(), "Text" | "Markdown" | "Code")
                || apps::extension(&self.name)
                    .is_some_and(|e| matches!(e.as_str(), "md" | "txt" | "csv" | "tsv" | "json" | "toml" | "yaml" | "yml" | "log")))
    }

    /// "DOCX document", "PNG image", "Folder" — what kind of file, in words. A file is named by its
    /// format, never by another vendor's product: the Craft app that opens it is said beside it.
    #[allow(dead_code)] // For the details and previews that follow.
    pub fn kind_label(&self) -> String {
        if self.is_folder() {
            return "Folder".into();
        }
        let ext = apps::extension(&self.name).unwrap_or_default();
        let format = ext.to_uppercase();
        match ext.as_str() {
            "docx" | "doc" | "docm" | "odt" | "rtf" => return format!("{format} document"),
            "xlsx" | "xls" | "xlsm" | "ods" => return format!("{format} spreadsheet"),
            "pptx" | "ppsx" | "potx" | "odp" => return format!("{format} presentation"),
            "deckcraft" => return "DeckCraft presentation".into(),
            _ => {}
        }
        let known = match ext.as_str() {
            "csv" | "tsv" => "Table",
            "pdf" => "PDF document",
            "md" => "Markdown",
            "txt" => "Text",
            "zip" | "tar" | "gz" | "7z" | "rar" => "Archive",
            _ => "",
        };
        if !known.is_empty() {
            return known.into();
        }
        let family = match self.category.as_str() {
            "Image" => "image",
            "Video" => "video",
            "Audio" => "audio",
            _ if self.mime.starts_with("image/") => "image",
            _ if self.mime.starts_with("video/") => "video",
            _ if self.mime.starts_with("audio/") => "audio",
            _ => "file",
        };
        if ext.is_empty() { format!("{}{}", family[..1].to_uppercase(), &family[1..]) } else { format!("{} {family}", ext.to_uppercase()) }
    }
}

/// What the list sorts by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Modified,
    Size,
    /// When it was last opened (Recent), newest first by default.
    Opened,
    /// When the trash deletes it for good (Trash), soonest first.
    Expires,
    /// When it went to the trash (Trash), newest first by default.
    Trashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub ascending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self { key: SortKey::Name, ascending: true }
    }
}

impl Sort {
    /// Recent's own order, as the server sends it: last opened first.
    pub const RECENT: Self = Self { key: SortKey::Opened, ascending: false };
    /// The trash's own order, as the server sends it: soonest deleted first.
    pub const TRASH: Self = Self { key: SortKey::Expires, ascending: true };

    /// Clicking a column: the same one flips the order, another one starts in its natural order.
    pub fn click(self, key: SortKey) -> Self {
        if self.key == key {
            Self { key, ascending: !self.ascending }
        } else {
            Self { key, ascending: matches!(key, SortKey::Name | SortKey::Expires) }
        }
    }
}

/// Folders first, then by `sort` (names compare without case, numbers by value: "file 2" < "file 10").
pub fn sort_items(items: &mut [Item], sort: Sort) {
    items.sort_by(|a, b| {
        // Recent and Trash keep their time order, folders among files.
        let folders_first = if matches!(sort.key, SortKey::Opened | SortKey::Expires | SortKey::Trashed) {
            std::cmp::Ordering::Equal
        } else {
            b.is_folder().cmp(&a.is_folder())
        };
        let by = match sort.key {
            SortKey::Name => natural(&a.name, &b.name),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortKey::Opened => a.opened.cmp(&b.opened),
            SortKey::Expires => a.expires.cmp(&b.expires),
            SortKey::Trashed => a.trashed.cmp(&b.trashed),
        };
        let by = if sort.ascending { by } else { by.reverse() };
        folders_first.then(by).then_with(|| natural(&a.name, &b.name))
    });
}

/// Compares names like a person would: case-insensitive, runs of digits by value.
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// `folder` + `name` as a cloud path.
pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() { name.to_owned() } else { format!("{folder}/{name}") }
}

/// The name for a copy that doesn't clash: `plan.txt` → `plan (2).txt`.
pub fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

/// A name the server accepts: not empty, no slashes, not `.`/`..`.
pub fn name_problem(name: &str) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        Some("Enter a name.")
    } else if n.contains('/') || n.contains('\\') {
        Some("Names can't contain / or \\.")
    } else if n == "." || n == ".." {
        Some("That name is reserved.")
    } else {
        None
    }
}

/// A MIME type for an upload when the browser didn't say.
pub fn guess_mime(name: &str) -> &'static str {
    match apps::extension(name).as_deref() {
        Some("txt" | "log") => "text/plain",
        Some("md") => "text/markdown",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("zip") => "application/zip",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::{Item, Sort, SortKey, guess_mime, name_problem, natural, numbered, sort_items};
    use craftcloud_core::api::resources::Kind;

    fn item(kind: Kind, name: &str, size: u64, modified: &str) -> Item {
        Item {
            kind,
            id: name.into(),
            name: name.into(),
            size: (kind == Kind::File).then_some(size),
            modified: modified.into(),
            mime: String::new(),
            category: String::new(),
            starred: false,
            shared: false,
            path: name.into(),
            etag: String::new(),
            parent: None,
            expires: None,
            trashed: None,
            opened: None,
        }
    }

    #[test]
    fn folders_come_first_then_the_chosen_order() {
        let mut items = vec![
            item(Kind::File, "b 10.txt", 5, "2026-11-02"),
            item(Kind::Folder, "Zeta", 0, "2026-11-01"),
            item(Kind::File, "B 9.txt", 50, "2026-11-03"),
            item(Kind::Folder, "alpha", 0, "2026-11-04"),
        ];
        sort_items(&mut items, Sort::default());
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "B 9.txt", "b 10.txt"]);
        sort_items(&mut items, Sort { key: SortKey::Size, ascending: false });
        assert_eq!(items[2].name, "B 9.txt");
        sort_items(&mut items, Sort::default().click(SortKey::Modified));
        assert_eq!(items[2].name, "B 9.txt", "newest first after the first click on Modified");
        assert!(!Sort::default().click(SortKey::Name).ascending);
    }

    #[test]
    fn names() {
        assert_eq!(natural("file 2", "file 10"), std::cmp::Ordering::Less);
        assert_eq!(numbered("plan.txt", 2), "plan (2).txt");
        assert_eq!(numbered("Makefile", 3), "Makefile (3)");
        assert_eq!(name_problem("a/b"), Some("Names can't contain / or \\."));
        assert_eq!(name_problem(" "), Some("Enter a name."));
        assert_eq!(name_problem("Q3 report.docx"), None);
        assert_eq!(guess_mime("x.MD"), "text/markdown");
    }

    #[test]
    fn kinds_in_words() {
        let mut png = item(Kind::File, "logo.png", 1, "");
        png.category = "Image".into();
        assert_eq!(png.kind_label(), "PNG image");
        assert!(png.has_thumbnail());
        assert_eq!(item(Kind::File, "Q3.docx", 1, "").kind_label(), "DOCX document");
        assert_eq!(item(Kind::File, "Notes.odt", 1, "").kind_label(), "ODT document");
        assert_eq!(item(Kind::File, "Budget.xlsx", 1, "").kind_label(), "XLSX spreadsheet");
        assert_eq!(item(Kind::File, "Pitch.pptx", 1, "").kind_label(), "PPTX presentation");
        assert_eq!(item(Kind::File, "Talk.deckcraft", 1, "").kind_label(), "DeckCraft presentation");
        assert_eq!(item(Kind::Folder, "Q3", 1, "").kind_label(), "Folder");
        assert_eq!(item(Kind::File, "README", 1, "").kind_label(), "File");
        assert!(item(Kind::File, "notes.md", 1, "").is_text());
    }
}
