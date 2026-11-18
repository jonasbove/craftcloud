//! Share links and file versions.

use craftcloud_core::api::expect_ok;
use craftcloud_core::api::shares::{self, NewShare, Share};
use craftcloud_core::api::versions::Version;
use serde::Serialize;
use serde_json::json;

use super::ShareCmd;
use super::output::{self, CliError, Result, print_json};
use super::remote::{Ctx, Remote, clean, split};

/// A link in `--json` output.
#[derive(Debug, Serialize)]
struct Link {
    id: String,
    url: String,
    /// `file` or `folder`.
    #[serde(rename = "type")]
    kind: String,
    item_id: String,
    /// Where the shared item is now (cloud path); `null` when it can't be found (deleted for good).
    path: Option<String>,
    /// The shared item's name: from `path`, else the one the link was made with, when known.
    name: Option<String>,
    expires_at: Option<String>,
    has_password: bool,
    created_at: String,
    access_count: u64,
}

impl Link {
    /// `path`: where the item is now, when known.
    fn new(s: &Share, path: Option<String>) -> Self {
        Self {
            id: s.id.clone(),
            url: s.url.clone(),
            kind: s.item_type.clone(),
            item_id: s.item_id.clone(),
            name: path.as_deref().map(|p| split(p).1.to_owned()).or_else(|| s.item_name.clone()),
            path,
            expires_at: s.expires_at.clone(),
            has_password: s.has_password,
            created_at: s.created_at.clone(),
            access_count: s.access_count,
        }
    }

    fn human(&self) -> String {
        let mut notes = Vec::new();
        if let Some(e) = &self.expires_at {
            notes.push(format!("until {}", output::time(e)));
        }
        if self.has_password {
            notes.push("password".to_owned());
        }
        let name = match (&self.path, &self.name) {
            (Some(path), _) => path.clone(),
            (None, Some(name)) => format!("{name} (not found)"),
            (None, None) => format!("a {} that wasn't found", self.kind),
        };
        let notes = if notes.is_empty() { String::new() } else { format!("  ({})", notes.join(", ")) };
        format!("{}  {name}{notes}", self.url)
    }
}

/// `7d`, `12h`, `2w`, `2027-01-31` (to the end of that day, UTC) or an RFC 3339 time → RFC 3339,
/// in the future. The rules are the web share sheet's too (`shares::expiry_at`).
pub fn parse_expiry(text: &str, now: jiff::Timestamp) -> Result<String> {
    let text = text.trim();
    match shares::expiry_at(text, now, jiff::tz::Offset::UTC) {
        Ok(when) => Ok(shares::expiry_text(when)),
        Err(shares::ExpiryError::Past) => Err(CliError::usage(format!("--expires {text}: that is in the past"))),
        Err(shares::ExpiryError::Unreadable) => {
            Err(CliError::usage(format!("--expires {text}: use a date (2027-01-31), a time (2027-01-31T18:00:00Z) or a duration (12h, 7d, 2w)")))
        }
    }
}

fn all_shares(remote: &mut Remote) -> Result<Vec<Share>> {
    let mut out = Vec::new();
    for page in 1.. {
        let list = remote.ask(&shares::list(remote.server(), page, 100), shares::parse_list)?;
        out.extend(list.items);
        if !list.has_next {
            break;
        }
    }
    Ok(out)
}

fn print_links(ctx: &Ctx, links: &[Link]) -> Result<()> {
    if ctx.json {
        return print_json(&links);
    }
    for l in links {
        println!("{}", l.human());
    }
    Ok(())
}

