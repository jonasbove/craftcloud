//! Single sign-on through OxiCloud's OpenID Connect support.
//!
//! The browser flow, through CraftCloud's own frontend and Caddy:
//! 1. The sign-in page asks [`providers`] whether a provider is set up and what it is called.
//! 2. "Sign in with …" *navigates* to [`authorize_url`] (`/api/auth/oidc/authorize`); OxiCloud
//!    answers 302 to the identity provider.
//! 3. The provider sends the browser back to `<public URL>/api/auth/oidc/callback`, and OxiCloud
//!    redirects to `<public URL>/login?oidc_code=…` (or `?login_error=…`): that is CraftCloud's
//!    frontend again.
//! 4. The frontend posts the one-time code to [`exchange`], which sets the session cookies.
//!
//! Admins read and change the settings with [`settings`] and [`save_settings`]; OxiCloud applies
//! them at once. Settings given as `OXICLOUD_OIDC_*` environment variables win and are read-only.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{ApiError, json as read_json};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// `GET /api/auth/oidc/providers` (no sign-in needed).
pub fn providers(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join("api/auth/oidc/providers"))
}

/// What the sign-in page offers.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Providers {
    pub enabled: bool,
    #[serde(default)]
    pub provider_name: String,
    /// A path on the server, `/api/auth/oidc/authorize`.
    #[serde(default)]
    pub authorize_endpoint: String,
    #[serde(default = "yes")]
    pub password_login_enabled: bool,
    /// The server wants the sign-in page to go straight to the provider (SSO-only servers).
    #[serde(default)]
    pub auto_redirect_to_oidc: bool,
}

fn yes() -> bool {
    true
}

impl Providers {
    /// The button label's name: the configured one, or "single sign-on".
    pub fn display_name(&self) -> &str {
        let name = self.provider_name.trim();
        if name.is_empty() { "single sign-on" } else { name }
    }
}

pub fn parse_providers(resp: &HttpResponse) -> Result<Providers, ApiError> {
    read_json(resp)
}

/// Where "Sign in with …" sends the browser (a navigation, never a fetch: the answer is a 302
/// to the identity provider).
pub fn authorize_url(server: &ServerUrl, providers: &Providers) -> String {
    let path = providers.authorize_endpoint.trim();
    let path = if path.is_empty() { "/api/auth/oidc/authorize" } else { path };
    server.join(path.trim_start_matches('/'))
}

/// `POST /api/auth/oidc/exchange`: swaps the one-time `oidc_code` (valid 60 s) for a session.
pub fn exchange(server: &ServerUrl, code: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/auth/oidc/exchange")).json(&json!({"code": code}))
}

/// Why the provider round trip failed, from `/login?login_error=<key>`, in words.
///
/// The keys come from OxiCloud's callback (`callback_denied`, `callback_failed` and the account
/// matching ones) and from CraftCloud's Caddyfile, which turns the raw error pages OxiCloud would
/// show into `provider_denied`, `provider_error`, `provider_unreachable` and `callback_failed`.
pub fn login_error_text(key: &str) -> String {
    let text = match key {
        // OxiCloud puts a refused or replayed state, a wrong client secret and "no account here and
        // accounts aren't created at sign-in" all under this one key.
        "callback_denied" => {
            "The identity provider's answer was refused. Try again; if it keeps happening, you may not have an account here yet, or an admin needs to check the single sign-on settings."
        }
        "callback_failed" => "Single sign-on didn't complete. Try again, or ask an admin to check the server log.",
        "provider_denied" => "The identity provider didn't sign you in (cancelled, or this account isn't allowed to use CraftCloud).",
        "provider_error" => "The identity provider reported an error. Try again, or ask an admin to check its settings for CraftCloud.",
        "provider_unreachable" => {
            "This server can't reach its identity provider right now. Try again later, or ask an admin to check the issuer URL."
        }
        "email_not_verified_at_idp" | "email_verification_required" | "auto_link_email_not_verified" => {
            "Your identity provider hasn't confirmed your email address. Verify it there, then try again."
        }
        "auto_link_disabled" | "already_linked_elsewhere" => {
            "An account with your email address already exists here and can't be joined to this sign-in. Ask an admin."
        }
        "email_ambiguous" => "More than one account here matches your email address. Ask an admin to sort it out.",
        other => return format!("Single sign-on didn't complete ({other})."),
    };
    text.to_owned()
}

