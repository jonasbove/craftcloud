//! Headless Chromium through chromedriver, behind a small blocking wrapper: the cases read top to
//! bottom, one browser step at a time. The app draws on a canvas, so the steps are keys and
//! pointer clicks; what happened is read from the server, the cookie jar and the network log.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::webdriver::{self, Cookie, Key, Session};

/// The window every run uses: the layout (and so the keyboard order) depends on the width.
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 800;

/// The programs a run needs, found on PATH or given by the environment.
pub struct Programs {
    pub driver: PathBuf,
    pub browser: PathBuf,
}

const BROWSERS: [&str; 5] = ["chromium", "chromium-browser", "google-chrome-stable", "google-chrome", "chrome"];

impl Programs {
    /// `CRAFTCLOUD_E2E_CHROMEDRIVER` and `CRAFTCLOUD_E2E_CHROME`, else the first on PATH.
    pub fn find() -> Result<Self, String> {
        let driver = std::env::var_os("CRAFTCLOUD_E2E_CHROMEDRIVER").map(PathBuf::from).or_else(|| on_path("chromedriver")).ok_or(
            "chromedriver not found: install it (the version matching your Chromium or Chrome) or set CRAFTCLOUD_E2E_CHROMEDRIVER (docs/TESTING.md)",
        )?;
        let browser = std::env::var_os("CRAFTCLOUD_E2E_CHROME").map(PathBuf::from).or_else(|| BROWSERS.iter().find_map(|b| on_path(b))).ok_or(
            "no Chromium or Chrome found (looked for chromium, google-chrome-stable, google-chrome): install one or set CRAFTCLOUD_E2E_CHROME (docs/TESTING.md)",
        )?;
        for path in [&driver, &browser] {
            if !path.is_file() {
                return Err(format!("{} doesn't exist", path.display()));
            }
        }
        Ok(Self { driver, browser })
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// chromedriver, on a free loopback port, stopped when this is dropped (also after a failure).
pub struct Driver {
    child: Child,
    pub url: String,
}

impl Driver {
    pub fn start(programs: &Programs, log: &Path) -> Result<Self, String> {
        let port = TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_err(|e| format!("no free port for chromedriver: {e}"))?.port();
        let log_file = std::fs::File::create(log).map_err(|e| format!("couldn't create {}: {e}", log.display()))?;
        let child = Command::new(&programs.driver)
            .arg(format!("--port={port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log_file)
            .spawn()
            .map_err(|e| format!("couldn't start {}: {e}", programs.driver.display()))?;
        let mut driver = Self { child, url: format!("http://127.0.0.1:{port}") };
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(2))).build().into();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if agent.get(format!("{}/status", driver.url)).call().is_ok() {
                return Ok(driver);
            }
            if let Ok(Some(status)) = driver.child.try_wait() {
                return Err(format!("chromedriver stopped at once ({status}); its log: {}", log.display()));
            }
            if Instant::now() > deadline {
                driver.stop();
                return Err(format!("chromedriver didn't answer in 20 s; its log: {}", log.display()));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One browser session: its own profile folder, so its cookies are its own.
pub struct Browser {
    session: Option<Session>,
    pub name: &'static str,
    shots: Option<PathBuf>,
    shot_count: usize,
}

impl Browser {
    /// A new browser: its profile (cookies) in `<dir>/<name>/profile`, its downloads in
    /// `<dir>/<name>/downloads`, and with `shots`, a screenshot per step in `<dir>/shots`.
    pub fn open(driver: &Driver, programs: &Programs, dir: &Path, name: &'static str, shots: bool) -> Result<Self, String> {
        let (profile, downloads) = (dir.join(name).join("profile"), Self::downloads_in(dir, name));
        for d in [&profile, &downloads] {
            std::fs::create_dir_all(d).map_err(|e| format!("couldn't create {}: {e}", d.display()))?;
        }
        let shots = shots.then(|| dir.join("shots"));
        if let Some(dir) = &shots {
            std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
        }
        let args = [
            "--headless=new".to_owned(),
            format!("--window-size={WIDTH},{HEIGHT}"),
            format!("--user-data-dir={}", profile.display()),
            "--no-first-run".to_owned(),
            "--no-default-browser-check".to_owned(),
            // WebGL through the software renderer: the app draws with wgpu's WebGL2 fallback, and a
            // headless browser on a machine without a GPU (CI, a server) has nothing else.
            "--use-angle=swiftshader".to_owned(),
            "--enable-unsafe-swiftshader".to_owned(),
            "--force-device-scale-factor=1".to_owned(),
            "--lang=en-US".to_owned(),
        ];
        // Downloads go to the run directory, without asking.
        let prefs = serde_json::json!({"download.default_directory": downloads.to_string_lossy(), "download.prompt_for_download": false});
        let caps = webdriver::chromium_capabilities(&programs.browser.to_string_lossy(), &args, &prefs);
        let session = Session::start(&driver.url, &caps)?;
        let browser = Self { session: Some(session), name, shots, shot_count: 0 };
        // Headless windows open at the size asked for, but say so in case a flag is ignored.
        let (width, height) = browser.web()?.window_size()?;
        if width < i64::from(WIDTH) - 20 {
            return Err(format!("the browser window is {width}×{height}, not {WIDTH}×{HEIGHT}"));
        }
        Ok(browser)
    }

    /// Where the browser `name` saves downloads.
    pub fn downloads_in(dir: &Path, name: &str) -> PathBuf {
        dir.join(name).join("downloads")
    }

    fn web(&self) -> Result<&Session, String> {
        self.session.as_ref().ok_or_else(|| "the browser is closed".to_owned())
    }

    pub fn goto(&self, url: &str) -> Result<(), String> {
        self.web()?.goto(url)
    }

    /// Waits for the app to replace its "Loading CraftCloud…" placeholder (eframe started), then a
    /// little longer for the first answers from the server.
    pub fn wait_for_app(&self) -> Result<(), String> {
        let web = self.web()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while !web.find_all("#craftcloud_loading")?.is_empty() {
            if Instant::now() > deadline {
                return Err("the app didn't start in 30 s (the page still says Loading CraftCloud…)".into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        std::thread::sleep(Duration::from_millis(1500));
        Ok(())
    }

    /// Gives the page the keyboard: one click on an empty spot of the header, which every page
    /// has at this window size and where nothing reacts to a click. The widgets themselves are
    /// reached with Tab, never by position.
    pub fn focus(&self) -> Result<(), String> {
        self.web()?.perform(&webdriver::click_actions(800, 24), "clicking the page")?;
        settle();
        Ok(())
    }

    /// Opens `url`, waits for the app, and gives it the keyboard.
    pub fn open_page(&mut self, url: &str, label: &str) -> Result<(), String> {
        self.goto(url)?;
        self.wait_for_app()?;
        self.focus()?;
        self.shot(label);
        Ok(())
    }

    /// Escape: egui lets go of the keyboard focus. Not on a page whose sheet closes on Escape.
    pub fn escape(&self) -> Result<(), String> {
        self.press(Key::Escape, 1)
    }

    pub fn tab(&self, times: usize) -> Result<(), String> {
        self.press(Key::Tab, times)
    }

    pub fn enter(&self) -> Result<(), String> {
        self.press(Key::Enter, 1)
    }

    /// Activates the last widget on the page that takes the keyboard: with nothing focused,
    /// Shift+Tab goes to the last one, and a sheet draws its main button last.
    pub fn press_last(&self) -> Result<(), String> {
        self.escape()?;
        self.shift_tab(1)?;
        self.enter()
    }

    /// Presses a key `times` times, with a frame or two between presses.
    pub fn press(&self, key: Key, times: usize) -> Result<(), String> {
        let web = self.web()?;
        for _ in 0..times {
            web.perform(&webdriver::key_actions(None, [key.code()]), "pressing a key")?;
            settle();
        }
        Ok(())
    }

    pub fn shift_tab(&self, times: usize) -> Result<(), String> {
        let web = self.web()?;
        for _ in 0..times {
            web.perform(&webdriver::key_actions(Some(Key::Shift), [Key::Tab.code()]), "pressing Shift+Tab")?;
            settle();
        }
        Ok(())
    }

    /// Types into whatever has the keyboard (egui's text field, through eframe's text agent).
    pub fn type_text(&self, text: &str) -> Result<(), String> {
        self.web()?.perform(&webdriver::key_actions(None, text.chars()), "typing")?;
        // A Tab that comes in the same frame as the last letters can be lost: let it draw.
        std::thread::sleep(Duration::from_millis(300));
        Ok(())
    }

    /// Fills form fields one after the other: the first has the keyboard, Tab goes to the next.
    pub fn fill(&self, values: &[&str]) -> Result<(), String> {
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                self.tab(1)?;
                std::thread::sleep(Duration::from_millis(150));
            }
            self.type_text(value)?;
        }
        Ok(())
    }

    /// Hands `file` to the page's upload input (`#craftcloud-upload-input`, made the first time
    /// the Upload button is used), as choosing it in the file dialog would.
    pub fn choose_file(&self, file: &Path) -> Result<(), String> {
        let web = self.web()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let input = loop {
            if let Some(input) = web.find_all("#craftcloud-upload-input")?.into_iter().next() {
                break input;
            }
            if Instant::now() > deadline {
                return Err("the page has no upload input: the Upload button wasn't activated".into());
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        web.send_keys_to(&input, &file.to_string_lossy()).map_err(|e| format!("choosing the file: {e}"))?;
        settle();
        Ok(())
    }

    /// Every cookie the browser holds for the page's site, HttpOnly ones included.
    pub fn cookies(&self) -> Result<Vec<Cookie>, String> {
        self.web()?.cookies()
    }

    /// `name=value; …` for the session's cookies, to replay a request outside the browser.
    pub fn cookie_header(&self) -> Result<String, String> {
        Ok(self.cookies()?.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; "))
    }

    /// What the page sent and got back since the last call (reading the log empties it).
    pub fn network(&self) -> Result<Network, String> {
        let mut net = Network::default();
        for message in self.web()?.log("performance")? {
            net.add(&message);
        }
        Ok(net)
    }

    /// Saves a screenshot as `<shots>/<NN>-<browser>-<label>.png` when `--shots` is on.
    pub fn shot(&mut self, label: &str) {
        let Some(dir) = self.shots.clone() else { return };
        self.shot_count += 1;
        let path = dir.join(format!("{:02}-{}-{}.png", self.shot_count, self.name, label.replace([' ', '/'], "-")));
        if let Err(e) = self.save_shot(&path) {
            eprintln!("  (couldn't save a screenshot: {e})");
        }
    }

    /// Saves a screenshot next to the run's log, whatever `--shots` says (after a failure).
    pub fn shot_to(&self, path: &Path) {
        let _ = self.save_shot(path);
    }

    fn save_shot(&self, path: &Path) -> Result<(), String> {
        let png = self.web()?.screenshot()?;
        std::fs::write(path, png).map_err(|e| format!("couldn't write {}: {e}", path.display()))
    }

    pub fn quit(&mut self) {
        if let Some(web) = self.session.take() {
            let _ = web.quit();
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.quit();
    }
}

/// A few frames for egui to take the input in and answer it. egui moves the focus once per frame,
/// so two Tabs that land in one frame count as one; with software rendering on a busy machine a
/// frame can take well over 100 ms (a shorter wait lost a Tab now and then at a load of 20).
fn settle() {
    std::thread::sleep(Duration::from_millis(300));
}

/// What Chromium's network log (DevTools `Network.*` events) says the page did. Header values
/// are compared, never printed: the run never shows a cookie or a token.
#[derive(Debug, Default)]
pub struct Network {
    pub requests: Vec<SentRequest>,
    /// `(url, status)` of every answer.
    pub responses: Vec<(String, u16)>,
}

impl Network {
    fn add(&mut self, message: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(message) else { return };
        let m = &v["message"];
        let params = &m["params"];
        match m["method"].as_str() {
            Some("Network.requestWillBeSent") => {
                let req = &params["request"];
                let headers = req["headers"]
                    .as_object()
                    .map(|h| h.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.as_str().unwrap_or_default().to_owned())).collect())
                    .unwrap_or_default();
                if let (Some(method), Some(url)) = (req["method"].as_str(), req["url"].as_str()) {
                    self.requests.push(SentRequest { method: method.to_owned(), url: url.to_owned(), headers });
                }
            }
            Some("Network.responseReceived") => {
                let resp = &params["response"];
                if let (Some(url), Some(status)) = (resp["url"].as_str(), resp["status"].as_u64()) {
                    self.responses.push((url.to_owned(), u16::try_from(status).unwrap_or(0)));
                }
            }
            _ => {}
        }
    }

    /// The first request with this method whose path (after the origin, decoded) starts with `path`.
    pub fn request(&self, method: &str, path: &str) -> Option<&SentRequest> {
        self.requests.iter().find(|r| r.method == method && path_of(&r.url).starts_with(path))
    }

    /// The status of the first answer whose path (decoded) starts with `path`.
    pub fn status(&self, path: &str) -> Option<u16> {
        self.responses.iter().find(|(url, _)| path_of(url).starts_with(path)).map(|(_, s)| *s)
    }
}

/// `/api/s/a-b?y` of `http://host:port/api/s/a%2Db?y`: the page encodes more than it must.
fn path_of(url: &str) -> String {
    let path = url.split_once("://").and_then(|(_, rest)| rest.find('/').map(|i| &rest[i..])).unwrap_or(url);
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One request the page sent.
#[derive(Debug, Clone)]
pub struct SentRequest {
    pub method: String,
    pub url: String,
    headers: Vec<(String, String)>,
}

impl SentRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log entry as chromedriver hands it over: the DevTools event as a JSON string.
    fn entry(method: &str, params: serde_json::Value) -> String {
        serde_json::json!({"message": {"method": method, "params": params}}).to_string()
    }

    #[test]
    fn the_network_log_gives_requests_with_their_headers_and_answers() {
        let mut net = Network::default();
        net.add(&entry(
            "Network.requestWillBeSent",
            serde_json::json!({"request": {"method": "PUT", "url": "http://127.0.0.1:8100/webdav/a%20b.txt", "headers": {"X-CSRF-Token": "t"}}}),
        ));
        net.add(&entry("Network.responseReceived", serde_json::json!({"response": {"url": "http://127.0.0.1:8100/api/s/ab%2Dcd", "status": 200}})));
        net.add("not json");
        let put = net.request("PUT", "/webdav/a b.txt").expect("the PUT, found by its decoded path");
        assert_eq!(put.header("x-csrf-token"), Some("t"), "header names match without case");
        assert!(net.request("POST", "/webdav/a b.txt").is_none());
        assert_eq!(net.status("/api/s/ab-cd"), Some(200), "the page encodes `-` as %2D");
        assert_eq!(net.status("/api/s/other"), None);
    }

    #[test]
    fn paths_are_decoded_after_the_origin() {
        assert_eq!(path_of("http://host:8100/api/folders/a%2Db/resources?limit=200"), "/api/folders/a-b/resources?limit=200");
        assert_eq!(path_of("https://cloud.example.org/"), "/");
        assert_eq!(path_of("http://host/bad%zzescape%"), "/bad%zzescape%", "a broken escape stays as it is");
    }
}
