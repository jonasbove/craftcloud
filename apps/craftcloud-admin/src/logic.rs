//! The decisions, without HTTP: what config.json says, whether an app may be served, who may
//! change settings, and reading and writing `settings.env`.

use std::io::Read;
use std::path::{Path, PathBuf};

use craftcloud_core::api::auth;
use craftcloud_core::http::HttpResponse;
use craftcloud_core::server_settings::{ServerSettings, app_of_path};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct Config {
    pub oxicloud: String,
    pub state_dir: PathBuf,
    pub public_url: String,
    pub version: String,
    /// App ids in this image's `apps.json` (empty on the slim image).
    pub bundled: Vec<String>,
}

pub const SETTINGS_FILE: &str = "settings.env";

/// The ids in `apps.json`; none when the image has no web apps.
pub fn bundled_apps(apps_json: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(apps_json) else { return Vec::new() };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else { return Vec::new() };
    doc["apps"].as_array().into_iter().flatten().filter_map(|a| a["id"].as_str().map(str::to_owned)).collect()
}

/// The settings now. A missing or unreadable file means the defaults (web apps off).
pub fn load(state_dir: &Path) -> ServerSettings {
    std::fs::read_to_string(state_dir.join(SETTINGS_FILE)).map(|t| ServerSettings::from_env_file(&t)).unwrap_or_default()
}

/// Writes the settings atomically (temporary file, then rename).
pub fn save(state_dir: &Path, settings: &ServerSettings) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let tmp = state_dir.join(format!("{SETTINGS_FILE}.tmp"));
    std::fs::write(&tmp, settings.to_env_file())?;
    std::fs::rename(&tmp, state_dir.join(SETTINGS_FILE))
}

/// The most a settings write may carry. The real payload is `web_apps` plus the ids of the apps
/// switched off: with every bundled app listed it stays well under 1 KiB.
pub const MAX_SETTINGS_BODY: usize = 4 * 1024;

/// What `PUT /craftcloud/api/settings` accepts, and nothing else: an unknown field is refused rather
/// than ignored, so a typo or a newer client never looks saved when it wasn't.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsWrite {
    web_apps: bool,
    #[serde(default)]
    disabled_apps: Vec<String>,
}

/// Reads and checks a settings write. `declared` is the request's `Content-Length`, if it sent one.
/// Err is the HTTP status and a short message: 413 above [`MAX_SETTINGS_BODY`], 400 for anything that
/// isn't exactly the settings.
pub fn read_settings_body(body: impl Read, declared: Option<usize>) -> Result<ServerSettings, (u16, &'static str)> {
    const TOO_LARGE: (u16, &str) = (413, "The settings are too large (at most 4 KiB).");
    if declared.is_some_and(|n| n > MAX_SETTINGS_BODY) {
        return Err(TOO_LARGE);
    }
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_SETTINGS_BODY).unwrap_or(u64::MAX).saturating_add(1);
    if body.take(limit).read_to_end(&mut bytes).is_err() {
        return Err((400, "Couldn't read the request body."));
    }
    if bytes.len() > MAX_SETTINGS_BODY {
        return Err(TOO_LARGE);
    }
    match serde_json::from_slice::<SettingsWrite>(&bytes) {
        Ok(w) => {
            let mut settings = ServerSettings { web_apps: w.web_apps, disabled_apps: w.disabled_apps };
            settings.normalise();
            Ok(settings)
        }
        Err(_) => Err((400, "Expected {\"web_apps\": bool, \"disabled_apps\": [ids]} and no other fields.")),
    }
}

/// 200 when the `/apps/<id>/…` file in `uri` may be served now, else 404.
pub fn gate(uri: &str, settings: &ServerSettings, bundled: &[String]) -> u16 {
    match app_of_path(uri) {
        Some(app) if settings.serves(app, bundled) => 200,
        _ => 404,
    }
}