/// `GET /api/admin/settings/oidc` (owners and admins).
pub fn settings(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join("api/admin/settings/oidc"))
}

/// The OIDC settings as the admin panel shows them. The client secret is never sent back, only
/// whether one is set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    #[serde(default)]
    pub issuer_url: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret_set: bool,
    #[serde(default)]
    pub scopes: String,
    #[serde(default)]
    pub auto_provision: bool,
    /// Comma-separated groups whose members become admins.
    #[serde(default)]
    pub admin_groups: String,
    /// Password sign-in is switched off (single sign-on only). The environment can switch it off
    /// too (`OXICLOUD_AUTH_METHODS`), which this doesn't show.
    #[serde(default)]
    pub disable_password_login: bool,
    #[serde(default)]
    pub provider_name: String,
    /// The redirect URI to register with the identity provider.
    #[serde(default)]
    pub callback_url: String,
    /// Field names set by `OXICLOUD_OIDC_*` environment variables (read-only here).
    #[serde(default)]
    pub env_overrides: Vec<String>,
}

impl Settings {
    /// Whether `field` (e.g. `issuer_url`) comes from the environment and can't be changed here.
    pub fn from_env(&self, field: &str) -> bool {
        self.env_overrides.iter().any(|f| f == field)
    }
}

pub fn parse_settings(resp: &HttpResponse) -> Result<Settings, ApiError> {
    read_json(resp)
}

/// What the admin panel saves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SaveSettings {
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    /// Only sent when a new one was typed; `None` keeps the stored secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub scopes: String,
    pub auto_provision: bool,
    pub admin_groups: String,
    pub disable_password_login: bool,
    pub provider_name: String,
}

/// `PUT /api/admin/settings/oidc`: saves and applies at once.
pub fn save_settings(server: &ServerUrl, s: &SaveSettings) -> HttpRequest {
    let mut s = s.clone();
    s.client_secret = s.client_secret.filter(|c| !c.is_empty());
    let body = serde_json::to_value(&s).unwrap_or_default();
    HttpRequest::new(Method::Put, server.join("api/admin/settings/oidc")).json(&body)
}

/// `POST /api/admin/settings/oidc/test`: the server fetches `<issuer>/.well-known/openid-configuration`
/// itself, so this shows whether *the server* reaches the provider (it always answers 200).
pub fn test_connection(server: &ServerUrl, issuer_url: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/admin/settings/oidc/test")).json(&json!({"issuer_url": issuer_url.trim()}))
}

/// The answer to [`test_connection`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TestResult {
    pub success: bool,
    /// What went wrong (or right), in the server's words.
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub issuer: Option<String>,
}

pub fn parse_test(resp: &HttpResponse) -> Result<TestResult, ApiError> {
    read_json(resp)
}

/// The host part of an issuer URL (`https://id.example.org/realms/x` → `id.example.org`), for
/// "Asking <host> …" while [`test_connection`] runs and for [`test_failure_text`].
pub fn issuer_host(issuer_url: &str) -> &str {
    let rest = issuer_url.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host.is_empty() { "the provider" } else { host }
}

/// A failed [`test_connection`] in plain words, and the server's own words to show under them
/// (OxiCloud's message without its fixed lead-in and its list of possible causes).
pub fn test_failure_text(issuer_url: &str, message: &str) -> (String, String) {
    let host = issuer_host(issuer_url);
    let said = message.trim();
    let said = said.strip_prefix("Failed to fetch OIDC discovery:").map_or(said, str::trim_start);
    let said = said.split_once(" Possible causes include").map_or(said, |(s, _)| s).trim().to_owned();
    // Matched on the cause alone: the list of possible causes names them all.
    let m = said.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
    let plain = if has(&["invalid discovery document"]) {
        format!("{host} answered, but not with an OpenID configuration. Check the issuer URL.")
    } else if has(&["returned http 404"]) {
        format!(
            "{host} answered, but there is no OpenID configuration at this issuer URL (HTTP 404). Check the issuer URL: it is the address in front of /.well-known/openid-configuration."
        )
    } else if has(&["returned http"]) {
        format!("{host} answered with an error instead of its OpenID configuration. Check the issuer URL, and that the provider is working.")
    } else if has(&["certificate"]) {
        format!(
            "The server doesn't trust {host}'s certificate. Give the provider a certificate from a public authority, or add the authority that signed it to the CraftCloud container."
        )
    } else if has(&["dns error", "failed to lookup address", "name does not resolve", "name or service not known", "no such host"]) {
        format!(
            "The server can't reach {host}: the name doesn't resolve from inside the CraftCloud container. Check the issuer URL, or give the provider a name the server can look up."
        )
    } else if has(&["connection refused"]) {
        format!(
            "The server can't reach {host}: nothing answers there (connection refused). Check the issuer URL, its port, and that the provider is running."
        )
    } else if has(&["timed out", "timeout", "deadline has elapsed"]) {
        format!(
            "The server can't reach {host}: it didn't answer in time. Check that the provider is running and that the CraftCloud container can reach it."
        )
    } else {
        "The server can't use this issuer URL.".to_owned()
    };
    (plain, said)
}

