//! HTTP for the desktop: `ureq`, blocking, always on worker threads. [`execute`] is for the sync
//! thread; [`ThreadBackend`] runs the UI's requests on its own thread so frames never wait.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use craftcloud_core::http::{HttpRequest, HttpResponse, Method};

/// Called when an answer is ready, so the UI repaints (e.g. `egui::Context::request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// How long a transfer may sit without a byte moving either way before it counts as stalled (a
/// half-open connection after a Wi-Fi drop or a suspend, a server that stopped answering). A slow
/// transfer that keeps moving is never cut off.
/// `CRAFTCLOUD_HTTP_IDLE_SECS` overrides it (tests of stalled connections).
pub fn idle() -> Duration {
    std::env::var("CRAFTCLOUD_HTTP_IDLE_SECS").ok().and_then(|s| s.parse().ok()).map_or(IDLE, Duration::from_secs)
}

/// The usual idle limit (see [`idle`]).
pub const IDLE: Duration = Duration::from_secs(60);

/// The shared agent and its pool of kept-alive connections. After a transport error the pool is
/// dropped (a fresh agent): connections kept from before a dropped route may be dead too, and
/// each would otherwise wait out its own idle limit.
static AGENT: LazyLock<Mutex<ureq::Agent>> = LazyLock::new(|| Mutex::new(agent(idle(), FOLLOW_REDIRECTS)));

/// The agent for another WebDAV server's requests (the generic driver and the probe over an app
/// password): the same, but it **never follows a redirect**. A 3xx comes back as the answer it
/// is, for the driver to report as an error (unknown, never "gone"). Apache's mod_dav answers 301
/// to a folder URL without its trailing slash; followed, that became a `GET` without the
/// `Authorization` header (redirects never carry it, rightly), a 401, and a session wrongly marked
/// signed out. CraftCloud's own server keeps the usual agent, so nothing changes for it.
static DIRECT_AGENT: LazyLock<Mutex<ureq::Agent>> = LazyLock::new(|| Mutex::new(agent(idle(), 0)));

/// ureq's own default: how many redirects the usual agent follows.
const FOLLOW_REDIRECTS: u32 = 10;

fn shared() -> ureq::Agent {
    AGENT.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn direct() -> ureq::Agent {
    DIRECT_AGENT.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Passes a result through, dropping the connection pool on a transport error.
fn pooled<T>(result: Result<T, String>) -> Result<T, String> {
    if result.is_err() {
        *AGENT.lock().unwrap_or_else(PoisonError::into_inner) = agent(idle(), FOLLOW_REDIRECTS);
    }
    result
}

/// [`pooled`] for the direct agent.
fn pooled_direct<T>(result: Result<T, String>) -> Result<T, String> {
    if result.is_err() {
        *DIRECT_AGENT.lock().unwrap_or_else(PoisonError::into_inner) = agent(idle(), 0);
    }
    result
}

/// What a cancelled transfer says (Pause or Quit while a file was moving).
pub const CANCELLED: &str = "cancelled";
/// What an upload says when its file changed while it was being sent.
pub const CHANGED_WHILE_UPLOADING: &str = "changed while uploading; trying again";

fn agent(idle: Duration, max_redirects: u32) -> ureq::Agent {
    ureq::Agent::config_builder()
        // 4xx/5xx are answers to read (412 = conflict, 401 = refresh), not transport errors.
        .http_status_as_error(false)
        // PROPFIND, MKCOL, COPY.
        .allow_non_standard_methods(true)
        .user_agent(concat!("CraftCloud/", env!("CARGO_PKG_VERSION")))
        // A server that never answers (a wrong port, a dropped packet) is an error, not a hang.
        .timeout_connect(Some(Duration::from_secs(10)))
        // Idle limits, per read and per write: a stalled transfer is an error (tried again next
        // cycle), however long a moving one takes.
        .timeout_per_read(Some(idle))
        .timeout_per_write(Some(idle))
        // 0 for another WebDAV server (see `DIRECT_AGENT`); the usual agent follows as before.
        .max_redirects(max_redirects)
        .build()
        .into()
}

fn method(m: Method) -> ureq::http::Method {
    ureq::http::Method::from_bytes(m.as_str().as_bytes()).unwrap_or(ureq::http::Method::GET)
}

/// Sends one request and waits for the whole answer. Errors are transport errors only.
pub fn execute(req: &HttpRequest) -> Result<HttpResponse, String> {
    pooled(execute_on(&shared(), req))
}

/// [`execute`] without following redirects (another WebDAV server; see `DIRECT_AGENT`).
pub fn execute_direct(req: &HttpRequest) -> Result<HttpResponse, String> {
    pooled_direct(execute_on(&direct(), req))
}

/// [`send`] without following redirects.
pub fn send_direct(req: &HttpRequest) -> Result<ureq::http::Response<ureq::Body>, String> {
    pooled_direct(send_on(&direct(), req))
}

/// [`send_file`] without following redirects.
pub fn send_file_direct(req: &HttpRequest, file: &std::path::Path, cancel: &Arc<AtomicBool>) -> Result<HttpResponse, String> {
    pooled_direct(send_file_on(&direct(), req, file, cancel))
}

fn execute_on(agent: &ureq::Agent, req: &HttpRequest) -> Result<HttpResponse, String> {
    let mut resp = send_on(agent, req)?;
    let body = resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())?;
    Ok(answer(&resp, body))
}

/// Sends a request and hands back the open response, for streaming a download to disk.
pub fn send(req: &HttpRequest) -> Result<ureq::http::Response<ureq::Body>, String> {
    pooled(send_on(&shared(), req))
}

fn send_on(agent: &ureq::Agent, req: &HttpRequest) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut builder = ureq::http::Request::builder().method(method(req.method)).uri(&req.url);
    for (name, value) in &req.headers {
        builder = builder.header(name, value);
    }
    if req.body.is_empty() {
        builder.body(()).map_err(|e| e.to_string()).and_then(|r| agent.run(r).map_err(|e| describe(&e)))
    } else {
        builder.body(req.body.clone()).map_err(|e| e.to_string()).and_then(|r| agent.run(r).map_err(|e| describe(&e)))
    }
}

