//! `craftcloud logout` against a stand-in server on a loopback port: the sign-in ends on the
//! server (the logout names it by its refresh token), and when the server can't end it, it is
//! forgotten here all the same and the output says the server may still have it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;

use serde_json::Value;

/// Answers one request with 200 when its body names `r1` (the stored refresh token), else with
/// `refused`; hands back the request line and body.
fn server(refused: u16) -> (String, JoinHandle<(String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::thread::spawn(move || {
        let (conn, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(conn);
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let body = String::from_utf8(body).unwrap();
        let status = if body.contains("\"r1\"") { 200 } else { refused };
        let mut conn = reader.into_inner();
        write!(conn, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        (first.trim().to_owned(), body)
    });
    (url, seen)
}

/// `craftcloud logout --json` with a stored sign-in for `url` (in a file, not the keychain).
fn logout(url: &str) -> (Output, tempfile::TempDir) {
    let config = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("tokens.json"), r#"{"access_token":"a1","refresh_token":"r1"}"#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_craftcloud"))
        .args(["logout", "--json", "--server", url])
        .env("CRAFTCLOUD_CONFIG_DIR", config.path())
        .env("CRAFTCLOUD_KEYCHAIN", "off")
        .env("CRAFTCLOUD_HTTP_IDLE_SECS", "5")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out, config)
}

fn json(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    serde_json::from_str(text.lines().last().unwrap_or("null")).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

#[test]
fn logout_ends_the_sign_in_on_the_server() {
    // Without the refresh token in the body OxiCloud 0.10.0 answers 401 and the session stays.
    let (url, seen) = server(401);
    let (out, config) = logout(&url);
    let (line, body) = seen.join().unwrap();
    assert!(line.starts_with("POST /api/auth/logout"), "{line}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["refresh_token"], "r1");
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let result = json(&out.stdout);
    assert_eq!(result["signed_out"], true);
    assert_eq!(result["server_session_ended"], true);
    assert!(!config.path().join("tokens.json").exists());
}

#[test]
fn a_logout_the_server_refuses_is_still_a_sign_out_here() {
    // The server can't end it (it fails): forgotten here, and the output says so (exit 1).
    let listener_url = {
        let (url, seen) = server(500);
        // The stand-in answers 200 to `r1`; send another token so it refuses.
        let config = tempfile::tempdir().unwrap();
        std::fs::write(config.path().join("tokens.json"), r#"{"access_token":"a1","refresh_token":"other"}"#).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_craftcloud"))
            .args(["logout", "--json", "--server", &url])
            .env("CRAFTCLOUD_CONFIG_DIR", config.path())
            .env("CRAFTCLOUD_KEYCHAIN", "off")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        seen.join().unwrap();
        assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
        let result = json(&out.stdout);
        assert_eq!(result["signed_out"], true);
        assert_eq!(result["was_signed_in"], true);
        assert_eq!(result["server_session_ended"], false);
        let error = json(&out.stderr);
        assert!(error["error"]["message"].as_str().unwrap().contains("may still be active"), "{error}");
        assert!(!config.path().join("tokens.json").exists(), "forgotten here all the same");
        url
    };

    // Nothing answers any more: exit 8 (the server can't be reached), forgotten here too.
    let (out, config) = logout(&listener_url);
    assert_eq!(out.status.code(), Some(8), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(json(&out.stdout)["server_session_ended"], false);
    assert!(!config.path().join("tokens.json").exists());
}