/// `/craftcloud/config.json`. Public: the sign-in page and the desktop client read it.
pub fn public_config(config: &Config, settings: &ServerSettings) -> Value {
    let on = settings.web_apps && !config.bundled.is_empty();
    let enabled: Vec<&String> = if on { config.bundled.iter().filter(|a| settings.serves(a, &config.bundled)).collect() } else { Vec::new() };
    json!({
        "version": config.version,
        "public_url": config.public_url,
        "web_apps": on,
        "web_apps_bundled": !config.bundled.is_empty(),
        "enabled_apps": enabled,
    })
}

/// `/craftcloud/api/settings` for admins: the stored settings plus what the image can do.
pub fn settings_view(config: &Config, settings: &ServerSettings) -> Value {
    json!({
        "web_apps": settings.web_apps,
        "disabled_apps": settings.disabled_apps,
        "bundled_apps": config.bundled,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    SignedOut,
    NotAdmin,
    BadCsrf,
    Unavailable,
}

/// The value of one cookie in a `Cookie` header.
pub fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies.split(';').filter_map(|c| c.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v)
}

/// Decides access the way OxiCloud does for its own API: a Bearer token, or the session cookie;
/// writes from a cookie session need `X-CSRF-Token` equal to the `oxicloud_csrf` cookie.
/// `ask` asks OxiCloud who the credentials belong to.
pub fn admin_check(
    cookie: Option<&str>,
    bearer: Option<&str>,
    csrf: Option<&str>,
    write: bool,
    ask: impl FnOnce(Option<&str>, Option<&str>) -> Option<HttpResponse>,
) -> Access {
    let has_session = cookie.is_some_and(|c| cookie_value(c, "oxicloud_access").is_some());
    if bearer.is_none() && !has_session {
        return Access::SignedOut;
    }
    if write && bearer.is_none() {
        let expected = cookie.and_then(|c| cookie_value(c, "oxicloud_csrf"));
        if expected.is_none() || expected != csrf {
            return Access::BadCsrf;
        }
    }
    let Some(resp) = ask(cookie, bearer) else { return Access::Unavailable };
    match auth::parse_me(&resp) {
        Ok(Some(me)) if me.user.is_admin() => Access::Allowed,
        Ok(Some(_)) => Access::NotAdmin,
        Ok(None) => Access::SignedOut,
        Err(_) => Access::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::{Access, Config, MAX_SETTINGS_BODY, admin_check, gate, load, public_config, read_settings_body, save};
    use craftcloud_core::http::HttpResponse;
    use craftcloud_core::server_settings::ServerSettings;

    fn me(role: &str) -> Option<HttpResponse> {
        let body = format!(r#"{{"full":{{"user":{{"id":"1","username":"john","email":"j@example.org","role":"{role}"}}}}}}"#);
        Some(HttpResponse::new(200, body.into_bytes()))
    }

    const SESSION: &str = "oxicloud_access=abc; oxicloud_csrf=tok";

    #[test]
    fn only_admins_with_a_valid_csrf_token_may_write() {
        assert_eq!(admin_check(Some(SESSION), None, Some("tok"), true, |_, _| me("owner")), Access::Allowed);
        assert_eq!(admin_check(Some(SESSION), None, Some("tok"), true, |_, _| me("user")), Access::NotAdmin);
        assert_eq!(admin_check(Some(SESSION), None, None, true, |_, _| me("owner")), Access::BadCsrf);
        assert_eq!(admin_check(Some(SESSION), None, Some("other"), true, |_, _| me("owner")), Access::BadCsrf);
        assert_eq!(admin_check(Some(SESSION), None, None, false, |_, _| me("admin")), Access::Allowed, "reads need no CSRF token");
        assert_eq!(admin_check(None, Some("Bearer x"), None, true, |_, _| me("admin")), Access::Allowed, "Bearer is not a CSRF risk");
        assert_eq!(admin_check(None, None, None, false, |_, _| panic!("no credentials, no call")), Access::SignedOut);
        assert_eq!(admin_check(Some(SESSION), None, None, false, |_, _| Some(HttpResponse::new(401, Vec::new()))), Access::SignedOut);
        assert_eq!(admin_check(Some(SESSION), None, None, false, |_, _| None), Access::Unavailable);
    }

    #[test]
    fn the_gate_follows_the_settings() {
        let bundled = vec!["wordcraft".to_owned(), "effectcraft".to_owned()];
        let on = ServerSettings { web_apps: true, disabled_apps: vec!["effectcraft".into()] };
        assert_eq!(gate("/apps/wordcraft/index.html", &on, &bundled), 200);
        assert_eq!(gate("/apps/effectcraft/index.html", &on, &bundled), 404);
        assert_eq!(gate("/apps/wordcraft/x.wasm", &ServerSettings::default(), &bundled), 404);
        assert_eq!(gate("/somewhere/else", &on, &bundled), 404);
    }

    #[test]
    fn settings_survive_a_restart_and_config_reflects_them() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), ServerSettings::default(), "no file: web apps off");
        let s = ServerSettings { web_apps: true, disabled_apps: vec!["effectcraft".into()] };
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);

        let config = Config {
            oxicloud: String::new(),
            state_dir: dir.path().to_owned(),
            public_url: "http://x".into(),
            version: "0.1.0".into(),
            bundled: vec!["wordcraft".into(), "effectcraft".into()],
        };
        let c = public_config(&config, &s);
        assert_eq!(c["web_apps"], true);
        assert_eq!(c["enabled_apps"], serde_json::json!(["wordcraft"]));
        let slim = Config { bundled: Vec::new(), ..config };
        assert_eq!(public_config(&slim, &s)["web_apps"], false, "nothing to serve on the slim image");
    }

    #[test]
    fn a_settings_write_is_exactly_the_settings() {
        let ok = read_settings_body(&br#"{"web_apps": true, "disabled_apps": ["effectcraft", "effectcraft"]}"#[..], None);
        assert_eq!(ok, Ok(ServerSettings { web_apps: true, disabled_apps: vec!["effectcraft".into()] }));
        assert_eq!(read_settings_body(&br#"{"web_apps": false}"#[..], None), Ok(ServerSettings::default()), "disabled_apps may be left out");

        let unknown = read_settings_body(&br#"{"web_apps": true, "disabled_apps": [], "bundled_apps": ["x"]}"#[..], None);
        assert_eq!(unknown.map_err(|(status, _)| status), Err(400), "an unknown field is refused, not ignored");
        assert_eq!(read_settings_body(&b"not json"[..], None).map_err(|(status, _)| status), Err(400));
    }

    #[test]
    fn a_settings_write_is_capped() {
        // Valid JSON, just too long: a list of ids past the cap.
        let ids = vec!["wordcraft"; MAX_SETTINGS_BODY / 8];
        let big = serde_json::to_vec(&serde_json::json!({ "web_apps": true, "disabled_apps": ids })).unwrap();
        assert!(big.len() > MAX_SETTINGS_BODY);
        assert_eq!(read_settings_body(&big[..], None).map_err(|(status, _)| status), Err(413), "no Content-Length: counted while reading");
        assert_eq!(
            read_settings_body(&b"{}"[..], Some(MAX_SETTINGS_BODY + 1)).map_err(|(status, _)| status),
            Err(413),
            "a large Content-Length is refused before reading"
        );

        let head = r#"{"web_apps": true}"#;
        let at_cap = format!("{head}{}", " ".repeat(MAX_SETTINGS_BODY - head.len()));
        assert_eq!(at_cap.len(), MAX_SETTINGS_BODY);
        assert!(read_settings_body(at_cap.as_bytes(), Some(at_cap.len())).is_ok(), "exactly the cap is fine");
    }

    #[test]
    fn what_the_web_frontend_sends_is_accepted() {
        // The admin and setup pages send a serialised `ServerSettings` (crates/ui-egui/src/web).
        let some_apps = ["designcraft", "effectcraft", "materialcraft", "photocraft", "soundcraft", "texturecraft", "vectorcraft"];
        let sent = ServerSettings { web_apps: true, disabled_apps: some_apps.iter().map(|s| (*s).to_owned()).collect() };
        let body = serde_json::to_vec(&sent).unwrap();
        assert_eq!(read_settings_body(&body[..], Some(body.len())), Ok(sent));
    }
}
