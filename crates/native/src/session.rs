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

use crate::http;
use crate::secrets::TokenStore;

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

    /// Keeps the cloud version of `path` in `.craftcloud/versions/<path>/<now>`, then deletes all
    /// but the newest [`versions::VERSIONS_KEPT`]. Nothing to keep (no such file) is fine.
    pub fn snapshot(&mut self, path: &str) -> Result<(), String> {
        // Already kept (an overwrite tried again after it failed, a save with the same bytes):
        // nothing to add.
        let current = self.call(&dav::stat(&self.server, path))?;
        if current.status == 404 {
            return Ok(());
        }
        let entry = dav::parse_list(&current).ok().and_then(|e| e.into_iter().next());
        // When the content was last changed: the copy gets the time of the copy (T-019).
        let modified = entry
            .as_ref()
            .and_then(|e| e.modified.as_deref())
            .and_then(|m| jiff::fmt::rfc2822::parse(m).ok())
            .map(|z| z.timestamp().strftime(versions::MODIFIED_FORMAT).to_string());
        if let Some(etag) = entry.and_then(|e| e.etag)
            && versions::already_kept(&etag, &self.versions(path)?)
        {
            return Ok(());
        }
        for folder in versions::make_folders(&self.server, path) {
            let resp = self.call(&folder)?;
            if !dav::folder_exists_after(&resp) {
                return Err(format!("can't make the versions folder for {path}: the server answered {}", resp.status));
            }
        }
        let stamp = versions::name_for(&jiff::Timestamp::now().strftime(versions::STAMP_FORMAT).to_string(), modified.as_deref());
        let resp = self.call(&versions::snapshot(&self.server, path, &stamp))?;
        versions::snapshot_done(&resp).map_err(|e| format!("keeping the previous version of {path} failed: {e}"))?;
        let names: Vec<String> = self.versions(path)?.into_iter().map(|v| v.stamp).collect();
        for old in versions::to_prune(names) {
            let _ = self.call(&versions::delete(&self.server, path, &old));
        }
        Ok(())
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

    pub fn move_versions(&mut self, from: &str, to: &str) -> Result<(), String> {
        for folder in versions::history_parents(to) {
            let resp = self.call(&dav::create_folder(&self.server, &folder))?;
            if !dav::folder_exists_after(&resp) {
                return Err(format!("can't make the versions folder for {to}: the server answered {}", resp.status));
            }
        }
        let resp = self.call(&versions::move_history(&self.server, from, to))?;
        if resp.is_success() || resp.status == 404 {
            return Ok(());
        }
        if resp.status != 412 {
            return Err(format!("moving the versions of {from}: the server answered {}", resp.status));
        }
        // `to` has a history already (an earlier file of that name): add these to it.
        let (old, new) = (versions::versions_folder(from), versions::versions_folder(to));
        let listed = self.call(&dav::list(&self.server, &old))?;
        let entries = dav::parse_list(&listed).map_err(|e| e.to_string())?;
        for e in entries.iter().filter(|e| e.path != old) {
            let _ = self.call(&dav::move_to(&self.server, &e.path, &format!("{new}/{}", e.name()), e.is_folder));
        }
        let left = self.call(&dav::list(&self.server, &old)).ok().and_then(|r| dav::parse_list(&r).ok()).map_or(0, |e| e.len());
        if left <= 1 {
            let _ = self.call(&dav::delete(&self.server, &old, true));
        }
        Ok(())
    }

    /// The kept versions of `path`, newest first.
    pub fn versions(&mut self, path: &str) -> Result<Vec<versions::Version>, String> {
        let resp = self.call(&versions::list(&self.server, path))?;
        versions::parse_list(&resp).map_err(|e| e.to_string())
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