/// Sends a request whose body is a file, streamed from disk (large uploads never sit in memory).
/// Exactly the file's length at the start is sent; a file that shrinks or grows meanwhile stops
/// the upload with [`CHANGED_WHILE_UPLOADING`] (tried again next cycle), and `cancel` stops it
/// with [`CANCELLED`].
pub fn send_file(req: &HttpRequest, file: &std::path::Path, cancel: &Arc<AtomicBool>) -> Result<HttpResponse, String> {
    pooled(send_file_on(&shared(), req, file, cancel))
}

fn send_file_on(agent: &ureq::Agent, req: &HttpRequest, file: &std::path::Path, cancel: &Arc<AtomicBool>) -> Result<HttpResponse, String> {
    let body = FileBody::open(file, Arc::clone(cancel))?;
    let mut builder = ureq::http::Request::builder().method(method(req.method)).uri(&req.url).header("Content-Length", body.len.to_string());
    for (name, value) in &req.headers {
        builder = builder.header(name, value);
    }
    let request = builder.body(ureq::SendBody::from_owned_reader(body)).map_err(|e| e.to_string())?;
    let mut resp = agent.run(request).map_err(|e| why_upload_failed(&e, cancel))?;
    let body = resp.body_mut().with_config().limit(1024 * 1024).read_to_vec().map_err(|e| e.to_string())?;
    Ok(answer(&resp, body))
}

/// A transport error during an upload, in words: our own reasons first (ureq hands them back
/// wrapped in its I/O error).
fn why_upload_failed(e: &ureq::Error, cancel: &AtomicBool) -> String {
    let text = e.to_string();
    if cancel.load(Ordering::Relaxed) {
        CANCELLED.into()
    } else if text.contains(CHANGED_WHILE_UPLOADING) || text.contains("than content-length") {
        CHANGED_WHILE_UPLOADING.into()
    } else {
        describe(e)
    }
}

/// A file as an upload body: exactly its length when opened, and an error as soon as it changes
/// (an app still saving) or the transfer is cancelled, so the server is never left waiting for
/// bytes that will not come.
struct FileBody {
    file: std::fs::File,
    left: u64,
    len: u64,
    modified: Option<SystemTime>,
    cancel: Arc<AtomicBool>,
}

impl FileBody {
    fn open(path: &std::path::Path, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        Ok(Self { file, left: meta.len(), len: meta.len(), modified: meta.modified().ok(), cancel })
    }

    fn changed(&self) -> bool {
        self.file.metadata().map_or(true, |m| m.len() != self.len || m.modified().ok() != self.modified)
    }
}

