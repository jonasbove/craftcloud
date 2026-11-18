//! `server info|users|settings`: administration, for owners and admins.

use craftcloud_core::RuntimeConfig;
use craftcloud_core::api::admin::{self, Person};
use craftcloud_core::api::{auth, expect_ok};
use craftcloud_core::http::{HttpRequest, Method};
use craftcloud_native::http::execute;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::output::{self, CliError, Fail, Result, print_json};
use super::remote::{Ctx, Remote};
use super::{ServerCmd, SettingsCmd, UsersCmd};

pub fn server(ctx: &Ctx, cmd: ServerCmd) -> Result<()> {
    match cmd {
        // For everyone, not only admins: its own module.
        ServerCmd::Check => super::check::server_check(ctx),
        // The rest is CraftCloud's own server's: its version and settings, its people.
        _ if ctx.is_webdav() => Err(CliError::unsupported("the server's administration")),
        ServerCmd::Info => info(ctx),
        ServerCmd::Users(u) => users(ctx, u),
        ServerCmd::Settings(s) => settings(ctx, s),
    }
}

fn info(ctx: &Ctx) -> Result<()> {
    let server = ctx.server()?;
    let status = auth::parse_status(&execute(&auth::status(&server)).map_err(CliError::transport)?).map_err(|e| CliError::api(&e))?;
    let config_resp = execute(&HttpRequest::new(Method::Get, server.join("craftcloud/config.json"))).map_err(CliError::transport)?;
    let config = expect_ok(&config_resp).ok().and_then(|()| RuntimeConfig::from_json(&config_resp.body).ok());
    // Who is signed in, when someone is.
    let me = ctx.remote().ok().and_then(|mut r| r.ask(&auth::me(r.server()), auth::parse_me).ok().flatten());
    if ctx.json {
        return print_json(&json!({
            "server": server.to_string(),
            "craftcloud": config.is_some(),
            "version": config.as_ref().map(|c| c.version.clone()),
            "public_url": config.as_ref().map(|c| c.public_url.clone()),
            "web_apps": config.as_ref().map(|c| c.web_apps),
            "initialized": status.initialized,
            "signed_in_as": me.as_ref().map(|m| m.user.username.clone()),
            "role": me.as_ref().map(|m| m.user.role.clone()),
        }));
    }
    println!("server      {server}");
    match &config {
        Some(c) => {
            println!("version     CraftCloud {}", c.version);
            println!("public URL  {}", c.public_url);
            println!("web apps    {}", if c.web_apps { "on" } else { "off" });
        }
        None => println!("version     (not a CraftCloud server, or an older one)"),
    }
    if !status.initialized {
        println!("setup       not done yet: open the server in a browser");
    }
    if let Some(m) = me {
        println!("signed in   {} ({})", m.user.username, m.user.role);
    }
    Ok(())
}

fn people(remote: &mut Remote) -> Result<Vec<Person>> {
    let mut out = Vec::new();
    loop {
        let (page, total) = remote.ask(&admin::list(remote.server(), 200, out.len() as u32), admin::parse_list)?;
        let empty = page.is_empty();
        out.extend(page);
        if empty || out.len() as u64 >= total {
            return Ok(out);
        }
    }
}

fn person(remote: &mut Remote, username: &str) -> Result<Person> {
    people(remote)?
        .into_iter()
        .find(|p| p.username == username)
        .ok_or_else(|| CliError::not_found(format!("nobody called {username} on this server")))
}

/// `10GB`, `500MB`, `1.5TB`, `1073741824`, `unlimited` → bytes (`0` = no limit).
pub fn parse_size(text: &str) -> Result<u64> {
    let t = text.trim().to_ascii_uppercase();
    if t == "UNLIMITED" || t == "NONE" {
        return Ok(0);
    }
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (number, unit) = t.split_at(split);
    let n: f64 = number.parse().map_err(|_| CliError::usage(format!("{text}: use a size like 10GB, 500MB or unlimited")))?;
    let factor = match unit.trim() {
        "" | "B" => 1.0,
        "K" | "KB" => 1e3,
        "M" | "MB" => 1e6,
        "G" | "GB" => 1e9,
        "T" | "TB" => 1e12,
        "KIB" => 1024.0,
        "MIB" => 1024.0 * 1024.0,
        "GIB" => 1024.0 * 1024.0 * 1024.0,
        _ => return Err(CliError::usage(format!("{text}: use a size like 10GB, 500MB or unlimited"))),
    };
    Ok((n * factor).round() as u64)
}

