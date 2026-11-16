//! The few REST calls the smoke test makes itself, with the test account's password sign-in: the
//! owner on a fresh server, approving the clients' device codes, the device list and the trash. The
//! clients under test are never asked about these, so a bug in them can't hide a failure here.

use std::time::Duration;

use serde_json::{Value, json};

/// A password sign-in to the test server. The password and the tokens are never printed.
pub struct Api {
    server: String,
    agent: ureq::Agent,
    access: String,
    /// Needed to end the sign-in ([`Api::sign_out`]).
    refresh: String,
}

/// A signed-in session on the account (a browser, or a client signed in with the device flow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    /// `device` for a desktop client.
    pub origin: String,
    pub user_agent: String,
    pub created_at: String,
    pub last_seen_at: String,
}

/// One trashed file or folder: `path` is where it was, `Personal/<folder>/<name>`.
#[derive(Clone, Debug)]
pub struct Trashed {
    pub id: String,
    pub path: String,
    pub folder: bool,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // 4xx/5xx are answers to read, not transport errors.
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent("craftcloud-xtask-e2e-sync")
        .build()
        .into()
}

fn answer(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>, what: &str) -> Result<(u16, Value), String> {
    let mut resp = resp.map_err(|e| format!("{what}: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn post(agent: &ureq::Agent, url: &str, bearer: Option<&str>, body: &Value, what: &str) -> Result<(u16, Value), String> {
    let mut req = agent.post(url).content_type("application/json");
    if let Some(token) = bearer {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    answer(req.send(body.to_string()), what)
}

impl Api {
    /// Signs in with a password, making the owner first when the server is fresh (as the live
    /// tests do).
    pub fn sign_in(server: &str, user: &str, password: &str) -> Result<Self, String> {
        let server = server.trim_end_matches('/').to_owned();
        let agent = agent();
        let (status, body) = answer(agent.get(format!("{server}/api/auth/status")).call(), "asking the server's status")?;
        if status != 200 {
            return Err(format!("{server}/api/auth/status answered {status}: is this a CraftCloud server?"));
        }
        if body["initialized"] == false {
            let owner = json!({"username": user, "email": format!("{user}@example.org"), "password": password});
            let (status, _) = post(&agent, &format!("{server}/api/setup"), None, &owner, "making the owner")?;
            if !(200..300).contains(&status) {
                return Err(format!("making the owner on a fresh server answered {status}"));
            }
        }
        let creds = json!({"username": user, "password": password});
        let (status, body) = post(&agent, &format!("{server}/api/auth/login"), None, &creds, "signing in")?;
        let access = body["access_token"].as_str().filter(|_| status == 200).map(str::to_owned);
        let access = access.ok_or_else(|| format!("signing in as CRAFTCLOUD_TEST_USER answered {status} (check the user and password)"))?;
        let refresh = body["refresh_token"].as_str().unwrap_or_default().to_owned();
        Ok(Self { server, agent, access, refresh })
    }

    fn get(&self, path: &str) -> Result<(u16, Value), String> {
        let req = self.agent.get(format!("{}{path}", self.server)).header("Authorization", format!("Bearer {}", self.access));
        answer(req.call(), path)
    }

    /// Approves a device code, as the person does on the `/device` page.
    pub fn approve(&self, user_code: &str) -> Result<(), String> {
        let body = json!({"user_code": user_code, "action": "approve"});
        let (status, _) = post(&self.agent, &format!("{}/api/auth/device/verify", self.server), Some(&self.access), &body, "approving")?;
        if (200..300).contains(&status) { Ok(()) } else { Err(format!("approving the device code answered {status}")) }
    }

    /// The account's active sessions, or `None` when the account can't list them. OxiCloud lists
    /// sessions only to admins (`/api/admin/sessions`); its device list (`/api/auth/device/devices`)
    /// only holds codes not yet collected, so a signed-in client is not in it.
    pub fn sessions(&self) -> Result<Option<Vec<Session>>, String> {
        let (status, me) = self.get("/api/auth/me")?;
        let user = &me["full"]["user"];
        if status != 200 || !matches!(user["role"].as_str(), Some("owner" | "admin")) {
            return Ok(None);
        }
        let id = user["id"].as_str().unwrap_or_default();
        let (status, body) = self.get(&format!("/api/admin/sessions?user_id={}&limit=1000", encode(id)))?;
        if status != 200 {
            return Ok(None);
        }
        let text = |s: &Value, key: &str| s[key].as_str().unwrap_or_default().to_owned();
        Ok(Some(
            body["sessions"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter(|s| s["is_active"] != false && s["is_revoked"] != true)
                .map(|s| Session {
                    id: text(s, "id"),
                    origin: text(s, "origin"),
                    user_agent: text(s, "user_agent"),
                    created_at: text(s, "created_at"),
                    last_seen_at: text(s, "last_seen_at"),
                })
                .collect(),
        ))
    }

    /// Everything in the trash (CraftCloud's own items too), page by page.
    pub fn trash(&self) -> Result<Vec<Trashed>, String> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        // Bounded: a server that keeps handing out cursors doesn't keep the test going forever.
        for _ in 0..100 {
            let query = cursor.as_deref().map_or(String::new(), |c| format!("&cursor={}", encode(c)));
            let (status, body) = self.get(&format!("/api/trash/resources?limit=200{query}"))?;
            if status != 200 {
                return Err(format!("listing the trash answered {status}"));
            }
            for item in body["items"].as_array().map(Vec::as_slice).unwrap_or_default() {
                out.push(Trashed {
                    id: item["resource"]["id"].as_str().unwrap_or_default().to_owned(),
                    path: item["resource"]["path"].as_str().unwrap_or_default().to_owned(),
                    folder: item["resource_type"] == "folder",
                });
            }
            cursor = body["next_cursor"].as_str().filter(|c| !c.is_empty()).map(str::to_owned);
            if cursor.is_none() {
                return Ok(out);
            }
        }
        Err("the trash listing didn't end after 100 pages".to_owned())
    }

    /// Deletes a trashed item for good (best effort: it may be gone with its folder already).
    pub fn purge(&self, id: &str) {
        let url = format!("{}/api/trash/{id}", self.server);
        let _ = self.agent.delete(url).header("Authorization", format!("Bearer {}", self.access)).call();
    }

    /// Ends a session of the account on the server (admins only), as Admin › Users does.
    pub fn revoke(&self, id: &str) -> bool {
        let url = format!("{}/api/admin/sessions/{}", self.server, encode(id));
        let resp = self.agent.delete(url).header("Authorization", format!("Bearer {}", self.access)).call();
        answer(resp, "ending a session").is_ok_and(|(status, _)| (200..300).contains(&status))
    }

    /// Ends the password sign-in. OxiCloud finds the session to end by its refresh token: a logout
    /// without it answers 401 and leaves the session signed in.
    pub fn sign_out(&self) {
        let body = json!({"refresh_token": self.refresh});
        let _ = post(&self.agent, &format!("{}/api/auth/logout", self.server), Some(&self.access), &body, "signing out");
    }
}

/// Percent-encodes a query value.
fn encode(value: &str) -> String {
    value.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
