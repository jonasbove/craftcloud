//! Which kind of server the live tests run against, and what each kind needs
//! (docs/WEBDAV-STORAGE.md § 13, the Live row; `tests/other-servers/README.md`).
//!
//! `CRAFTCLOUD_TEST_KIND` names it: `oxicloud` (the default, CraftCloud's own server, exactly the
//! suite CI runs), `nextcloud`, `owncloud`, `opencloud` or `webdav` (any other WebDAV server). Every
//! kind but `oxicloud` runs only with `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` (§ 16, "The opt-in until
//! 0.3.0"); a kind the test can't run is a failure, never a silent skip. The other variables:
//!
//! - `CRAFTCLOUD_TEST_SERVER`: the server (OxiCloud: its address; the others: the origin, e.g.
//!   `http://127.0.0.1:8111`; `webdav`: the WebDAV address itself).
//! - `CRAFTCLOUD_TEST_USER`, `CRAFTCLOUD_TEST_PASSWORD`: the account. On Nextcloud and ownCloud the
//!   password is used once, to make an app password the way a person would; the client then signs
//!   in with that, never the account password (§ 10). On a plain WebDAV server it is the WebDAV
//!   password (§ 16, Q6). `CRAFTCLOUD_TEST_APP_PASSWORD` hands one in instead of making one.
//! - `CRAFTCLOUD_TEST_DAV_URL`: the WebDAV base where it isn't `<server>/remote.php/dav/files/<user>/`.
//! - `CRAFTCLOUD_TEST_MODE`: the sync mode the server must get (`two_way` or `download_only`);
//!   required for `webdav`, where it depends on the server, and otherwise what § 5 and § 16 say.
//!
//! Shared by `crates/native/tests/live.rs` and `apps/craftcloud/tests/cli_live.rs` (by path), each
//! using part of it.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use craftcloud_core::api::dav;
use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
use craftcloud_core::storage::Mode;
use craftcloud_core::storage::probe::{Base, Listed, PROPFIND_BODY, parse_listing};
use craftcloud_native::session::AppPasswordSession;

/// The variable that names the kind.
pub const KIND_VAR: &str = "CRAFTCLOUD_TEST_KIND";
/// The opt-in every other kind needs (`craftcloud_native::experimental::VARIABLE`).
pub const OPT_IN_VAR: &str = "CRAFTCLOUD_EXPERIMENTAL_WEBDAV";

/// A kind of server the live tests can run against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Oxicloud,
    Nextcloud,
    Owncloud,
    Opencloud,
    /// Any other WebDAV server (`rclone serve webdav`, Apache mod_dav, a NAS …).
    Webdav,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Oxicloud, Kind::Nextcloud, Kind::Owncloud, Kind::Opencloud, Kind::Webdav];

    /// The word in `CRAFTCLOUD_TEST_KIND` (any case, spaces around it ignored); an unknown word is
    /// nobody's kind.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str().eq_ignore_ascii_case(word.trim()))
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Oxicloud => "oxicloud",
            Self::Nextcloud => "nextcloud",
            Self::Owncloud => "owncloud",
            Self::Opencloud => "opencloud",
            Self::Webdav => "webdav",
        }
    }

    /// The sync mode a server of this kind is expected to get (docs/WEBDAV-STORAGE.md § 5, § 16:
    /// OpenCloud ignores `If-None-Match: *`, Q1). A plain WebDAV server's depends on the server
    /// (`rclone serve webdav` ignores the `PUT` guards, Q2; Apache mod_dav keeps them), so none here.
    pub const fn expected_mode(self) -> Option<Mode> {
        match self {
            Self::Oxicloud | Self::Nextcloud | Self::Owncloud => Some(Mode::TwoWay),
            Self::Opencloud => Some(Mode::DownloadOnly),
            Self::Webdav => None,
        }
    }

    /// How long one sync pass may take. Nextcloud takes about 4 uploads a second (§ 16, Q7), so
    /// the 300-file test needs well over a minute there; the test keeps its 300 files (its
    /// assertion on the activity list needs more than 200 uploads) and waits longer instead.
    pub const fn sync_deadline(self) -> Duration {
        match self {
            Self::Oxicloud => Duration::from_secs(60),
            Self::Nextcloud => Duration::from_secs(300),
            Self::Owncloud | Self::Opencloud | Self::Webdav => Duration::from_secs(180),
        }
    }
}

