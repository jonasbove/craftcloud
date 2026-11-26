//! How screens reach the network without blocking the frame: they hand requests to a [`Backend`]
//! and read the answers on a later frame (the launcher's send/poll pattern). The desktop client
//! sends with `ureq` on a worker thread, the web frontend with `fetch`; tests use `FakeBackend`.

use craftcloud_core::http::{HttpRequest, HttpResponse};

/// A transport error (no connection, DNS, TLS…), as text for the screen.
pub type NetResult = Result<HttpResponse, String>;

/// A file the person picked or dropped on the page. The backend keeps the file itself (on the web,
/// the browser's `File`) under `key`, so its bytes go straight from disk into the upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFile {
    pub key: u64,
    pub name: String,
    pub size: u64,
    /// As the browser reports it; may be empty.
    pub mime: String,
}

/// The wall clock, for showing dates ("Today 09:10") and naming versions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallClock {
    /// Seconds since 1970-01-01 UTC.
    pub unix: f64,
    /// Local time minus UTC, in minutes (UTC+1 is 60).
    pub utc_offset_minutes: i32,
}

/// What screens need from the platform.
pub trait Backend {
    /// Starts a request; its answer comes back from [`Backend::poll`] with the same `id`.
    fn send(&mut self, id: u64, request: HttpRequest);
    /// Answers since the last call.
    fn poll(&mut self) -> Vec<(u64, NetResult)>;
    /// Opens a page in the system browser (the desktop sign-in). The web frontend navigates.
    fn open_url(&mut self, url: &str);
    /// Seconds on a monotonic clock, for polling intervals.
    fn now(&self) -> f64;

    // The web file browser's needs. The desktop client has the sync folder instead, so these
    // default to doing nothing.

    /// Opens the system's file chooser; the chosen files arrive through [`Backend::take_picked`].
    fn pick_files(&mut self) {}
    /// Files chosen since the last call.
    fn take_picked(&mut self) -> Vec<LocalFile> {
        Vec::new()
    }
    /// Keeps files dropped on the page for upload.
    fn accept_dropped(&mut self, _files: &[egui::DroppedFileHandle]) -> Vec<LocalFile> {
        Vec::new()
    }
    /// Starts `request` with the local file `file` as its body; the answer comes back from
    /// [`Backend::poll`] with the same `id`, and progress from [`Backend::poll_progress`].
    fn send_file(&mut self, id: u64, request: HttpRequest, _file: u64) {
        self.send(id, request);
    }
    /// Bytes sent so far of uploads still running: `(id, sent)`.
    fn poll_progress(&mut self) -> Vec<(u64, u64)> {
        Vec::new()
    }
    /// Lets go of a local file once it is uploaded or skipped.
    fn forget_file(&mut self, _file: u64) {}
    /// Saves what `url` answers as a file called `name` (the browser's download).
    fn download(&mut self, url: &str, _name: &str) {
        self.open_url(url);
    }
    /// Opens `url` in a new browser tab (the web frontend) or the system browser.
    fn open_tab(&mut self, url: &str) {
        self.open_url(url);
    }
    /// The wall clock, when the platform knows it.
    fn wall_clock(&self) -> Option<WallClock> {
        None
    }
    /// A choice this browser keeps for the next visit (the colour theme), `None` when it has none
    /// or keeps nothing (storage switched off or blocked).
    fn load_pref(&self, _key: &str) -> Option<String> {
        None
    }
    /// Keeps a choice for the next visit. A browser that won't keep it only loses it at the end of
    /// the visit, so a failure is not shown.
    fn save_pref(&mut self, _key: &str, _value: &str) {}
}

/// Request ids, unique across every [`Net`] in the process, so several of them can share one
/// backend (the web app's pages each keep their own).
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Request bookkeeping for one screen: which answer belongs to which question.
pub struct Net<K> {
    pending: Vec<(u64, K)>,
}

impl<K: Clone + PartialEq> Default for Net<K> {
    fn default() -> Self {
        Self { pending: Vec::new() }
    }
}

impl<K: Clone + PartialEq> Net<K> {
    pub fn send(&mut self, backend: &mut dyn Backend, key: K, request: HttpRequest) {
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.pending.push((id, key));
        backend.send(id, request);
    }

    /// Sends `request` with a local file as its body (an upload); returns the request id, which
    /// progress reports use.
    pub fn send_file(&mut self, backend: &mut dyn Backend, key: K, request: HttpRequest, file: u64) -> u64 {
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.pending.push((id, key));
        backend.send_file(id, request, file);
        id
    }

