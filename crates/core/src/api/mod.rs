//! OxiCloud's API as CraftCloud uses it: request builders and response parsers, no I/O.
//!
//! - [`auth`]: first-run setup, sign-in, token refresh, the device flow (desktop sign-in).
//! - [`dav`]: file bytes over WebDAV (list, download, create, overwrite with an ETag check, copy,
//!   move), by path.
//! - [`folders`], [`files`], [`batch`]: browsing and changing files and folders over REST, by id.
//! - [`trash`], [`favorites`] (and recent), [`search`], [`shares`]: what the web file browser and
//!   the command line show.
//! - [`versions`]: CraftCloud's own file versions in `.craftcloud/versions/`.
//! - [`admin`]: people on the server (owners and admins).
//! - [`resources`]: the file and folder records the REST answers share, and their paths.
//! - [`oidc`]: single sign-on through an OpenID Connect provider, and its admin settings.
//!
//! Written against OxiCloud v0.10.0; the recorded responses in
//! `tests/fixtures/oxicloud-0.10.0/` are what the tests read.

pub mod admin;
pub mod auth;
pub mod batch;
pub mod dav;
pub mod favorites;
pub mod files;
pub mod folders;
pub mod oidc;
pub mod resources;
pub mod search;
pub mod shares;
pub mod trash;
pub mod versions;

use std::fmt;

use crate::http::HttpResponse;

/// What can go wrong reading a response.
#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    /// The server answered with this status where success was expected; `message` is its body
    /// (cut to 300 bytes) or empty.
    Status { status: u16, message: String },
    /// A 2xx answer whose body was not what this endpoint returns.
    Body(String),
}

impl ApiError {
    /// The HTTP status of a refused request.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::Body(_) => None,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status { status, message } if message.is_empty() => write!(f, "the server answered {status}"),
            // OxiCloud's errors are written for people ("Password must be at least 8 characters long").
            Self::Status { message, .. } => f.write_str(message),
            Self::Body(why) => write!(f, "unexpected answer from the server: {why}"),
        }
    }
}

impl std::error::Error for ApiError {}

#[derive(serde::Deserialize)]
struct ErrorBody {
    message: Option<String>,
    error: Option<String>,
}

/// Turns a non-2xx response into [`ApiError::Status`], with OxiCloud's own `message` when the body
/// is its JSON error, else the start of the body.
pub(crate) fn expect_success(resp: &HttpResponse) -> Result<(), ApiError> {
    if resp.is_success() {
        return Ok(());
    }
    let from_json = serde_json::from_slice::<ErrorBody>(&resp.body).ok().and_then(|b| b.message.or(b.error));
    let message = from_json.unwrap_or_else(|| {
        let text = String::from_utf8_lossy(&resp.body);
        text.trim().chars().take(300).collect()
    });
    Err(ApiError::Status { status: resp.status, message })
}

/// For answers with no body worth reading: an error unless 2xx.
pub fn expect_ok(resp: &HttpResponse) -> Result<(), ApiError> {
    expect_success(resp)
}

/// Reads a 2xx JSON body.
pub(crate) fn json<T: serde::de::DeserializeOwned>(resp: &HttpResponse) -> Result<T, ApiError> {
    expect_success(resp)?;
    serde_json::from_slice(&resp.body).map_err(|e| ApiError::Body(e.to_string()))
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::PathBuf;

    pub fn read(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oxicloud-0.10.0").join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
}
