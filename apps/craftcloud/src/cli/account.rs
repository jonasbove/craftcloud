//! Signing in and out, the desktop client's sync (through its control channel), storage use, and
//! opening the local copy of a file.

use std::time::{Duration, Instant};

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::{self, DevicePoll};
use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
use craftcloud_core::names::{self, Platform};
use craftcloud_core::storage::probe::{Base, PROPFIND_BODY};
use craftcloud_core::sync::{Storage, SyncPlan};
use craftcloud_native::control::{self, LinkHandoff, Request, SendError, Status};
use craftcloud_native::experimental;
use craftcloud_native::http::execute;
use craftcloud_native::open::{self, OpenError};
use craftcloud_native::secrets::Credential;
use craftcloud_native::session::{AppPasswordSession, Session};
use craftcloud_native::settings::Prefs;
use craftcloud_native::storage::probe::Limits;
use craftcloud_native::storage::profile::nextcloud::{self, Nextcloud};
use craftcloud_native::storage::profile::opencloud::{self, Opencloud};
use craftcloud_native::storage::webdav::WebDav;
use serde_json::json;

use super::output::{self, CliError, Fail, Result, print_json};
use super::remote::{Ctx, clean};

fn send(req: &craftcloud_core::http::HttpRequest) -> Result<craftcloud_core::http::HttpResponse> {
    execute(req).map_err(CliError::transport)
}

/// The environment variable `login --kind webdav` reads the app password from, instead of asking
/// (never a command-line argument: it would land in the shell history and the process list).
pub const APP_PASSWORD_VAR: &str = "CRAFTCLOUD_APP_PASSWORD";

/// `login [server] [--kind KIND] [--user NAME]`: the device flow for CraftCloud's own server, as
/// in the desktop client; with `--kind nextcloud`, Nextcloud's own browser sign-in (Login Flow v2,
/// or an app password with `--user`); with `--kind opencloud`, an app token the person made in
/// OpenCloud's web UI (`--user`); with `--kind webdav`, another WebDAV server with an app
/// password. Without `--kind`, and with the opt-in, a server that says it is Nextcloud or
/// OpenCloud (`status.php`) is signed in to as one; detection only suggests the kind, the check
/// on connect measures what it allows (docs/WEBDAV-STORAGE.md § 2, § 11).
pub fn login(ctx: &mut Ctx, server: Option<&str>, no_browser: bool, kind: Option<&str>, user: Option<&str>) -> Result<()> {
    let mut kind = match kind {
        None => Storage::Oxicloud,
        Some(word) => Storage::parse(word).ok_or_else(|| CliError::usage(format!("--kind {word}: use oxicloud, nextcloud, opencloud or webdav")))?,
    };
    if kind == Storage::Oxicloud
        && experimental::experimental_webdav()
        && let Some(given) = server
        && let Ok(origin) = ServerUrl::parse(given)
    {
        let detected = nextcloud::detect(&origin.to_string())
            .map(|d| (d, Storage::Nextcloud))
            .or_else(|| opencloud::detect(&origin.to_string()).map(|d| (d, Storage::Opencloud)));
        if let Some((found, as_kind)) = detected {
            if !ctx.json {
                eprintln!("{} at {origin}: signing in the way {} does.", found.words(), as_kind.words());
            }
            kind = as_kind;
        }
    }
    match kind {
        Storage::Oxicloud => {}
        // Only with the opt-in (docs/WEBDAV-STORAGE.md § 16), and only a kind with a driver.
        _ if !experimental::experimental_webdav() => return Err(CliError::usage(experimental::REFUSED)),
        Storage::Webdav => return login_webdav(ctx, server, user),
        Storage::Nextcloud => return login_nextcloud(ctx, server, user, no_browser),
        Storage::Opencloud => return login_opencloud(ctx, server, user),
        Storage::Owncloud => return Err(CliError::usage(experimental::NOT_YET)),
    }
    if user.is_some() {
        return Err(CliError::usage("--user goes with --kind webdav, nextcloud or opencloud; a CraftCloud server signs you in through the browser"));
    }
    let server = match server {
        Some(s) => ServerUrl::parse(s).map_err(|e| CliError::usage(format!("{s}: {e}")))?,
        None => ctx.server().map_err(|_| CliError::usage("Which server? Run `craftcloud login <server>`."))?,
    };
    let resp = send(&auth::device_authorize(&server, &craftcloud_native::client_name()))?;
    let start = auth::parse_device_authorize(&resp).map_err(|e| CliError::api(&e).context("starting the sign-in"))?;
    if ctx.json {
        print_json(&json!({"user_code": start.user_code, "verification_uri": start.browser_url(), "expires_in": start.expires_in}))?;
    } else {
        eprintln!("To sign in, open {} and approve the code {}", start.browser_url(), start.user_code);
    }
    if !no_browser && craftcloud_native::open_url(start.browser_url()).is_err() && !ctx.json {
        eprintln!("(Couldn't open a browser; open the address yourself.)");
    }
    let mut interval = Duration::from_secs(start.interval.max(1));
    let deadline = Instant::now() + Duration::from_secs(start.expires_in);
    let tokens = loop {
        if Instant::now() > deadline {
            return Err(CliError::new(Fail::SignedOut, "The code ran out before it was approved. Run `craftcloud login` again."));
        }
        std::thread::sleep(interval);
        let resp = send(&auth::device_token(&server, &start.device_code))?;
        match auth::parse_device_token(&resp).map_err(|e| CliError::api(&e))? {
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval += Duration::from_secs(5),
            DevicePoll::Approved(t) => break t,
            DevicePoll::Denied => return Err(CliError::new(Fail::SignedOut, "The sign-in was denied.")),
            DevicePoll::Expired => return Err(CliError::new(Fail::SignedOut, "The code ran out. Run `craftcloud login` again.")),
        }
    };
    ctx.tokens(&server).save(&tokens).map_err(|e| CliError::error(format!("couldn't keep the sign-in: {e}")))?;
    let me = auth::parse_me(&send(&tokens.authorize(auth::me(&server)))?).map_err(|e| CliError::api(&e))?;
    let username = me.map(|m| m.user.username).unwrap_or_default();
    // The desktop client's server, when it has none yet; a different one stays as it is.
    let mut settings = ctx.settings();
    let default = settings.server.is_empty() || settings.server == server.to_string();
    if settings.server.is_empty() {
        settings.server = server.to_string();
        ctx.store.set(Prefs(settings));
        ctx.store.flush().map_err(|e| CliError::error(format!("couldn't save the settings: {e}")))?;
    }
    // A desktop client running and waiting for a sign-in takes this one (one that isn't, finds it
    // when it starts next).
    if default && let Some(dir) = ctx.dir() {
        control::signed_in(&dir);
    }
    if ctx.json {
        print_json(&json!({"signed_in": true, "server": server.to_string(), "username": username, "default": default}))
    } else {
        println!("Signed in to {server} as {username}.");
        if !default {
            println!("The desktop client uses another server; pass --server {server} (or set CRAFTCLOUD_SERVER) to use this one.");
        }
        Ok(())
    }
}