    /// Takes the answers that belong to this screen. Answers to requests a screen no longer
    /// waits for are dropped.
    pub fn receive(&mut self, answers: &mut Vec<(u64, NetResult)>) -> Vec<(K, NetResult)> {
        let mut mine = Vec::new();
        answers.retain_mut(|(id, result)| {
            let Some(pos) = self.pending.iter().position(|(p, _)| p == id) else { return true };
            let (_, key) = self.pending.remove(pos);
            mine.push((key, std::mem::replace(result, Err(String::new()))));
            false
        });
        mine
    }

    pub fn is_waiting(&self, key: &K) -> bool {
        self.pending.iter().any(|(_, k)| k == key)
    }

    /// Whether any pending request matches.
    pub fn any_waiting(&self, matches: impl Fn(&K) -> bool) -> bool {
        self.pending.iter().any(|(_, k)| matches(k))
    }

    pub fn is_busy(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// A scripted backend for tests: records what was sent, answers with what the test queues. Test-only,
/// so its lookups may panic: a missing request is a failing test.
#[cfg(test)]
#[derive(Default)]
pub struct FakeBackend {
    pub sent: Vec<(u64, HttpRequest)>,
    pub answers: Vec<(u64, NetResult)>,
    answered: Vec<u64>,
    pub opened: Vec<String>,
    pub clock: f64,
    /// What the file chooser hands over the next time it is opened.
    pub to_pick: Vec<LocalFile>,
    picked: Vec<LocalFile>,
    /// How often the file chooser was opened.
    pub picks: usize,
    /// `(request id, local file key)` of every upload.
    pub uploads: Vec<(u64, u64)>,
    /// Progress reports for the next poll.
    pub progress: Vec<(u64, u64)>,
    /// `(url, name)` of every download.
    pub downloads: Vec<(String, String)>,
    /// Pages opened in a new tab.
    pub tabs: Vec<String>,
    pub wall: Option<WallClock>,
    /// What the browser keeps between visits ([`Backend::load_pref`]).
    pub prefs: std::collections::BTreeMap<String, String>,
}

#[cfg(test)]
impl FakeBackend {
    /// Answers the oldest unanswered request whose URL ends with `url_end`.
    pub fn answer(&mut self, url_end: &str, response: NetResult) {
        let found = self.sent.iter().find(|(id, req)| req.url.ends_with(url_end) && !self.answered.contains(id)).map(|(id, _)| *id);
        let id = found.unwrap_or_else(|| panic!("nothing sent to …{url_end}; sent: {:?}", self.sent.iter().map(|(_, r)| &r.url).collect::<Vec<_>>()));
        self.answered.push(id);
        self.answers.push((id, response));
    }

    /// The last request sent to a URL ending with `url_end`.
    pub fn last_to(&self, url_end: &str) -> &HttpRequest {
        self.sent.iter().rev().find(|(_, r)| r.url.ends_with(url_end)).map(|(_, r)| r).unwrap_or_else(|| panic!("nothing sent to …{url_end}"))
    }

    pub fn sent_to(&self, url_end: &str) -> usize {
        self.sent.iter().filter(|(_, r)| r.url.ends_with(url_end)).count()
    }

    /// Whether a request to a URL ending with `url_end` still waits for its answer.
    pub fn waiting(&self, url_end: &str) -> bool {
        self.sent.iter().any(|(id, r)| r.url.ends_with(url_end) && !self.answered.contains(id))
    }
}

#[cfg(test)]
impl Backend for FakeBackend {
    fn send(&mut self, id: u64, request: HttpRequest) {
        self.sent.push((id, request));
    }

    fn poll(&mut self) -> Vec<(u64, NetResult)> {
        std::mem::take(&mut self.answers)
    }

    fn open_url(&mut self, url: &str) {
        self.opened.push(url.to_owned());
    }

    fn now(&self) -> f64 {
        self.clock
    }

    fn pick_files(&mut self) {
        self.picks += 1;
        self.picked.append(&mut self.to_pick);
    }

    fn take_picked(&mut self) -> Vec<LocalFile> {
        std::mem::take(&mut self.picked)
    }

    fn send_file(&mut self, id: u64, request: HttpRequest, file: u64) {
        self.uploads.push((id, file));
        self.sent.push((id, request));
    }

    fn poll_progress(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut self.progress)
    }

    fn download(&mut self, url: &str, name: &str) {
        self.downloads.push((url.to_owned(), name.to_owned()));
    }

    fn open_tab(&mut self, url: &str) {
        self.tabs.push(url.to_owned());
    }

    fn wall_clock(&self) -> Option<WallClock> {
        self.wall
    }

    fn load_pref(&self, key: &str) -> Option<String> {
        self.prefs.get(key).cloned()
    }

    fn save_pref(&mut self, key: &str, value: &str) {
        self.prefs.insert(key.to_owned(), value.to_owned());
    }
}
