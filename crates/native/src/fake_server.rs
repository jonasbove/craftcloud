//! A scripted HTTP server on `127.0.0.1:0` for unit tests: the real `Session` and sync engine talk
//! to it over the loopback, so the paths that need a server answer run in `cargo xtask ci`
//! without Docker. Each connection carries one request and is closed after the answer.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// One request the server got.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    /// The path with its query, `/webdav/Docs/`.
    pub path: String,
    /// Header names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The bearer token the request carried, if any.
    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization").and_then(|v| v.strip_prefix("Bearer "))
    }

    pub fn is(&self, method: &str, path_prefix: &str) -> bool {
        self.method == method && self.path.starts_with(path_prefix)
    }
}

/// What the server answers: a status and a body (JSON, or a WebDAV multistatus for 207).
pub type Reply = (u16, String);

/// A full answer: a status, extra headers (an `ETag`, a `Content-Range` …; a `Content-Type` here
/// replaces the one the status would get) and a body. Status [`Answer::DROP`] (0) closes the
/// connection without answering: a server that stops answering mid-way.
#[derive(Clone, Debug)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Answer {
    /// The status of an answer that never comes: the connection is closed instead.
    pub const DROP: u16 = 0;

    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self { status, headers: Vec::new(), body: body.into() }
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }
}

impl From<Reply> for Answer {
    fn from((status, body): Reply) -> Self {
        Self::new(status, body)
    }
}

pub struct FakeServer {
    pub url: String,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Seen>>>,
    thread: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// Starts a server that answers every request with `answer(request)`.
    pub fn start(answer: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Self {
        Self::start_full(move |r| Answer::from(answer(r)))
    }

    /// Like [`Self::start`], with answers that can carry headers.
    pub fn start_full(answer: impl Fn(&Seen) -> Answer + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let answer = Arc::new(answer);
        let thread = {
            let (stop, seen) = (Arc::clone(&stop), Arc::clone(&seen));
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(conn) = conn else { continue };
                    // One thread per connection: the engine lists several folders at once.
                    let (seen, answer) = (Arc::clone(&seen), Arc::clone(&answer));
                    std::thread::spawn(move || {
                        if let Some(request) = read_request(&conn) {
                            let Answer { status, headers, body } = answer(&request);
                            seen.lock().unwrap_or_else(PoisonError::into_inner).push(request);
                            if status == Answer::DROP {
                                // Closed without an answer (the thread's `conn` drops here).
                                return;
                            }
                            let kind = if status == 207 { "application/xml; charset=utf-8" } else { "application/json" };
                            let mut head = String::new();
                            if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-type")) {
                                head.push_str(&format!("Content-Type: {kind}\r\n"));
                            }
                            for (name, value) in &headers {
                                head.push_str(&format!("{name}: {value}\r\n"));
                            }
                            let mut conn = conn;
                            let _ = write!(conn, "HTTP/1.1 {status} X\r\n{head}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        }
                    });
                }
            })
        };
        Self { url, stop, seen, thread: Some(thread) }
    }

    pub fn server(&self) -> craftcloud_core::ServerUrl {
        craftcloud_core::ServerUrl::parse(&self.url).unwrap()
    }

    /// The requests answered so far, in the order they were answered.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// How many requests answered so far match `method` and start with `path_prefix`.
    pub fn count(&self, method: &str, path_prefix: &str) -> usize {
        self.seen().iter().filter(|s| s.is(method, path_prefix)).count()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(conn: &TcpStream) -> Option<Seen> {
    let mut reader = BufReader::new(conn);
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next()?.to_owned(), parts.next()?.to_owned());
    let mut headers = Vec::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_owned());
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }
    let chunked = headers.iter().any(|(n, v)| n == "transfer-encoding" && v.eq_ignore_ascii_case("chunked"));
    let body = if chunked {
        // ureq sends a request without a body (a POST that triggers a job) chunked. Read it to the
        // end: closing the connection with its last chunk unread resets it, and the client then
        // sometimes loses the answer ("Peer disconnected"), which made tests flaky.
        read_chunked(&mut reader)?
    } else {
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        body
    };
    Some(Seen { method, path, headers, body: String::from_utf8_lossy(&body).into_owned() })
}