/// `login <webdav address> --kind webdav --user NAME`: another WebDAV server, with an app or
/// WebDAV password the person made there, from [`APP_PASSWORD_VAR`] or a prompt (never the
/// account password, never an argument). The address is checked with one listing, the sign-in
/// saved in the keychain, the settings set to this kind of server, and the check on connect run
/// and saved (docs/WEBDAV-STORAGE.md § 10, § 11).
fn login_webdav(ctx: &mut Ctx, server: Option<&str>, user: Option<&str>) -> Result<()> {
    let address = server.ok_or_else(|| {
        CliError::usage("Which server? Give its WebDAV address: `craftcloud login https://cloud.example.org/remote.php/dav/files/NAME/ --kind webdav --user NAME`.")
    })?;
    let base = Base::parse(address).map_err(|e| CliError::usage(format!("{address}: {e}")))?;
    let user = user
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| CliError::usage("--kind webdav needs --user NAME (the user name on that server)"))?
        .trim();
    let secret = match std::env::var(APP_PASSWORD_VAR) {
        Ok(s) if !s.is_empty() => s,
        _ => super::read_secret(&format!("App or WebDAV password for {user} (an app or WebDAV password, not your account password)"))?,
    };
    let mut session = AppPasswordSession::new(base.clone(), user, &secret).map_err(CliError::usage)?;
    // One listing shows the address and the password work before anything is saved.
    let probe = HttpRequest::new(Method::Propfind, base.url().to_owned())
        .header("Depth", "0")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(PROPFIND_BODY.as_bytes().to_vec());
    let answer = session.call(&probe);
    if session.signed_out {
        return Err(CliError::new(Fail::SignedOut, format!("{} refused the user name or password (401).", base.url())));
    }
    let resp = answer.map_err(CliError::transport)?;
    if resp.status != 207 {
        return Err(CliError::error(format!("{} doesn't answer like a WebDAV server (a listing answered {})", base.url(), resp.status)));
    }
    let origin = ServerUrl::parse(&base.origin()).map_err(|e| CliError::error(format!("{}: {e}", base.origin())))?;
    ctx.tokens(&origin)
        .save_credential(&Credential::AppPassword { user: user.to_owned(), secret })
        .map_err(|e| CliError::error(format!("couldn't keep the sign-in: {e}")))?;
    // The desktop client's server, when it has none yet (or this one); a different one stays.
    let mut settings = ctx.settings();
    let default = settings.server.is_empty() || settings.server == origin.to_string();
    if default {
        settings.server = origin.to_string();
        settings.storage = Storage::Webdav;
        settings.dav_url = base.url().to_owned();
        ctx.store.set(Prefs(settings));
        ctx.store.flush().map_err(|e| CliError::error(format!("couldn't save the settings: {e}")))?;
    }
    // The check on connect, so the sync (and this output) know what this server allows.
    let dir = ctx.dir();
    let (_, check) = WebDav::connect(session, &ctx.device(), dir.as_deref(), Limits::default());
    if default && let Some(dir) = dir {
        control::signed_in(&dir);
    }
    if ctx.json {
        return print_json(&json!({
            "signed_in": true,
            "server": origin.to_string(),
            "dav_url": base.url(),
            "kind": "webdav",
            "username": user,
            "mode": check.mode.as_str(),
            "refused_because": check.refused_because,
            "default": default,
        }));
    }
    println!("Signed in to {} as {user}.", base.url());
    match &check.refused_because {
        Some(why) => println!("Sync mode: {}: {why}. `craftcloud server check` says what was checked.", check.mode.words()),
        None => println!("Sync mode: {} (`craftcloud server check` says what was checked).", check.mode.words()),
    }
    if !default {
        println!("The desktop client uses another server; sign out of it first to use this one.");
    }
    Ok(())
}