/// What a run that names another kind without the opt-in says.
pub const NEEDS_OPT_IN: &str = "the live tests run against another kind of server only with CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1";

/// The kind a run is for, from the variable's value and the opt-in's: unset is OxiCloud; an
/// unknown word, or another kind without the opt-in (exactly `1`), is an error the run must show.
pub fn kind_from(kind: Option<&str>, opt_in: Option<&OsStr>) -> Result<Kind, String> {
    let kind = match kind.map(str::trim).filter(|k| !k.is_empty()) {
        None => Kind::Oxicloud,
        Some(word) => Kind::parse(word)
            .ok_or_else(|| format!("{KIND_VAR}={word}: unknown kind of server (oxicloud, nextcloud, owncloud, opencloud or webdav)"))?,
    };
    if kind != Kind::Oxicloud && !opt_in.is_some_and(|v| v == "1") {
        return Err(format!("{KIND_VAR}={}: {NEEDS_OPT_IN}", kind.as_str()));
    }
    Ok(kind)
}

/// The kind this process runs against. A kind it can't run is a failure (`panic`), never a skip.
pub fn kind() -> Kind {
    match kind_from(std::env::var(KIND_VAR).ok().as_deref(), std::env::var_os(OPT_IN_VAR).as_deref()) {
        Ok(kind) => kind,
        Err(why) => panic!("{why}"),
    }
}

/// The test server named in the environment, with what this kind needs to reach it.
#[derive(Clone, Debug)]
pub struct Server {
    pub kind: Kind,
    /// `CRAFTCLOUD_TEST_SERVER` as given (without a trailing slash).
    pub url: String,
    pub user: String,
    password: String,
    app_password: Option<String>,
    /// The WebDAV base (not used for OxiCloud).
    dav_url: String,
    /// The sync mode this server must get.
    pub mode: Mode,
}

impl std::fmt::Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {} as {} ({} expected)", self.kind.as_str(), self.url, self.user, self.mode.words())
    }
}

