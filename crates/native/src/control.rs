//! The running desktop client's control channel, for `craftcloud status|sync|pause|resume`, for
//! `craftcloud://` links opened while a client runs (the link is handed over, see [`send_link`]),
//! and for `craftcloud files` (the synced files here, from the client's local view, see [`files`]).
//!
//! The client listens on a loopback TCP port (any free one) and writes the port and a random
//! secret to `control.json` in its settings folder, readable only by the user. A command connects,
//! sends one JSON line `{"secret": …, "request": …}` and reads one JSON line back. TCP on
//! 127.0.0.1 works the same on Linux, macOS and Windows with the standard library alone; the
//! secret keeps other local users out (they can't read the file).

use std::hash::BuildHasher;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craftcloud_core::sync::{ActivityEvent, ActivityKind, LeftOut, SyncPlan, SyncProgress, SyncState};
use serde::{Deserialize, Serialize};

use crate::local_files::{LocalView, SyncedFile, by_extension};
use crate::open::OpenError;
use crate::sync::{Command, Commands, Progress};

const FILE: &str = "control.json";
/// How long `sync --wait` waits for the cycle.
const SYNC_WAIT: Duration = Duration::from_secs(30 * 60);

/// What a command asks of the client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// Sync now; with `wait`, answer once that sync has finished.
    Sync {
        wait: bool,
    },
    Pause,
    Resume,
    /// Carry out the deletions a mass-delete hold is waiting on, then answer once that sync is done.
    ConfirmDeletes,
    /// Bring back the files a mass-delete hold would delete instead, then answer once that sync is done.
    DiscardDeletes,
    /// Act on a `craftcloud://` link (the running client checks it and opens the file).
    Link {
        url: String,
    },
    /// The synced files present in the local folder with one of these extensions (lower case;
    /// empty: all files), with their sync state. Answered from the disk, no network.
    Files {
        #[serde(default)]
        extensions: Vec<String>,
    },
}

/// The client's state, as `craftcloud status --json` prints it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Whether the client is signed in and syncing (or paused); `false` while it waits for a sign-in.
    pub syncing: bool,
    pub server: String,
    pub local_folder: String,
    /// One of [`state_name`]'s words.
    pub state: String,
    /// What went wrong, for `failed` and `offline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// For `mass_delete`: the files whose deletion waits for a decision (`sync
    /// --confirm-deletes`, `sync --discard-deletes`, or the desktop client).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_deletes: Vec<String>,
    pub files_done: u64,
    pub files_total: u64,
    /// Finished sync cycles since the client started.
    pub cycles: u64,
    /// When a cycle last finished without an error (seconds since 1970).
    #[serde(default)]
    pub last_synced: Option<i64>,
    /// Conflict copies waiting for a decision (cloud paths).
    pub conflicts: Vec<String>,
    /// Local places in synced folders that the last sync left out, and why; the activity says
    /// more. Doesn't change `state`, but `status` exits 9 while one can't be read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_synced: Vec<NotSynced>,
    /// Files that don't fit in the cloud (its quota is full): tried again once there is room.
    /// Doesn't change `state` or the exit code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_room: Vec<String>,
    /// Files whose last transfer failed (tried again every pass; the activity says why).
    /// Doesn't change `state` or the exit code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
    /// Files the last pass held because they were written moments before (an app may still be
    /// saving them); the next pass syncs them. Doesn't change `state` or the exit code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<String>,
    /// The latest activity, oldest first.
    pub activity: Vec<Activity>,
}

/// A place the sync left out, in `--json` output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "NotSyncedWire")]
pub struct NotSynced {
    /// Cloud path.
    pub path: String,
    /// `unreadable` (needs attention: `status` exits 9), `symlink` (never followed) or `name`
    /// (the cloud can't take it; it syncs once renamed).
    pub reason: String,
}

/// What a client may answer: a bare path from one started before reasons were said (read as
/// `unreadable`, the cautious reading), or `{path, reason}`.
#[derive(Deserialize)]
#[serde(untagged)]
enum NotSyncedWire {
    Path(String),
    Full { path: String, reason: String },
}