impl Read for FileBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other(CANCELLED));
        }
        if self.left == 0 {
            return Ok(0);
        }
        if self.changed() {
            return Err(std::io::Error::other(CHANGED_WHILE_UPLOADING));
        }
        let want = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.file.read(&mut buf[..want])?;
        if n == 0 {
            // Shorter than it was: the server would wait for the rest forever.
            return Err(std::io::Error::other(CHANGED_WHILE_UPLOADING));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// Copies a streamed response body into `out`, returning the bytes written. `cancel` stops it
/// between chunks with [`CANCELLED`].
pub fn stream_body(resp: &mut ureq::http::Response<ureq::Body>, out: &mut impl std::io::Write, cancel: &AtomicBool) -> Result<u64, String> {
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(total);
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        total += n as u64;
    }
}

pub fn answer(resp: &ureq::http::Response<ureq::Body>, body: Vec<u8>) -> HttpResponse {
    let mut out = HttpResponse::new(resp.status().as_u16(), body);
    for (name, value) in resp.headers() {
        if let Ok(v) = value.to_str() {
            out = out.with_header(name.as_str(), v);
        }
    }
    out
}

/// How a transport error starts when the server couldn't be reached: nothing listens there, the
/// connection was refused, reset or timed out, or there is no route to it.
pub const CANT_CONNECT: &str = "Can't connect to the server";
/// How a transport error starts when the server's name couldn't be looked up.
pub const CANT_FIND: &str = "Can't find the server";

/// Whether a transport error (an `Err` from [`execute`], [`send`] or [`send_file`]) means the
/// server couldn't be reached at all.
pub fn is_unreachable(e: &str) -> bool {
    e.starts_with(CANT_CONNECT) || e.starts_with(CANT_FIND)
}

/// A transport error in words, starting with [`CANT_CONNECT`] or [`CANT_FIND`] when the server
/// couldn't be reached.
fn describe(e: &ureq::Error) -> String {
    match unreachable(e) {
        Some(why) => format!("{why} ({e})."),
        None => e.to_string(),
    }
}

/// Why the server couldn't be reached, read from the error's kind and never from its words: the
/// system's messages differ by OS and language ("Connection refused" on Linux, "No connection
/// could be made because the target machine actively refused it." on Windows).
fn unreachable(e: &ureq::Error) -> Option<&'static str> {
    use std::io::ErrorKind as Kind;
    match e {
        ureq::Error::HostNotFound | ureq::Error::Timeout(ureq::Timeout::Resolve) => Some(CANT_FIND),
        ureq::Error::ConnectionFailed | ureq::Error::Timeout(ureq::Timeout::Connect) => Some(CANT_CONNECT),
        ureq::Error::Io(io) => match io.kind() {
            Kind::ConnectionRefused
            | Kind::ConnectionReset
            | Kind::ConnectionAborted
            | Kind::TimedOut
            | Kind::HostUnreachable
            | Kind::NetworkUnreachable
            | Kind::NetworkDown
            | Kind::AddrNotAvailable => Some(CANT_CONNECT),
            _ if lookup_failed(io) => Some(CANT_FIND),
            _ => None,
        },
        _ => None,
    }
}

/// A failed name lookup, which has no `ErrorKind` of its own. On Windows it carries Winsock's
/// lookup codes (WSAHOST_NOT_FOUND, WSATRY_AGAIN, WSANO_RECOVERY and WSANO_DATA: 11001 to 11004,
/// numbers no Unix errno uses). On Unix getaddrinfo's errors are not errnos, and the standard
/// library words them itself, the same way on every system and in every language.
fn lookup_failed(io: &std::io::Error) -> bool {
    matches!(io.raw_os_error(), Some(11001..=11004)) || io.to_string().starts_with("failed to lookup address information")
}

/// The UI's transport: one worker thread that runs requests in order and wakes the UI.
pub struct ThreadBackend {
    jobs: Sender<(u64, HttpRequest)>,
    done: Receiver<(u64, Result<HttpResponse, String>)>,
}

impl ThreadBackend {
    pub fn new(waker: Waker) -> Self {
        let (jobs, inbox) = channel::<(u64, HttpRequest)>();
        let (outbox, done) = channel();
        std::thread::Builder::new()
            .name("craftcloud-http".into())
            .spawn(move || {
                for (id, req) in inbox {
                    let result = execute(&req);
                    if outbox.send((id, result)).is_err() {
                        return;
                    }
                    waker();
                }
            })
            .map_err(|e| log::error!("couldn't start the HTTP thread: {e}"))
            .ok();
        Self { jobs, done }
    }