/// `share create|ls|rm`.
pub fn share(ctx: &Ctx, cmd: ShareCmd) -> Result<()> {
    if ctx.is_webdav() {
        return Err(CliError::unsupported("share links"));
    }
    let mut remote = ctx.remote()?;
    match cmd {
        ShareCmd::Create { path, expires, password } => {
            let path = clean(&path)?;
            if path.is_empty() {
                return Err(CliError::usage("share a file or folder, not the whole cloud"));
            }
            let expires_at = expires.map(|e| parse_expiry(&e, jiff::Timestamp::now())).transpose()?;
            let password = if password { Some(super::read_secret("Password for the link")?) } else { None };
            let found = remote.resolve(&path)?;
            let name = split(&path).1.to_owned();
            let new = NewShare { kind: found.kind(), id: found.id().to_owned(), name: Some(name.clone()), expires_at, password };
            let share = remote.ask(&shares::create(remote.server(), &new), shares::parse_share)?;
            let link = Link::new(&share, Some(path));
            if ctx.json {
                return print_json(&link);
            }
            println!("{}", link.url);
            Ok(())
        }
        ShareCmd::Ls { path: Some(path) } => {
            let path = clean(&path)?;
            let found = remote.resolve(&path)?;
            let list = remote.ask(&shares::list_for(remote.server(), found.kind(), found.id()), shares::parse_list)?;
            let links: Vec<Link> = list.items.iter().map(|s| Link::new(s, Some(path.clone()))).collect();
            print_links(ctx, &links)
        }
        ShareCmd::Ls { path: None } => {
            // One lookup per link: OxiCloud keeps only the name a link was made with (T-040).
            let mut links = Vec::new();
            for s in all_shares(&mut remote)? {
                let path = remote.call(&shares::item_info(remote.server(), &s)).ok().and_then(|resp| shares::parse_item_path(&s, &resp));
                links.push(Link::new(&s, path));
            }
            print_links(ctx, &links)
        }
        ShareCmd::Rm { link } => remove(ctx, &mut remote, &link),
    }
}

/// Removes a link by id, URL or token, or every link to a path.
fn remove(ctx: &Ctx, remote: &mut Remote, link: &str) -> Result<()> {
    let all = all_shares(remote)?;
    let token = link.trim_end_matches('/').rsplit('/').next().unwrap_or(link);
    let ids: Vec<String> = match all.iter().find(|s| s.id == link || s.url == link || s.token == token) {
        Some(s) => vec![s.id.clone()],
        None => {
            let path = clean(link)?;
            let found = remote.resolve(&path).map_err(|_| CliError::not_found(format!("{link}: no such link, file or folder")))?;
            let ids: Vec<String> = all.iter().filter(|s| s.item_id == found.id()).map(|s| s.id.clone()).collect();
            if ids.is_empty() {
                return Err(CliError::not_found(format!("{path} has no links")));
            }
            ids
        }
    };
    for id in &ids {
        remote.ask(&shares::delete(remote.server(), id), expect_ok)?;
    }
    if ctx.json {
        print_json(&json!({"removed": ids}))
    } else {
        println!("Removed {} link{}.", ids.len(), if ids.len() == 1 { "" } else { "s" });
        Ok(())
    }
}

/// `versions <path> [restore <n>]`.
pub fn versions(ctx: &Ctx, path: &str, restore: bool, n: Option<usize>) -> Result<()> {
    let path = clean(path)?;
    if ctx.is_webdav() {
        // The kept versions are listed and restored through the trait in a later pull request
        // (docs/WEBDAV-STORAGE.md § 14, 4b); the sync keeps them already.
        return Err(CliError::unsupported("the version commands"));
    }
    let mut remote = ctx.remote()?;
    if remote.stat(&path)?.is_none() {
        return Err(CliError::not_found(format!("{path}: no such file")));
    }
    let list = remote.session.versions(&path).map_err(CliError::transport)?;
    if !restore {
        if n.is_some() {
            return Err(CliError::usage("to restore a version: craftcloud versions <path> restore <n>"));
        }
        if ctx.json {
            let rows: Vec<_> = list.iter().enumerate().map(|(i, v)| version_json(i + 1, v)).collect();
            return print_json(&rows);
        }
        if list.is_empty() {
            println!("{path} has no earlier versions.");
        }
        for (i, v) in list.iter().enumerate() {
            println!("{}", version_line(i + 1, v));
        }
        return Ok(());
    }
    let n = n.ok_or_else(|| CliError::usage("which version? craftcloud versions <path> restore <n> (1 = newest)"))?;
    let version = n
        .checked_sub(1)
        .and_then(|i| list.get(i))
        .ok_or_else(|| CliError::not_found(format!("{path} has no version {n} (it has {})", list.len())))?;
    remote.session.restore_version(&path, &version.stamp).map_err(CliError::transport)?;
    if ctx.json {
        let mut out = version_json(n, version);
        out["path"] = json!(path);
        out["restored"] = json!(version.stamp);
        print_json(&out)
    } else {
        let when = version.modified_at().or_else(|| version.replaced_at()).map_or_else(|| version.when(), |t| output::time(&t));
        println!("Restored {path} to version {n}, last changed {when} (the replaced one is now version 1).");
        Ok(())
    }
}

