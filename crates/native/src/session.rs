//! Authorised requests for one signed-in server, shared by the sync engine and the command line.
//!
//! **One sign-in, several processes.** OxiCloud rotates refresh tokens and treats a reused one as
//! stolen: it ends the whole sign-in. The desktop client and the `craftcloud` command line keep the
//! same tokens in the keychain, so a refresh is done under a lock file in the settings folder, and
//! first re-reads the stored tokens: when another process has already refreshed, its tokens are
//! taken over instead of refreshing again with a spent refresh token.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::{self, Tokens};
use craftcloud_core::api::{dav, versions};
use craftcloud_core::http::{HttpRequest, HttpResponse};
use craftcloud_core::storage::probe::Base;

use crate::http;
use crate::secrets::TokenStore;
use crate::storage::versions as kept;

/// Called with new tokens after a refresh (when there is no shared store to save them in).
pub type TokenSink = Box<dyn Fn(&Tokens) + Send>;

/// What a signed-out session says; the command line maps it to its "not signed in" exit code.
pub const SIGNED_OUT: &str = "Signed out: this device was removed or its sign-in ran out. Sign in again.";

/// Where refreshed tokens go.
enum Keep {
    /// Saved through a callback only (tests, or a caller that stores them itself).
    Sink(TokenSink),
    /// The shared store, refreshed under `lock`.
    Shared { store: TokenStore, lock: Option<PathBuf>, notify: Option<TokenSink> },
}

pub struct Session {
    pub server: ServerUrl,
    pub tokens: Tokens,
    keep: Keep,
    /// The last transport error (no answer at all); the sync engine clears it each cycle.
    pub offline: Option<String>,
    /// The refresh token was refused: only a new sign-in helps.
    pub signed_out: bool,
    /// Set to stop a transfer that is under way (Pause, Quit); checked between chunks.
    pub cancel: Arc<AtomicBool>,
}

impl Session {
    /// A session whose refreshed tokens go to `on_tokens` only.
    pub fn new(server: ServerUrl, tokens: Tokens, on_tokens: TokenSink) -> Self {
        Self { server, tokens, keep: Keep::Sink(on_tokens), offline: None, signed_out: false, cancel: Arc::default() }
    }

    /// A session on the tokens in `store`, refreshing under a lock file in `dir` (the settings
    /// folder) so other processes on the same sign-in stay signed in.
    pub fn shared(server: ServerUrl, tokens: Tokens, store: TokenStore, dir: Option<&Path>) -> Self {
        Self {
            server,
            tokens,
            keep: Keep::Shared { store, lock: dir.map(|d| d.join("refresh.lock")), notify: None },
            offline: None,
            signed_out: false,
            cancel: Arc::default(),
        }
    }

    /// Also tells `notify` whenever this session's tokens change (a refresh, or tokens taken over
    /// from another process), e.g. for the UI's own requests. Only for [`Self::shared`] sessions;
    /// [`Self::new`] already has its callback.
    #[must_use]
    pub fn on_new_tokens(mut self, notify: TokenSink) -> Self {
        if let Keep::Shared { notify: slot, .. } = &mut self.keep {
            *slot = Some(notify);
        }
        self
    }

    fn tell(&self) {
        match &self.keep {
            Keep::Sink(sink) | Keep::Shared { notify: Some(sink), .. } => sink(&self.tokens),
            Keep::Shared { notify: None, .. } => {}
        }
    }

