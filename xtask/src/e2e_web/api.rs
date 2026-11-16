//! What the run asks the server itself, to check what the browser did: plain HTTP with `ureq`,
//! written against the server's API and not through CraftCloud's own client code, so a bug there
//! can't hide a failure here. Passwords, tokens and cookie values are never printed.

use std::time::Duration;

use serde_json::{Value, json};

pub struct Api {
    pub server: String,
    agent: ureq::Agent,
}

/// An answer: the status and the body (as JSON when it is JSON).
pub struct Answer {
    pub status: u16,
    pub json: Value,
    pub bytes: Vec<u8>,
}

impl Answer {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub enum Auth<'a> {
    None,
    Bearer(&'a str),
    /// A `Cookie` header (a browser's session, replayed), with an optional `X-CSRF-Token`.
    Cookie(&'a str, Option<&'a str>),
}

impl Api {
    pub fn new(server: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            // 4xx/5xx are answers to read, not transport errors.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("craftcloud-xtask-e2e-web")
            .build()
            .into();
        Self { server: server.trim_end_matches('/').to_owned(), agent }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.server, path.trim_start_matches('/'))
    }

    /// One request. `body` is sent as JSON.
    pub fn call(&self, method: &str, path: &str, auth: Auth<'_>, body: Option<&Value>) -> Result<Answer, String> {
        let url = self.url(path);
        let mut req = ureq::http::Request::builder().method(method).uri(&url);
        match auth {
            Auth::None => {}
            Auth::Bearer(token) => req = req.header("Authorization", format!("Bearer {token}")),
            Auth::Cookie(cookies, csrf) => {
                req = req.header("Cookie", cookies);
                if let Some(token) = csrf {
                    req = req.header("X-CSRF-Token", token);
                }
            }
        }
        let payload = match body {
            Some(v) => {
                req = req.header("Content-Type", "application/json");
                v.to_string().into_bytes()
            }
            None => Vec::new(),
        };
        let req = req.body(payload).map_err(|e| format!("{method} {path}: {e}"))?;
        let mut resp = self.agent.run(req).map_err(|e| format!("{method} {path}: {e}"))?;
        let status = resp.status().as_u16();
        let bytes =
            resp.body_mut().with_config().limit(16 * 1024 * 1024).read_to_vec().map_err(|e| format!("{method} {path}: reading the answer: {e}"))?;
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Ok(Answer { status, json, bytes })
    }

    /// `GET /api/auth/status`: `initialized`.
    pub fn initialized(&self) -> Result<bool, String> {
        let a = self.call("GET", "api/auth/status", Auth::None, None)?;
        if a.status != 200 {
            return Err(format!("{}/api/auth/status answered {}: is this a CraftCloud server?", self.server, a.status));
        }
        a.json["initialized"].as_bool().ok_or_else(|| "/api/auth/status didn't say whether the server is set up".into())
    }

    /// A password sign-in: the access and refresh tokens.
    pub fn sign_in(&self, user: &str, password: &str) -> Result<(String, String), String> {
        let a = self.call("POST", "api/auth/login", Auth::None, Some(&json!({"username": user, "password": password})))?;
        tokens(&a.json).filter(|_| a.status == 200).ok_or_else(|| format!("signing in as {user} answered {}", a.status))
    }

    /// Ends a sign-in on the server: without a cookie, OxiCloud ends the session whose refresh
    /// token comes in the body (and answers 401, ending nothing, without it).
    pub fn sign_out(&self, access: &str, refresh: &str) {
        let _ = self.call("POST", "api/auth/logout", Auth::Bearer(access), Some(&json!({"refresh_token": refresh})));
    }

    /// A file's bytes over WebDAV, `None` when it isn't there.
    pub fn dav_get(&self, token: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        let a = self.call("GET", &format!("webdav/{}", encode(name)), Auth::Bearer(token), None)?;
        match a.status {
            200 => Ok(Some(a.bytes)),
            404 => Ok(None),
            s => Err(format!("GET /webdav/{name} answered {s}")),
        }
    }

    /// The account's share links (`GET /api/shares`), as `(token, item name)`.
    pub fn shares(&self, token: &str) -> Result<Vec<(String, String)>, String> {
        let a = self.call("GET", "api/shares?page=1&per_page=100", Auth::Bearer(token), None)?;
        if a.status != 200 {
            return Err(format!("GET /api/shares answered {}", a.status));
        }
        let list = a.json["items"].as_array().or_else(|| a.json.as_array()).cloned().unwrap_or_default();
        Ok(list.iter().filter_map(|s| Some((s["token"].as_str()?.to_owned(), s["item_name"].as_str().unwrap_or_default().to_owned()))).collect())
    }

    /// The usernames of the server's accounts (admin only).
    pub fn usernames(&self, token: &str) -> Result<Vec<String>, String> {
        let a = self.call("GET", "api/admin/users?limit=200&offset=0", Auth::Bearer(token), None)?;
        if a.status != 200 {
            return Err(format!("GET /api/admin/users answered {}", a.status));
        }
        let users = a.json["users"].as_array().cloned().unwrap_or_default();
        Ok(users.iter().filter_map(|u| u["user"]["username"].as_str().map(str::to_owned)).collect())
    }
}

/// `(access_token, refresh_token)` of a sign-in's answer (password or device flow).
pub fn tokens(answer: &Value) -> Option<(String, String)> {
    Some((answer["access_token"].as_str()?.to_owned(), answer["refresh_token"].as_str()?.to_owned()))
}

/// Percent-encodes one path segment (names here are plain ASCII, but spaces happen).
fn encode(name: &str) -> String {
    name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    use super::tokens;

    #[test]
    fn both_tokens_are_read_from_a_sign_in() {
        for fixture in [
            include_str!("../../../crates/core/tests/fixtures/oxicloud-0.10.0/login.json"),
            include_str!("../../../crates/core/tests/fixtures/oxicloud-0.10.0/device-token-ok.json"),
        ] {
            let answer: serde_json::Value = serde_json::from_str(fixture).unwrap();
            let (access, refresh) = tokens(&answer).expect("both tokens");
            assert!(!access.is_empty() && !refresh.is_empty());
        }
        assert_eq!(tokens(&serde_json::json!({"access_token": "a"})), None, "sign-out needs the refresh token");
    }
}