/// `login <server> --kind nextcloud [--no-browser] [--user NAME]`: Nextcloud's own browser sign-in
/// (Login Flow v2, docs/WEBDAV-STORAGE.md § 10): the browser opens the server's sign-in page, the
/// client polls, and the server hands it an app password made for this device, listed there under
/// this client's name. With `--user`, an app password the person made in Settings › Security
/// instead (from [`APP_PASSWORD_VAR`] or a prompt, never an argument). Either way the client never
/// sees the account password. The WebDAV base uses the user id (`current-user-principal`), since
/// the login name can differ; the sign-in is saved in the keychain, the settings set to Nextcloud,
/// and the check on connect run with the Nextcloud profile and saved.
fn login_nextcloud(ctx: &mut Ctx, server: Option<&str>, user: Option<&str>, no_browser: bool) -> Result<()> {
    let given = server.ok_or_else(|| CliError::usage("Which server? Give its address: `craftcloud login cloud.example.org --kind nextcloud`."))?;
    let origin = ServerUrl::parse(given).map_err(|e| CliError::usage(format!("{given}: {e}")))?;
    let origin_base = Base::parse(&format!("{origin}/")).map_err(CliError::usage)?;
    if origin_base.is_plain_http() && !origin_base.is_loopback() {
        return Err(CliError::usage(format!("{origin} isn't https: a password is never sent over plain http to another computer")));
    }
    let (login_name, secret) = match user.map(str::trim).filter(|u| !u.is_empty()) {
        Some(user) => {
            let secret = match std::env::var(APP_PASSWORD_VAR) {
                Ok(s) if !s.is_empty() => s,
                _ => super::read_secret(&format!("App password for {user} (made in Settings › Security on the server, not your account password)"))?,
            };
            (user.to_owned(), secret)
        }
        None => {
            let agent = craftcloud_native::client_name();
            let json = ctx.json;
            let mut open = |url: &str| {
                if json {
                    let _ = print_json(&json!({"login_url": url}));
                } else {
                    eprintln!("To sign in, open {url} and approve this computer.");
                }
                if !no_browser && craftcloud_native::open_url(url).is_err() && !json {
                    eprintln!("(Couldn't open a browser; open the address yourself.)");
                }
            };
            let grant = nextcloud::sign_in(&origin.to_string(), &agent, &mut open, Duration::from_secs(1), Duration::from_secs(20 * 60))
                .map_err(|e| CliError::new(Fail::SignedOut, format!("{e}. Run `craftcloud login` again.")))?;
            (grant.login_name, grant.app_password)
        }
    };
    // The user id the WebDAV paths use; this also shows the password works before anything is saved.
    let mut probe = AppPasswordSession::new(origin_base, &login_name, &secret).map_err(CliError::usage)?;
    let user_id = match nextcloud::user_id(&mut probe, &origin.to_string()) {
        Ok(id) => id,
        Err(_) if probe.signed_out => return Err(CliError::new(Fail::SignedOut, format!("{origin} refused the user name or password (401)."))),
        // No answer at all: the server can't be reached (exit code 8), not a wrong kind of server.
        Err(e) if probe.offline.is_some() => return Err(CliError::transport(e)),
        Err(e) => return Err(CliError::error(format!("{origin} doesn't answer like a Nextcloud server: {e}"))),
    };
    let profile = Nextcloud::new(&origin.to_string(), &user_id);
    let base = Base::parse(&profile.files_base()).map_err(CliError::error)?;
    let session = AppPasswordSession::new(base.clone(), &login_name, &secret).map_err(CliError::usage)?;
    ctx.tokens(&origin)
        .save_credential(&Credential::AppPassword { user: login_name.clone(), secret })
        .map_err(|e| CliError::error(format!("couldn't keep the sign-in: {e}")))?;
    // The desktop client's server, when it has none yet (or this one); a different one stays.
    let mut settings = ctx.settings();
    let default = settings.server.is_empty() || settings.server == origin.to_string();
    if default {
        settings.server = origin.to_string();
        settings.storage = Storage::Nextcloud;
        settings.dav_url = base.url().to_owned();
        ctx.store.set(Prefs(settings));
        ctx.store.flush().map_err(|e| CliError::error(format!("couldn't save the settings: {e}")))?;
    }
    // The check on connect, with the Nextcloud profile's trash and versions checks.
    let dir = ctx.dir();
    let (_, check) = WebDav::connect_with(session, Box::new(profile), &ctx.device(), dir.as_deref(), Limits::default());
    if default && let Some(dir) = dir {
        control::signed_in(&dir);
    }
    if ctx.json {
        return print_json(&json!({
            "signed_in": true,
            "server": origin.to_string(),
            "dav_url": base.url(),
            "kind": "nextcloud",
            "username": login_name,
            "user_id": user_id,
            "server_version": check.server_version,
            "mode": check.mode.as_str(),
            "refused_because": check.refused_because,
            "default": default,
        }));
    }
    println!("Signed in to {origin} as {login_name} (Nextcloud {}).", check.server_version);
    match &check.refused_because {
        Some(why) => println!("Sync mode: {}: {why}. `craftcloud server check` says what was checked.", check.mode.words()),
        None => println!("Sync mode: {} (`craftcloud server check` says what was checked).", check.mode.words()),
    }
    if !default {
        println!("The desktop client uses another server; sign out of it first to use this one.");
    }
    Ok(())
}