    /// Remembers a transport error (the server didn't answer).
    fn note<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        if let Err(e) = &result {
            self.offline = Some(e.clone());
        }
        result
    }

    /// Sends a request with the access token; on 401 refreshes (or takes over another process's
    /// refresh) and tries again.
    pub fn call(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
        self.retrying(|t| t.authorize(req.clone()), http::execute, |r| r.status)
    }

    /// Like [`Self::call`], but hands back the open response for streaming a download.
    pub fn open(&mut self, req: &HttpRequest) -> Result<ureq::http::Response<ureq::Body>, String> {
        self.retrying(|t| t.authorize(req.clone()), http::send, |r| r.status().as_u16())
    }

    /// Like [`Self::call`], with the body streamed from a file.
    pub fn upload(&mut self, req: &HttpRequest, file: &Path) -> Result<HttpResponse, String> {
        let cancel = Arc::clone(&self.cancel);
        self.retrying(|t| t.authorize(req.clone()), |r| http::send_file(r, file, &cancel), |r| r.status)
    }

    /// Ends this sign-in on the server (`POST /api/auth/logout` with the refresh token). Its answer:
    /// 2xx when the server ended it. The request is built afresh after a refresh, so it names the
    /// refresh token the server knows now: the spent one names no session, and OxiCloud answers
    /// 200 for that while the real one stays. `Err` with [`SIGNED_OUT`] (and `signed_out` set)
    /// when the refresh was refused: the server had already ended the sign-in.
    pub fn logout(&mut self) -> Result<HttpResponse, String> {
        let server = self.server.clone();
        self.retrying(|t| auth::logout_device(&server, t), http::execute, |r| r.status)
    }

    /// Sends `build(tokens)`; on 401 gets a working access token and sends it again, built with the
    /// new tokens.
    fn retrying<R>(
        &mut self,
        build: impl Fn(&Tokens) -> HttpRequest,
        send: impl Fn(&HttpRequest) -> Result<R, String>,
        status: impl Fn(&R) -> u16,
    ) -> Result<R, String> {
        let first = send(&build(&self.tokens));
        let resp = self.note(first)?;
        if status(&resp) != 401 {
            return Ok(resp);
        }
        let adopted = self.refresh(false)?;
        let resp = send(&build(&self.tokens))?;
        if status(&resp) != 401 || !adopted {
            return Ok(resp);
        }
        // The tokens taken over were stale as well: refresh for real.
        self.refresh(true)?;
        send(&build(&self.tokens))
    }

    /// Gets a working access token. Returns `true` when it took over tokens another process saved
    /// rather than refreshing itself. `force` skips the take-over.
    fn refresh(&mut self, force: bool) -> Result<bool, String> {
        let _guard = match &self.keep {
            Keep::Shared { lock: Some(path), .. } => Some(lock(path)?),
            _ => None,
        };
        if let Keep::Shared { store, .. } = &self.keep
            && !force
            && let Some(stored) = store.load()
            && stored.access_token != self.tokens.access_token
        {
            self.tokens = stored;
            self.tell();
            return Ok(true);
        }
        let answer = http::execute(&auth::refresh(&self.server, &self.tokens.refresh_token));
        let resp = self.note(answer)?;
        if refresh_refused(resp.status) {
            self.signed_out = true;
            return Err(SIGNED_OUT.into());
        }
        self.tokens = auth::parse_refresh(&resp).map_err(|e| e.to_string())?;
        if let Keep::Shared { store, .. } = &self.keep
            && let Err(e) = store.save(&self.tokens)
        {
            log::warn!("couldn't save the refreshed sign-in: {e}");
        }
        self.tell();
        Ok(false)
    }

    /// OxiCloud's WebDAV base, `<server>/webdav/`, for the code shared with the other drivers.
    fn dav_base(&self) -> Result<Base, String> {
        Base::parse(&self.server.join("webdav/"))
    }

    /// Keeps the cloud version of `path` in `.craftcloud/versions/<path>/<now>`, then deletes all
    /// but the newest [`versions::VERSIONS_KEPT`]. Nothing to keep (no such file) is fine
    /// (`storage::versions::keep`, shared with the generic WebDAV driver).
    pub fn snapshot(&mut self, path: &str) -> Result<(), String> {
        let base = self.dav_base()?;
        kept::keep(self, &base, path)
    }

    /// Takes the versions of `from` (a file, or a folder's files) along to `to` after a rename or
    /// move. No history is fine; where `to` already has one, the versions are added to it.
    /// Deletes the whole history of `path` for good (none is fine): see `versions::drop_history`.
    pub fn drop_versions(&mut self, path: &str) -> Result<(), String> {
        let resp = self.call(&versions::drop_history(&self.server, path))?;
        if resp.is_success() || resp.status == 404 {
            Ok(())
        } else {
            Err(format!("deleting the earlier versions of {path}: the server answered {}", resp.status))
        }
    }

    /// The bytes all kept versions take (they count against the quota): `.craftcloud/versions`,
    /// listed folder by folder (the server refuses `Depth: infinity`).
    pub fn versions_bytes(&mut self) -> Result<u64, String> {
        let mut total = 0;
        let mut folders = vec![format!("{}/versions", dav::CRAFTCLOUD_DIR)];
        while let Some(folder) = folders.pop() {
            let resp = self.call(&dav::list(&self.server, &folder))?;
            if resp.status == 404 {
                continue;
            }
            for e in dav::parse_list(&resp).map_err(|e| e.to_string())? {
                if e.is_folder {
                    if e.path != folder {
                        folders.push(e.path);
                    }
                } else {
                    total += e.size;
                }
            }
        }
        Ok(total)
    }

    /// Takes the versions of `from` along to `to` after a rename or move (`storage::versions::follow_move`).
    pub fn move_versions(&mut self, from: &str, to: &str) -> Result<(), String> {
        let base = self.dav_base()?;
        kept::follow_move(self, &base, from, to)
    }

    /// The kept versions of `path`, newest first.
    pub fn versions(&mut self, path: &str) -> Result<Vec<versions::Version>, String> {
        let base = self.dav_base()?;
        kept::list(self, &base, path)
    }

    /// Puts version `stamp` of `path` back, after keeping the current file as a version of its own
    /// (so the restore can be undone).
    pub fn restore_version(&mut self, path: &str, stamp: &str) -> Result<(), String> {
        if !self.versions(path)?.iter().any(|v| v.stamp == stamp) {
            return Err(format!("{path} has no version {stamp}"));
        }
        self.snapshot(path)?;
        let resp = self.call(&versions::restore(&self.server, path, stamp))?;
        craftcloud_core::api::expect_ok(&resp).map_err(|e| format!("restoring {path} failed: {e}"))
    }
}