/// A chunked body, up to and including its last (empty) chunk and the trailer's end.
/// The largest chunk a test request may send (16 MiB).
const MAX_CHUNK: usize = 16 << 20;

fn read_chunked(reader: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        reader.read_line(&mut size).ok()?;
        let size = usize::from_str_radix(size.trim().split(';').next()?.trim(), 16).ok()?;
        if size == 0 {
            loop {
                let mut trailer = String::new();
                if reader.read_line(&mut trailer).ok()? == 0 || trailer.trim().is_empty() {
                    return Some(body);
                }
            }
        }
        // A test server: a chunk bigger than any test sends is a broken request, not an
        // allocation to try (and `size + 2` can't overflow below the cap).
        if size > MAX_CHUNK {
            return None;
        }
        let mut chunk = vec![0; size + 2];
        reader.read_exact(&mut chunk).ok()?;
        chunk.truncate(size);
        body.extend(chunk);
    }
}

/// A WebDAV `PROPFIND` answer (207) for `folder` holding `files` (name, ETag; `None` for an entry
/// without one) and `subfolders`.
pub fn listing(folder: &str, files: &[(&str, Option<&str>)], subfolders: &[&str]) -> Reply {
    let base = if folder.is_empty() { "/webdav/".to_owned() } else { format!("/webdav/{folder}/") };
    let mut xml = String::from(r#"<D:multistatus xmlns:D="DAV:">"#);
    let folder_entry = |href: &str| {
        format!(
            "<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype>\
             <D:getetag>&quot;folder&quot;</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
        )
    };
    xml.push_str(&folder_entry(&base));
    for sub in subfolders {
        xml.push_str(&folder_entry(&format!("{base}{sub}/")));
    }
    for (name, etag) in files {
        let etag = etag.map(|e| format!("<D:getetag>&quot;{e}&quot;</D:getetag>")).unwrap_or_default();
        xml.push_str(&format!(
            "<D:response><D:href>{base}{name}</D:href><D:propstat><D:prop><D:resourcetype/>{etag}\
             <D:getcontentlength>3</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
        ));
    }
    xml.push_str("</D:multistatus>");
    (207, xml)
}

#[cfg(test)]
mod tests {
    use craftcloud_core::http::{HttpRequest, Method};

    use super::{FakeServer, read_chunked};

    /// #97: a chunk size no test sends (up to `usize::MAX`, where `size + 2` overflowed) is a
    /// broken request, read as none, never a panic or a huge allocation.
    #[test]
    fn an_absurd_chunk_size_is_refused() {
        for size in ["ffffffffffffffff", "fffffffffffffffe", "7fffffffffffffff", "1000001"] {
            let mut body = std::io::Cursor::new(format!("{size}\r\nabc\r\n0\r\n\r\n").into_bytes());
            assert_eq!(read_chunked(&mut body), None, "{size}");
        }
        let mut body = std::io::Cursor::new(b"3\r\nabc\r\n0\r\n\r\n".to_vec());
        assert_eq!(read_chunked(&mut body), Some(b"abc".to_vec()));
    }

    #[test]
    fn a_post_without_a_body_is_always_answered() {
        // ureq sends it chunked; with its last chunk left unread the connection was reset and the
        // answer lost now and then (about 1 in 300), which made the golden no-room test flaky.
        let server = FakeServer::start(|_| (200, "{}".into()));
        for n in 0..500 {
            let req = HttpRequest::new(Method::Post, format!("{}/api/admin/jobs/x/trigger", server.url));
            let answer = crate::http::execute(&req);
            assert_eq!(answer.map(|r| r.status), Ok(200), "request {n}");
        }
        let seen = server.seen();
        assert_eq!(seen[0].header("transfer-encoding"), Some("chunked"), "the case this covers");
        assert!(seen.iter().all(|r| r.body.is_empty()));
    }
}