/// A version in `--json`: `modified` (when its content was last changed; `null` for versions kept
/// before CraftCloud wrote that down) and `replaced_at` (when it was replaced), ISO 8601 in this
/// computer's time zone, with the offset. `stamp` and `time` (the replacement time, UTC, as text)
/// stay for scripts that read them.
fn version_json(n: usize, v: &Version) -> serde_json::Value {
    let local = |t: Option<String>| t.and_then(|t| output::local_iso(&t));
    json!({
        "n": n,
        "stamp": v.stamp,
        "time": v.when(),
        "modified": local(v.modified_at()),
        "replaced_at": local(v.replaced_at()),
        "size": v.size,
    })
}

/// ` 1  2026-11-13 14:02      11 B  replaced 2026-11-13 15:10` (local times; `-` when the
/// content's time isn't known).
fn version_line(n: usize, v: &Version) -> String {
    let modified = v.modified_at().map_or_else(|| "-".to_owned(), |t| output::time(&t));
    let replaced = v.replaced_at().map_or_else(|| v.when(), |t| output::time(&t));
    format!("{n:>2}  {modified:<16}  {:>8}  replaced {replaced}", output::size(v.size))
}

#[cfg(test)]
mod tests {
    use super::parse_expiry;

    /// T-019: local times, the content's own time first; JSON keeps both, with the offset.
    #[test]
    fn versions_show_when_their_content_changed() {
        use craftcloud_core::api::versions::Version;
        let v = Version { stamp: "20261113T151013.123Z_20261113T140205Z".into(), size: 11, etag: None };
        let local = |t: &str| super::super::output::time(t);
        assert_eq!(
            super::version_line(1, &v),
            format!(" 1  {}      11 B  replaced {}", local("2026-11-13T14:02:05Z"), local("2026-11-13T15:10:13Z"))
        );
        let json = super::version_json(1, &v);
        let text = |key: &str| json[key].as_str().unwrap().to_owned();
        let at = |t: &str| t.parse::<jiff::Timestamp>().unwrap();
        assert_eq!(at(&text("modified")), at("2026-11-13T14:02:05Z"));
        assert!(!text("modified").ends_with('Z') && text("modified").len() == 25, "with the local offset: {json}");
        assert_eq!(at(&text("replaced_at")), at("2026-11-13T15:10:13Z"));
        assert_eq!((text("stamp"), text("time"), json["size"].as_u64()), (v.stamp.clone(), "2026-11-13 15:10:13 UTC".to_owned(), Some(11)));
        let old = Version { stamp: "20261113T151013.123Z".into(), size: 11, etag: None };
        assert!(super::version_json(1, &old)["modified"].is_null());
        assert!(super::version_line(1, &old).starts_with(" 1  -   "));
    }

    fn now() -> jiff::Timestamp {
        "2026-11-13T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn expiry_takes_dates_times_and_durations() {
        assert_eq!(parse_expiry("7d", now()).unwrap(), "2026-11-20T12:00:00Z");
        assert_eq!(parse_expiry("12h", now()).unwrap(), "2026-11-14T00:00:00Z");
        assert_eq!(parse_expiry("2w", now()).unwrap(), "2026-11-27T12:00:00Z");
        assert_eq!(parse_expiry("2027-01-31", now()).unwrap(), "2027-02-01T00:00:00Z", "to the end of that day");
        assert_eq!(parse_expiry("2027-01-31T18:00:00Z", now()).unwrap(), "2027-01-31T18:00:00Z");
        assert_eq!(parse_expiry("2026-01-01", now()).unwrap_err().kind.code(), 2, "in the past");
        assert!(parse_expiry("soon", now()).is_err());
        assert!(parse_expiry("xd", now()).is_err());
    }
}