/// What a session on an app password says when the server refused it (401): it was revoked on
/// the server, and only a new sign-in helps.
pub const APP_PASSWORD_REFUSED: &str = "Signed out: the server no longer takes this app password. Sign in again.";

/// Authorised requests to another WebDAV server, with the user name and an app or WebDAV
/// password the person made there (docs/WEBDAV-STORAGE.md § 10). Nothing refreshes: a 401 is
/// "signed out" (the password was revoked). The password travels only over https, or over plain
/// http to this computer itself (a local server, tests); [`Self::new`] refuses anything else.
pub struct AppPasswordSession {
    base: Base,
    user: String,
    /// The `Authorization` header, made once.
    authorization: String,
    /// The last transport error (no answer at all); the sync engine clears it each pass.
    pub offline: Option<String>,
    /// The server refused the password: only a new sign-in helps.
    pub signed_out: bool,
    /// Set to stop a transfer that is under way (Pause, Quit); checked between chunks.
    pub cancel: Arc<AtomicBool>,
}

impl std::fmt::Debug for AppPasswordSession {
    // Never print the password, nor the header that carries it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppPasswordSession").field("base", &self.base.url()).field("user", &self.user).finish_non_exhaustive()
    }
}

impl AppPasswordSession {
    /// A session on `base` as `user`. `Err` when the password would travel unencrypted to another
    /// computer (an `http://` base that isn't this computer).
    pub fn new(base: Base, user: &str, secret: &str) -> Result<Self, String> {
        if base.is_plain_http() && !base.is_loopback() {
            return Err(format!("{} isn't https: a password is never sent over plain http to another computer", base.url()));
        }
        Ok(Self {
            authorization: format!("Basic {}", base64(format!("{user}:{secret}").as_bytes())),
            base,
            user: user.to_owned(),
            offline: None,
            signed_out: false,
            cancel: Arc::default(),
        })
    }

