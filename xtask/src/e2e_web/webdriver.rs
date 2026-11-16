//! The few W3C WebDriver commands the run uses, over plain HTTP to chromedriver on loopback
//! (<https://www.w3.org/TR/webdriver2/>). No WebDriver crate: the ones available bring in
//! dependencies outside the licence allowlist, and the protocol is JSON over HTTP.
//!
//! Every request has a deadline, so a hung driver or browser stops the run with an error instead
//! of hanging it. The request bodies and the answer parsing are plain functions, tested below.

use std::time::Duration;

use serde_json::{Value, json};

/// How long one command may take. Starting a browser is the slowest; a page load waits for the
/// page's `load` event (the app's wasm is fetched before it).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// The key on a W3C element reference.
const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

/// The keys the run presses, as WebDriver's code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    Enter,
    Shift,
    Escape,
}

impl Key {
    pub fn code(self) -> char {
        match self {
            Self::Tab => '\u{E004}',
            Self::Enter => '\u{E007}',
            Self::Shift => '\u{E008}',
            Self::Escape => '\u{E00C}',
        }
    }
}

/// A cookie as WebDriver reports it (`GET /session/{id}/cookie`), HttpOnly ones included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub secure: Option<bool>,
    pub http_only: Option<bool>,
    /// `Strict`, `Lax` or `None`.
    pub same_site: Option<String>,
}

/// One session on a driver.
pub struct Session {
    agent: ureq::Agent,
    base: String,
}

impl Session {
    /// `POST /session` with `capabilities`; the session's commands go to `<driver>/session/<id>`.
    pub fn start(driver: &str, capabilities: &Value) -> Result<Self, String> {
        let agent = agent();
        let value = send(&agent, "POST", &format!("{driver}/session"), Some(capabilities), "starting the browser")?;
        let id = session_id(&value).ok_or("starting the browser: the driver's answer has no sessionId")?;
        Ok(Self { agent, base: format!("{driver}/session/{id}") })
    }

    fn cmd(&self, method: &str, path: &str, body: Option<&Value>, what: &str) -> Result<Value, String> {
        send(&self.agent, method, &format!("{}{path}", self.base), body, what)
    }

    /// `GET /window/rect`: `(width, height)`.
    pub fn window_size(&self) -> Result<(i64, i64), String> {
        let v = self.cmd("GET", "/window/rect", None, "reading the window size")?;
        Ok((v["width"].as_i64().unwrap_or(0), v["height"].as_i64().unwrap_or(0)))
    }

    /// `POST /url`: opens a page (and waits for its load event).
    pub fn goto(&self, url: &str) -> Result<(), String> {
        self.cmd("POST", "/url", Some(&json!({"url": url})), "opening the page").map(drop)
    }

    /// `POST /elements` by CSS selector: the matching elements' references.
    pub fn find_all(&self, css: &str) -> Result<Vec<String>, String> {
        let v = self.cmd("POST", "/elements", Some(&json!({"using": "css selector", "value": css})), "looking at the page")?;
        Ok(element_ids(&v))
    }

    /// `POST /element/{id}/value`: for a file input, the path of the file to choose.
    pub fn send_keys_to(&self, element: &str, text: &str) -> Result<(), String> {
        self.cmd("POST", &format!("/element/{element}/value"), Some(&json!({"text": text})), "typing into an element").map(drop)
    }

    /// `POST /actions`, then `DELETE /actions` so no key or button stays down.
    pub fn perform(&self, actions: &Value, what: &str) -> Result<(), String> {
        self.cmd("POST", "/actions", Some(actions), what)?;
        self.cmd("DELETE", "/actions", None, what).map(drop)
    }

    /// `GET /cookie`.
    pub fn cookies(&self) -> Result<Vec<Cookie>, String> {
        let v = self.cmd("GET", "/cookie", None, "reading the cookies")?;
        Ok(parse_cookies(&v))
    }

    /// `POST /log` (chromedriver's, not W3C): drains the named log; each entry's `message`.
    pub fn log(&self, kind: &str) -> Result<Vec<String>, String> {
        let v = self.cmd("POST", "/log", Some(&json!({"type": kind})), "reading the browser's log")?;
        Ok(v.as_array().map(|entries| entries.iter().filter_map(|e| e["message"].as_str().map(str::to_owned)).collect()).unwrap_or_default())
    }

    /// `GET /screenshot`: the PNG's bytes.
    pub fn screenshot(&self) -> Result<Vec<u8>, String> {
        let v = self.cmd("GET", "/screenshot", None, "taking a screenshot")?;
        base64_decode(v.as_str().ok_or("the screenshot answer isn't a string")?).ok_or_else(|| "the screenshot isn't valid base64".to_owned())
    }