/// `login <server> --kind opencloud --user NAME`: an app token the person made in OpenCloud's web
/// UI (Account › App Tokens › New), from [`APP_PASSWORD_VAR`] or a prompt, never an argument, as
/// the password of Basic auth with the login name (docs/WEBDAV-STORAGE.md § 10: the token works
/// whether or not the admin turned basic auth on; the client never sees the account password, and
/// no OpenID Connect flow is built, since OpenCloud registers only its own clients). The Graph API
/// names the user and the personal space (which also shows the token works), the WebDAV base is
/// that space's URL, the sign-in is saved in the keychain, the settings set to OpenCloud, and the
/// check on connect run with the OpenCloud profile and saved: download only (§ 16, Q1).
fn login_opencloud(ctx: &mut Ctx, server: Option<&str>, user: Option<&str>) -> Result<()> {
    let given = server
        .ok_or_else(|| CliError::usage("Which server? Give its address: `craftcloud login cloud.example.org --kind opencloud --user NAME`."))?;
    let origin = ServerUrl::parse(given).map_err(|e| CliError::usage(format!("{given}: {e}")))?;
    let origin_base = Base::parse(&format!("{origin}/")).map_err(CliError::usage)?;
    if origin_base.is_plain_http() && !origin_base.is_loopback() {
        return Err(CliError::usage(format!("{origin} isn't https: a password is never sent over plain http to another computer")));
    }
    let user = user.map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| {
        CliError::usage("--kind opencloud needs --user NAME (your login name) and an app token made in the server's web UI (Account › App Tokens)")
    })?;
    let secret = match std::env::var(APP_PASSWORD_VAR) {
        Ok(s) if !s.is_empty() => s,
        _ => {
            super::read_secret(&format!("App token for {user} (made in the server's web UI under Account › App Tokens, not your account password)"))?
        }
    };
    // Who is signed in and their personal space; this also shows the token works before anything is saved.
    let mut probe = AppPasswordSession::new(origin_base, user, &secret).map_err(CliError::usage)?;
    let (me, drive) = match opencloud::personal_space(&mut probe, &origin.to_string()) {
        Ok(found) => found,
        Err(_) if probe.signed_out => return Err(CliError::new(Fail::SignedOut, format!("{origin} refused the user name or app token (401)."))),
        // No answer at all: the server can't be reached (exit code 8), not a wrong kind of server.
        Err(e) if probe.offline.is_some() => return Err(CliError::transport(e)),
        Err(e) => return Err(CliError::error(format!("{origin} doesn't answer like an OpenCloud server: {e}"))),
    };
    let profile = Opencloud::new(&origin.to_string(), &drive.id);
    let base = Base::parse(&profile.files_base()).map_err(CliError::error)?;
    let session = AppPasswordSession::new(base.clone(), user, &secret).map_err(CliError::usage)?;
    ctx.tokens(&origin)
        .save_credential(&Credential::AppPassword { user: user.to_owned(), secret })
        .map_err(|e| CliError::error(format!("couldn't keep the sign-in: {e}")))?;
    // The desktop client's server, when it has none yet (or this one); a different one stays.
    let mut settings = ctx.settings();
    let default = settings.server.is_empty() || settings.server == origin.to_string();
    if default {
        settings.server = origin.to_string();
        settings.storage = Storage::Opencloud;
        settings.dav_url = base.url().to_owned();
        ctx.store.set(Prefs(settings));
        ctx.store.flush().map_err(|e| CliError::error(format!("couldn't save the settings: {e}")))?;
    }
    // The check on connect, with the OpenCloud profile's trash and versions checks and its ceiling.
    let dir = ctx.dir();
    let (_, check) = WebDav::connect_with(session, Box::new(profile), &ctx.device(), dir.as_deref(), Limits::default());
    if default && let Some(dir) = dir {
        control::signed_in(&dir);
    }
    if ctx.json {
        return print_json(&json!({
            "signed_in": true,
            "server": origin.to_string(),
            "dav_url": base.url(),
            "kind": "opencloud",
            "username": user,
            "user_id": me.id,
            "space": drive.id,
            "server_version": check.server_version,
            "mode": check.mode.as_str(),
            "refused_because": check.refused_because,
            "default": default,
        }));
    }
    println!("Signed in to {origin} as {user} (OpenCloud {}; the space \"{}\").", check.server_version, drive.name);
    match &check.refused_because {
        Some(why) => println!("Sync mode: {}: {why}. `craftcloud server check` says what was checked.", check.mode.words()),
        None => println!("Sync mode: {} (`craftcloud server check` says what was checked).", check.mode.words()),
    }
    if !default {
        println!("The desktop client uses another server; sign out of it first to use this one.");
    }
    Ok(())
}