    pub fn base(&self) -> &Base {
        &self.base
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// Adds the password to a request.
    pub fn authorize(&self, req: HttpRequest) -> HttpRequest {
        req.header("Authorization", self.authorization.clone())
    }

    /// Remembers a transport error, and a refused password.
    fn note<R>(&mut self, result: Result<R, String>, status: impl Fn(&R) -> u16) -> Result<R, String> {
        match result {
            Err(e) => {
                self.offline = Some(e.clone());
                Err(e)
            }
            Ok(r) if status(&r) == 401 => {
                self.signed_out = true;
                Err(APP_PASSWORD_REFUSED.into())
            }
            Ok(r) => Ok(r),
        }
    }

    /// Sends a request and waits for the whole answer.
    pub fn call(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let answer = http::execute(&self.authorize(req.clone()));
        self.note(answer, |r| r.status)
    }

    /// Like [`Self::call`], but hands back the open response for streaming a download.
    pub fn open(&mut self, req: &HttpRequest) -> Result<ureq::http::Response<ureq::Body>, String> {
        let answer = http::send(&self.authorize(req.clone()));
        self.note(answer, |r| r.status().as_u16())
    }

    /// Like [`Self::call`], with the body streamed from a file.
    pub fn upload(&mut self, req: &HttpRequest, file: &Path) -> Result<HttpResponse, String> {
        let answer = http::send_file(&self.authorize(req.clone()), file, &self.cancel);
        self.note(answer, |r| r.status)
    }
}

/// Standard base64 (RFC 4648) of `bytes`, for the Basic authorization header.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Holds the refresh lock until dropped.
struct LockGuard(std::fs::File);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn lock(path: &Path) -> Result<LockGuard, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    }
    let file =
        std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(|e| format!("can't open {}: {e}", path.display()))?;
    file.lock().map_err(|e| format!("can't lock {}: {e}", path.display()))?;
    Ok(LockGuard(file))
}

/// Whether the server's answer to a refresh means the sign-in is over. OxiCloud v0.10.0 answers 404
/// "Session not found" once the device is removed or its session has ended (UPSTREAM.md C10), and
/// that must show as "Signed out", not as an error that never clears.
fn refresh_refused(status: u16) -> bool {
    matches!(status, 401 | 403 | 404)
}

#[cfg(test)]
mod tests {
    use super::{lock, refresh_refused};

    #[test]
    fn a_session_the_server_no_longer_knows_is_signed_out() {
        assert!(refresh_refused(404), "OxiCloud's answer for a removed device");
        assert!(refresh_refused(401) && refresh_refused(403));
        assert!(!refresh_refused(500) && !refresh_refused(502) && !refresh_refused(429), "trouble on the server is tried again");
    }