    pub fn send(&mut self, id: u64, request: HttpRequest) {
        if self.jobs.send((id, request)).is_err() {
            log::error!("the HTTP thread has stopped");
        }
    }

    pub fn poll(&mut self) -> Vec<(u64, Result<HttpResponse, String>)> {
        self.done.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CANCELLED, CANT_CONNECT, CANT_FIND, CHANGED_WHILE_UPLOADING, FOLLOW_REDIRECTS, FileBody, agent, describe, execute_on, is_unreachable,
        send_file_on,
    };

    /// The direct agent (another WebDAV server) hands a 3xx back as the answer it is, never as an
    /// error and never followed; the usual agent follows it as before, without the password.
    #[test]
    fn the_direct_agent_never_follows_a_redirect() {
        use crate::fake_server::{Answer, FakeServer, Seen};
        let server =
            FakeServer::start_full(
                |r: &Seen| {
                    if r.path == "/old" { Answer::new(301, "").header("Location", "/new") } else { Answer::new(200, "here") }
                },
            );
        let req = HttpRequest::new(Method::Get, format!("{}/old", server.url)).header("Authorization", "Basic c2VjcmV0");
        let direct = execute_on(&agent(Duration::from_secs(5), 0), &req).unwrap();
        assert_eq!(direct.status, 301, "the redirect is the answer, not an error");
        assert_eq!(direct.header("Location"), Some("/new"));
        assert_eq!(server.seen().len(), 1, "never followed: {:?}", server.seen());
        let followed = execute_on(&agent(Duration::from_secs(5), FOLLOW_REDIRECTS), &req).unwrap();
        assert_eq!((followed.status, followed.body.as_slice()), (200, b"here".as_slice()), "the usual agent follows as before");
        let seen = server.seen();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(seen[2].header("authorization").is_none(), "the password never travels to the redirect target: {:?}", seen[2]);
    }
    use craftcloud_core::http::{HttpRequest, Method};
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    fn file_of(len: usize) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        std::fs::write(&path, vec![b'x'; len]).unwrap();
        (dir, path)
    }

    fn read_all(body: &mut FileBody) -> std::io::Result<u64> {
        let mut buf = [0u8; 1000];
        let mut total = 0;
        loop {
            match body.read(&mut buf)? {
                0 => return Ok(total),
                n => total += n as u64,
            }
        }
    }

    fn first_chunk(path: &std::path::Path, cancel: Arc<AtomicBool>) -> FileBody {
        let mut body = FileBody::open(path, cancel).unwrap();
        assert_eq!(body.read(&mut [0u8; 1000]).unwrap(), 1000);
        body
    }

    #[test]
    fn an_unchanged_file_is_sent_whole() {
        let (_dir, path) = file_of(10_000);
        assert_eq!(read_all(&mut FileBody::open(&path, Arc::default()).unwrap()).unwrap(), 10_000);
    }

    #[test]
    fn a_file_that_shrinks_while_sending_stops_the_upload() {
        let (_dir, path) = file_of(10_000);
        let mut body = first_chunk(&path, Arc::default());
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(3000).unwrap();
        assert_eq!(read_all(&mut body).unwrap_err().to_string(), CHANGED_WHILE_UPLOADING);
    }

    #[test]
    fn a_file_that_grows_while_sending_stops_the_upload() {
        let (_dir, path) = file_of(10_000);
        let mut body = first_chunk(&path, Arc::default());
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"more").unwrap();
        assert_eq!(read_all(&mut body).unwrap_err().to_string(), CHANGED_WHILE_UPLOADING);
    }

    #[test]
    fn a_reader_that_ends_early_is_an_error_not_a_short_body() {
        let (_dir, path) = file_of(10_000);
        let mut body = FileBody::open(&path, Arc::default()).unwrap();
        // More promised to the server than the file holds when read.
        body.left = 20_000;
        assert_eq!(read_all(&mut body).unwrap_err().to_string(), CHANGED_WHILE_UPLOADING);
    }

    #[test]
    fn a_cancelled_upload_stops_between_chunks() {
        let (_dir, path) = file_of(10_000);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut body = first_chunk(&path, Arc::clone(&cancel));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(read_all(&mut body).unwrap_err().to_string(), CANCELLED);
    }

    /// A server that takes one connection, reads whatever comes and never answers.
    fn stalled_server() -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/webdav/f.bin", listener.local_addr().unwrap());
        let held = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            while conn.read(&mut buf).is_ok_and(|n| n > 0) {}
        });
        (url, held)
    }

    #[test]
    fn a_stalled_server_is_an_error_not_a_hang() {
        let quick = agent(Duration::from_millis(500), FOLLOW_REDIRECTS);
        let (url, held) = stalled_server();
        let started = Instant::now();
        assert!(execute_on(&quick, &HttpRequest::new(Method::Get, url)).is_err());
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        held.join().unwrap();

        let (url, held) = stalled_server();
        let (_dir, path) = file_of(100_000);
        let started = Instant::now();
        assert!(send_file_on(&quick, &HttpRequest::new(Method::Put, url), &path, &Arc::default()).is_err());
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        held.join().unwrap();
    }

    fn io(kind: std::io::ErrorKind, message: &str) -> ureq::Error {
        ureq::Error::Io(std::io::Error::new(kind, message))
    }

    #[test]
    fn a_server_that_cant_be_reached_reads_the_same_on_every_system() {
        use std::io::ErrorKind as Kind;
        for e in [
            // Linux and macOS.
            io(Kind::ConnectionRefused, "Connection refused (os error 111)"),
            io(Kind::ConnectionReset, "Connection reset by peer (os error 104)"),
            io(Kind::NetworkUnreachable, "Network is unreachable (os error 101)"),
            // Windows, whose words differ (and are translated).
            io(Kind::ConnectionRefused, "No connection could be made because the target machine actively refused it. (os error 10061)"),
            io(Kind::ConnectionReset, "An existing connection was forcibly closed by the remote host. (os error 10054)"),
            io(
                Kind::TimedOut,
                "A connection attempt failed because the connected party did not properly respond after a period of time. (os error 10060)",
            ),
            io(Kind::HostUnreachable, "A socket operation was attempted to an unreachable host. (os error 10065)"),
            ureq::Error::ConnectionFailed,
            ureq::Error::Timeout(ureq::Timeout::Connect),
        ] {
            assert!(describe(&e).starts_with(CANT_CONNECT), "{e}");
            assert!(is_unreachable(&describe(&e)), "{e}");
        }
    }

    #[test]
    fn a_name_that_cant_be_looked_up_reads_the_same_on_every_system() {
        for e in [
            ureq::Error::Io(std::io::Error::other("failed to lookup address information: Name or service not known")),
            // Windows: WSAHOST_NOT_FOUND and WSATRY_AGAIN.
            ureq::Error::Io(std::io::Error::from_raw_os_error(11001)),
            ureq::Error::Io(std::io::Error::from_raw_os_error(11002)),
            ureq::Error::HostNotFound,
            ureq::Error::Timeout(ureq::Timeout::Resolve),
        ] {
            assert!(describe(&e).starts_with(CANT_FIND), "{e}");
            assert!(is_unreachable(&describe(&e)), "{e}");
        }
    }

    #[test]
    fn an_answer_that_stalls_or_breaks_is_not_an_unreachable_server() {
        for e in [
            ureq::Error::Timeout(ureq::Timeout::PerRead),
            ureq::Error::Io(std::io::Error::other(CHANGED_WHILE_UPLOADING)),
            io(std::io::ErrorKind::UnexpectedEof, "Peer disconnected"),
            ureq::Error::BadUri("x".into()),
        ] {
            assert!(!is_unreachable(&describe(&e)), "{e}");
        }
    }

    #[test]
    fn a_port_nothing_listens_on_is_unreachable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let e = execute_on(&agent(Duration::from_secs(5), FOLLOW_REDIRECTS), &HttpRequest::new(Method::Get, format!("http://127.0.0.1:{port}/")))
            .unwrap_err();
        assert!(e.starts_with(CANT_CONNECT), "{e}");
    }

    #[test]
    fn a_name_that_doesnt_exist_is_unreachable() {
        // `.invalid` never resolves (RFC 2606); without a network the lookup fails all the same.
        let e = execute_on(&agent(Duration::from_secs(5), FOLLOW_REDIRECTS), &HttpRequest::new(Method::Get, "http://craftcloud-test.invalid/"))
            .unwrap_err();
        assert!(e.starts_with(CANT_FIND), "{e}");
    }
}