impl From<NotSyncedWire> for NotSynced {
    fn from(w: NotSyncedWire) -> Self {
        match w {
            NotSyncedWire::Path(path) => Self { path, reason: LeftOut::Unreadable.as_str().to_owned() },
            NotSyncedWire::Full { path, reason } => Self { path, reason },
        }
    }
}

impl NotSynced {
    /// Whether it can't be read: unlike a symbolic link or a name, that needs the person.
    pub fn unreadable(&self) -> bool {
        self.reason == LeftOut::Unreadable.as_str()
    }
}

/// One thing the sync did, in `--json` output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// Seconds since 1970 (UTC).
    pub at: i64,
    /// `up`, `down`, `deleted`, `conflict`, `error` or `skipped` (left out on purpose).
    pub kind: String,
    /// The cloud path.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// A few words more ("to the cloud trash", "kept both", an error); may be empty.
    #[serde(default)]
    pub detail: String,
}

impl From<&ActivityEvent> for Activity {
    fn from(e: &ActivityEvent) -> Self {
        let kind = match e.kind {
            ActivityKind::Up => "up",
            ActivityKind::Down => "down",
            ActivityKind::Deleted => "deleted",
            ActivityKind::Conflict => "conflict",
            ActivityKind::Error => "error",
            ActivityKind::Skipped => "skipped",
            ActivityKind::Warning => "warning",
            ActivityKind::Moved => "moved",
        };
        Self { at: e.at, kind: kind.to_owned(), path: e.path.clone(), size: e.size, detail: e.detail.clone() }
    }
}

impl Activity {
    /// "Uploaded Documents/notes.md", "Deleted Old/a.txt (to the cloud trash)".
    pub fn line(&self) -> String {
        let verb = match self.kind.as_str() {
            "up" => "Uploaded",
            "down" => "Downloaded",
            "deleted" => "Deleted",
            "conflict" => "Conflict on",
            "skipped" => "Skipped",
            "warning" => "Warning about",
            "moved" => "Moved",
            _ => "Couldn't sync",
        };
        if self.path.is_empty() {
            // About the whole sync, not one place: the detail says it all (T-051).
            let mut chars = self.detail.chars();
            return chars.next().map_or_else(|| verb.to_owned(), |c| c.to_uppercase().chain(chars).collect());
        }
        if self.detail.is_empty() { format!("{verb} {}", self.path) } else { format!("{verb} {} ({})", self.path, self.detail) }
    }
}

/// `SyncState` as one word: `idle`, `scanning`, `downloading`, `up_to_date`, `paused`, `offline`,
/// `signed_out`, `mass_delete`, `folder_missing` or `failed` (and `signed_out` when the client
/// waits for a sign-in).
pub fn state_name(state: &SyncState) -> &'static str {
    match state {
        SyncState::Idle => "idle",
        SyncState::Scanning => "scanning",
        SyncState::Downloading => "downloading",
        SyncState::UpToDate => "up_to_date",
        SyncState::Paused => "paused",
        SyncState::Offline(_) => "offline",
        SyncState::SignedOut => "signed_out",
        SyncState::MassDelete { .. } => "mass_delete",
        SyncState::FolderMissing => "folder_missing",
        SyncState::Failed(_) => "failed",
    }
}