/// `logout`: ends the sign-in on the server and forgets it here. It is forgotten here even when
/// the server can't be told; then the output says so (`server_session_ended: false`) and the exit
/// code is 8 (the server can't be reached) or 1 (it refused). On Nextcloud the app password is
/// deleted on the server (`DELETE /ocs/v2.php/core/apppassword`); on another WebDAV server it isn't
/// ended there (the person removes it), only forgotten here.
pub fn logout(ctx: &mut Ctx) -> Result<()> {
    let server = ctx.server()?;
    let store = ctx.tokens(&server);
    let had = store.load_credential();
    let nextcloud = ctx.settings().storage == Storage::Nextcloud;
    let ended = had.as_ref().map(|credential| match credential {
        Credential::Bearer(tokens) => {
            // Shared with the desktop client: a refresh on the way is done under its lock.
            let mut session = Session::shared(server.clone(), tokens.clone(), ctx.tokens(&server), ctx.store.dir());
            let answer = session.logout();
            server_logout(answer, session.signed_out)
        }
        Credential::AppPassword { user, secret } if nextcloud => {
            let origin = server.to_string();
            let answer = Base::parse(&format!("{origin}/")).and_then(|base| AppPasswordSession::new(base, user, secret)).and_then(|mut session| {
                session.call(&craftcloud_core::api::nextcloud::app_password_delete(&origin)).map(|r| (r, session.signed_out))
            });
            match answer {
                Ok((resp, signed_out)) => server_logout(Ok(resp), signed_out),
                Err(e) => server_logout(Err(e), false),
            }
        }
        Credential::AppPassword { .. } => Ok(()),
    });
    let app_password = matches!(had, Some(Credential::AppPassword { .. })) && !nextcloud;
    store.delete();
    let mut settings = ctx.settings();
    if settings.server == server.to_string() {
        settings = craftcloud_core::sync::ClientSettings { server: settings.server, ..Default::default() };
        ctx.store.set(Prefs(settings));
        ctx.store.flush().map_err(|e| CliError::error(format!("couldn't save the settings: {e}")))?;
    }
    let server_session_ended = matches!(ended, Some(Ok(()))) && !app_password;
    if ctx.json {
        print_json(&json!({
            "signed_out": true,
            "server": server.to_string(),
            "was_signed_in": had.is_some(),
            "server_session_ended": server_session_ended,
        }))?;
    } else {
        println!(
            "{}",
            match &ended {
                None => format!("Wasn't signed in to {server}."),
                Some(Ok(())) if app_password => {
                    format!("Signed out of {server} on this computer. The app password or token stays valid on the server until you remove it there.")
                }
                Some(Ok(())) => format!("Signed out of {server}."),
                Some(Err(_)) => format!("Signed out of {server} on this computer."),
            }
        );
    }
    match ended {
        Some(Err(e)) => Err(e),
        _ => Ok(()),
    }
}

