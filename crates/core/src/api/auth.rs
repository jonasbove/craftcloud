//! Setup, sign-in, tokens and the device flow (RFC 8628) that the desktop client signs in with.

use serde::Deserialize;
use serde_json::json;

use super::{ApiError, json};
use crate::ServerUrl;
use crate::http::{HttpRequest, HttpResponse, Method};

/// `GET /api/auth/status`: whether the server has its first (owner) account yet.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct InitStatus {
    pub initialized: bool,
    pub admin_count: u32,
    pub registration_allowed: bool,
}

pub fn status(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join("api/auth/status"))
}

pub fn parse_status(resp: &HttpResponse) -> Result<InitStatus, ApiError> {
    json(resp)
}

/// A user as the setup and sign-in answers describe them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: String,
}

impl User {
    /// Owners and admins see the ADMIN section.
    pub fn is_admin(&self) -> bool {
        matches!(self.role.as_str(), "owner" | "admin")
    }
}

/// `GET /api/auth/me`: who is signed in (401 when nobody is).
pub fn me(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join("api/auth/me"))
}

/// The signed-in account, with its storage use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Me {
    pub user: User,
    pub storage_used_bytes: u64,
    pub storage_quota_bytes: u64,
}

#[derive(Deserialize)]
struct MeBody {
    full: MeFull,
}

#[derive(Deserialize)]
struct MeFull {
    user: User,
    #[serde(default)]
    storage_used_bytes: u64,
    #[serde(default)]
    storage_quota_bytes: u64,
}

/// `Ok(None)` when nobody is signed in.
pub fn parse_me(resp: &HttpResponse) -> Result<Option<Me>, ApiError> {
    if resp.status == 401 {
        return Ok(None);
    }
    let body: MeBody = json(resp)?;
    Ok(Some(Me { user: body.full.user, storage_used_bytes: body.full.storage_used_bytes, storage_quota_bytes: body.full.storage_quota_bytes }))
}

/// `POST /api/setup`: creates the owner account on a server that has none (no auth needed).
pub fn setup(server: &ServerUrl, username: &str, email: &str, password: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/setup")).json(&json!({"username": username, "email": email, "password": password}))
}

/// Setup does not sign the owner in; the caller logs in right after.
pub fn parse_setup(resp: &HttpResponse) -> Result<User, ApiError> {
    json(resp)
}

/// `POST /api/auth/logout`.
pub fn logout(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/auth/logout"))
}

/// The tokens a sign-in (password, refresh or device flow) gives.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
}

impl std::fmt::Debug for Tokens {
    // Never print the tokens themselves.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens { .. }")
    }
}

impl Tokens {
    /// The `Authorization` header value for a request made with these tokens.
    pub fn bearer(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    /// Adds the access token to a request.
    pub fn authorize(&self, req: HttpRequest) -> HttpRequest {
        req.header("Authorization", self.bearer())
    }
}

/// `POST /api/auth/login` (password sign-in, used by the web frontend).
pub fn login(server: &ServerUrl, username: &str, password: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/auth/login")).json(&json!({"username": username, "password": password}))
}

pub fn parse_login(resp: &HttpResponse) -> Result<Tokens, ApiError> {
    json(resp)
}

/// `POST /api/auth/refresh`: new tokens for a refresh token.
pub fn refresh(server: &ServerUrl, refresh_token: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/auth/refresh")).json(&json!({"refresh_token": refresh_token}))
}

pub fn parse_refresh(resp: &HttpResponse) -> Result<Tokens, ApiError> {
    json(resp)
}

/// Whether a response says the access token is no longer accepted (refresh, then retry once).
pub fn is_expired(resp: &HttpResponse) -> bool {
    resp.status == 401
}

/// The start of a device sign-in: show `user_code`, open `verification_uri_complete`, and poll
/// every `interval` seconds until `expires_in` runs out.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

impl std::fmt::Debug for DeviceStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceStart").field("user_code", &self.user_code).field("verification_uri", &self.verification_uri).finish_non_exhaustive()
    }
}