impl Server {
    /// `None` when `CRAFTCLOUD_TEST_SERVER` is unset (the tests then pass without running). The
    /// kind and the mode are checked here, so a run that names what it can't do fails at once.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("CRAFTCLOUD_TEST_SERVER").ok()?.trim().trim_end_matches('/').to_owned();
        let user = std::env::var("CRAFTCLOUD_TEST_USER").ok()?;
        let password = std::env::var("CRAFTCLOUD_TEST_PASSWORD").ok()?;
        let kind = kind();
        let app_password = std::env::var("CRAFTCLOUD_TEST_APP_PASSWORD").ok().filter(|p| !p.is_empty());
        let dav_url = match std::env::var("CRAFTCLOUD_TEST_DAV_URL").ok().filter(|u| !u.trim().is_empty()) {
            Some(given) => given.trim().to_owned(),
            None => match kind {
                Kind::Oxicloud => format!("{url}/webdav/"),
                Kind::Webdav => format!("{url}/"),
                Kind::Nextcloud | Kind::Owncloud | Kind::Opencloud => format!("{url}/remote.php/dav/files/{user}/"),
            },
        };
        let mode = match std::env::var("CRAFTCLOUD_TEST_MODE").ok().filter(|m| !m.trim().is_empty()) {
            Some(word) => match word.trim() {
                "two_way" | "two-way" => Mode::TwoWay,
                "download_only" | "download-only" => Mode::DownloadOnly,
                other => panic!("CRAFTCLOUD_TEST_MODE={other}: two_way or download_only"),
            },
            None => kind.expected_mode().unwrap_or_else(|| {
                panic!("CRAFTCLOUD_TEST_KIND=webdav needs CRAFTCLOUD_TEST_MODE (two_way or download_only): the mode depends on the server")
            }),
        };
        Some(Self { kind, url, user, password, app_password, dav_url, mode })
    }

    /// The account password: OxiCloud's sign-in, and on a plain WebDAV server the WebDAV password.
    /// Nextcloud and ownCloud take it once, to make the app password.
    pub fn account_password(&self) -> &str {
        &self.password
    }

    /// The WebDAV base.
    pub fn base(&self) -> Base {
        Base::parse(&self.dav_url).unwrap_or_else(|e| panic!("{}: {e}", self.dav_url))
    }

    /// Another address for the WebDAV base (a forwarder in front of the server).
    pub fn set_dav_url(&mut self, url: &str) {
        self.dav_url = url.to_owned();
    }

    /// Whether this kind of server keeps files' ids across a move (`oc:fileid`).
    pub const fn has_file_ids(&self) -> bool {
        matches!(self.kind, Kind::Nextcloud | Kind::Owncloud | Kind::Opencloud)
    }

    /// What the client signs in with: an app password, made once per test run the way a person
    /// would (Nextcloud: the OCS app-password call; ownCloud: Settings › Security, through the
    /// web session as a browser does; OpenCloud: its `auth-app` token call), or the one handed in;
    /// a plain WebDAV server's WebDAV password (§ 16, Q6). The account password is used once, to
    /// make the app password. Never printed.
    pub fn app_password(&self) -> String {
        static MADE: Mutex<Option<String>> = Mutex::new(None);
        let mut made = MADE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(p) = made.as_ref() {
            return p.clone();
        }
        let secret = match (&self.app_password, self.kind) {
            (Some(given), _) => given.clone(),
            (None, Kind::Nextcloud) => self.nextcloud_app_password(),
            (None, Kind::Owncloud) => self.owncloud_app_password(),
            (None, Kind::Opencloud) => self.opencloud_app_token(),
            (None, Kind::Oxicloud | Kind::Webdav) => self.password.clone(),
        };
        *made = Some(secret.clone());
        secret
    }

    /// A session on the WebDAV base with the app password.
    pub fn session(&self) -> AppPasswordSession {
        AppPasswordSession::new(self.base(), &self.user, &self.app_password()).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Nextcloud: `GET /ocs/v2.php/core/getapppassword` with the account's basic auth gives an
    /// app password for this client (the call Nextcloud's own clients make).
    fn nextcloud_app_password(&self) -> String {
        let origin = Base::parse(&format!("{}/", self.url)).unwrap_or_else(|e| panic!("{e}"));
        let mut account = AppPasswordSession::new(origin, &self.user, &self.password).unwrap_or_else(|e| panic!("{e}"));
        let req = HttpRequest::new(Method::Get, format!("{}/ocs/v2.php/core/getapppassword?format=json", self.url)).header("OCS-APIRequest", "true");
        // Making one takes Nextcloud a while (the password is hashed, and a busy server is slower),
        // longer than the short idle limit the connection test sets: a timeout is tried again.
        let mut resp = account.call(&req);
        for _ in 0..3 {
            if resp.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_secs(5));
            resp = account.call(&req);
        }
        let resp = resp.unwrap_or_else(|e| panic!("asking Nextcloud for an app password: {e}"));
        assert_eq!(resp.status, 200, "Nextcloud refused to make an app password ({}); is the account password right?", resp.status);
        let body: serde_json::Value =
            serde_json::from_slice(&resp.body).unwrap_or_else(|e| panic!("Nextcloud's app-password answer isn't JSON: {e}"));
        body["ocs"]["data"]["apppassword"]
            .as_str()
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("no app password in Nextcloud's answer"))
    }

    /// OpenCloud: `POST /auth-app/tokens?expiry=…` with the account's basic auth (the call behind
    /// its web UI's "App tokens") gives a token that is then the password for WebDAV. Measured on
    /// 9.1.1 (2026-11-18): the route is on by default, and the token works on `/remote.php/dav/`.
    fn opencloud_app_token(&self) -> String {
        let origin = Base::parse(&format!("{}/", self.url)).unwrap_or_else(|e| panic!("{e}"));
        let mut account = AppPasswordSession::new(origin, &self.user, &self.password).unwrap_or_else(|e| panic!("{e}"));
        let req = HttpRequest::new(Method::Post, format!("{}/auth-app/tokens?expiry=72h", self.url));
        let resp = account.call(&req).unwrap_or_else(|e| panic!("asking OpenCloud for an app token: {e}"));
        assert_eq!(resp.status, 200, "OpenCloud refused to make an app token ({}); is the account password right?", resp.status);
        let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_else(|e| panic!("OpenCloud's app-token answer isn't JSON: {e}"));
        body["token"].as_str().filter(|t| !t.is_empty()).map(str::to_owned).unwrap_or_else(|| panic!("no token in OpenCloud's answer"))
    }

    /// ownCloud 10 has no API call for it: a person signs in on the web and makes one in
    /// Settings › Security ("App passwords / tokens"). This does the same with `curl`: the login
    /// page's request token, the sign-in form, then the settings' `authtokens` call with the
    /// session's cookies. The password goes to curl on stdin, never on its command line.
    fn owncloud_app_password(&self) -> String {
        let jar = tempfile::NamedTempFile::new().unwrap();
        let jar_path = jar.path().display().to_string();
        let login_page = curl(&["-c", &jar_path, &format!("{}/index.php/login", self.url)], "");
        let token = request_token(&login_page.1).unwrap_or_else(|| panic!("ownCloud's login page has no request token: is {} ownCloud?", self.url));
        // The form, as the browser posts it; the credentials in curl's config on stdin.
        let form = format!(
            "data-urlencode = \"user={}\"\ndata-urlencode = \"password={}\"\ndata-urlencode = \"requesttoken={}\"\ndata-urlencode = \"timezone=UTC\"\n",
            curl_quote(&self.user),
            curl_quote(&self.password),
            curl_quote(&token)
        );
        let signed_in =
            curl(&["-b", &jar_path, "-c", &jar_path, "-o", "/dev/null", "-w", "%{http_code}", &format!("{}/index.php/login", self.url)], &form);
        assert_eq!(signed_in.1.trim(), "303", "ownCloud didn't take the sign-in ({}); is the account password right?", signed_in.1.trim());
        // A signed-in page carries a fresh request token.
        let settings = curl(&["-b", &jar_path, "-c", &jar_path, &format!("{}/index.php/settings/personal?sectionid=security", self.url)], "");
        let token = request_token(&settings.1).unwrap_or_else(|| panic!("no request token after signing in to ownCloud"));
        let made = curl(
            &[
                "-b",
                &jar_path,
                "-c",
                &jar_path,
                "-X",
                "POST",
                "-H",
                &format!("requesttoken: {token}"),
                "-H",
                "OCS-APIRequest: true",
                "--data-urlencode",
                "name=CraftCloud live tests",
                &format!("{}/index.php/settings/personal/authtokens", self.url),
            ],
            "",
        );
        let body: serde_json::Value =
            serde_json::from_str(&made.1).unwrap_or_else(|e| panic!("ownCloud's app-password answer isn't JSON ({e}): curl exit {:?}", made.0));
        body["token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("no app password in ownCloud's answer (curl exit {:?})", made.0))
    }
}