/// What the server's answer to ending the sign-in means: `Ok` when it is over there (ended now, or
/// the server had already ended it: `signed_out`, its refresh was refused), else the error to
/// exit with, which says the sign-in may still be active on the server.
pub(super) fn server_logout(answer: std::result::Result<HttpResponse, String>, signed_out: bool) -> Result<()> {
    const STILL: &str = "so the sign-in may still be active there until it runs out";
    match answer {
        Ok(resp) if resp.is_success() => Ok(()),
        Ok(resp) => Err(CliError::error(format!("the server didn't end the sign-in ({}), {STILL}.", resp.status))),
        Err(_) if signed_out => Ok(()),
        Err(e) => {
            let kind = CliError::transport(e.clone()).kind;
            Err(CliError::new(kind, format!("couldn't tell the server ({e}), {STILL}.")))
        }
    }
}

fn client(ctx: &Ctx, request: &Request) -> Result<Status> {
    let dir = ctx.dir().ok_or_else(|| CliError::error("no settings folder"))?;
    control::send(&dir, request).map_err(|e| match e {
        SendError::NotRunning => CliError::new(Fail::NoClient, "The desktop client isn't running. Start CraftCloud first."),
        SendError::Failed(why) => CliError::error(why),
    })
}

fn show_status(ctx: &Ctx, s: &Status) -> Result<()> {
    if ctx.json {
        return print_json(s);
    }
    if !s.syncing {
        println!("The desktop client is running but not signed in.");
        return Ok(());
    }
    println!("{}", state_words(s));
    println!("server   {}", s.server);
    println!("folder   {}", s.local_folder);
    if !s.mode.is_empty() && s.mode != "two_way" {
        // Said only where it isn't the usual: CraftCloud's own server syncs both ways.
        println!("mode     {}", mode_words(&s.mode));
    }
    if s.files_total > 0 {
        println!("files    {} of {}", s.files_done, s.files_total);
    }
    for c in &s.conflicts {
        println!("conflict {c}");
    }
    for n in &s.no_room {
        println!("not synced {n}  (no room in the cloud)");
    }
    for n in &s.not_synced {
        println!("{}", not_synced_line(n));
    }
    for a in &s.activity {
        println!("  {}", a.line());
    }
    Ok(())
}

/// The sync mode in plain words (`download_only` → "download only").
pub(super) fn mode_words(mode: &str) -> &str {
    match mode {
        "two_way" => "two-way",
        "download_only" => "download only (nothing is written to this server; changes made here stay here)",
        "refused" => "refused (this server can't show when a file changed; nothing is synced)",
        other => other,
    }
}

/// `not synced t13/locked.txt  (can't be read)`.
pub(super) fn not_synced_line(n: &control::NotSynced) -> String {
    format!("not synced {}  ({})", n.path, reason_words(&n.path, &n.reason, Platform::current()))
}

/// The reason in words. `--json` says `name` for every name the sync leaves out; the words tell
/// a name the cloud can't take from one this computer (Windows) can't have, by the path.
pub(super) fn reason_words<'a>(path: &str, reason: &'a str, platform: Platform) -> &'a str {
    match reason {
        "unreadable" => "can't be read",
        "symlink" => "a symbolic link, never followed",
        "name" if names::refused_in(path).is_some() => "a name the cloud can't take",
        "name" if names::local_refused_in(path, platform).is_some() => "a name this computer (Windows) can't have",
        "name" => "a name this computer can't keep apart from another (they differ only in upper and lower case)",
        "temporary" => "looks like a temporary file",
        "not_uploaded" => "new or changed here, not uploaded: this server only syncs downwards",
        other => other,
    }
}

/// How many places the last sync couldn't read (they hold the sync of what is under them).
fn unreadable(s: &Status) -> usize {
    s.not_synced.iter().filter(|n| n.unreadable()).count()
}

/// The state in a few words.
pub(super) fn state_words(s: &Status) -> String {
    let error = s.error.clone().unwrap_or_default();
    match s.state.as_str() {
        "up_to_date" if unreadable(s) > 0 => {
            let n = unreadable(s);
            format!("Up to date · {n} {} can't be read (see `not synced` below; fix the permissions to sync {}).", if n == 1 { "place" } else { "places" }, if n == 1 { "it" } else { "them" })
        }
        "up_to_date" if !s.waiting.is_empty() => {
            let n = s.waiting.len();
            format!(
                "Up to date · {n} {} still being saved: {} sync{} in the next pass.",
                if n == 1 { "file is" } else { "files are" },
                if n == 1 { "it" } else { "they" },
                if n == 1 { "s" } else { "" }
            )
        }
        "up_to_date" if !s.no_room.is_empty() => {
            let n = s.no_room.len();
            format!("Up to date · {n} not synced (no room in the cloud: free some, or empty the trash).")
        }
        "up_to_date" if !s.failed.is_empty() => {
            let n = s.failed.len();
            format!("Up to date · {n} {} couldn't sync (see the activity; tried again every pass).", if n == 1 { "file" } else { "files" })
        }
        "up_to_date" => "Up to date.".to_owned(),
        "paused" => "Paused.".to_owned(),
        "offline" => format!("Offline: {error}"),
        "signed_out" => "Signed out: sign in again.".to_owned(),
        "mass_delete" => format!(
            "Held: this sync would delete {} files (everything else syncs). Run `craftcloud sync --confirm-deletes` to delete them, or `craftcloud sync --discard-deletes` to bring them back.",
            s.pending_deletes.len()
        ),
        "folder_missing" => "Held: the sync folder is missing (moved, deleted, or on a drive that isn't there); nothing was deleted. Put it back, or make it again (empty) to download everything again.".to_owned(),
        "failed" => format!("Failed: {error}"),
        other => format!("{}…", other[..1].to_uppercase() + &other[1..]),
    }
}

