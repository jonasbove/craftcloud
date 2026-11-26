//! `craftcloud-admin`: the part of CraftCloud that runs next to OxiCloud in the container.
//!
//! Caddy sends it three things (docker/Caddyfile):
//! - `GET /craftcloud/config.json`: what the server says about itself, live.
//! - `GET` / `PUT /craftcloud/api/settings`: the server settings, for owners and admins only
//!   (checked by asking OxiCloud who the caller is; cookie sessions must send `X-CSRF-Token`).
//! - `GET /internal/apps-gate` (Caddy's `forward_auth` before every `/apps/*` file): 200 when that
//!   app may be served, else 404. Settings changes therefore apply on the next request.
//!
//! Settings live in one file on the data volume (`$CRAFTCLOUD_STATE_DIR/settings.env`).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod logic;

use std::path::PathBuf;
use std::sync::Arc;

use tiny_http::{Header, Method, Request, Response, Server};

use crate::logic::{Access, Config, admin_check, gate, public_config};

fn env(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_owned())
}

/// The logger `builder` makes, wrapped so the HTTP library's trace lines never pass, and the
/// most it logs.
fn wire_capped(mut builder: env_logger::Builder) -> (craftcloud_core::logging::WireCapped<env_logger::Logger>, log::LevelFilter) {
    let logger = builder.build();
    let max = logger.filter();
    (craftcloud_core::logging::WireCapped(logger), max)
}

fn main() {
    // `RUST_LOG` as usual, except that the HTTP library's trace lines (the forwarded cookies and
    // `Authorization` headers) never pass: `craftcloud_core::logging`.
    let (logger, max) = wire_capped(env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")));
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max);
    }
    let config = Arc::new(Config {
        oxicloud: env("CRAFTCLOUD_OXICLOUD_URL", "http://127.0.0.1:8087"),
        state_dir: PathBuf::from(env("CRAFTCLOUD_STATE_DIR", "/app/storage/.craftcloud")),
        public_url: env("CRAFTCLOUD_PUBLIC_URL", "http://localhost:8086").trim_end_matches('/').to_owned(),
        version: env("CRAFTCLOUD_VERSION", "dev"),
        bundled: logic::bundled_apps(&PathBuf::from(env("CRAFTCLOUD_APPS_JSON", "/srv/craftcloud/craftcloud/apps.json"))),
    });
    let listen = env("CRAFTCLOUD_ADMIN_LISTEN", "127.0.0.1:8088");
    let server = match Server::http(&listen) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("craftcloud-admin: can't listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    log::info!("listening on {listen}; {} web apps bundled", config.bundled.len());
    for request in server.incoming_requests() {
        let config = Arc::clone(&config);
        // One thread per request: the settings calls wait on OxiCloud, the gate must not wait on them.
        std::thread::spawn(move || handle(&config, request));
    }
}

fn json_response(status: u16, body: &serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = Response::from_data(body.to_string().into_bytes()).with_status_code(status);
    for (k, v) in [("Content-Type", "application/json"), ("Cache-Control", "no-store")] {
        if let Ok(h) = Header::from_bytes(k, v) {
            resp.add_header(h);
        }
    }
    resp
}

fn error(status: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(status, &serde_json::json!({ "message": message }))
}

fn header(request: &Request, name: &str) -> Option<String> {
    request.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_owned())
}

fn handle(config: &Config, mut request: Request) {
    let path = request.url().split('?').next().unwrap_or("").to_owned();
    let response = match (request.method(), path.as_str()) {
        (Method::Get, "/internal/apps-gate") => {
            let uri = header(&request, "X-Forwarded-Uri").unwrap_or_default();
            let status = gate(&uri, &logic::load(&config.state_dir), &config.bundled);
            Response::from_data(Vec::new()).with_status_code(status)
        }
        (Method::Get, "/craftcloud/config.json") => json_response(200, &public_config(config, &logic::load(&config.state_dir))),
        (Method::Get, "/craftcloud/api/settings") => match authorize(config, &request, false) {
            Ok(()) => json_response(200, &logic::settings_view(config, &logic::load(&config.state_dir))),
            Err((status, why)) => error(status, why),
        },
        (Method::Put, "/craftcloud/api/settings") => match authorize(config, &request, true) {
            Ok(()) => {
                let declared = request.body_length();
                match logic::read_settings_body(request.as_reader(), declared) {
                    Ok(settings) => match logic::save(&config.state_dir, &settings) {
                        Ok(()) => {
                            log::info!("settings changed: web apps {}, disabled {:?}", settings.web_apps, settings.disabled_apps);
                            json_response(200, &logic::settings_view(config, &settings))
                        }
                        Err(e) => error(500, &format!("couldn't save the settings: {e}")),
                    },
                    Err((status, why)) => error(status, why),
                }
            }
            Err((status, why)) => error(status, why),
        },
        _ => error(404, "not found"),
    };
    if let Err(e) = request.respond(response) {
        log::debug!("client went away: {e}");
    }
}

/// Owners and admins only; cookie sessions must pass the CSRF double-submit check on writes.
fn authorize(config: &Config, request: &Request, write: bool) -> Result<(), (u16, &'static str)> {
    let cookie = header(request, "Cookie");
    let bearer = header(request, "Authorization");
    let csrf = header(request, "X-CSRF-Token");
    match admin_check(cookie.as_deref(), bearer.as_deref(), csrf.as_deref(), write, |cookie, bearer| ask_oxicloud(config, cookie, bearer)) {
        Access::Allowed => Ok(()),
        Access::SignedOut => Err((401, "Sign in first.")),
        Access::NotAdmin => Err((403, "Only the owner or an admin can change server settings.")),
        Access::BadCsrf => Err((403, "Missing or wrong X-CSRF-Token.")),
        Access::Unavailable => Err((502, "Can't reach OxiCloud to check who you are.")),
    }
}

/// `GET /api/auth/me` on OxiCloud with the caller's credentials; the answer, if any.
fn ask_oxicloud(config: &Config, cookie: Option<&str>, bearer: Option<&str>) -> Option<craftcloud_core::http::HttpResponse> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut req = agent.get(format!("{}/api/auth/me", config.oxicloud));
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    if let Some(b) = bearer {
        req = req.header("Authorization", b);
    }
    let mut resp = req.call().map_err(|e| log::warn!("asking OxiCloud: {e}")).ok()?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().with_config().limit(1024 * 1024).read_to_vec().ok()?;
    Some(craftcloud_core::http::HttpResponse::new(status, body))
}

#[cfg(test)]
mod logging_tests {
    use log::{Level, Log, Metadata};

    /// #77: even with `RUST_LOG=trace` the forwarded cookies and `Authorization` headers (ureq's
    /// trace lines) never reach the log; everything else at trace does.
    #[test]
    fn the_admin_log_never_carries_the_wire_bytes() {
        let mut builder = env_logger::Builder::new();
        builder.parse_filters("trace");
        let (logger, max) = super::wire_capped(builder);
        assert_eq!(max, log::LevelFilter::Trace);
        let at = |target: &str, level| logger.enabled(&Metadata::builder().target(target).level(level).build());
        assert!(!at("ureq", Level::Trace) && !at("ureq_proto::client", Level::Trace), "the wire bytes stop");
        assert!(at("ureq", Level::Debug) && at("craftcloud_admin", Level::Trace), "the rest passes");
    }
}