impl Status {
    /// Builds the status from a sync's progress (`activity` keeps the last 10 events).
    pub fn from_progress(p: &SyncProgress, server: &str, local_folder: &str) -> Self {
        let skip = p.activity.len().saturating_sub(10);
        Self {
            syncing: true,
            server: server.to_owned(),
            local_folder: local_folder.to_owned(),
            state: state_name(&p.state).to_owned(),
            error: match &p.state {
                SyncState::Failed(e) | SyncState::Offline(e) => Some(e.clone()),
                _ => None,
            },
            pending_deletes: match p.state {
                SyncState::MassDelete { .. } => p.pending_deletes.clone(),
                _ => Vec::new(),
            },
            files_done: p.files_done,
            files_total: p.files_total,
            cycles: p.cycles,
            last_synced: p.last_synced,
            conflicts: p.conflicts.iter().map(|c| c.copy.clone()).collect(),
            not_synced: p
                .not_synced
                .iter()
                .map(|path| NotSynced {
                    path: path.clone(),
                    reason: p.left_out.get(path).copied().unwrap_or(LeftOut::Unreadable).as_str().to_owned(),
                })
                .collect(),
            no_room: p.no_room.clone(),
            failed: p.unsynced.iter().filter(|(_, s)| **s == craftcloud_core::sync::FileStatus::Error).map(|(path, _)| path.clone()).collect(),
            waiting: p.waiting.clone(),
            activity: p.activity[skip..].iter().map(Activity::from).collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    secret: String,
    request: Request,
}

#[derive(Debug, Serialize, Deserialize)]
struct Answer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// For a link: the local path that was opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opened: Option<String>,
    /// For a link: why it wasn't opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refused: Option<OpenError>,
    /// For `files`: what is in the local folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    files: Option<Vec<SyncedFile>>,
}

impl Answer {
    fn error(e: impl Into<String>) -> Self {
        Self { status: None, error: Some(e.into()), opened: None, refused: None, files: None }
    }

