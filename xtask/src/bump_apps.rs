//! `cargo xtask bump-apps`: update the pins (version, asset, sha256) in `apps.toml` from each app's
//! latest GitHub release.
//!
//! Only the three pin lines inside each `[apps.<id>]` table are rewritten; every other line is kept
//! as written, so hand-edited fields (associations, bridge, notes) survive a bump. The sha256 comes
//! from the GitHub release API's asset `digest`. Set `GITHUB_TOKEN` to avoid the 60 requests an
//! hour limit.

use std::time::Duration;

use serde_json::Value;

use crate::manifest::{self, is_sha256_hex};

pub const USAGE: &str = "usage: cargo xtask bump-apps [--check] [--app <id>]...
  --check   report what would change and fail if anything is out of date; write nothing";

/// The newest web build in a release list: (version, asset name, sha256, release tag).
type Found = (String, String, String, String);

pub fn cmd(args: &[String]) -> Result<(), String> {
    let mut check = false;
    let mut only = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--app" => only.push(it.next().ok_or(USAGE)?.clone()),
            _ => return Err(format!("unknown argument `{arg}`\n{USAGE}")),
        }
    }
    let path = crate::root().join("apps.toml");
    let mut text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let apps = manifest::parse(&text)?;
    if let Some(unknown) = only.iter().find(|id| !apps.iter().any(|(app, _)| app == *id)) {
        return Err(format!("no [apps.{unknown}] in apps.toml"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
    let token = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty());
    let mut stale = false;
    for (app, spec) in &apps {
        if !only.is_empty() && !only.contains(app) {
            continue;
        }
        let url = format!("https://api.github.com/repos/{}/releases?per_page=10", spec.repo);
        let mut request = agent.get(&url).header("Accept", "application/vnd.github+json").header("User-Agent", "craftcloud-bump-apps");
        if let Some(token) = &token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        let body = request.call().map_err(|e| format!("{url}: {e}"))?.into_body().read_to_string().map_err(|e| format!("{url}: {e}"))?;
        let releases: Value = serde_json::from_str(&body).map_err(|e| format!("{url}: {e}"))?;
        let Some((version, asset, sha, tag)) = latest_web_asset(&releases, app)? else {
            println!("{app:<14} no web asset in recent releases, kept {}", spec.version);
            continue;
        };
        if (&version, &asset, &sha) == (&spec.version, &spec.asset, &spec.sha256) {
            println!("{app:<14} {version} up to date");
            continue;
        }
        stale = true;
        println!("{app:<14} {} -> {version} ({tag})", spec.version);
        for (key, value) in [("version", &version), ("asset", &asset), ("sha256", &sha)] {
            text = set_pin(&text, app, key, value)?;
        }
    }
    if check {
        return if stale { Err("some pins are out of date (run without --check to update them)".to_owned()) } else { Ok(()) };
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The first `<app>-web-<x.y.z>.zip` in a GitHub release list (newest first), skipping drafts and
/// pre-releases. An asset without a sha256 digest is an error, not something to skip.
fn latest_web_asset(releases: &Value, app: &str) -> Result<Option<Found>, String> {
    let prefix = format!("{app}-web-");
    let releases = releases.as_array().ok_or("the GitHub API answered with something other than a release list")?;
    for rel in releases {
        if rel["draft"].as_bool() == Some(true) || rel["prerelease"].as_bool() == Some(true) {
            continue;
        }
        for asset in rel["assets"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let Some(name) = asset["name"].as_str() else { continue };
            let Some(version) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".zip")).filter(|v| is_xyz(v)) else {
                continue;
            };
            let tag = rel["tag_name"].as_str().unwrap_or_default();
            let sha = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).filter(|s| is_sha256_hex(s));
            let Some(sha) = sha else {
                return Err(format!("{app}: release {tag} asset has no sha256 digest"));
            };
            return Ok(Some((version.to_owned(), name.to_owned(), sha.to_owned(), tag.to_owned())));
        }
    }
    Ok(None)
}

