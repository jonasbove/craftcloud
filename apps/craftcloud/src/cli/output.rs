//! What the command line prints: errors with their exit codes, the JSON shapes, and the human
//! formatting of sizes, times and listings.

use std::fmt;
use std::process::ExitCode;

use craftcloud_core::api::ApiError;
use craftcloud_core::api::resources::Resource;
use craftcloud_native::local_files::SyncedFile;
use serde::Serialize;

/// Why a command failed; each kind has its own exit code (docs/CLI.md, "Exit codes").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 1: anything else.
    Error,
    /// 2: the command line itself is wrong (clap uses 2 for its own errors too).
    Usage,
    /// 3: no sign-in for this server, or it ran out.
    SignedOut,
    /// 4: no such file, folder, version, link or person.
    NotFound,
    /// 5: something is already there, or it changed meanwhile.
    Conflict,
    /// 6: `status`, `pause`, `resume` need the desktop client running.
    NoClient,
    /// 7: the server refused (e.g. an admin command without admin rights).
    Forbidden,
    /// 8: the server can't be reached.
    Unreachable,
    /// 9: the sync is held and waits for a decision (a mass delete, a missing sync folder).
    Held,
}

impl Fail {
    pub fn code(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Usage => 2,
            Self::SignedOut => 3,
            Self::NotFound => 4,
            Self::Conflict => 5,
            Self::NoClient => 6,
            Self::Forbidden => 7,
            Self::Unreachable => 8,
            Self::Held => 9,
        }
    }

    /// The `kind` in a JSON error.
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Usage => "usage",
            Self::SignedOut => "signed_out",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::NoClient => "no_client",
            Self::Forbidden => "forbidden",
            Self::Unreachable => "unreachable",
            Self::Held => "held",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError {
    pub kind: Fail,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    pub fn new(kind: Fail, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Fail::Error, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Fail::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(Fail::Conflict, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Fail::Usage, message)
    }

    /// Adds what was being done: "mkdir Documents: …".
    pub fn context(mut self, what: &str) -> Self {
        self.message = format!("{what}: {}", self.message);
        self
    }

    /// Reads a server's refusal by its status.
    pub fn api(e: &ApiError) -> Self {
        let kind = match e.status() {
            Some(401) => Fail::SignedOut,
            Some(403) => Fail::Forbidden,
            Some(404) => Fail::NotFound,
            Some(405 | 409 | 412) => Fail::Conflict,
            _ => Fail::Error,
        };
        match e.status() {
            Some(status @ 500..) => Self::new(kind, format!("the server couldn't do it ({status}): {e}")),
            _ => Self::new(kind, e.to_string()),
        }
    }

    /// Says cloud paths where the server's message has its own (`/Personal/Documents/a.txt` →
    /// `Documents/a.txt`), `root` being the name of the account's top folder.
    pub fn plain_paths(mut self, root: &str) -> Self {
        if !root.is_empty() {
            for (server, plain) in [(format!("/{root}/"), ""), (format!(" {root}/"), " "), (format!("'{root}/"), "'"), (format!("\"{root}/"), "\"")] {
                self.message = self.message.replace(&server, plain);
            }
        }
        self
    }

    /// Reads a transport error (or the session's "signed out").
    pub fn transport(e: String) -> Self {
        let kind = if e == craftcloud_native::session::SIGNED_OUT {
            Fail::SignedOut
        } else if craftcloud_native::http::is_unreachable(&e) {
            Fail::Unreachable
        } else {
            Fail::Error
        };
        Self::new(kind, e)
    }

    pub fn exit(&self, json: bool) -> ExitCode {
        if json {
            let body = serde_json::json!({"error": {"kind": self.kind.name(), "message": self.message, "exit_code": self.kind.code()}});
            eprintln!("{body}");
        } else {
            eprintln!("craftcloud: {}", self.message);
        }
        ExitCode::from(self.kind.code())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// A file or folder in `--json` output (ls, starred, recent, search, trash).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    /// Cloud path, e.g. `Documents/notes.md`.
    pub path: String,
    pub name: String,
    /// `file` or `folder`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Bytes; `null` for folders.
    pub size: Option<u64>,
    /// RFC 3339.
    pub modified: String,
    pub starred: bool,
    /// The server's id; left out for files listed from the local folder (`files` with a client).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The synced local copy, absolute: `null` when it isn't on this computer (not synced, not
    /// downloaded yet, or no sync folder); left out where it doesn't apply (trash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<Option<String>>,
    /// Where the file stands here: `synced`, `pending`, `syncing`, `error`, `conflict` or
    /// `excluded`; left out when unknown (no sync folder, folders, trash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<String>,
    /// When the trash deletes it for good.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
}

impl Item {
    /// `path` is the item's cloud path (listings don't carry one; the caller knows it).
    pub fn new(r: &Resource, path: String) -> Self {
        Self {
            path,
            name: r.name().to_owned(),
            kind: r.kind().as_str(),
            size: r.size(),
            modified: r.modified_at().to_owned(),
            starred: r.is_favorite(),
            id: r.id().to_owned(),
            local_path: None,
            sync: None,
            starred_at: None,
            opened_at: None,
            trashed_at: None,
            expires_at: None,
            score: None,
        }
    }

    /// With the path the server sent.
    pub fn listed(r: &Resource) -> Self {
        Self::new(r, r.cloud_path().unwrap_or_else(|| r.name().to_owned()))
    }

    /// A file the desktop client listed from its local folder (no server id, no star).
    pub fn local(f: SyncedFile) -> Self {
        Self {
            path: f.path,
            name: f.name,
            kind: "file",
            size: Some(f.size),
            modified: f.modified,
            starred: false,
            id: String::new(),
            local_path: Some(Some(f.local_path)),
            sync: Some(f.sync),
            starred_at: None,
            opened_at: None,
            trashed_at: None,
            expires_at: None,
            score: None,
        }
    }
}

/// Prints a value as one line of JSON.
pub fn print_json(value: &impl Serialize) -> Result<()> {
    let text = serde_json::to_string(value).map_err(|e| CliError::error(e.to_string()))?;
    println!("{text}");
    Ok(())
}

/// `1234567` → `1.2 MB` (powers of 1000, as file managers show them).
pub fn size(bytes: u64) -> String {
    craftcloud_core::size::text(bytes)
}

/// An RFC 3339 time as `2026-11-13 21:54` in the computer's time zone (the text as it is when it
/// can't be read).
pub fn time(rfc3339: &str) -> String {
    match rfc3339.parse::<jiff::Timestamp>() {
        Ok(ts) => ts.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d %H:%M").to_string(),
        Err(_) => rfc3339.to_owned(),
    }
}

/// An RFC 3339 time as ISO 8601 in the computer's time zone, with its offset:
/// `2026-11-13T22:54:00+01:00` (for `--json` where the local time matters).
pub fn local_iso(rfc3339: &str) -> Option<String> {
    let ts = rfc3339.parse::<jiff::Timestamp>().ok()?;
    Some(ts.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
}

/// One listing line: `size  modified  name` (folders end in `/`, starred ones get a `*`).
pub fn line(item: &Item, show: &str) -> String {
    let size = item.size.map_or_else(|| "-".to_owned(), size);
    let star = if item.starred { " *" } else { "" };
    let slash = if item.kind == "folder" { "/" } else { "" };
    format!("{size:>8}  {}  {show}{slash}{star}", time(&item.modified))
}

#[cfg(test)]
mod tests {
    use super::{CliError, Fail, Item, line, size};
    use craftcloud_core::api::ApiError;

    #[test]
    fn sizes_read_like_a_file_manager() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1_234), "1.2 KB");
        assert_eq!(size(56_000_000), "56 MB");
        assert_eq!(size(107_374_182_400), "107 GB");
    }

    #[test]
    fn statuses_become_exit_codes() {
        let code = |status| CliError::api(&ApiError::Status { status, message: String::new() }).kind.code();
        assert_eq!([code(401), code(403), code(404), code(409), code(412), code(500)], [3, 7, 4, 5, 5, 1]);
        assert_eq!(CliError::transport(craftcloud_native::session::SIGNED_OUT.into()).kind, Fail::SignedOut);
        assert_eq!(CliError::transport("Can't connect to the server (refused).".into()).kind, Fail::Unreachable);
        assert_eq!(CliError::not_found("x").context("rm a").message, "rm a: x");
        let internal = CliError::api(&ApiError::Status { status: 500, message: "An internal error occurred".into() });
        assert_eq!((internal.kind.code(), internal.message.as_str()), (1, "the server couldn't do it (500): An internal error occurred"));
    }

    /// Errors name cloud paths, never the server's own (`/Personal/…`).
    #[test]
    fn errors_say_cloud_paths() {
        let e = CliError::error("Folder already exists at /Personal/Documents/Q3 (was 'Personal/Old')").plain_paths("Personal");
        assert_eq!(e.message, "Folder already exists at Documents/Q3 (was 'Old')");
        assert_eq!(CliError::error("Personal/x").plain_paths("").message, "Personal/x", "no root known: as it is");
    }

    #[test]
    fn listing_lines_and_json() {
        let item = Item {
            path: "Documents".into(),
            name: "Documents".into(),
            kind: "folder",
            size: None,
            modified: "not a time".into(),
            starred: true,
            id: "d1".into(),
            local_path: None,
            sync: None,
            starred_at: None,
            opened_at: None,
            trashed_at: None,
            expires_at: None,
            score: None,
        };
        assert_eq!(line(&item, "Documents"), "       -  not a time  Documents/ *");
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "folder");
        assert!(json["size"].is_null());
        assert!(json.get("trashed_at").is_none(), "extras only where they apply");
        assert!(json.get("local_path").is_none() && json.get("sync").is_none());
    }

    /// The shape an integration reads (docs/CRAFT-INTEGRATION.md, section 3).
    #[test]
    fn files_say_where_they_are_here() {
        let local = Item::local(craftcloud_native::local_files::SyncedFile {
            path: "Documents/Q3 report.docx".into(),
            name: "Q3 report.docx".into(),
            local_path: "/home/john/CraftCloud/Documents/Q3 report.docx".into(),
            size: 18_234,
            modified: "2026-11-13T21:54:00Z".into(),
            sync: "synced".into(),
        });
        let json = serde_json::to_value(&local).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "path": "Documents/Q3 report.docx", "name": "Q3 report.docx", "type": "file", "size": 18234,
                "modified": "2026-11-13T21:54:00Z", "starred": false,
                "local_path": "/home/john/CraftCloud/Documents/Q3 report.docx", "sync": "synced"
            })
        );
        // From the server, not on this computer: the path is there, as null.
        let remote = Item { id: "f1".into(), local_path: Some(None), sync: Some("excluded".into()), ..local };
        let json = serde_json::to_value(&remote).unwrap();
        assert!(json["local_path"].is_null() && json.as_object().unwrap().contains_key("local_path"));
        assert_eq!((json["id"].as_str(), json["sync"].as_str()), (Some("f1"), Some("excluded")));
    }
}