impl DeviceStart {
    /// The page to open in the browser: with the code filled in when the server offers that.
    pub fn browser_url(&self) -> &str {
        self.verification_uri_complete.as_deref().unwrap_or(&self.verification_uri)
    }
}

/// `POST /api/auth/device/authorize`. `client_name` is what the approval page and the device
/// list show, e.g. "CraftCloud on Studio".
pub fn device_authorize(server: &ServerUrl, client_name: &str) -> HttpRequest {
    // OxiCloud v0.10.0 stores but does not enforce the scope (docs/UPSTREAM.md C7); ask for the
    // narrowest one anyway so the device list says what the client is for.
    HttpRequest::new(Method::Post, server.join("api/auth/device/authorize")).json(&json!({"client_name": client_name, "scope": "webdav"}))
}

pub fn parse_device_authorize(resp: &HttpResponse) -> Result<DeviceStart, ApiError> {
    json(resp)
}

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// `POST /api/auth/device/token`: one poll.
pub fn device_token(server: &ServerUrl, device_code: &str) -> HttpRequest {
    HttpRequest::new(Method::Post, server.join("api/auth/device/token")).json(&json!({"device_code": device_code, "grant_type": DEVICE_GRANT}))
}

/// What one poll found.
#[derive(Debug, PartialEq, Eq)]
pub enum DevicePoll {
    /// Not approved yet: poll again after `interval` seconds.
    Pending,
    /// Polling too often: add 5 seconds to the interval (RFC 8628 §3.5).
    SlowDown,
    /// The person approved: signed in.
    Approved(Tokens),
    /// The person said no.
    Denied,
    /// The code ran out; start again.
    Expired,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
}

pub fn parse_device_token(resp: &HttpResponse) -> Result<DevicePoll, ApiError> {
    if resp.is_success() {
        return json(resp).map(DevicePoll::Approved);
    }
    if resp.status == 400
        && let Ok(e) = serde_json::from_slice::<OAuthError>(&resp.body)
    {
        match e.error.as_str() {
            "authorization_pending" => return Ok(DevicePoll::Pending),
            "slow_down" => return Ok(DevicePoll::SlowDown),
            "access_denied" => return Ok(DevicePoll::Denied),
            "expired_token" => return Ok(DevicePoll::Expired),
            _ => {}
        }
    }
    super::expect_success(resp).map(|()| DevicePoll::Pending)
}

/// `GET /api/auth/device/verify?code=…`, for the web frontend's `/device` approval page.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DeviceRequest {
    pub client_name: String,
    pub scopes: String,
    pub valid: bool,
}

pub fn device_verify_info(server: &ServerUrl, user_code: &str) -> HttpRequest {
    let code: String = percent_encoding::utf8_percent_encode(user_code, percent_encoding::NON_ALPHANUMERIC).collect();
    HttpRequest::new(Method::Get, format!("{}?code={code}", server.join("api/auth/device/verify")))
}

pub fn parse_device_verify_info(resp: &HttpResponse) -> Result<DeviceRequest, ApiError> {
    json(resp)
}

/// `POST /api/auth/device/verify`: the signed-in person approves or denies a device.
pub fn device_verify(server: &ServerUrl, user_code: &str, approve: bool) -> HttpRequest {
    let action = if approve { "approve" } else { "deny" };
    HttpRequest::new(Method::Post, server.join("api/auth/device/verify")).json(&json!({"user_code": user_code, "action": action}))
}

/// A device (desktop client) the signed-in person connected with the device flow.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Device {
    pub id: String,
    pub client_name: String,
    /// `authorized` once approved.
    #[serde(default)]
    pub status: String,
    /// RFC 3339.
    #[serde(default)]
    pub authorized_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// `GET /api/auth/device/devices`: the signed-in person's own devices.
pub fn devices(server: &ServerUrl) -> HttpRequest {
    HttpRequest::new(Method::Get, server.join("api/auth/device/devices"))
}

