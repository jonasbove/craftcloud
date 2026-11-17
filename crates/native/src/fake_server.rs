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
/// replaces the one the status would get) and a body.
#[derive(Clone, Debug)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Answer {
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
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Seen { method, path, headers, body: String::from_utf8_lossy(&body).into_owned() })
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