#[derive(Serialize)]
struct PersonOut<'a> {
    username: &'a str,
    email: &'a str,
    role: &'a str,
    active: bool,
    used_bytes: u64,
    quota_bytes: u64,
    last_login_at: Option<&'a str>,
    id: &'a str,
}

impl<'a> From<&'a Person> for PersonOut<'a> {
    fn from(p: &'a Person) -> Self {
        Self {
            username: &p.username,
            email: &p.email,
            role: &p.role,
            active: p.active,
            used_bytes: p.storage_used_bytes,
            quota_bytes: p.storage_quota_bytes,
            last_login_at: p.last_login_at.as_deref(),
            id: &p.id,
        }
    }
}

fn users(ctx: &Ctx, cmd: UsersCmd) -> Result<()> {
    let mut remote = ctx.remote()?;
    let forbidden = |e: CliError| if e.kind == Fail::Forbidden { CliError::new(Fail::Forbidden, "Only owners and admins can do that.") } else { e };
    match cmd {
        UsersCmd::Ls => {
            let all = people(&mut remote).map_err(forbidden)?;
            if ctx.json {
                return print_json(&all.iter().map(PersonOut::from).collect::<Vec<_>>());
            }
            for p in &all {
                let state = if p.active { "" } else { "  (disabled)" };
                let quota = if p.storage_quota_bytes == 0 { "no limit".to_owned() } else { output::size(p.storage_quota_bytes) };
                println!("{:<16} {:<8} {:>8} of {:<9} {}{state}", p.username, p.role, output::size(p.storage_used_bytes), quota, p.email);
            }
            Ok(())
        }
        UsersCmd::Add { username, email, quota, admin: make_admin } => {
            let quota = quota.as_deref().map(parse_size).transpose()?;
            let password = super::read_secret(&format!("Password for {username}"))?;
            let role = if make_admin { admin::Role::Admin } else { admin::Role::User };
            let made =
                remote.ask(&admin::create(remote.server(), &username, &email, &password, quota, role), admin::parse_create).map_err(forbidden)?;
            report(ctx, json!({"added": made.username, "id": made.id}), &format!("Added {}.", made.username))
        }
        UsersCmd::Passwd { username } => {
            let p = person(&mut remote, &username).map_err(forbidden)?;
            let password = super::read_secret(&format!("New password for {username}"))?;
            remote.ask(&admin::reset_password(remote.server(), &p.id, &password), expect_ok).map_err(forbidden)?;
            report(ctx, json!({"password_set": username}), &format!("Set a new password for {username}."))
        }
        UsersCmd::Enable { username } => set_active(ctx, &mut remote, &username, true),
        UsersCmd::Disable { username } => set_active(ctx, &mut remote, &username, false),
        UsersCmd::Quota { username, size } => {
            let bytes = parse_size(&size)?;
            let p = person(&mut remote, &username).map_err(forbidden)?;
            remote.ask(&admin::set_quota(remote.server(), &p.id, bytes), expect_ok).map_err(forbidden)?;
            let shown = if bytes == 0 { "no limit".to_owned() } else { output::size(bytes) };
            report(ctx, json!({"username": username, "quota_bytes": bytes}), &format!("{username} now has {shown}."))
        }
    }
}

fn set_active(ctx: &Ctx, remote: &mut Remote, username: &str, active: bool) -> Result<()> {
    let p = person(remote, username)?;
    remote.ask(&admin::set_active(remote.server(), &p.id, active), expect_ok)?;
    let word = if active { "can sign in again" } else { "can no longer sign in" };
    report(ctx, json!({"username": username, "active": active}), &format!("{username} {word}."))
}

fn report(ctx: &Ctx, value: serde_json::Value, human: &str) -> Result<()> {
    if ctx.json {
        print_json(&value)
    } else {
        println!("{human}");
        Ok(())
    }
}