#[cfg(test)]
mod tests {
    use super::{
        SaveSettings, authorize_url, exchange, issuer_host, login_error_text, parse_providers, parse_settings, parse_test, save_settings,
        test_connection, test_failure_text,
    };
    use crate::ServerUrl;
    use crate::api::fixtures::read;
    use crate::api::{ApiError, expect_ok};
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8093").unwrap()
    }

    #[test]
    fn reads_the_providers_answer_off_and_on() {
        let off = parse_providers(&HttpResponse::new(200, read("oidc-providers-off.json"))).unwrap();
        assert!(!off.enabled);
        assert!(off.password_login_enabled);
        let on = parse_providers(&HttpResponse::new(200, read("oidc-providers-on.json"))).unwrap();
        assert!(on.enabled);
        assert_eq!(on.display_name(), "Example ID");
        assert!(!on.auto_redirect_to_oidc);
        assert_eq!(authorize_url(&server(), &on), "http://127.0.0.1:8093/api/auth/oidc/authorize");
    }

    #[test]
    fn an_unnamed_provider_is_single_sign_on() {
        let p = parse_providers(&HttpResponse::new(
            200,
            br#"{"enabled":true,"provider_name":" ","authorize_endpoint":"","password_login_enabled":false}"#.to_vec(),
        ))
        .unwrap();
        assert_eq!(p.display_name(), "single sign-on");
        assert_eq!(authorize_url(&server(), &p), "http://127.0.0.1:8093/api/auth/oidc/authorize");
    }

    #[test]
    fn exchanges_the_one_time_code() {
        let r = exchange(&server(), "abc");
        assert_eq!((r.method, r.url.as_str()), (Method::Post, "http://127.0.0.1:8093/api/auth/oidc/exchange"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&r.body).unwrap(), serde_json::json!({"code": "abc"}));
        let e = expect_ok(&HttpResponse::new(403, read("oidc-exchange-invalid.json"))).unwrap_err();
        assert!(matches!(e, ApiError::Status { status: 403, .. }));
        assert!(e.to_string().contains("expired exchange code"), "{e}");
        assert!(login_error_text("callback_denied").contains("Try again"));
    }

    #[test]
    fn reads_the_admin_settings() {
        let fresh = parse_settings(&HttpResponse::new(200, read("oidc-settings.json"))).unwrap();
        assert!(!fresh.enabled && !fresh.client_secret_set);
        assert_eq!(fresh.scopes, "openid profile email");
        assert_eq!(fresh.callback_url, "http://127.0.0.1:8097/api/auth/oidc/callback");
        let saved = parse_settings(&HttpResponse::new(200, read("oidc-settings-saved.json"))).unwrap();
        assert!(saved.enabled && saved.client_secret_set);
        assert_eq!((saved.issuer_url.as_str(), saved.admin_groups.as_str()), ("https://id.example.org", "craftcloud-admins"));
        assert!(!saved.from_env("issuer_url"));
        assert!(!saved.disable_password_login);
    }

    #[test]
    fn every_callback_error_key_reads_as_words() {
        for key in [
            "callback_denied",
            "callback_failed",
            "provider_denied",
            "provider_error",
            "provider_unreachable",
            "email_not_verified_at_idp",
            "email_verification_required",
            "auto_link_disabled",
            "auto_link_email_not_verified",
            "already_linked_elsewhere",
            "email_ambiguous",
        ] {
            assert!(!login_error_text(key).contains(key), "{key} has its own text");
        }
        assert_eq!(login_error_text("brand_new"), "Single sign-on didn't complete (brand_new).");
    }

    #[test]
    fn tests_the_connection_from_the_server() {
        let req = test_connection(&server(), " https://id.example.org ");
        assert_eq!((req.method, req.url.as_str()), (Method::Post, "http://127.0.0.1:8093/api/admin/settings/oidc/test"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&req.body).unwrap(), serde_json::json!({"issuer_url": "https://id.example.org"}));
        let bad = parse_test(&HttpResponse::new(200, read("oidc-test-unreachable.json"))).unwrap();
        assert!(!bad.success && bad.message.contains("Failed to fetch OIDC discovery"));
        let good = parse_test(&HttpResponse::new(200, read("oidc-test-ok.json"))).unwrap();
        assert_eq!((good.success, good.issuer.as_deref()), (true, Some("https://id.example.org/realms/test")));
    }

    #[test]
    fn the_issuer_host_is_its_address_without_the_path() {
        assert_eq!(issuer_host(" https://id.example.org/application/o/craftcloud/ "), "id.example.org");
        assert_eq!(issuer_host("http://user@id.example.org:8443/realms/x?y"), "id.example.org:8443");
        assert_eq!(issuer_host("id.example.org"), "id.example.org");
        assert_eq!(issuer_host("https://"), "the provider");
    }

    #[test]
    fn a_failed_test_reads_as_words_with_the_servers_own_under_them() {
        let bad = parse_test(&HttpResponse::new(200, read("oidc-test-unreachable.json"))).unwrap();
        let (plain, said) = test_failure_text("https://id.example.org/realms/test", &bad.message);
        assert!(plain.starts_with("The server can't reach id.example.org: the name doesn't resolve"), "{plain}");
        assert!(!plain.contains("dns error"));
        assert!(said.starts_with("error sending request -> client error (Connect) -> dns error"), "{said}");
        assert!(!said.contains("Failed to fetch") && !said.contains("Possible causes"), "{said}");
        for (message, words) in [
            (
                "Failed to fetch OIDC discovery: error sending request -> tcp connect error -> Connection refused (os error 111).",
                "connection refused",
            ),
            ("Failed to fetch OIDC discovery: error sending request -> operation timed out.", "didn't answer in time"),
            (
                "Failed to fetch OIDC discovery: error sending request -> invalid peer certificate: UnknownIssuer.",
                "doesn't trust id.example.org's certificate",
            ),
            ("OIDC discovery returned HTTP 404 Not Found — the Issuer URL may be incorrect.", "(HTTP 404)"),
            ("OIDC discovery returned HTTP 504 Gateway Timeout — the Issuer URL may be incorrect.", "answered with an error"),
            ("Invalid discovery document: missing field `issuer`", "not with an OpenID configuration"),
            ("something new", "The server can't use this issuer URL."),
        ] {
            let (plain, said) = test_failure_text("https://id.example.org", message);
            assert!(plain.contains(words), "{message}: {plain}");
            assert!(!said.is_empty());
        }
        assert_eq!(test_failure_text("https://id.example.org", "something new").1, "something new");
    }

    #[test]
    fn saves_without_resending_the_secret() {
        let s = SaveSettings { enabled: true, issuer_url: "https://id.example.org".into(), client_id: "craftcloud".into(), ..Default::default() };
        let body: serde_json::Value = serde_json::from_slice(&save_settings(&server(), &s).body).unwrap();
        assert!(body.get("client_secret").is_none(), "no secret typed: keep the stored one");
        let with = SaveSettings { client_secret: Some("s3cret".into()), ..s.clone() };
        let req = save_settings(&server(), &with);
        assert_eq!((req.method, req.url.as_str()), (Method::Put, "http://127.0.0.1:8093/api/admin/settings/oidc"));
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!((body["client_secret"].as_str(), body["enabled"].as_bool()), (Some("s3cret"), Some(true)));
        let empty = SaveSettings { client_secret: Some(String::new()), ..s };
        let body: serde_json::Value = serde_json::from_slice(&save_settings(&server(), &empty).body).unwrap();
        assert!(body.get("client_secret").is_none());
    }
}