    /// A server that answers each request it gets (one per connection) with what `answer` says
    /// for its first line and body; the thread hands back the requests it saw.
    #[allow(clippy::type_complexity)]
    fn fake_server(n: usize, answer: fn(&str, &str) -> (u16, &'static str)) -> (String, std::thread::JoinHandle<Vec<(String, String)>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..n {
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
                let (status, reply) = answer(&first, &body);
                let mut conn = reader.into_inner();
                write!(
                    conn,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .unwrap();
                seen.push((first.trim().to_owned(), body));
            }
            seen
        });
        (url, seen)
    }

    #[test]
    fn a_logout_after_a_refresh_names_the_new_refresh_token() {
        // The access token ran out: the logout is refused (401), the session refreshes, and the
        // logout goes again with the NEW refresh token. With the spent one OxiCloud answers 200
        // without ending anything (it names no session).
        let (url, seen) = fake_server(3, |first, body| {
            if first.contains("/api/auth/refresh") {
                (200, r#"{"access_token":"a2","refresh_token":"r2"}"#)
            } else if body.contains("\"r2\"") {
                (200, "{}")
            } else {
                (401, "{}")
            }
        });
        let server = craftcloud_core::ServerUrl::parse(&url).unwrap();
        let tokens = super::Tokens { access_token: "a1".into(), refresh_token: "r1".into() };
        let mut session = super::Session::new(server, tokens, Box::new(|_| {}));
        assert_eq!(session.logout().unwrap().status, 200);
        let seen = seen.join().unwrap();
        assert!(seen[0].0.starts_with("POST /api/auth/logout") && seen[0].1.contains("\"r1\""), "{:?}", seen[0]);
        assert!(seen[1].0.starts_with("POST /api/auth/refresh"));
        assert!(seen[2].0.starts_with("POST /api/auth/logout") && seen[2].1.contains("\"r2\""), "{:?}", seen[2]);
    }

    use std::sync::{Arc, Mutex};

    use craftcloud_core::http::{HttpRequest, Method};

    use super::{SIGNED_OUT, Session, Tokens};
    use crate::fake_server::{FakeServer, Seen};
    use crate::secrets::TokenStore;

    fn tokens(access: &str, refresh: &str) -> Tokens {
        Tokens { access_token: access.into(), refresh_token: refresh.into() }
    }

    /// The answer to a refresh that gives `access`/`refresh`.
    fn new_tokens(access: &str, refresh: &str) -> (u16, String) {
        (200, format!(r#"{{"access_token":"{access}","refresh_token":"{refresh}"}}"#))
    }

    fn is_refresh(r: &Seen) -> bool {
        r.is("POST", "/api/auth/refresh")
    }

    /// Any authorised request: what it asks for doesn't matter here.
    fn me(server: &FakeServer) -> HttpRequest {
        HttpRequest::new(Method::Get, server.server().join("api/auth/me"))
    }

    /// A session whose refreshed tokens are collected in the returned list.
    fn session(server: &FakeServer, start: Tokens) -> (Session, Arc<Mutex<Vec<Tokens>>>) {
        let told: Arc<Mutex<Vec<Tokens>>> = Arc::default();
        let sink = Arc::clone(&told);
        (Session::new(server.server(), start, Box::new(move |t| sink.lock().unwrap().push(t.clone()))), told)
    }

    #[test]
    fn an_expired_access_token_is_refreshed_once_and_the_request_sent_again() {
        let server = FakeServer::start(|r| match r.bearer() {
            _ if is_refresh(r) => new_tokens("a2", "r2"),
            Some("a2") => (200, "{}".into()),
            _ => (401, "{}".into()),
        });
        let (mut session, told) = session(&server, tokens("a1", "r1"));
        assert_eq!(session.call(&me(&server)).unwrap().status, 200);
        let seen = server.seen();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(seen[0].bearer(), Some("a1"));
        assert!(is_refresh(&seen[1]) && seen[1].body.contains("\"r1\""), "{:?}", seen[1]);
        assert_eq!(seen[2].bearer(), Some("a2"), "sent again with the new token");
        assert_eq!(session.tokens, tokens("a2", "r2"));
        assert_eq!(*told.lock().unwrap(), vec![tokens("a2", "r2")], "the new tokens are handed on to be saved");
        assert!(!session.signed_out && session.offline.is_none());
    }

    #[test]
    fn a_request_refused_again_after_a_refresh_is_not_retried_forever() {
        // A 401 that a fresh token doesn't cure: one refresh, one more try, then the 401 is the
        // answer.
        let server = FakeServer::start(|r| if is_refresh(r) { new_tokens("a2", "r2") } else { (401, "{}".into()) });
        let (mut session, _) = session(&server, tokens("a1", "r1"));
        assert_eq!(session.call(&me(&server)).unwrap().status, 401);
        assert_eq!(server.seen().len(), 3);
        assert_eq!(server.count("POST", "/api/auth/refresh"), 1);
        assert!(!session.signed_out);
    }

    #[test]
    fn a_refused_refresh_signs_out_instead_of_retrying() {
        // 404 "Session not found" is what OxiCloud v0.10.0 answers for a removed device.
        for refused in [401, 403, 404] {
            let server =
                FakeServer::start(move |r| if is_refresh(r) { (refused, r#"{"error":"Session not found"}"#.into()) } else { (401, "{}".into()) });
            let (mut session, told) = session(&server, tokens("a1", "r1"));
            assert_eq!(session.call(&me(&server)).unwrap_err(), SIGNED_OUT, "refresh answered {refused}");
            assert!(session.signed_out, "refresh answered {refused}");
            assert!(session.offline.is_none(), "a refusal is an answer, not being offline");
            assert_eq!(server.seen().len(), 2, "the request and one refresh, nothing more (refresh answered {refused})");
            assert!(told.lock().unwrap().is_empty());
            assert_eq!(session.tokens, tokens("a1", "r1"));
        }
    }

    #[test]
    fn trouble_on_the_server_during_a_refresh_is_not_a_sign_out() {
        let server = FakeServer::start(|r| if is_refresh(r) { (502, "{}".into()) } else { (401, "{}".into()) });
        let (mut session, _) = session(&server, tokens("a1", "r1"));
        let err = session.call(&me(&server)).unwrap_err();
        assert_ne!(err, SIGNED_OUT);
        assert!(!session.signed_out, "a 502 is tried again later; the sign-in stays");
        assert_eq!(server.seen().len(), 2);
    }

    #[test]
    fn tokens_another_process_saved_are_taken_over_without_a_refresh() {
        // The command line refreshed while the desktop client's token ran out: refreshing again
        // with the spent refresh token would end the sign-in, so the client takes the saved ones.
        let dir = tempfile::tempdir().unwrap();
        let server = FakeServer::start(|r| match r.bearer() {
            _ if is_refresh(r) => (401, "{}".into()),
            Some("a2") => (200, "{}".into()),
            _ => (401, "{}".into()),
        });
        TokenStore::file_only(&server.url, dir.path()).save(&tokens("a2", "r2")).unwrap();
        let told: Arc<Mutex<Vec<Tokens>>> = Arc::default();
        let sink = Arc::clone(&told);
        let store = TokenStore::file_only(&server.url, dir.path());
        let mut session = Session::shared(server.server(), tokens("a1", "r1"), store, Some(dir.path()))
            .on_new_tokens(Box::new(move |t| sink.lock().unwrap().push(t.clone())));
        assert_eq!(session.call(&me(&server)).unwrap().status, 200);
        assert_eq!(server.count("POST", "/api/auth/refresh"), 0, "no refresh with the spent token");
        assert_eq!(server.seen().iter().map(|r| r.bearer().unwrap_or("")).collect::<Vec<_>>(), ["a1", "a2"]);
        assert_eq!(session.tokens, tokens("a2", "r2"));
        assert_eq!(*told.lock().unwrap(), vec![tokens("a2", "r2")]);
        assert!(dir.path().join("refresh.lock").exists(), "taken over under the refresh lock");
    }

    #[test]
    fn stale_tokens_taken_over_are_refreshed_with_their_own_refresh_token() {
        // The saved tokens ran out as well: a real refresh follows, with the refresh token that
        // came with them (r1 is spent: using it would end the sign-in), and the result is saved.
        let dir = tempfile::tempdir().unwrap();
        let server = FakeServer::start(|r| match r.bearer() {
            _ if is_refresh(r) && r.body.contains("\"r2\"") => new_tokens("a3", "r3"),
            _ if is_refresh(r) => (401, "{}".into()),
            Some("a3") => (200, "{}".into()),
            _ => (401, "{}".into()),
        });
        let store = || TokenStore::file_only(&server.url, dir.path());
        store().save(&tokens("a2", "r2")).unwrap();
        let mut session = Session::shared(server.server(), tokens("a1", "r1"), store(), Some(dir.path()));
        assert_eq!(session.call(&me(&server)).unwrap().status, 200);
        let seen = server.seen();
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert_eq!((seen[0].bearer(), seen[1].bearer()), (Some("a1"), Some("a2")));
        assert!(is_refresh(&seen[2]) && seen[2].body.contains("\"r2\""), "{:?}", seen[2]);
        assert_eq!(seen[3].bearer(), Some("a3"));
        assert_eq!(store().load(), Some(tokens("a3", "r3")), "saved for the other processes");
        assert!(!session.signed_out);
    }

    /// Every log line of this test process (installed once; other tests' lines land here too),
    /// through the same filter as the apps' loggers (`craftcloud_core::logging::WireCapped`).
    fn captured_logs() -> &'static Mutex<Vec<(log::Level, String)>> {
        struct Capture(Mutex<Vec<(log::Level, String)>>);
        impl log::Log for Capture {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                let line = format!("{} {}", record.target(), record.args());
                self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((record.level(), line));
            }
            fn flush(&self) {}
        }
        static LOGS: craftcloud_core::logging::WireCapped<Capture> = craftcloud_core::logging::WireCapped(Capture(Mutex::new(Vec::new())));
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        INSTALLED.call_once(|| {
            if log::set_logger(&LOGS).is_ok() {
                log::set_max_level(log::LevelFilter::Trace);
            }
        });
        &LOGS.0.0
    }

    /// Runs a session through a refresh whose new tokens can't be saved (logged), a refused
    /// refresh and a server that's gone, with tokens named `<marker>-…`. Hands back what the
    /// session said (answers and errors) and the log lines at `max` and above that name a token.
    fn token_trouble(marker: &'static str, max: log::Level) -> (Vec<String>, Vec<String>) {
        let logs = captured_logs();
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "not a folder").unwrap();
        let server = FakeServer::start(move |r| match r.bearer() {
            _ if is_refresh(r) && r.body.contains(&format!("{marker}-R1")) => new_tokens(&format!("{marker}-A2"), &format!("{marker}-R2")),
            _ if is_refresh(r) => (404, r#"{"error":"Session not found"}"#.into()),
            _ => (401, "{}".into()),
        });
        // The store's folder is under a file: saving the refreshed tokens fails and says so.
        let store = TokenStore::file_only(&server.url, &blocked.join("sub"));
        let mut session = Session::shared(server.server(), tokens(&format!("{marker}-A1"), &format!("{marker}-R1")), store, None);
        let mut said = vec![session.call(&me(&server)).map(|r| r.status.to_string()).unwrap_or_else(|e| e)];
        said.push(session.call(&me(&server)).map(|r| r.status.to_string()).unwrap_or_else(|e| e));
        assert!(session.signed_out, "the second refresh was refused: {said:?}");
        let url = server.url.clone();
        drop(server);
        let gone = HttpRequest::new(Method::Get, format!("{url}/api/auth/me"));
        said.push(session.call(&gone).map(|r| r.status.to_string()).unwrap_or_else(|e| e));
        said.extend(session.offline.clone());
        let logs = logs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(logs.iter().any(|(_, l)| l.contains("couldn't save the refreshed sign-in")), "the save failure is logged");
        let leaked = logs.iter().filter(|(level, l)| *level <= max && l.contains(marker)).map(|(level, l)| format!("{level} {l}")).collect();
        (said, leaked)
    }

    #[test]
    fn tokens_never_reach_an_error_or_the_log() {
        // Up to debug: what `RUST_LOG=debug` shows, CraftCloud's own lines and the libraries'.
        let (said, leaked) = token_trouble("SECRET", log::Level::Debug);
        assert!(said.iter().all(|s| !s.contains("SECRET")), "a token in what the session said: {said:?}");
        assert!(leaked.is_empty(), "a token in the log: {leaked:?}");
    }

    #[test]
    fn tokens_never_reach_the_trace_log() {
        let (_, leaked) = token_trouble("TRACED", log::Level::Trace);
        assert!(leaked.is_empty(), "a token in the log: {leaked:?}");
    }

    #[test]
    fn the_refresh_lock_is_exclusive_across_handles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("refresh.lock");
        let held = lock(&path).unwrap();
        let other = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        assert!(other.try_lock().is_err(), "a second holder must wait");
        drop(held);
        assert!(other.try_lock().is_ok());
    }
}