/// `GET /craftcloud/api/settings`.
#[derive(Debug, Serialize, Deserialize)]
struct SettingsView {
    web_apps: bool,
    #[serde(default)]
    disabled_apps: Vec<String>,
    #[serde(default)]
    bundled_apps: Vec<String>,
}

fn show(ctx: &Ctx, s: &SettingsView) -> Result<()> {
    if ctx.json {
        return print_json(s);
    }
    println!("web apps  {}", if s.web_apps { "on" } else { "off" });
    if s.bundled_apps.is_empty() {
        println!("apps      none in this image (the -apps image has them)");
    }
    for app in &s.bundled_apps {
        let off = if s.disabled_apps.contains(app) { "  (switched off)" } else { "" };
        println!("  {app}{off}");
    }
    Ok(())
}

/// Applies `settings set` to the current settings.
fn change(mut s: SettingsView, web_apps: Option<bool>, disable: &[String], enable: &[String]) -> Result<SettingsView> {
    for app in disable.iter().chain(enable) {
        if !s.bundled_apps.contains(app) {
            return Err(CliError::not_found(format!("{app} isn't in this image ({})", s.bundled_apps.join(", "))));
        }
    }
    if web_apps == Some(true) && s.bundled_apps.is_empty() {
        return Err(CliError::conflict("Needs the -apps image: this one has no Craft apps to serve."));
    }
    if let Some(on) = web_apps {
        s.web_apps = on;
    }
    s.disabled_apps.retain(|a| !enable.contains(a));
    s.disabled_apps.extend(disable.iter().filter(|a| !s.disabled_apps.contains(a)).cloned().collect::<Vec<_>>());
    s.disabled_apps.sort();
    Ok(s)
}

fn settings(ctx: &Ctx, cmd: SettingsCmd) -> Result<()> {
    let mut remote = ctx.remote()?;
    let url = remote.server().join("craftcloud/api/settings");
    let read = |resp: &craftcloud_core::http::HttpResponse| -> Result<SettingsView> {
        if resp.status == 401 || resp.status == 403 {
            return Err(CliError::new(Fail::Forbidden, "Only owners and admins can see the server settings."));
        }
        expect_ok(resp).map_err(|e| CliError::api(&e))?;
        serde_json::from_slice(&resp.body).map_err(|e| CliError::error(format!("unexpected settings answer: {e}")))
    };
    let current = read(&remote.call(&HttpRequest::new(Method::Get, url.clone()))?)?;
    match cmd {
        SettingsCmd::Show => show(ctx, &current),
        SettingsCmd::Set { web_apps, disable_app, enable_app } => {
            let new = change(current, web_apps.map(|w| w == "on"), &disable_app, &enable_app)?;
            let body = json!({"web_apps": new.web_apps, "disabled_apps": new.disabled_apps});
            let saved = read(&remote.call(&HttpRequest::new(Method::Put, url).json(&body))?)?;
            show(ctx, &saved)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsView, change, parse_size};

    #[test]
    fn sizes() {
        assert_eq!(parse_size("10GB").unwrap(), 10_000_000_000);
        assert_eq!(parse_size("1.5tb").unwrap(), 1_500_000_000_000);
        assert_eq!(parse_size("512MiB").unwrap(), 536_870_912);
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("unlimited").unwrap(), 0);
        assert_eq!(parse_size("lots").unwrap_err().kind.code(), 2);
    }

    fn view(bundled: &[&str]) -> SettingsView {
        SettingsView { web_apps: false, disabled_apps: vec![], bundled_apps: bundled.iter().map(|s| (*s).to_owned()).collect() }
    }

    #[test]
    fn settings_changes() {
        let s = change(view(&["wordcraft", "effectcraft"]), Some(true), &["effectcraft".into()], &[]).unwrap();
        assert!(s.web_apps);
        assert_eq!(s.disabled_apps, ["effectcraft"]);
        let s = change(s, None, &[], &["effectcraft".into()]).unwrap();
        assert!(s.disabled_apps.is_empty());
        assert_eq!(change(view(&[]), Some(true), &[], &[]).unwrap_err().kind.code(), 5, "slim image");
        assert_eq!(change(view(&["wordcraft"]), None, &["nope".into()], &[]).unwrap_err().kind.code(), 4);
    }
}