/// `status`.
/// Exits with code 9 while the sync is held and waits for a decision, or a place can't be read.
pub fn status(ctx: &Ctx) -> Result<()> {
    let s = client(ctx, &Request::Status)?;
    show_status(ctx, &s)?;
    held(&s)
}

/// Code 9 while the sync waits for the person: a mass delete, a missing sync folder, or a place
/// in a synced folder that can't be read. Symbolic links and names the cloud or this computer can't have are left
/// out for good (until renamed), so they don't count.
pub(super) fn held(s: &Status) -> Result<()> {
    match s.state.as_str() {
        "mass_delete" | "folder_missing" => Err(CliError::new(Fail::Held, state_words(s))),
        "up_to_date" if unreadable(s) > 0 => Err(CliError::new(Fail::Held, state_words(s))),
        _ => Ok(()),
    }
}

/// What `sync` does about a mass-delete hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deletes {
    Hold,
    Confirm,
    Discard,
}

/// `pause` / `resume`.
pub fn pause(ctx: &Ctx, on: bool) -> Result<()> {
    let s = client(ctx, if on { &Request::Pause } else { &Request::Resume })?;
    show_status(ctx, &s)
}

/// `sync`: through the desktop client when it runs, else one pass here.
pub fn sync(ctx: &Ctx, wait: bool, once: bool, deletes: Deletes) -> Result<()> {
    let request = match deletes {
        Deletes::Hold => Request::Sync { wait },
        Deletes::Confirm => Request::ConfirmDeletes,
        Deletes::Discard => Request::DiscardDeletes,
    };
    if !once {
        // How many deletions wait, to say what is about to happen (T-042: not silence).
        let waiting = match deletes {
            Deletes::Hold => 0,
            _ => client(ctx, &Request::Status).map_or(0, |s| s.pending_deletes.len()),
        };
        if !ctx.json && waiting > 0 {
            eprintln!("{}", deletes_words(deletes, waiting, false));
        }
        match client(ctx, &request) {
            Ok(s) => {
                if !ctx.json && waiting > 0 && s.state != "mass_delete" {
                    println!("{}", deletes_words(deletes, waiting, true));
                }
                show_status(ctx, &s)?;
                return failed(&s);
            }
            Err(e) if e.kind == Fail::NoClient => {}
            Err(e) => return Err(e),
        }
    }
    if !ctx.json && deletes != Deletes::Hold {
        eprintln!("{}", deletes_words(deletes, 0, false));
    }
    super::headless::once(ctx, deletes)
}

/// `Deleting 3000 files…` / `Deleted 3000 files.` (and the same for bringing them back); `n` 0:
/// not known yet.
pub(super) fn deletes_words(deletes: Deletes, n: usize, done: bool) -> String {
    let files = if n == 0 { "the held files".to_owned() } else { format!("{n} file{}", if n == 1 { "" } else { "s" }) };
    match (deletes, done) {
        (Deletes::Discard, false) => format!("Bringing back {files}…"),
        (Deletes::Discard, true) => format!("Brought back {files}."),
        (_, false) => format!("Deleting {files}…"),
        (_, true) => format!("Deleted {files}."),
    }
}

/// A sync that ended anywhere but up to date (or paused, or still busy) is an error, with the
/// exit code for its kind.
pub(super) fn failed(s: &Status) -> Result<()> {
    let kind = match s.state.as_str() {
        "signed_out" => Fail::SignedOut,
        "offline" => Fail::Unreachable,
        "failed" if s.error.as_deref() == Some(craftcloud_native::session::SIGNED_OUT) => Fail::SignedOut,
        "mass_delete" | "folder_missing" => Fail::Held,
        "failed" => Fail::Error,
        _ => return held(s),
    };
    Err(CliError::new(kind, state_words(s)))
}

/// `usage`.
pub fn usage(ctx: &Ctx) -> Result<()> {
    if ctx.is_webdav() {
        return Err(CliError::unsupported("storage use"));
    }
    let mut remote = ctx.remote()?;
    let me = remote.ask(&auth::me(remote.server()), auth::parse_me)?.ok_or_else(|| CliError::new(Fail::SignedOut, "Not signed in."))?;
    // Kept versions count against the quota too (T-049).
    let versions = remote.session.versions_bytes().map_err(CliError::transport)?;
    if ctx.json {
        return print_json(&json!({
            "username": me.user.username,
            "used_bytes": me.storage_used_bytes,
            "quota_bytes": me.storage_quota_bytes,
            "versions_bytes": versions,
        }));
    }
    println!("{}", usage_words(me.storage_used_bytes, me.storage_quota_bytes, versions));
    Ok(())
}

