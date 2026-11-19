//! `server check`: what the server guarantees the sync, and the sync mode that allows
//! (docs/WEBDAV-STORAGE.md § 11). For everyone, not only admins.
//!
//! On OxiCloud, CraftCloud's own server, no probe runs (§ 16, Q3): its capabilities are known and
//! checked by the live tests, so the command prints them and says they are known, not measured.
//! On another WebDAV server (with the opt-in) it runs the check on connect (§ 7.5) afresh,
//! saves it for the sync, and prints every step and the mode. The wording is one table in
//! `craftcloud_core::storage::probe::checks`, which the desktop's "What was checked" reuses.

use craftcloud_core::api::auth;
use craftcloud_core::storage::probe::{StoredCheck, checks};
use craftcloud_core::storage::{Capabilities, mode_for};
use craftcloud_native::http::execute;
use craftcloud_native::storage::probe::{Limits, save_check};
use craftcloud_native::storage::webdav;
use serde_json::json;

use super::output::{CliError, Result, print_json};
use super::remote::Ctx;

pub fn server_check(ctx: &Ctx) -> Result<()> {
    if ctx.is_webdav() {
        return webdav_check(ctx);
    }
    let server = ctx.server()?;
    // Reachable, and a CraftCloud server: its status answers without a sign-in.
    let status = execute(&auth::status(&server)).map_err(CliError::transport)?;
    auth::parse_status(&status)
        .map_err(|_| CliError::error(format!("{server} doesn't answer like a CraftCloud server (status {})", status.status)))?;
    // CraftCloud's own server: known, not measured (Q3). Another kind of server runs the probe above.
    let capabilities = Capabilities::oxicloud();
    let mode = mode_for(&capabilities);
    let rows = checks(&capabilities);
    if ctx.json {
        return print_json(&json!({
            "server": server.to_string(),
            "kind": "oxicloud",
            "mode": mode.as_str(),
            "measured": false,
            "checks": rows,
        }));
    }
    println!("server  {server}");
    println!("kind    CraftCloud (OxiCloud)");
    println!("mode    {} (known for CraftCloud's server, not measured)", mode.words());
    println!();
    for row in rows {
        println!("  {}", row.check);
        println!("      {}", row.result);
    }
    Ok(())
}

/// The check on connect against another WebDAV server (with its profile's known-server checks,
/// steps 16 and 17, where the kind has a profile), run now and saved for the sync.
fn webdav_check(ctx: &Ctx) -> Result<()> {
    let mut session = ctx.app_password_session()?;
    let profile = ctx.profile()?;
    let base = session.base().clone();
    let (verdict, version) = webdav::probe_with(&mut session, &base, profile.as_ref(), Limits::default());
    if session.signed_out {
        return Err(CliError::transport(craftcloud_native::session::APP_PASSWORD_REFUSED.into()));
    }
    let now = jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    let check = StoredCheck::from_verdict(&verdict, base.url(), version.as_deref().unwrap_or(""), &now);
    if let Some(dir) = ctx.dir() {
        save_check(&dir, &check).map_err(CliError::error)?;
    }
    let kind = profile.kind();
    if ctx.json {
        return print_json(&json!({
            "server": base.url(),
            "kind": kind.as_str(),
            "server_version": check.server_version,
            "mode": check.mode.as_str(),
            "measured": true,
            "checked_at": check.checked_at,
            "refused_because": check.refused_because,
            "checks": check.checks,
            "steps": check.steps,
        }));
    }
    println!("server  {}", base.url());
    match kind {
        craftcloud_core::sync::Storage::Webdav => println!("kind    another WebDAV server"),
        other => println!("kind    {} {}", other.words(), check.server_version),
    }
    match &check.refused_because {
        Some(why) => println!("mode    {} ({why})", check.mode.words()),
        None => println!("mode    {} (measured now)", check.mode.words()),
    }
    println!();
    for step in &check.steps {
        println!("  {:>2}. {} [{}]", step.number, step.what, if step.ok { "ok" } else { "no" });
        println!("      {}", step.result);
    }
    if !check.checks.is_empty() {
        println!();
        for row in &check.checks {
            println!("  {}", row.check);
            println!("      {}", row.result);
        }
    }
    Ok(())
}
