//! `craftcloud logout` against a stand-in server on a loopback port: the sign-in ends on the
//! server (the logout names it by its refresh token), and when the server can't end it, it is
//! forgotten here all the same and the output says the server may still have it.

use std::net::TcpListener;
use std::process::{Command, Output, Stdio};

use craftcloud_native::fake_server::{FakeServer, Seen};
use serde_json::Value;

/// A server that answers 200 to a request whose body names `r1` (the stored refresh token), else
/// `refused` (crates/native's scripted server, the one its own tests use).
fn server(refused: u16) -> FakeServer {
    FakeServer::start(move |r| (if r.body.contains("\"r1\"") { 200 } else { refused }, "{}".into()))
}

/// The one request `server` got.
fn the_request(server: &FakeServer) -> Seen {
    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    seen.into_iter().next().unwrap()
}

/// `craftcloud logout --json` with a stored sign-in for `url` whose refresh token is
/// `refresh_token` (in a file, not the keychain).
fn logout(url: &str, refresh_token: &str) -> (Output, tempfile::TempDir) {
    logout_logged(url, "a1", refresh_token, None)
}

/// [`logout`] with an access token of choice and `RUST_LOG` set to `rust_log` (unset for `None`).
fn logout_logged(url: &str, access_token: &str, refresh_token: &str, rust_log: Option<&str>) -> (Output, tempfile::TempDir) {
    let config = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("tokens.json"), format!(r#"{{"access_token":"{access_token}","refresh_token":"{refresh_token}"}}"#)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_craftcloud"));
    command.args(["logout", "--json", "--server", url]).env("CRAFTCLOUD_CONFIG_DIR", config.path()).env("CRAFTCLOUD_KEYCHAIN", "off");
    match rust_log {
        Some(filter) => command.env("RUST_LOG", filter),
        None => command.env_remove("RUST_LOG"),
    };
    let out = command.stdin(Stdio::null()).output().unwrap();
    (out, config)
}

fn json(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    serde_json::from_str(text.lines().last().unwrap_or("null")).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

#[test]
fn logout_ends_the_sign_in_on_the_server() {
    // Without the refresh token in the body OxiCloud 0.10.0 answers 401 and the session stays.
    let server = server(401);
    let (out, config) = logout(&server.url, "r1");
    let request = the_request(&server);
    assert!(request.is("POST", "/api/auth/logout"), "{request:?}");
    assert_eq!(serde_json::from_str::<Value>(&request.body).unwrap()["refresh_token"], "r1");
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let result = json(&out.stdout);
    assert_eq!(result["signed_out"], true);
    assert_eq!(result["server_session_ended"], true);
    assert!(!config.path().join("tokens.json").exists());
}

#[test]
fn a_logout_the_server_refuses_is_still_a_sign_out_here() {
    // The server fails (500 for a token other than `r1`): forgotten here, and it says so (exit 1).
    let server = server(500);
    let (out, config) = logout(&server.url, "other");
    the_request(&server);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let result = json(&out.stdout);
    assert_eq!(result["signed_out"], true);
    assert_eq!(result["was_signed_in"], true);
    assert_eq!(result["server_session_ended"], false);
    let error = json(&out.stderr);
    assert!(error["error"]["message"].as_str().unwrap().contains("may still be active"), "{error}");
    assert!(!config.path().join("tokens.json").exists(), "forgotten here all the same");
}

#[test]
fn trace_logging_leaves_out_the_tokens() {
    // The logout request carries the refresh token in its body (and the access token in its
    // header): at trace level the HTTP library would log both. Its debug lines stay.
    for filter in ["trace", "ureq_proto=trace,ureq=trace"] {
        let server = server(200);
        let (out, _config) = logout_logged(&server.url, "FAKE-TRACE-ACCESS", "FAKE-TRACE-REFRESH", Some(filter));
        let body = the_request(&server).body;
        assert!(body.contains("FAKE-TRACE-REFRESH"), "the request did carry the token: {body}");
        assert_eq!(out.status.code(), Some(0), "RUST_LOG={filter}");
        let log = String::from_utf8_lossy(&out.stderr);
        assert!(log.contains("DEBUG ureq"), "RUST_LOG={filter}: the HTTP library's debug lines are kept:\n{log}");
        // The library logs the bytes 16 to a line, so a token can be split: no trace line of it at all.
        assert!(!log.contains("TRACE ureq"), "RUST_LOG={filter}: the HTTP library's trace lines:\n{log}");
        assert!(!log.contains("FAKE-TRACE"), "RUST_LOG={filter}: a token in the log:\n{log}");
    }
}

#[test]
fn a_logout_without_the_server_is_still_a_sign_out_here() {
    // Nothing listens on the port: exit 8 (the server can't be reached), forgotten here too.
    let url = format!("http://{}", TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap());
    let (out, config) = logout(&url, "r1");
    assert_eq!(out.status.code(), Some(8), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(json(&out.stdout)["server_session_ended"], false);
    assert!(!config.path().join("tokens.json").exists());
}
