//! The cases, in order: each one starts from the state the one before left. Every step is the
//! keyboard (the Tab order is the drawing order: header, footer, then the page, and a sheet last);
//! every check is the server's API, the cookie jar or the network log.

use std::time::{Duration, Instant};

use serde_json::json;

use super::Run;
use super::api::Auth;

/// OxiCloud's session cookie (HttpOnly) and its CSRF cookie (readable by the page, by design).
const SESSION_COOKIE: &str = "oxicloud_access";
const CSRF_COOKIE: &str = "oxicloud_csrf";

// The cases after sign-in, by name (the table's rows).
const UPLOAD: &str = "upload";
const SHARE: &str = "share link, opened signed out";
const DEVICE: &str = "device approval";
const ADMIN: &str = "admin change carries the CSRF header";
const FORGED: &str = "forged admin writes are refused";

pub fn all(run: &mut Run) {
    let result = setup(run);
    if !run.row("first-run setup makes the owner", result) {
        // Everything after needs the owner.
        for case in ["sign out", "sign in, session cookie", UPLOAD, SHARE, DEVICE, ADMIN, FORGED] {
            run.skip(case, "first-run setup failed");
        }
        return;
    }
    let result = sign_out(run);
    run.row("sign out", result);
    let result = sign_in(run);
    let signed_in = run.row("sign in, session cookie", result);
    // The run's own sign-in, for checking what the browser did. One sign-in, kept for the run:
    // OxiCloud limits sign-ins per address, and the browser has already used two.
    match run.api.sign_in(&run.user, &run.password) {
        Ok(tokens) => run.bearer = Some(tokens),
        Err(e) => {
            for case in [UPLOAD, SHARE, DEVICE, ADMIN, FORGED] {
                run.skip(case, &format!("the run couldn't sign in to check: {e}"));
            }
            return;
        }
    }
    if !signed_in {
        for case in [UPLOAD, SHARE, DEVICE, ADMIN, FORGED] {
            run.skip(case, "the browser isn't signed in");
        }
        return;
    }
    let result = upload(run);
    let name = result.as_ref().ok().map(|(name, _)| name.clone());
    run.row(UPLOAD, result.map(|(_, detail)| detail));
    match name {
        Some(name) => {
            let result = share(run, &name);
            run.row(SHARE, result);
        }
        None => run.skip(SHARE, "nothing was uploaded to share"),
    }
    let result = device(run);
    run.row(DEVICE, result);
    let result = admin_change(run);
    run.row(ADMIN, result);
    let result = forged(run);
    run.row(FORGED, result);
}