/// `data-requesttoken="…"` on an ownCloud page.
fn request_token(page: &str) -> Option<String> {
    let start = page.find("data-requesttoken=\"")? + "data-requesttoken=\"".len();
    let end = page[start..].find('"')?;
    Some(page[start..start + end].to_owned()).filter(|t| !t.is_empty())
}

/// A value inside a double-quoted string in curl's config file syntax.
fn curl_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Runs `curl -s <args>` with `config` (curl's `-K` syntax) on stdin: (exit code, stdout).
fn curl(args: &[&str], config: &str) -> (Option<i32>, String) {
    let mut cmd = Command::new("curl");
    cmd.arg("-s").arg("-K").arg("-").args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("curl is needed to make an ownCloud app password: {e}"));
    child.stdin.take().unwrap().write_all(config.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The test's own WebDAV client on a server, in the role of another client or the person: it
/// lists, reads, writes and deletes without the sync's guards (what someone else does to the
/// cloud), and cleans up what a test made. Never for the sync under test, which goes through the
/// storage trait.
pub struct Dav {
    session: AppPasswordSession,
    base: Base,
}

impl Dav {
    pub fn new(server: &Server) -> Self {
        let session = server.session();
        let base = session.base().clone();
        Self { session, base }
    }

    /// On another base of the same server (its trash bin, say), with the same sign-in.
    pub fn at(server: &Server, base: &str) -> Self {
        let base = Base::parse(base).unwrap_or_else(|e| panic!("{base}: {e}"));
        let session = AppPasswordSession::new(base.clone(), &server.user, &server.app_password()).unwrap_or_else(|e| panic!("{e}"));
        Self { session, base }
    }

    pub fn base(&self) -> &Base {
        &self.base
    }

    fn call(&mut self, req: &HttpRequest) -> HttpResponse {
        self.session.call(req).unwrap_or_else(|e| panic!("{} {}: {e}", req.method.as_str(), req.url))
    }

    /// One folder, one level, without the folder itself (`""` is the base).
    pub fn list(&mut self, folder: &str) -> Result<Vec<Listed>, String> {
        let req = HttpRequest::new(Method::Propfind, dav::url_under(self.base.url(), folder, true))
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec());
        let resp = self.session.call(&req)?;
        if resp.status != 207 {
            return Err(format!("listing {folder:?}: {}", resp.status));
        }
        let me = folder.trim_matches('/');
        Ok(parse_listing(&resp, &self.base)?.into_iter().filter(|e| e.path != me).collect())
    }

    /// Everything under `folder`, every level, sorted by path.
    pub fn list_all(&mut self, folder: &str) -> Vec<Listed> {
        let mut out = Vec::new();
        let mut folders = vec![folder.to_owned()];
        while let Some(f) = folders.pop() {
            for e in self.list(&f).unwrap_or_else(|e| panic!("{e}")) {
                if e.is_folder {
                    folders.push(e.path.clone());
                }
                out.push(e);
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Writes `bytes` at `path` as another client would (no guard: this is the other side).
    pub fn put(&mut self, path: &str, bytes: &[u8]) -> u16 {
        self.call(&HttpRequest::new(Method::Put, dav::url_under(self.base.url(), path, false)).body(bytes.to_vec())).status
    }

    pub fn get(&mut self, path: &str) -> Option<Vec<u8>> {
        let resp = self.call(&HttpRequest::new(Method::Get, dav::url_under(self.base.url(), path, false)));
        resp.is_success().then_some(resp.body)
    }

    pub fn mkcol(&mut self, path: &str) -> u16 {
        self.call(&dav::create_folder_under(self.base.url(), path)).status
    }

    /// Makes `path` and its parents (one that is there already is fine).
    pub fn mkdirs(&mut self, path: &str) {
        let mut so_far = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(part);
            let status = self.mkcol(&so_far);
            assert!(matches!(status, 201 | 405), "making {so_far}: {status}");
        }
    }

    /// Deletes for good, as the person would on the server (on Nextcloud, ownCloud and OpenCloud
    /// into their trash). Gone already is fine.
    pub fn delete(&mut self, path: &str, folder: bool) -> u16 {
        self.call(&dav::delete_under(self.base.url(), path, folder)).status
    }
}
