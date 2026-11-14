//! User admin (owners and admins): list, create, reset a password, (de)activate, change the role
//! or the storage quota, and the sign-in sessions of one person.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{ApiError, json as read_json};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// One person on the server, as the admin list shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub active: bool,
    pub storage_used_bytes: u64,
    pub storage_quota_bytes: u64,
    /// RFC 3339, `None` if they never signed in.
    pub last_login_at: Option<String>,
    /// RFC 3339.
    pub created_at: Option<String>,
}

impl Person {
    pub fn is_owner(&self) -> bool {
        self.role == "owner"
    }

    pub fn is_admin(&self) -> bool {
        matches!(self.role.as_str(), "owner" | "admin")
    }
}

#[derive(Deserialize)]
struct Page {
    users: Vec<FullUser>,
    total: u64,
}

#[derive(Deserialize)]
struct FullUser {
    user: super::auth::User,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    storage_used_bytes: u64,
    #[serde(default)]
    storage_quota_bytes: u64,
    last_login_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

impl From<FullUser> for Person {
    fn from(f: FullUser) -> Self {
        Self {
            id: f.user.id,
            username: f.user.username,
            email: f.user.email,
            role: f.user.role,
            active: f.active,
            storage_used_bytes: f.storage_used_bytes,
            storage_quota_bytes: f.storage_quota_bytes,
            last_login_at: f.last_login_at,
            created_at: f.created_at,
        }
    }
}

pub fn list(server: &ServerUrl, limit: u32, offset: u32) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}?limit={limit}&offset={offset}", server.join("api/admin/users")))
}

/// The people on this page and how many there are in total.
pub fn parse_list(resp: &HttpResponse) -> Result<(Vec<Person>, u64), ApiError> {
    let page: Page = read_json(resp)?;
    Ok((page.users.into_iter().map(Person::from).collect(), page.total))
}

/// The roles an admin can give. Ownership moves only with OxiCloud's transfer-ownership call (the
/// role endpoint answers 400 for `owner`), which CraftCloud doesn't offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
        }
    }
}

/// A new account. `quota_bytes: None` keeps the server's default (OxiCloud: 1 GB).
pub fn create(server: &ServerUrl, username: &str, email: &str, password: &str, quota_bytes: Option<u64>, role: Role) -> HttpRequest {
    let mut body = json!({"username": username, "email": email, "password": password, "role": role.as_str()});
    if let Some(q) = quota_bytes {
        body["quota_bytes"] = Value::from(q);
    }
    HttpRequest::new(Method::Post, server.join("api/admin/users")).json(&body)
}

pub fn parse_create(resp: &HttpResponse) -> Result<Person, ApiError> {
    read_json::<FullUser>(resp).map(Person::from)
}

pub fn reset_password(server: &ServerUrl, id: &str, password: &str) -> HttpRequest {
    HttpRequest::new(Method::Put, server.join(&format!("api/admin/users/{id}/password"))).json(&json!({"new_password": password}))
}

pub fn set_active(server: &ServerUrl, id: &str, active: bool) -> HttpRequest {
    HttpRequest::new(Method::Put, server.join(&format!("api/admin/users/{id}/active"))).json(&json!({"active": active}))
}

/// `PUT /api/admin/users/{id}/role`. The server refuses your own role and the owner's.
pub fn set_role(server: &ServerUrl, id: &str, role: Role) -> HttpRequest {
    HttpRequest::new(Method::Put, server.join(&format!("api/admin/users/{id}/role"))).json(&json!({"role": role.as_str()}))
}

/// `PUT /api/admin/users/{id}/quota`. `0` is unlimited.
/// The background job that counts each person's storage use again. OxiCloud runs it every 10
/// minutes (`OXICLOUD_STORAGE_USAGE_RECONCILE_SECS`), and emptying the trash doesn't update the
/// use by itself (docs/UPSTREAM.md C11), so uploads are refused for the quota meanwhile.
pub const USAGE_RECONCILE: &str = "usage_reconcile";

/// `POST /api/admin/jobs/{name}/trigger`: runs a background job now (admins only; 200 when done).
pub fn trigger_job(server: &ServerUrl, name: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join(&format!("api/admin/jobs/{name}/trigger")))
}

pub fn set_quota(server: &ServerUrl, id: &str, quota_bytes: u64) -> HttpRequest {
    HttpRequest::new(Method::Put, server.join(&format!("api/admin/users/{id}/quota"))).json(&json!({"quota_bytes": quota_bytes}))
}

/// `GET /api/admin/users/{id}`.
pub fn get(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join(&format!("api/admin/users/{id}")))
}

pub fn parse_get(resp: &HttpResponse) -> Result<Person, ApiError> {
    read_json::<FullUser>(resp).map(Person::from)
}

/// One signed-in session: a browser, or a desktop client signed in with the device flow.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Session {
    pub id: String,
    /// `password`, `oidc`, `device`, `magic_link`, `opaque` or `unknown`.
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub ip_address: Option<String>,
    /// RFC 3339.
    #[serde(default)]
    pub created_at: Option<String>,
    /// RFC 3339.
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub is_online: bool,
}

impl Session {
    /// A desktop client (or another device-flow client) rather than a browser.
    pub fn is_device(&self) -> bool {
        self.origin == "device"
    }
}