/// `1.2 GB of 10 GB used (200 MB in earlier versions)`.
pub(super) fn usage_words(used: u64, quota: u64, versions: u64) -> String {
    let quota = if quota == 0 { "no limit".to_owned() } else { output::size(quota) };
    let versions = if versions == 0 { String::new() } else { format!(" ({} in earlier versions)", output::size(versions)) };
    format!("{} of {quota} used{versions}", output::size(used))
}

/// `open <path>`: the synced local copy, with the app the system picks.
pub fn open(ctx: &Ctx, path: &str) -> Result<()> {
    let path = clean(path)?;
    let local = open::locate(&SyncPlan::from(&ctx.settings()), &path).map_err(open_error)?;
    let how = open::open_how(&local).map_err(CliError::error)?;
    if ctx.json && how == open::Launched::Chooser {
        // Windows had no app for this type and shows its "Open with" dialog instead.
        return print_json(&json!({"opened": local.display().to_string(), "with": "chooser"}));
    }
    opened(ctx, &local.display().to_string())
}

/// `craftcloud craftcloud://open?path=…` (how the system starts us for a link): hands the link to
/// the running desktop client or daemon, or, when none runs, opens the file here and exits.
pub fn link(ctx: &Ctx, url: &str) -> Result<()> {
    craftcloud_core::deeplink::parse(url).map_err(|e| CliError::usage(e.to_string()))?;
    if let Some(dir) = ctx.dir() {
        match control::send_link(&dir, url) {
            LinkHandoff::Opened(local) => return opened(ctx, &local),
            LinkHandoff::Refused(e) => return Err(open_error(e)),
            LinkHandoff::NotTaken => {}
        }
    }
    let local = open::follow(&SyncPlan::from(&ctx.settings()), url).map_err(open_error)?;
    opened(ctx, &local.display().to_string())
}

fn opened(ctx: &Ctx, local: &str) -> Result<()> {
    if ctx.json { print_json(&json!({"opened": local})) } else { Ok(()) }
}

/// Exit codes: a bad link or no sync folder 2, not synced or not here 4, the system's opener 1.
fn open_error(e: OpenError) -> CliError {
    let kind = match e {
        OpenError::BadLink(_) | OpenError::NoFolder(_) => Fail::Usage,
        OpenError::NotSynced(_) | OpenError::NotHere(_) => Fail::NotFound,
        OpenError::Failed(_) => Fail::Error,
    };
    CliError::new(kind, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{failed, server_logout};
    use craftcloud_core::http::HttpResponse;
    use craftcloud_native::control::Status;

    #[test]
    fn a_logout_the_server_refused_says_so() {
        assert!(server_logout(Ok(HttpResponse::new(200, b"{}".to_vec())), false).is_ok());
        // OxiCloud 0.10.0 answered 401 to a logout without the refresh token, and `logout` said
        // "signed out" all the same while the session stayed.
        for status in [401, 500] {
            let e = server_logout(Ok(HttpResponse::new(status, Vec::new())), false).unwrap_err();
            assert_eq!(e.kind.code(), 1, "{status}: not 3, which would say \"run craftcloud login\"");
            assert!(e.message.contains("may still be active"), "{}", e.message);
        }
        let e = server_logout(Err("Can't connect to the server (refused).".into()), false).unwrap_err();
        assert_eq!(e.kind.code(), 8);
        assert!(e.message.contains("may still be active"), "{}", e.message);
        // The refresh was refused: the server had already ended the sign-in.
        assert!(server_logout(Err(craftcloud_native::session::SIGNED_OUT.into()), true).is_ok());
    }

    #[test]
    fn a_failed_sync_is_an_error() {
        let ok = Status { state: "up_to_date".into(), ..Status::default() };
        assert!(failed(&ok).is_ok());
        let bad = Status { state: "failed".into(), error: Some("disk full".into()), ..Status::default() };
        assert_eq!(failed(&bad).unwrap_err().kind.code(), 1);
        let out = Status { state: "failed".into(), error: Some(craftcloud_native::session::SIGNED_OUT.into()), ..Status::default() };
        assert_eq!(failed(&out).unwrap_err().kind.code(), 3);
        let codes: Vec<u8> = ["signed_out", "offline", "mass_delete", "folder_missing", "paused", "scanning"]
            .iter()
            .map(|s| failed(&Status { state: (*s).into(), ..Status::default() }).err().map_or(0, |e| e.kind.code()))
            .collect();
        assert_eq!(codes, [3, 8, 9, 9, 0, 0]);
    }
}