    /// `DELETE /session/{id}`: closes the browser.
    pub fn quit(&self) -> Result<(), String> {
        send(&self.agent, "DELETE", &self.base, None, "closing the browser").map(drop)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // WebDriver errors come as 4xx/5xx with a JSON body: read them, don't drop them.
        .http_status_as_error(false)
        .timeout_global(Some(COMMAND_TIMEOUT))
        .user_agent("craftcloud-xtask-e2e-web")
        .build()
        .into()
}

fn send(agent: &ureq::Agent, method: &str, url: &str, body: Option<&Value>, what: &str) -> Result<Value, String> {
    let payload = body.map_or_else(Vec::new, |b| b.to_string().into_bytes());
    let mut req = ureq::http::Request::builder().method(method).uri(url);
    if body.is_some() {
        req = req.header("Content-Type", "application/json; charset=utf-8");
    }
    let req = req.body(payload).map_err(|e| format!("{what}: {e}"))?;
    let mut resp = agent.run(req).map_err(|e| match e {
        ureq::Error::Timeout(_) => format!("{what}: the driver didn't answer in {} s", COMMAND_TIMEOUT.as_secs()),
        e => format!("{what}: the driver can't be reached ({e})"),
    })?;
    let status = resp.status().as_u16();
    // Screenshots are the largest answers (a 1280×800 PNG, base64): far under this.
    let text = resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string().map_err(|e| format!("{what}: reading the answer: {e}"))?;
    answer(status, &text).map_err(|e| format!("{what}: {e}"))
}

/// A WebDriver answer: `{"value": …}`, or on an error `{"value": {"error", "message"}}` with a
/// 4xx/5xx status.
pub fn answer(status: u16, body: &str) -> Result<Value, String> {
    let json: Value = serde_json::from_str(body).map_err(|_| {
        let start: String = body.chars().take(200).collect();
        format!("the driver answered {status} with something that isn't JSON: {start}")
    })?;
    let value = json.get("value").cloned().unwrap_or(Value::Null);
    let error = value.get("error").and_then(Value::as_str);
    if (200..300).contains(&status) && error.is_none() {
        return Ok(value);
    }
    let message = value.get("message").and_then(Value::as_str).unwrap_or_default();
    // chromedriver's messages run on with a session info line and a stack trace.
    let message = message.lines().next().unwrap_or_default();
    Err(match error {
        Some(error) if message.is_empty() => format!("{error} ({status})"),
        Some(error) => format!("{error}: {message}"),
        None => format!("the driver answered {status}"),
    })
}

/// The new session's id: `value.sessionId` (W3C).
pub fn session_id(value: &Value) -> Option<&str> {
    value["sessionId"].as_str().filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// The references in a `POST /elements` answer.
pub fn element_ids(value: &Value) -> Vec<String> {
    value.as_array().map(|all| all.iter().filter_map(|e| e[ELEMENT_KEY].as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

pub fn parse_cookies(value: &Value) -> Vec<Cookie> {
    value
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|c| {
                    Some(Cookie {
                        name: c["name"].as_str()?.to_owned(),
                        value: c["value"].as_str().unwrap_or_default().to_owned(),
                        secure: c["secure"].as_bool(),
                        http_only: c["httpOnly"].as_bool(),
                        same_site: c["sameSite"].as_str().map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The capabilities for headless Chromium: the browser program, its command line, its
/// preferences, and Chromium's logs (the network log needs `performance`).
pub fn chromium_capabilities(binary: &str, args: &[String], prefs: &Value) -> Value {
    json!({
        "capabilities": {
            "alwaysMatch": {
                "browserName": "chrome",
                "goog:chromeOptions": {"binary": binary, "args": args, "prefs": prefs},
                "goog:loggingPrefs": {"performance": "ALL", "browser": "ALL"},
            }
        }
    })
}

/// Keys pressed and released in turn: `keys` one after the other, with `held` (Shift) down
/// around all of them.
pub fn key_actions(held: Option<Key>, keys: impl IntoIterator<Item = char>) -> Value {
    let mut steps = Vec::new();
    if let Some(h) = held {
        steps.push(json!({"type": "keyDown", "value": h.code().to_string()}));
    }
    for k in keys {
        steps.push(json!({"type": "keyDown", "value": k.to_string()}));
        steps.push(json!({"type": "keyUp", "value": k.to_string()}));
    }
    if let Some(h) = held {
        steps.push(json!({"type": "keyUp", "value": h.code().to_string()}));
    }
    json!({"actions": [{"type": "key", "id": "keyboard", "actions": steps}]})
}

/// A left click at `(x, y)` CSS pixels from the viewport's top left.
pub fn click_actions(x: i64, y: i64) -> Value {
    json!({"actions": [{
        "type": "pointer",
        "id": "mouse",
        "parameters": {"pointerType": "mouse"},
        "actions": [
            {"type": "pointerMove", "duration": 0, "origin": "viewport", "x": x, "y": y},
            {"type": "pointerDown", "button": 0},
            {"type": "pointerUp", "button": 0},
        ],
    }]})
}

/// Standard base64 (with or without padding), as WebDriver sends screenshots; `None` if broken.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    }
    let bytes: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let data = bytes.strip_suffix(b"==").or_else(|| bytes.strip_suffix(b"=")).unwrap_or(&bytes);
    if data.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3 + 2);
    for chunk in data.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= sextet(c)? << (18 - 6 * i);
        }
        let [_, a, b, c] = n.to_be_bytes();
        out.extend_from_slice(&[a, b, c][..chunk.len() - 1]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_and_errors() {
        assert_eq!(answer(200, r#"{"value": null}"#), Ok(Value::Null));
        assert_eq!(answer(200, r#"{"value": {"sessionId": "abc", "capabilities": {}}}"#).unwrap()["sessionId"], "abc");
        assert_eq!(
            answer(
                404,
                r#"{"value": {"error": "no such element", "message": "no such element: Unable to locate\n  (Session info: chrome=152)", "stacktrace": "0 x"}}"#
            ),
            Err("no such element: no such element: Unable to locate".to_owned()),
            "the message's first line only, no stack trace"
        );
        assert_eq!(answer(500, r#"{"value": {"error": "unknown error"}}"#), Err("unknown error (500)".to_owned()));
        assert_eq!(answer(200, r#"{"value": {"error": "timeout", "message": "t"}}"#), Err("timeout: t".to_owned()), "an error body wins over 200");
        assert_eq!(answer(502, "{}"), Err("the driver answered 502".to_owned()));
        assert!(answer(200, "<html>proxy</html>").unwrap_err().contains("isn't JSON"));
    }

    #[test]
    fn session_ids_and_elements() {
        assert_eq!(session_id(&json!({"sessionId": "6b1f-a2"})), Some("6b1f-a2"));
        assert_eq!(session_id(&json!({"sessionId": "../x"})), None, "it goes into URLs");
        assert_eq!(session_id(&json!({})), None);
        let found = json!([{ELEMENT_KEY: "e1"}, {"other": "x"}, {ELEMENT_KEY: "e2"}]);
        assert_eq!(element_ids(&found), ["e1", "e2"]);
        assert!(element_ids(&json!([])).is_empty());
    }

    #[test]
    fn cookies_keep_their_flags() {
        let v = json!([
            {"name": "oxicloud_access", "value": "v", "httpOnly": true, "secure": false, "sameSite": "Lax", "path": "/"},
            {"name": "oxicloud_csrf", "value": "c", "httpOnly": false},
            {"value": "nameless"},
        ]);
        let c = parse_cookies(&v);
        assert_eq!(c.len(), 2);
        assert_eq!(
            c[0],
            Cookie { name: "oxicloud_access".into(), value: "v".into(), secure: Some(false), http_only: Some(true), same_site: Some("Lax".into()) }
        );
        assert_eq!((c[1].secure, c[1].same_site.as_deref()), (None, None));
    }

    #[test]
    fn request_bodies() {
        let caps = chromium_capabilities("/usr/bin/chromium", &["--headless=new".into()], &json!({"a": 1}));
        let m = &caps["capabilities"]["alwaysMatch"];
        assert_eq!(m["browserName"], "chrome");
        assert_eq!(m["goog:chromeOptions"]["binary"], "/usr/bin/chromium");
        assert_eq!(m["goog:chromeOptions"]["args"][0], "--headless=new");
        assert_eq!(m["goog:chromeOptions"]["prefs"]["a"], 1);
        assert_eq!(m["goog:loggingPrefs"]["performance"], "ALL");

        let shift_tab = key_actions(Some(Key::Shift), [Key::Tab.code()]);
        let steps = &shift_tab["actions"][0]["actions"];
        let seq: Vec<(&str, &str)> = steps.as_array().unwrap().iter().map(|s| (s["type"].as_str().unwrap(), s["value"].as_str().unwrap())).collect();
        assert_eq!(seq, [("keyDown", "\u{E008}"), ("keyDown", "\u{E004}"), ("keyUp", "\u{E004}"), ("keyUp", "\u{E008}")]);
        assert_eq!(shift_tab["actions"][0]["type"], "key");
        assert_eq!(key_actions(None, "ab".chars())["actions"][0]["actions"].as_array().unwrap().len(), 4);

        let click = click_actions(800, 24);
        let a = &click["actions"][0];
        assert_eq!((a["type"].as_str(), a["parameters"]["pointerType"].as_str()), (Some("pointer"), Some("mouse")));
        assert_eq!(
            (a["actions"][0]["x"].as_i64(), a["actions"][0]["y"].as_i64(), a["actions"][0]["origin"].as_str()),
            (Some(800), Some(24), Some("viewport"))
        );
        assert_eq!(a["actions"][1]["type"], "pointerDown");
        assert_eq!(a["actions"][2]["type"], "pointerUp");
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg").unwrap(), b"foob", "without padding");
        assert_eq!(base64_decode("iVBORw0KGgo=").unwrap(), b"\x89PNG\r\n\x1a\n", "a PNG's signature");
        assert_eq!(base64_decode("Zm9v\nYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("Zm9v*"), None);
        assert_eq!(base64_decode("Z"), None);
    }
}