    fn status(status: Status) -> Self {
        Self { status: Some(status), error: None, opened: None, refused: None, files: None }
    }
}

#[derive(Serialize, Deserialize)]
struct Address {
    port: u16,
    secret: String,
    pid: u32,
}

/// The sync the client is running, if any; the client updates it as it signs in and out.
#[derive(Clone)]
pub struct Attached {
    pub progress: Progress,
    pub commands: Commands,
    pub server: String,
    pub local_folder: String,
    /// What syncs, for checking `craftcloud://` links.
    pub plan: SyncPlan,
}

pub type Slot = Arc<Mutex<Option<Attached>>>;

/// Keeps the channel open; dropping it removes `control.json` (when it is still ours).
pub struct Server {
    file: PathBuf,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        if read_address(&self.file).is_some_and(|a| a.port == self.port) {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

/// Starts listening for commands; `dir` is the settings folder.
pub fn serve(dir: &Path, slot: Slot) -> Result<Server, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("can't open the control channel: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let secret = new_secret();
    let file = dir.join(FILE);
    let address = serde_json::to_vec(&Address { port, secret: secret.clone(), pid: std::process::id() }).map_err(|e| e.to_string())?;
    crate::secrets::write_private(&file, &address)?;
    let tickets = Arc::new(AtomicU64::new(1 << 32));
    std::thread::Builder::new()
        .name("craftcloud-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (slot, secret, tickets) = (Arc::clone(&slot), secret.clone(), Arc::clone(&tickets));
                let _ = std::thread::Builder::new().name("craftcloud-control-conn".into()).spawn(move || answer(stream, &slot, &secret, &tickets));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Server { file, port })
}

fn new_secret() -> String {
    // RandomState is seeded from the OS's random source; four draws give 256 bits.
    (0..4)
        .map(|i| std::collections::hash_map::RandomState::new().hash_one(i as u64 ^ std::process::id() as u64))
        .map(|n| format!("{n:016x}"))
        .collect()
}

fn answer(stream: TcpStream, slot: &Slot, secret: &str, tickets: &AtomicU64) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    let Ok(read) = stream.try_clone() else { return };
    if BufReader::new(read.take_limited()).read_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Envelope>(&line) {
        Ok(env) if same(&env.secret, secret) => handle(&env.request, slot, tickets),
        Ok(_) => Answer::error("refused: wrong secret"),
        Err(e) => Answer::error(format!("unreadable request: {e}")),
    };
    let mut out = stream;
    if let Ok(mut bytes) = serde_json::to_vec(&reply) {
        bytes.push(b'\n');
        let _ = out.write_all(&bytes);
    }
}

trait TakeLimited: std::io::Read + Sized {
    fn take_limited(self) -> std::io::Take<Self> {
        self.take(64 * 1024)
    }
}

impl<R: std::io::Read> TakeLimited for R {}

/// Compares secrets without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn handle(request: &Request, slot: &Slot, tickets: &AtomicU64) -> Answer {
    let attached = slot.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Request::Link { url } = request {
        // Not syncing: the sender handles the link itself.
        let Some(a) = attached else { return Answer::error(NOT_SYNCING) };
        return match crate::open::follow(&a.plan, url) {
            Ok(path) => Answer { opened: Some(path.display().to_string()), ..Answer::status(Status::default()) },
            Err(e) => Answer { refused: Some(e), ..Answer::status(Status::default()) },
        };
    }
    if let Request::Files { extensions } = request {
        let Some(a) = attached else { return Answer::error(NOT_SYNCING) };
        let unsynced = a.progress.lock().unwrap_or_else(PoisonError::into_inner).unsynced.clone();
        let Some(view) = LocalView::new(&a.plan, unsynced) else { return Answer::error("no sync folder is set up") };
        return Answer { status: None, error: None, opened: None, refused: None, files: Some(view.files(by_extension(extensions))) };
    }
    let Some(a) = attached else {
        return Answer::status(Status { state: "signed_out".into(), ..Status::default() });
    };
    let snapshot = |a: &Attached| Status::from_progress(&a.progress.lock().unwrap_or_else(PoisonError::into_inner), &a.server, &a.local_folder);
    match request {
        // (Links and file lists are answered above.)
        Request::Status | Request::Link { .. } | Request::Files { .. } => {}
        Request::Pause | Request::Resume => {
            let _ = a.commands.send(Command::Pause(matches!(request, Request::Pause)));
            let _ = a.commands.send(Command::SyncNow);
            // Let the sync thread take the command, so the answer shows the new state.
            wait_for(Duration::from_secs(5), || {
                let paused = matches!(a.progress.lock().unwrap_or_else(PoisonError::into_inner).state, SyncState::Paused);
                paused == matches!(request, Request::Pause)
            });
        }
        Request::Sync { .. } | Request::ConfirmDeletes | Request::DiscardDeletes => {
            let wait = !matches!(request, Request::Sync { wait: false });
            let decided = match request {
                Request::ConfirmDeletes => a.commands.send(Command::ConfirmDeletes).is_ok(),
                Request::DiscardDeletes => a.commands.send(Command::DiscardDeletes).is_ok(),
                _ => true,
            };
            let served = a.progress.lock().unwrap_or_else(PoisonError::into_inner).served;
            let ticket = tickets.fetch_add(1, Ordering::SeqCst).max(served + 1);
            if !decided || a.commands.send(Command::SyncFor(ticket)).is_err() {
                return Answer::error("the sync has stopped");
            }
            if wait && !wait_for(SYNC_WAIT, || a.progress.lock().unwrap_or_else(PoisonError::into_inner).served >= ticket) {
                return Answer { error: Some("the sync is still running".into()), ..Answer::status(snapshot(&a)) };
            }
        }
    }
    Answer::status(snapshot(&a))
}

const NOT_SYNCING: &str = "the client isn't syncing";

fn wait_for(limit: Duration, done: impl Fn() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    done()
}

fn read_address(file: &Path) -> Option<Address> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

/// Proof that this process is the one syncing a folder; released when dropped (or when the
/// process ends, however it ends). Empty while the folder is missing (nothing syncs then).
pub struct FolderLock(Option<std::fs::File>);

impl FolderLock {
    /// Whether nothing is locked (the folder was missing when claimed): claim again once it's back.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }
}

impl Drop for FolderLock {
    fn drop(&mut self) {
        if let Some(file) = &self.0 {
            let _ = file.unlock();
        }
    }
}

/// Claims a sync folder for this process's sync engine: one engine per folder, whether it runs in
/// the desktop client, `craftcloud daemon` or `craftcloud sync --once`. The lock is
/// `<folder>/.craftcloud/engine.lock`, which the sync never uploads.
///
/// A folder that isn't there is not made (it may be on a drive that isn't plugged in): the claim
/// is empty, and the sync holds as "folder missing" until it is back, then the host claims again.
pub fn claim_folder(local_folder: &Path) -> Result<FolderLock, String> {
    if !local_folder.is_dir() {
        return Ok(FolderLock(None));
    }
    let dir = local_folder.join(craftcloud_core::api::dav::CRAFTCLOUD_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't use {}: {e}", dir.display()))?;
    let path = dir.join("engine.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("can't open {}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(FolderLock(Some(file))),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err(format!("Another CraftCloud (the desktop client or `craftcloud daemon`) is already syncing {}.", local_folder.display()))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(format!("can't lock {}: {e}", path.display())),
    }
}

/// Whether a desktop client or daemon answers on this settings folder's control channel.
pub fn is_running(dir: &Path) -> bool {
    send(dir, &Request::Status).is_ok()
}

/// Why a command couldn't be delivered.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// No desktop client is running (no control file, or nobody listening on it).
    NotRunning,
    /// It answered with an error.
    Failed(String),
}

/// Sends one request to the running client and returns its status.
pub fn send(dir: &Path, request: &Request) -> Result<Status, SendError> {
    let answer = exchange(dir, request)?;
    match (answer.error, answer.status) {
        (Some(e), _) => Err(SendError::Failed(e)),
        (None, Some(s)) => Ok(s),
        (None, None) => Err(SendError::Failed("empty answer".into())),
    }
}

/// What came of handing a link to the running client.
#[derive(Debug, PartialEq, Eq)]
pub enum LinkHandoff {
    /// It opened this local path.
    Opened(String),
    /// It checked the link and refused it (or the system couldn't open the file).
    Refused(OpenError),
    /// Nobody took it: no client runs, it isn't signed in, or it is too old to know links. The
    /// caller handles the link itself.
    NotTaken,
}

/// Hands a `craftcloud://` link to the running desktop client or daemon (one instance acts on
/// links; a second start passes them on and exits).
pub fn send_link(dir: &Path, url: &str) -> LinkHandoff {
    match exchange(dir, &Request::Link { url: url.to_owned() }) {
        Ok(Answer { refused: Some(e), .. }) => LinkHandoff::Refused(e),
        Ok(Answer { opened: Some(path), error: None, .. }) => LinkHandoff::Opened(path),
        _ => LinkHandoff::NotTaken,
    }
}

/// The synced files in the running client's local folder with one of these extensions (any case,
/// with or without the dot; empty: all), from its local view: no server round trip.
/// [`SendError::NotRunning`] when no client runs; [`SendError::Failed`] when it isn't signed in or
/// is too old to know the request (callers then ask the server instead).
pub fn files(dir: &Path, extensions: &[String]) -> Result<Vec<SyncedFile>, SendError> {
    let extensions = extensions.iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect();
    let answer = exchange(dir, &Request::Files { extensions })?;
    match (answer.error, answer.files) {
        (Some(e), _) => Err(SendError::Failed(e)),
        (None, Some(f)) => Ok(f),
        (None, None) => Err(SendError::Failed("this desktop client doesn't list files (update it)".into())),
    }
}

fn exchange(dir: &Path, request: &Request) -> Result<Answer, SendError> {
    let address = read_address(&dir.join(FILE)).ok_or(SendError::NotRunning)?;
    let to = SocketAddr::from(([127, 0, 0, 1], address.port));
    let mut stream = TcpStream::connect_timeout(&to, Duration::from_secs(2)).map_err(|_| SendError::NotRunning)?;
    let wait = matches!(request, Request::Sync { wait: true } | Request::ConfirmDeletes | Request::DiscardDeletes);
    let _ = stream.set_read_timeout(Some(if wait { SYNC_WAIT + Duration::from_secs(30) } else { Duration::from_secs(30) }));
    let mut bytes =
        serde_json::to_vec(&Envelope { secret: address.secret, request: request.clone() }).map_err(|e| SendError::Failed(e.to_string()))?;
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(|_| SendError::NotRunning)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| SendError::Failed(format!("no answer from the desktop client: {e}")))?;
    if line.trim().is_empty() {
        return Err(SendError::NotRunning);
    }
    serde_json::from_str(&line).map_err(|e| SendError::Failed(format!("unreadable answer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::{Attached, LinkHandoff, Request, SendError, Slot, files, send, send_link, serve};
    use crate::open::OpenError;
    use crate::sync::Command;
    use craftcloud_core::sync::{FileStatus, SyncPlan, SyncProgress, SyncState};
    use std::sync::{Arc, Mutex};

    /// T-051: a line about the whole sync (no path) is its detail, not "Skipped  (…)".
    #[test]
    fn activity_lines_read_as_sentences() {
        let line = |kind: &str, path: &str, detail: &str| {
            super::Activity { at: 0, kind: kind.into(), path: path.into(), size: None, detail: detail.into() }.line()
        };
        assert_eq!(
            line("skipped", "", "kept 10 files instead of deleting them (10 put back from the cloud's trash)"),
            "Kept 10 files instead of deleting them (10 put back from the cloud's trash)"
        );
        assert_eq!(
            line("deleted", "t14/many", "folder with 3000 files, to the cloud trash"),
            "Deleted t14/many (folder with 3000 files, to the cloud trash)"
        );
        assert_eq!(line("up", "a.txt", ""), "Uploaded a.txt");
    }

    #[test]
    fn a_link_is_handed_to_the_running_client() {
        let dir = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("Private")).unwrap();
        let slot: Slot = Arc::new(Mutex::new(None));
        let _server = serve(dir.path(), Arc::clone(&slot)).unwrap();
        let link = "craftcloud://open?path=/Documents/Q3%20report.docx";
        assert_eq!(send_link(dir.path(), link), LinkHandoff::NotTaken, "not signed in: the sender handles it");

        let (commands, _inbox) = std::sync::mpsc::channel();
        let local_folder = folder.path().display().to_string();
        let plan = SyncPlan { local_folder: local_folder.clone(), excluded: vec!["Private".into()], ..SyncPlan::default() };
        *slot.lock().unwrap() =
            Some(Attached { progress: Arc::default(), commands: commands.into(), server: "https://cloud.example.org".into(), local_folder, plan });
        // The running client checks the link itself (nothing here gets as far as opening a file).
        assert!(matches!(send_link(dir.path(), link), LinkHandoff::Refused(OpenError::NotHere(m)) if m.contains("Documents/Q3 report.docx")));
        assert!(matches!(send_link(dir.path(), "craftcloud://open?path=Private/x"), LinkHandoff::Refused(OpenError::NotSynced(_))));
        assert!(matches!(send_link(dir.path(), "craftcloud://open?path=../x"), LinkHandoff::Refused(OpenError::BadLink(_))));

        let empty = tempfile::tempdir().unwrap();
        assert_eq!(send_link(empty.path(), link), LinkHandoff::NotTaken, "no client running");
    }

    #[test]
    fn files_come_from_the_running_clients_local_view() {
        let dir = tempfile::tempdir().unwrap();
        let slot: Slot = Arc::new(Mutex::new(None));
        let _server = serve(dir.path(), Arc::clone(&slot)).unwrap();
        assert!(matches!(files(dir.path(), &[]), Err(SendError::Failed(e)) if e.contains("isn't syncing")), "signed out: ask the server");
        assert_eq!(files(tempfile::tempdir().unwrap().path(), &[]), Err(SendError::NotRunning));

        let (folder, plan) = crate::local_files::tests::folder();
        let progress = Arc::new(Mutex::new(SyncProgress::default()));
        progress.lock().unwrap().unsynced.insert("Documents/Sheets/budget.xlsx".into(), FileStatus::Syncing);
        let (commands, _inbox) = std::sync::mpsc::channel();
        let local_folder = folder.path().display().to_string();
        *slot.lock().unwrap() =
            Some(Attached { progress, commands: commands.into(), server: "https://cloud.example.org".into(), local_folder, plan });

        let word: Vec<String> = craftcloud_core::catalog::app("wordcraft").unwrap().opens.iter().map(|e| (*e).to_owned()).collect();
        let found = files(dir.path(), &word).unwrap();
        let paths: Vec<&str> = found.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["Documents/Notes.MD", "Documents/Q3 report.docx"], "not excluded, internal or temporary files");
        assert!(found.iter().all(|f| f.sync == "pending" && std::path::Path::new(&f.local_path).is_file()), "never synced yet");

        let sheets = files(dir.path(), &[".XLSX".to_owned()]).unwrap();
        assert_eq!((sheets.len(), sheets[0].sync.as_str()), (1, "syncing"), "the client's own word for a moving file");
        assert_eq!(files(dir.path(), &[]).unwrap().len(), 3);

        // The request on the wire, as an integrator would send it.
        let wire = serde_json::to_value(Request::Files { extensions: vec!["md".into()] }).unwrap();
        assert_eq!(wire, serde_json::json!({"cmd": "files", "extensions": ["md"]}));
        assert_eq!(serde_json::from_str::<Request>(r#"{"cmd":"files"}"#).unwrap(), Request::Files { extensions: vec![] });
    }

    #[test]
    fn no_client_means_not_running() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(send(dir.path(), &Request::Status), Err(SendError::NotRunning));
        std::fs::write(dir.path().join("control.json"), r#"{"port":1,"secret":"x","pid":1}"#).unwrap();
        assert_eq!(send(dir.path(), &Request::Status), Err(SendError::NotRunning), "a stale file");
    }

    #[test]
    fn commands_reach_the_sync() {
        let dir = tempfile::tempdir().unwrap();
        let slot: Slot = Arc::new(Mutex::new(None));
        let server = serve(dir.path(), Arc::clone(&slot)).unwrap();
        assert_eq!(send(dir.path(), &Request::Status).unwrap().state, "signed_out");

        // A stand-in sync thread that serves tickets, pauses, and holds 2 deletes until told.
        let progress = Arc::new(Mutex::new(SyncProgress {
            state: SyncState::MassDelete { deletes: 2 },
            pending_deletes: vec!["Old/a.txt".into(), "Old/b.txt".into()],
            ..SyncProgress::default()
        }));
        let (commands, inbox) = std::sync::mpsc::channel();
        let p = Arc::clone(&progress);
        std::thread::spawn(move || {
            for c in inbox {
                let mut p = p.lock().unwrap();
                match c {
                    Command::SyncFor(n) => {
                        p.cycles += 1;
                        p.served = n;
                    }
                    Command::Pause(on) => p.state = if on { SyncState::Paused } else { SyncState::UpToDate },
                    Command::ConfirmDeletes => {
                        p.state = SyncState::UpToDate;
                        p.pending_deletes.clear();
                    }
                    _ => {}
                }
            }
        });
        *slot.lock().unwrap() = Some(Attached {
            progress,
            commands: commands.into(),
            server: "https://cloud.example.org".into(),
            local_folder: "/home/john/CraftCloud".into(),
            plan: SyncPlan::default(),
        });

        // Held: the status names the files, and a confirmation reaches the sync.
        let s = send(dir.path(), &Request::Status).unwrap();
        assert_eq!((s.state.as_str(), s.pending_deletes.as_slice()), ("mass_delete", ["Old/a.txt".to_owned(), "Old/b.txt".to_owned()].as_slice()));
        let s = send(dir.path(), &Request::ConfirmDeletes).unwrap();
        assert_eq!((s.state.as_str(), s.pending_deletes.len()), ("up_to_date", 0));

        let s = send(dir.path(), &Request::Sync { wait: true }).unwrap();
        assert_eq!((s.cycles, s.state.as_str(), s.server.as_str()), (2, "up_to_date", "https://cloud.example.org"));
        assert_eq!(send(dir.path(), &Request::Pause).unwrap().state, "paused");
        assert_eq!(send(dir.path(), &Request::Resume).unwrap().state, "up_to_date");

        drop(server);
        assert!(!dir.path().join("control.json").exists(), "the file goes with the client");
    }

    #[test]
    fn one_engine_per_folder() {
        let dir = tempfile::tempdir().unwrap();
        let first = super::claim_folder(dir.path()).unwrap();
        let second = super::claim_folder(dir.path()).err().unwrap();
        assert!(second.contains("already syncing"), "{second}");
        drop(first);
        assert!(super::claim_folder(dir.path()).is_ok(), "free again once the first lets go");
    }

    #[test]
    fn the_secret_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let _server = serve(dir.path(), Arc::new(Mutex::new(None))).unwrap();
        let file = dir.path().join("control.json");
        let mut address: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(address["secret"].as_str().unwrap().len(), 64);
        address["secret"] = "0".repeat(64).into();
        std::fs::write(&file, address.to_string()).unwrap();
        assert!(matches!(send(dir.path(), &Request::Status), Err(SendError::Failed(e)) if e.contains("wrong secret")));
    }
}
