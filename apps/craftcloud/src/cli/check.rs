//! `server check`: what the server guarantees the sync, and the sync mode that allows
//! (docs/WEBDAV-STORAGE.md § 11). For everyone, not only admins.
//!
//! On OxiCloud, CraftCloud's own server (the only kind there is until other servers are added), no
//! probe runs (§ 16, Q3): its capabilities are known and checked by the live tests, so the command
//! prints them and says they are known, not measured. The wording is one table in
//! `craftcloud_core::storage::probe::checks`, which the desktop's "What was checked" reuses.

use craftcloud_core::api::auth;
use craftcloud_core::storage::probe::checks;
use craftcloud_core::storage::{Capabilities, mode_for};
use craftcloud_native::http::execute;
use serde_json::json;

use super::output::{CliError, Result, print_json};
use super::remote::Ctx;

pub fn server_check(ctx: &Ctx) -> Result<()> {
    let server = ctx.server()?;
    // Reachable, and a CraftCloud server: its status answers without a sign-in.
    let status = execute(&auth::status(&server)).map_err(CliError::transport)?;
    auth::parse_status(&status)
        .map_err(|_| CliError::error(format!("{server} doesn't answer like a CraftCloud server (status {})", status.status)))?;
    // Until another kind of server can be signed in to (docs/WEBDAV-STORAGE.md § 14, pull request
    // 4), every server here is OxiCloud. From then on the kind comes from the settings
    // (`storage`), and for any other kind this runs the probe and reports what it measured;
    // the "known, not measured" answer below stays OxiCloud's only.
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