#[derive(Deserialize)]
struct Sessions {
    sessions: Vec<Session>,
}

/// `GET /api/admin/sessions?user_id=…`: one person's active sessions.
pub fn sessions(server: &ServerUrl, user_id: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, format!("{}?user_id={user_id}", server.join("api/admin/sessions")))
}

pub fn parse_sessions(resp: &HttpResponse) -> Result<Vec<Session>, ApiError> {
    read_json::<Sessions>(resp).map(|s| s.sessions)
}

/// `DELETE /api/admin/sessions/{id}`: signs that browser or device out.
pub fn revoke_session(server: &ServerUrl, id: &str) -> HttpRequest {
    HttpRequest::new(Method::Delete, server.join(&format!("api/admin/sessions/{id}")))
}

#[cfg(test)]
mod tests {
    use super::{Role, create, parse_create, parse_get, parse_list, parse_sessions, reset_password, revoke_session, set_active, set_quota, set_role};
    use crate::ServerUrl;
    use crate::api::expect_ok;
    use crate::api::fixtures::read;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8093").unwrap()
    }

    #[test]
    fn reads_the_people_list() {
        let (people, total) = parse_list(&HttpResponse::new(200, read("admin-users.json"))).unwrap();
        assert_eq!(total, 2);
        let jane = people.iter().find(|p| p.username == "jane").unwrap();
        assert!(jane.active);
        assert_eq!(jane.role, "user");
        assert_eq!(jane.storage_quota_bytes, 1_073_741_824, "OxiCloud's default for new users");
        assert_eq!(jane.last_login_at, None);
        let john = people.iter().find(|p| p.username == "john").unwrap();
        assert_eq!(john.role, "owner");
    }

    #[test]
    fn creates_users_and_admins() {
        let req = create(&server(), "jane", "jane@example.org", "secret-123", Some(10_000_000_000), Role::User);
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!((body["role"].as_str(), body["quota_bytes"].as_u64()), (Some("user"), Some(10_000_000_000)));
        let body: serde_json::Value = serde_json::from_slice(&create(&server(), "a", "b", "c", None, Role::Admin).body).unwrap();
        assert!(body.get("quota_bytes").is_none(), "no quota: the server's default");
        assert_eq!(body["role"], "admin");
        assert_eq!(parse_create(&HttpResponse::new(201, read("admin-user-created.json"))).unwrap().username, "jane");
    }

    #[test]
    fn password_and_active_requests() {
        let r = reset_password(&server(), "id1", "new-password");
        assert_eq!((r.method, r.url.as_str()), (Method::Put, "http://127.0.0.1:8093/api/admin/users/id1/password"));
        let a = set_active(&server(), "id1", false);
        let body: serde_json::Value = serde_json::from_slice(&a.body).unwrap();
        assert_eq!(body["active"], false);
        assert!(expect_ok(&HttpResponse::new(200, read("admin-user-deactivated.json"))).is_ok());
    }

    #[test]
    fn role_and_quota_requests() {
        let r = set_role(&server(), "id1", Role::Admin);
        assert_eq!((r.method, r.url.as_str()), (Method::Put, "http://127.0.0.1:8093/api/admin/users/id1/role"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&r.body).unwrap(), serde_json::json!({"role": "admin"}));
        assert!(expect_ok(&HttpResponse::new(200, read("admin-user-role.json"))).is_ok());
        let refused = expect_ok(&HttpResponse::new(400, read("admin-user-role-owner.json"))).unwrap_err();
        assert!(refused.to_string().contains("transfer-ownership"), "{refused}");

        let q = set_quota(&server(), "id1", 5_000_000_000);
        assert_eq!((q.method, q.url.as_str()), (Method::Put, "http://127.0.0.1:8093/api/admin/users/id1/quota"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&q.body).unwrap(), serde_json::json!({"quota_bytes": 5_000_000_000_u64}));
        assert!(expect_ok(&HttpResponse::new(200, read("admin-user-quota.json"))).is_ok());
    }

    #[test]
    fn reads_one_person_and_a_list_of_three() {
        let jane = parse_get(&HttpResponse::new(200, read("admin-user.json"))).unwrap();
        assert_eq!((jane.username.as_str(), jane.role.as_str()), ("jane", "admin"));
        assert_eq!(jane.storage_quota_bytes, 5_000_000_000);
        assert!(jane.created_at.is_some());
        let (people, total) = parse_list(&HttpResponse::new(200, read("admin-users-three.json"))).unwrap();
        assert_eq!(total, 3);
        assert!(!people.iter().find(|p| p.username == "test").unwrap().active, "deactivated in the recording");
        assert!(people.iter().any(|p| p.is_owner()));
    }

    #[test]
    fn reads_sessions_and_revokes_one() {
        let sessions = parse_sessions(&HttpResponse::new(200, read("admin-sessions.json"))).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].origin, "password");
        assert!(!sessions[0].is_device());
        assert_eq!(sessions[0].user_agent.as_deref(), Some("curl/8.22.0"));
        let r = revoke_session(&server(), "s1");
        assert_eq!((r.method, r.url.as_str()), (Method::Delete, "http://127.0.0.1:8093/api/admin/sessions/s1"));
    }
}