/// The approved devices.
pub fn parse_devices(resp: &HttpResponse) -> Result<Vec<Device>, ApiError> {
    let all: Vec<Device> = json(resp)?;
    Ok(all.into_iter().filter(|d| d.status == "authorized").collect())
}

#[cfg(test)]
mod tests {
    use super::{
        DevicePoll, ServerUrl, device_authorize, device_token, device_verify_info, parse_device_authorize, parse_device_token, parse_setup,
        parse_status,
    };
    use crate::api::fixtures::read;
    use crate::http::{HttpResponse, Method};

    fn server() -> ServerUrl {
        ServerUrl::parse("http://127.0.0.1:8093").unwrap()
    }

    #[test]
    fn a_new_server_needs_setup() {
        let s = parse_status(&HttpResponse::new(200, read("auth-status-uninit.json"))).unwrap();
        assert!(!s.initialized);
        assert_eq!(s.admin_count, 0);
    }

    #[test]
    fn setup_creates_the_owner() {
        let user = parse_setup(&HttpResponse::new(201, read("setup.json"))).unwrap();
        assert_eq!(user.username, "john");
        assert!(user.is_admin(), "the first account is the owner");
    }

    #[test]
    fn me_says_who_is_signed_in() {
        let me = super::parse_me(&HttpResponse::new(200, read("me.json"))).unwrap().unwrap();
        assert_eq!(me.user.username, "john");
        assert!(me.user.is_admin());
        assert_eq!(me.storage_quota_bytes, 107_374_182_400);
        assert_eq!(super::parse_me(&HttpResponse::new(401, Vec::new())), Ok(None));
    }

    #[test]
    fn a_rejected_setup_says_why() {
        let err = parse_setup(&HttpResponse::new(400, read("setup-rejected.json"))).unwrap_err();
        assert_eq!(err.to_string(), "Password must be at least 8 characters long");
    }

    #[test]
    fn device_flow_requests() {
        let req = device_authorize(&server(), "CraftCloud on Studio");
        assert_eq!((req.method, req.url.as_str()), (Method::Post, "http://127.0.0.1:8093/api/auth/device/authorize"));
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["client_name"], "CraftCloud on Studio");

        let poll = device_token(&server(), "abc");
        let body: serde_json::Value = serde_json::from_slice(&poll.body).unwrap();
        assert_eq!(body["grant_type"], "urn:ietf:params:oauth:grant-type:device_code");

        assert_eq!(device_verify_info(&server(), "KOQY-2239").url, "http://127.0.0.1:8093/api/auth/device/verify?code=KOQY%2D2239");
    }

    #[test]
    fn device_flow_answers() {
        let start = parse_device_authorize(&HttpResponse::new(200, read("device-authorize.json"))).unwrap();
        assert!(start.browser_url().contains("/device?code="), "{}", start.browser_url());
        assert_eq!(start.interval, 5);
        assert!(!format!("{start:?}").contains("redacted"), "Debug never prints the device code");

        assert_eq!(parse_device_token(&HttpResponse::new(400, read("device-token-pending.json"))), Ok(DevicePoll::Pending));
        let Ok(DevicePoll::Approved(tokens)) = parse_device_token(&HttpResponse::new(200, read("device-token-ok.json"))) else {
            panic!("approved poll not read")
        };
        assert_eq!(tokens.access_token, "<redacted access_token>");
        assert_eq!(format!("{tokens:?}"), "Tokens { .. }");
        assert_eq!(parse_device_token(&HttpResponse::new(400, br#"{"error":"access_denied"}"#.to_vec())), Ok(DevicePoll::Denied));
        assert!(parse_device_token(&HttpResponse::new(500, b"boom".to_vec())).is_err());
    }

    #[test]
    fn lists_the_connected_devices() {
        let r = super::devices(&server());
        assert_eq!((r.method, r.url.as_str()), (Method::Get, "http://127.0.0.1:8093/api/auth/device/devices"));
        let list = super::parse_devices(&HttpResponse::new(200, read("device-devices.json"))).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].client_name, "CraftCloud on Laptop");
        assert!(list[0].authorized_at.is_some());
    }
}