/// Polls `check` every half second until it says yes, for at most `secs` seconds.
fn wait_for(secs: u64, what: &str, mut check: impl FnMut() -> Result<bool, String>) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if check()? {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("{what} (waited {secs} s)"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn bearer(run: &Run) -> Result<String, String> {
    run.bearer.as_ref().map(|(access, _)| access.clone()).ok_or_else(|| "the run isn't signed in".to_owned())
}

/// The value of one of the browser's cookies.
fn cookie(run: &Run, name: &str) -> Result<Option<super::webdriver::Cookie>, String> {
    Ok(run.owner.cookies()?.into_iter().find(|c| c.name == name))
}

/// `GET /api/auth/me` with the browser's cookies: the status and the username.
fn me_in_browser(run: &Run) -> Result<(u16, String), String> {
    let cookies = run.owner.cookie_header()?;
    let a = run.api.call("GET", "api/auth/me", Auth::Cookie(&cookies, None), None)?;
    Ok((a.status, username_of(&a.json)))
}

/// The username in `/api/auth/me`'s answer (`{"full": {"user": {"username": …}}}` in OxiCloud
/// 0.10.0).
fn username_of(me: &serde_json::Value) -> String {
    [&me["full"]["user"]["username"], &me["user"]["username"], &me["username"]].iter().find_map(|v| v.as_str()).unwrap_or_default().to_owned()
}

/// Whether a request the page sent carried `X-CSRF-Token` equal to the `oxicloud_csrf` cookie
/// (compared here, never printed).
fn csrf_matches(run: &Run, req: &super::browser::SentRequest) -> Result<bool, String> {
    let expected = cookie(run, CSRF_COOKIE)?.map(|c| c.value);
    Ok(matches!((req.header("X-CSRF-Token"), expected), (Some(sent), Some(want)) if !sent.is_empty() && sent == want))
}

/// The five setup pages, by keyboard: Start setup; the owner's four fields, then Create account;
/// Continue (address); Continue (web apps, the slim image); Go to home.
fn setup(run: &mut Run) -> Result<String, String> {
    let url = run.url("/");
    // Shift+Tab from nothing goes to the last widget of the frame before, so a press too soon
    // after the page changed can start one widget off (seen once in about ten runs: the username
    // went into Email). Then nothing is made, and a second try from a fresh page follows.
    let mut tries = 0;
    loop {
        tries += 1;
        let b = &mut run.owner;
        b.open_page(&url, "setup-welcome")?;
        b.press_last()?; // Start setup
        pause(1000);
        b.shot("setup-owner");
        // Create account stays disabled (and out of the Tab order) until the fields are filled,
        // so from the end: Back, Repeat password, Password, Email, Username.
        b.escape()?;
        pause(200);
        b.shift_tab(5)?;
        b.fill(&[&run.user, &format!("{}@example.org", run.user), &run.password, &run.password])?;
        b.tab(2)?; // Back, then Create account
        b.shot("setup-owner-filled");
        b.enter()?;
        let api = &run.api;
        match wait_for(15, "the server has no owner after Create account", || api.initialized()) {
            Ok(()) => break,
            Err(e) if tries >= 2 => return Err(format!("{e}, in two tries")),
            Err(_) => {}
        }
    }
    let b = &mut run.owner;
    // The page signs in with the same details; the session cookie arrives with that.
    wait_for(15, "no session cookie after setup signed in", || Ok(b.cookies()?.iter().any(|c| c.name == SESSION_COOKIE)))?;
    pause(500);
    b.shot("setup-address");
    b.press_last()?; // Continue
    // The web-apps page loads the app list and the settings before its Continue works.
    pause(2500);
    b.shot("setup-web-apps");
    b.press_last()?; // Continue
    pause(500);
    b.shot("setup-done");
    b.press_last()?; // Go to home
    pause(1000);
    b.shot("setup-home");
    let (status, name) = me_in_browser(run)?;
    if status != 200 || name != run.user {
        return Err(format!("after setup, /api/auth/me with the browser's cookies answered {status} for `{name}`"));
    }
    let second = if tries > 1 { " (on the second try)" } else { "" };
    Ok(format!("owner `{}` made through the five setup pages{second}; the browser is signed in as them", run.user))
}

/// SIGN OUT in the header: the fourth widget on the home page (HOME, FILES, ADMIN, SIGN OUT).
fn sign_out(run: &mut Run) -> Result<String, String> {
    let url = run.url("/");
    let b = &mut run.owner;
    b.open_page(&url, "home")?;
    // The home page asks for the devices first; SIGN OUT waits for that.
    pause(1500);
    let _ = b.network()?;
    b.escape()?;
    b.tab(4)?;
    b.enter()?;
    wait_for(10, "the session cookie is still there after SIGN OUT", || Ok(!b.cookies()?.iter().any(|c| c.name == SESSION_COOKIE)))?;
    b.shot("signed-out");
    let net = b.network()?;
    let logout = net.request("POST", "/api/auth/logout").ok_or("the page sent no POST /api/auth/logout")?;
    let csrf = if logout.header("X-CSRF-Token").is_some() { "with" } else { "without" };
    let (status, _) = me_in_browser(run)?;
    if status != 401 {
        return Err(format!("signed out, but /api/auth/me with the browser's cookies answered {status}, not 401"));
    }
    Ok(format!("POST /api/auth/logout ({csrf} X-CSRF-Token); the session cookie is gone and /api/auth/me answers 401"))
}

/// The sign-in page by keyboard: Username, Tab, Password, Enter. Then the session cookie's flags.
fn sign_in(run: &mut Run) -> Result<String, String> {
    let url = run.url("/");
    let https = url.starts_with("https://");
    let b = &mut run.owner;
    b.open_page(&url, "sign-in")?;
    // SIGN IN is disabled until both fields are filled, so the last two widgets are the fields.
    b.escape()?;
    b.shift_tab(2)?;
    b.fill(&[&run.user, &run.password])?;
    b.enter()?;
    wait_for(15, "no session cookie after signing in", || Ok(b.cookies()?.iter().any(|c| c.name == SESSION_COOKIE)))?;
    pause(800);
    b.shot("signed-in");
    let (status, name) = me_in_browser(run)?;
    if status != 200 || name != run.user {
        return Err(format!("/api/auth/me with the browser's cookies answered {status} for `{name}`"));
    }
    let session = cookie(run, SESSION_COOKIE)?.ok_or("no session cookie")?;
    let csrf = cookie(run, CSRF_COOKIE)?.ok_or("no CSRF cookie")?;
    let mut problems = Vec::new();
    if session.http_only != Some(true) {
        problems.push(format!("{SESSION_COOKIE} isn't HttpOnly"));
    }
    if session.secure.unwrap_or(false) != https {
        problems.push(format!("{SESSION_COOKIE} Secure is {:?} on {}", session.secure, if https { "https" } else { "http" }));
    }
    if csrf.http_only == Some(true) {
        problems.push(format!("{CSRF_COOKIE} is HttpOnly, so the page can't send it back"));
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    let same_site = session.same_site.unwrap_or_else(|| "unset".to_owned());
    Ok(format!(
        "signed in as `{name}`; {SESSION_COOKIE}: HttpOnly, Secure {} ({}), SameSite {same_site}; {CSRF_COOKIE} readable by the page",
        if https { "on" } else { "off" },
        if https { "https" } else { "http" }
    ))
}

/// UPLOAD FILES on the empty My files page (the last widget there), the file handed to the page's
/// file input; then the file over WebDAV, byte for byte.
fn upload(run: &mut Run) -> Result<(String, String), String> {
    let name = format!("e2e-web-{}.txt", run.secs);
    let content = format!("Uploaded by cargo xtask e2e-web at {}.\n", run.secs);
    let local = run.dir.join(&name);
    std::fs::write(&local, &content).map_err(|e| format!("couldn't write {}: {e}", local.display()))?;
    let url = run.url("/files");
    let b = &mut run.owner;
    b.open_page(&url, "files-empty")?;
    pause(1000);
    let _ = b.network()?;
    b.press_last()?; // Upload files
    b.choose_file(&local)?;
    let token = bearer(run)?;
    let api = &run.api;
    wait_for(20, &format!("{name} isn't in the cloud"), || Ok(api.dav_get(&token, &name)?.is_some_and(|got| got == content.as_bytes())))?;
    let b = &mut run.owner;
    pause(500);
    b.shot("uploaded");
    let net = b.network()?;
    let put = net.request("PUT", &format!("/webdav/{name}")).ok_or_else(|| format!("the page sent no PUT /webdav/{name}"))?;
    if !csrf_matches(run, put)? {
        return Err("the upload's PUT carried no X-CSRF-Token matching the CSRF cookie".into());
    }
    Ok((name.clone(), format!("{name} chosen in the page's file input, uploaded with the CSRF header, and read back the same over WebDAV")))
}

/// The file's "…" menu (the last widget on the page), SHARE LINK… (its third item), CREATE LINK
/// (the sheet's last); then the link in a second browser that has never signed in.
fn share(run: &mut Run, name: &str) -> Result<String, String> {
    let token = bearer(run)?;
    let before = run.api.shares(&token)?;
    let url = run.url("/files");
    let b = &mut run.owner;
    b.open_page(&url, "files")?;
    pause(1000);
    b.press_last()?; // the row's "…"
    pause(300);
    b.tab(3)?; // Download, Versions…, Share link…
    b.enter()?;
    pause(800);
    b.shot("share-sheet");
    // The menu item is gone, so nothing has focus: Shift+Tab is the sheet's last button.
    b.shift_tab(1)?;
    b.enter()?; // Create link
    let api = &run.api;
    let mut made = None;
    wait_for(15, "no new share link on the server", || {
        let now = api.shares(&token)?;
        made = now.into_iter().find(|s| !before.contains(s));
        Ok(made.is_some())
    })?;
    let (link, item) = made.ok_or("no new share link")?;
    run.owner.shot("share-made");
    if !item.is_empty() && item != name {
        return Err(format!("the new link shares `{item}`, not `{name}`"));
    }
    let page = run.url(&format!("/s/{link}"));
    let v = run.visitor()?;
    v.open_page(&page, "share-page")?;
    pause(1000);
    let net = v.network()?;
    let status = net.status(&format!("/api/s/{link}")).ok_or("the share page never asked the server for its link")?;
    if status != 200 {
        return Err(format!("the share page's GET /api/s/<token> answered {status}"));
    }
    if v.cookies()?.iter().any(|c| c.name == SESSION_COOKIE) {
        return Err("the visitor's browser has a session cookie".into());
    }
    // DOWNLOAD is the page's last widget; the file lands in the visitor's download folder.
    v.press_last()?;
    let saved = super::Browser::downloads_in(&run.dir, "visitor").join(name);
    let want = run.api.dav_get(&token, name)?.ok_or_else(|| format!("{name} is gone from the cloud"))?;
    wait_for(20, "the visitor's DOWNLOAD saved nothing", || Ok(std::fs::read(&saved).is_ok_and(|got| got == want)))?;
    Ok(format!(
        "link made from the file's menu; /s/<token> opened in a second browser without a session (GET /api/s/<token> {status}), and its DOWNLOAD saved the same bytes"
    ))
}

/// The device flow started over the API, approved on `/device?code=…` (APPROVE is the sheet's
/// last button), and the token collected by polling.
fn device(run: &mut Run) -> Result<String, String> {
    let start =
        run.api.call("POST", "api/auth/device/authorize", Auth::None, Some(&json!({"client_name": "CraftCloud on Studio", "scope": "webdav"})))?;
    if !start.ok() {
        return Err(format!("POST /api/auth/device/authorize answered {}", start.status));
    }
    let device_code = start.json["device_code"].as_str().ok_or("no device_code")?.to_owned();
    let page = start.json["verification_uri_complete"].as_str().ok_or("no verification_uri_complete")?;
    // The page under the server's own address, whatever public URL it was given.
    let path = page.find("/device").map(|i| &page[i..]).ok_or("verification_uri_complete has no /device")?;
    let interval = start.json["interval"].as_u64().unwrap_or(5).max(1);
    let url = run.url(path);
    let b = &mut run.owner;
    b.open_page(&url, "device")?;
    // It asks the server about the code before it shows APPROVE.
    pause(1500);
    b.shot("device-approve");
    let _ = b.network()?;
    b.press_last()?; // Approve
    pause(800);
    b.shot("device-approved");
    let net = b.network()?;
    let verify = net.request("POST", "/api/auth/device/verify").ok_or("the page sent no POST /api/auth/device/verify")?;
    let csrf = csrf_matches(run, verify)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut wait = interval;
    let access = loop {
        let poll = run.api.call(
            "POST",
            "api/auth/device/token",
            Auth::None,
            Some(&json!({"device_code": device_code, "grant_type": "urn:ietf:params:oauth:grant-type:device_code"})),
        )?;
        if poll.status == 200 {
            break super::api::tokens(&poll.json).ok_or("the token answer has no access_token and refresh_token")?;
        }
        match poll.json["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => wait += 5,
            other => return Err(format!("polling the device token answered {} ({})", poll.status, other.unwrap_or("?"))),
        }
        if Instant::now() > deadline {
            return Err("the device was never approved (polled 60 s)".into());
        }
        std::thread::sleep(Duration::from_secs(wait));
    };
    let (access, refresh) = access;
    let me = run.api.call("GET", "api/auth/me", Auth::Bearer(&access), None)?;
    run.api.sign_out(&access, &refresh);
    let name = username_of(&me.json);
    if me.status != 200 || name != run.user {
        return Err(format!("the device's token: /api/auth/me answered {} for `{name}`", me.status));
    }
    if !csrf {
        return Err("approved, but the approval's POST carried no X-CSRF-Token matching the CSRF cookie".into());
    }
    Ok(format!("approved on /device with the CSRF header; the polled token signs in as `{name}`"))
}

/// NEW USER on Admin › Users (the eleventh widget: HOME, FILES, ADMIN, SIGN OUT, the account
/// cell, DOCS, SOURCE, refresh, USERS, SERVER, NEW USER), its four fields, CREATE.
fn admin_change(run: &mut Run) -> Result<String, String> {
    let token = bearer(run)?;
    let user = format!("e2eweb{}", run.secs);
    let password = format!("{}-{}", run.password, "new");
    let url = run.url("/admin/users");
    // NEW USER keeps the focus under the sheet, and Escape would close the sheet: a click on the
    // empty header lets go of it instead. That click doesn't always take (one run in four or five
    // the fields were filled two places off), so a second try starts again from a fresh page; the
    // detail says when it was needed.
    let mut tries = 0;
    loop {
        tries += 1;
        let b = &mut run.owner;
        b.open_page(&url, "admin-users")?;
        pause(1500);
        let _ = b.network()?;
        b.escape()?;
        b.tab(11)?;
        b.enter()?; // New user
        pause(600);
        b.shot("new-user");
        b.focus()?;
        pause(300);
        // CREATE is disabled until filled, so from the end: CANCEL, ADMIN, USER, Storage,
        // Password, Email, Username.
        b.shift_tab(7)?;
        b.fill(&[&user, &format!("{user}@example.org"), &password])?;
        b.tab(5)?; // Storage, USER, ADMIN, CANCEL, CREATE
        b.shot("new-user-filled");
        b.enter()?;
        let api = &run.api;
        let made = wait_for(10, &format!("no user `{user}` on the server after CREATE"), || Ok(api.usernames(&token)?.contains(&user)));
        match made {
            Ok(()) => break,
            Err(e) if tries >= 2 => return Err(format!("{e}, in two tries")),
            Err(_) => {}
        }
    }
    let b = &mut run.owner;
    pause(500);
    b.shot("new-user-made");
    let net = b.network()?;
    let post = net.request("POST", "/api/admin/users").ok_or("the page sent no POST /api/admin/users")?;
    if !csrf_matches(run, post)? {
        return Err("the user was made, but the POST carried no X-CSRF-Token matching the CSRF cookie".into());
    }
    let second = if tries > 1 { " (on the second try)" } else { "" };
    Ok(format!("user `{user}` made on Admin › Users{second}; the POST carried X-CSRF-Token equal to the {CSRF_COOKIE} cookie"))
}

/// The browser's session replayed outside it, as a forged cross-site request would arrive: the
/// CraftCloud settings write and OxiCloud's user creation, without the CSRF header and with a
/// wrong one, refused; the same settings write with the right header (the control) allowed.
fn forged(run: &mut Run) -> Result<String, String> {
    let token = bearer(run)?;
    let cookies = run.owner.cookie_header()?;
    let csrf = cookie(run, CSRF_COOKIE)?.map(|c| c.value).ok_or("no CSRF cookie in the browser")?;
    // What the slim image has: web apps off. Writing it again changes nothing.
    let settings = json!({"web_apps": false, "disabled_apps": []});
    let mut seen = Vec::new();
    for (label, header, want) in
        [("without X-CSRF-Token", None, 403), ("with a wrong X-CSRF-Token", Some("forged"), 403), ("with the right one", Some(csrf.as_str()), 200)]
    {
        let a = run.api.call("PUT", "craftcloud/api/settings", Auth::Cookie(&cookies, header), Some(&settings))?;
        if a.status != want {
            return Err(format!("PUT /craftcloud/api/settings {label} answered {}, not {want}", a.status));
        }
        seen.push(format!("{label} {}", a.status));
    }
    let user = format!("e2eforged{}", run.secs);
    let body = json!({"username": user, "email": format!("{user}@example.org"), "password": format!("{}-x", run.password), "role": "user"});
    let a = run.api.call("POST", "api/admin/users", Auth::Cookie(&cookies, None), Some(&body))?;
    if a.ok() || run.api.usernames(&token)?.contains(&user) {
        return Err(format!("POST /api/admin/users without X-CSRF-Token answered {} and the user exists", a.status));
    }
    Ok(format!("settings write {}; POST /api/admin/users without it {}", seen.join(", "), a.status))
}

#[cfg(test)]
mod tests {
    use super::username_of;

    #[test]
    fn the_username_is_read_from_oxicloud_0_10_0s_me_answer() {
        let me: serde_json::Value = serde_json::from_str(include_str!("../../../crates/core/tests/fixtures/oxicloud-0.10.0/me.json")).unwrap();
        assert_eq!(username_of(&me), "john");
        assert_eq!(username_of(&serde_json::json!({})), "");
    }
}