/// `1.2.3`: three dot-separated runs of ASCII digits.
fn is_xyz(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Replaces the quoted value of the first `key = "…"` line inside the `[apps.<app>]` table only,
/// leaving everything else on that line (alignment, comments) and in the file as it was.
fn set_pin(text: &str, app: &str, key: &str, value: &str) -> Result<String, String> {
    let header = format!("[apps.{app}]");
    let mut offset = 0;
    let mut in_table = false;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if !in_table {
            in_table = line.strip_suffix('\n') == Some(header.as_str());
            continue;
        }
        if line.starts_with('[') {
            break;
        }
        let Some(rest) = line.strip_prefix(key) else { continue };
        let after_eq = rest.trim_start_matches([' ', '\t']).strip_prefix('=').map(|r| r.trim_start_matches([' ', '\t']));
        let Some(quoted) = after_eq.and_then(|r| r.strip_prefix('"')) else { continue };
        let Some(old_len) = quoted.find('"') else { continue };
        let value_start = start + line.len() - quoted.len();
        let mut out = String::with_capacity(text.len() + value.len());
        out.push_str(text.get(..value_start).unwrap_or_default());
        out.push_str(value);
        out.push_str(text.get(value_start + old_len..).unwrap_or_default());
        return Ok(out);
    }
    if in_table { Err(format!("{app}: no `{key}` line")) } else { Err(format!("{app}: no [apps.{app}] table")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = "# head\n[apps.wordcraft]\nname    = \"WordCraft\"\nversion = \"0.18.1\"   # pinned\nasset   = \"wordcraft-web-0.18.1.zip\"\n\n[apps.gridcraft]\nversion = \"0.19.1\"\nassets_note = \"x\"\nasset = \"gridcraft-web-0.19.1.zip\"\n";

    #[test]
    fn set_pin_rewrites_only_the_value_in_its_own_table() {
        let out = set_pin(TOML, "gridcraft", "version", "0.20.0").unwrap();
        assert_eq!(out, TOML.replace("version = \"0.19.1\"", "version = \"0.20.0\""));
        let out = set_pin(TOML, "wordcraft", "version", "1.0.0").unwrap();
        assert!(out.contains("version = \"1.0.0\"   # pinned\n") && out.contains("version = \"0.19.1\""));
        // `asset` does not match `assets_note`.
        let out = set_pin(TOML, "gridcraft", "asset", "g.zip").unwrap();
        assert!(out.contains("assets_note = \"x\"\nasset = \"g.zip\"\n"), "{out}");
        assert!(set_pin(TOML, "wordcraft", "sha256", "x").unwrap_err().contains("no `sha256` line"));
        assert!(set_pin(TOML, "photocraft", "version", "x").unwrap_err().contains("no [apps.photocraft] table"));
    }

    #[test]
    fn latest_web_asset_skips_drafts_and_prereleases() {
        let sha = "a".repeat(64);
        let releases = serde_json::json!([
            { "tag_name": "v0.20.0", "draft": true, "assets": [{ "name": "wordcraft-web-0.20.0.zip", "digest": format!("sha256:{sha}") }] },
            { "tag_name": "v0.19.9", "prerelease": true, "assets": [{ "name": "wordcraft-web-0.19.9.zip", "digest": format!("sha256:{sha}") }] },
            { "tag_name": "v0.19.0", "assets": [
                { "name": "wordcraft-0.19.0.AppImage", "digest": "sha256:bad" },
                { "name": "wordcraft-web-0.19.0.zip", "digest": format!("sha256:{sha}") },
            ] },
        ]);
        let found = latest_web_asset(&releases, "wordcraft").unwrap().unwrap();
        assert_eq!(found, ("0.19.0".into(), "wordcraft-web-0.19.0.zip".into(), sha, "v0.19.0".into()));
        assert_eq!(latest_web_asset(&releases, "gridcraft").unwrap(), None);
        let no_digest = serde_json::json!([{ "tag_name": "v1.0.0", "assets": [{ "name": "wordcraft-web-1.0.0.zip" }] }]);
        assert!(latest_web_asset(&no_digest, "wordcraft").is_err());
    }
}
