//! The engine's guards against losing a file, driven without a real server: the engine is built
//! here (no thread, no folder watcher) and talks to `crate::fake_server` over the loopback.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::Tokens;
use craftcloud_core::names::Platform;
use craftcloud_core::reconcile::{Action, Known, State, Tree};
use craftcloud_core::storage::{Capabilities, Guard};
use craftcloud_core::sync::{ActivityEvent, ActivityKind, SyncPlan, SyncState};

use super::{Deletes, Engine, Saved, local_meta, save_saved, synced_state};
use crate::fake_server::{FakeServer, Reply, Seen, listing};
use crate::session::Session;
use crate::storage::oxicloud::OxiCloud;
use crate::storage::{Download, Entry, MoveGuard, Moved, OwnPath, RemoteStorage, StorageError, TrashGuard, TrashItem, Trashed, Written};

fn plan(server: &str, root: &Path, excluded: &[&str]) -> SyncPlan {
    SyncPlan {
        server: server.to_owned(),
        local_folder: root.to_string_lossy().into_owned(),
        excluded: excluded.iter().map(|s| (*s).to_owned()).collect(),
        folders: Vec::new(),
        sync_root_files: true,
    }
}

/// An engine on `server` syncing `root`, as `sync::start_with` builds it but on this thread.
pub(super) fn engine(server: ServerUrl, root: &Path, excluded: &[&str]) -> Engine {
    let session = Session::new(server.clone(), Tokens { access_token: "a1".into(), refresh_token: "r1".into() }, Box::new(|_| {}));
    Engine {
        plan: plan(server.as_str(), root, excluded),
        cancel: Arc::clone(&session.cancel),
        storage: OxiCloud::new(session),
        device: "Laptop".into(),
        progress: Arc::default(),
        waker: Arc::new(|| {}),
        paused: false,
        settled: BTreeSet::new(),
        reported: BTreeSet::new(),
        no_room: BTreeMap::new(),
        refused: BTreeMap::new(),
        no_folder_move: BTreeSet::new(),
        move_tries: BTreeMap::new(),
        now: test_now,
        platform: Platform::current(),
        parts_swept: None,
        two_way_off: false,
        _watcher: None,
    }
}

thread_local! {
    /// How far this thread's test clock is ahead ([`test_now`]).
    static AHEAD: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// The engines' clock in these tests: the real one, moved on by [`time_passes`] (no test sleeps).
pub(super) fn test_now() -> std::time::Instant {
    std::time::Instant::now() + AHEAD.get()
}

/// Moves this thread's test clock on.
pub(super) fn time_passes(by: Duration) {
    AHEAD.set(AHEAD.get() + by);
}

fn activity(engine: &Engine) -> Vec<ActivityEvent> {
    engine.progress.lock().unwrap_or_else(PoisonError::into_inner).activity.clone()
}

fn write(root: &Path, path: &str, bytes: &str) -> std::path::PathBuf {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, bytes).unwrap();
    file
}

/// How the last sync left `file`: its size and time as they are now.
fn as_synced(file: &Path) -> Known {
    let meta = local_meta(file).unwrap();
    Known { etag: "\"e1\"".into(), size: meta.size, mtime: meta.mtime, hash: None }
}

// --- Switching a folder off (selective sync) -------------------------------------------------

#[test]
fn switching_a_folder_off_keeps_the_files_changed_since_the_last_sync() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut state = State::default();
    // Exactly as last synced: removed here (the cloud keeps it).
    let same = write(root, "Docs/same.txt", "abc");
    state.files.insert("Docs/same.txt".into(), as_synced(&same));
    // Grown since.
    let grown = write(root, "Docs/grown.txt", "abc");
    state.files.insert("Docs/grown.txt".into(), as_synced(&grown));
    std::fs::write(&grown, "abcd").unwrap();
    // The same size, saved again later: only the time tells.
    let touched = write(root, "Docs/touched.txt", "abc");
    let known = as_synced(&touched);
    state.files.insert("Docs/touched.txt".into(), known.clone());
    let later = std::time::UNIX_EPOCH + Duration::from_millis(u64::try_from(known.mtime).unwrap() + 5_000);
    std::fs::File::options().write(true).open(&touched).unwrap().set_modified(later).unwrap();
    // Never synced, and synced but deleted here since.
    let new = write(root, "Docs/new.txt", "mine");
    state.files.insert("Docs/deleted.txt".into(), Known { etag: "\"e1\"".into(), size: 3, mtime: 1, hash: None });
    // A folder that stays in the plan.
    let other = write(root, "Keep/other.txt", "abc");
    state.files.insert("Keep/other.txt".into(), as_synced(&other));
    state.folders.extend(["Docs".to_owned(), "Keep".to_owned()]);

    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    let mut engine = engine(server, root, &[]);
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    let new_plan = plan(&engine.plan.server, root, &["Docs"]);
    engine.replan(new_plan);

    assert!(!same.exists(), "unchanged since the last sync: removed here");
    assert_eq!(std::fs::read_to_string(&grown).unwrap(), "abcd", "changed since: kept");
    assert!(touched.exists(), "saved again since (same size, newer time): kept");
    assert!(new.exists(), "never synced: not the sync's to remove");
    assert!(other.exists(), "still in the plan: untouched");
    assert!(root.join("Docs").is_dir(), "a folder with files kept in it stays");

    let events = activity(&engine);
    assert!(
        events.iter().any(|e| e.kind == ActivityKind::Error && e.detail == "kept 2 changed files that are no longer synced"),
        "the person is told: {events:?}"
    );
    assert!(events.iter().any(|e| e.kind == ActivityKind::Deleted && e.path == "Docs"), "{events:?}");
    let saved = synced_state(root);
    assert_eq!(saved.files.keys().collect::<Vec<_>>(), ["Keep/other.txt"], "the switched-off folder is forgotten");
    assert!(!saved.folders.contains("Docs") && saved.folders.contains("Keep"));
}

#[test]
fn switching_a_folder_off_without_changes_reports_nothing_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut state = State::default();
    let a = write(root, "Docs/Sub/a.txt", "abc");
    state.files.insert("Docs/Sub/a.txt".into(), as_synced(&a));
    state.folders.extend(["Docs".to_owned(), "Docs/Sub".to_owned()]);
    let mut engine = engine(ServerUrl::parse("http://127.0.0.1:9").unwrap(), root, &[]);
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    let new_plan = plan(&engine.plan.server, root, &["Docs"]);
    engine.replan(new_plan);
    assert!(!root.join("Docs").exists(), "emptied folders go too");
    assert!(!activity(&engine).iter().any(|e| e.kind == ActivityKind::Error), "{:?}", activity(&engine));
    assert!(synced_state(root).folders.is_empty());
}

// --- Moving to the cloud's trash: changed in the cloud meanwhile ------------------------------

/// The REST side of a trash: the `Docs` folder's id is known, its resources hold `a.txt` (id
/// `f1`), and deletes succeed.
fn rest(r: &Seen) -> Option<Reply> {
    if r.is("GET", "/api/folders/docs1/resources") {
        return Some((200, r#"{"items":[{"resource_type":"file","resource":{"id":"f1","name":"a.txt"}}]}"#.into()));
    }
    if r.method == "DELETE" {
        return Some((204, String::new()));
    }
    None
}

/// An engine on `server` that knows the `Docs` folder's REST id.
fn docs_engine(server: &FakeServer, root: &Path) -> Engine {
    let mut engine = engine(server.server(), root, &[]);
    engine.storage.ids.insert("Docs".into(), "docs1".into());
    engine
}

fn trashes(server: &FakeServer) -> usize {
    server.count("DELETE", "/api/")
}

fn apply(engine: &mut Engine, action: &Action, state: &mut State, folders: &mut BTreeSet<String>) -> Result<Option<super::Done>, String> {
    let root = engine.root();
    engine.apply(&root, action, state, folders, &Tree::default())
}

fn one_file_state() -> State {
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), Known { etag: "\"e1\"".into(), size: 3, mtime: 1, hash: None });
    state
}

#[test]
fn a_file_changed_in_the_cloud_since_the_plan_is_not_trashed() {
    let server = FakeServer::start(|r| rest(r).unwrap_or_else(|| listing("Docs", &[("a.txt", Some("e2"))], &[])));
    let dir = tempfile::tempdir().unwrap();
    let mut engine = docs_engine(&server, dir.path());
    let mut state = one_file_state();
    let done = apply(&mut engine, &Action::DeleteRemote { path: "Docs/a.txt".into(), etag: "\"e1\"".into() }, &mut state, &mut BTreeSet::new());
    assert_eq!(done.unwrap(), None, "nothing to report: it stays");
    assert_eq!(trashes(&server), 0, "the newer version stays in the cloud: {:?}", server.seen());
    assert!(!state.files.contains_key("Docs/a.txt"), "forgotten, so the next cycle downloads the new version");
}

#[test]
fn an_unchanged_file_goes_to_the_trash() {
    // The contrast to the test above: the fake answers what the guard needs, and the trash runs.
    let server = FakeServer::start(|r| rest(r).unwrap_or_else(|| listing("Docs", &[("a.txt", Some("e1"))], &[])));
    let dir = tempfile::tempdir().unwrap();
    let mut engine = docs_engine(&server, dir.path());
    let mut state = one_file_state();
    let done = apply(&mut engine, &Action::DeleteRemote { path: "Docs/a.txt".into(), etag: "\"e1\"".into() }, &mut state, &mut BTreeSet::new());
    assert_eq!(done.unwrap().map(|d| d.0), Some(ActivityKind::Deleted));
    assert_eq!(server.count("DELETE", "/api/files/f1"), 1, "{:?}", server.seen());
    assert!(server.seen().iter().all(|r| r.method != "DELETE" || r.path.starts_with("/api/")), "never a WebDAV DELETE");
}

#[test]
fn a_file_whose_folder_cant_be_listed_is_not_trashed() {
    let server = FakeServer::start(|r| rest(r).unwrap_or((500, "{}".into())));
    let dir = tempfile::tempdir().unwrap();
    let mut engine = docs_engine(&server, dir.path());
    let mut state = one_file_state();
    let done = apply(&mut engine, &Action::DeleteRemote { path: "Docs/a.txt".into(), etag: "\"e1\"".into() }, &mut state, &mut BTreeSet::new());
    assert!(done.is_err(), "unknown is an error, tried again next cycle: {done:?}");
    assert_eq!(trashes(&server), 0);
    assert!(state.files.contains_key("Docs/a.txt"), "still known as synced");
}

#[test]
fn a_file_listed_without_an_etag_is_not_trashed() {
    // Without an ETag the cloud file can't be shown to be the version planned: unknown, so it
    // stays, whatever the plan carries (an empty ETag too: a scan that listed it without one
    // plans it so), and the activity list says so once.
    for planned in ["\"e1\"", ""] {
        let server = FakeServer::start(|r| rest(r).unwrap_or_else(|| listing("Docs", &[("a.txt", None)], &[])));
        let dir = tempfile::tempdir().unwrap();
        let mut engine = docs_engine(&server, dir.path());
        let delete = Action::DeleteRemote { path: "Docs/a.txt".into(), etag: planned.into() };
        for _ in 0..2 {
            let mut state = one_file_state();
            let done = apply(&mut engine, &delete, &mut state, &mut BTreeSet::new());
            assert_eq!(done, Ok(None), "planned {planned:?}");
            assert!(!state.files.contains_key("Docs/a.txt"), "forgotten, so the next cycle brings it back down");
        }
        assert_eq!(trashes(&server), 0, "planned {planned:?}: {:?}", server.seen());
        let said = activity(&engine).into_iter().filter(|e| e.kind == ActivityKind::Skipped && e.path == "Docs/a.txt").count();
        assert_eq!(said, 1, "said once");
    }
}

// --- A whole folder to the trash: only when it holds exactly what was planned ------------------

fn folder_delete() -> Action {
    Action::DeleteRemoteFolder {
        path: "Docs".into(),
        files: vec![("Docs/a.txt".into(), "\"e1\"".into()), ("Docs/Sub/b.txt".into(), "\"e1\"".into())],
    }
}

/// A server whose `Docs` holds `a.txt` and `Sub/b.txt` as `docs` and `sub` say.
fn folder_server(docs: Reply, sub: Reply) -> FakeServer {
    FakeServer::start(move |r| {
        if let Some(reply) = rest(r) {
            reply
        } else if r.path.starts_with("/webdav/Docs/Sub/") {
            sub.clone()
        } else {
            docs.clone()
        }
    })
}

fn delete_folder(server: &FakeServer) -> (Result<Option<super::Done>, String>, State, BTreeSet<String>) {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = docs_engine(server, dir.path());
    let mut state = one_file_state();
    state.files.insert("Docs/Sub/b.txt".into(), Known { etag: "\"e1\"".into(), size: 3, mtime: 1, hash: None });
    let mut folders: BTreeSet<String> = ["Docs".to_owned(), "Docs/Sub".to_owned()].into();
    let done = apply(&mut engine, &folder_delete(), &mut state, &mut folders);
    (done, state, folders)
}

#[test]
fn a_folder_holding_exactly_what_was_planned_goes_to_the_trash_whole() {
    let server = folder_server(listing("Docs", &[("a.txt", Some("e1"))], &["Sub"]), listing("Docs/Sub", &[("b.txt", Some("e1"))], &[]));
    let (done, state, folders) = delete_folder(&server);
    assert_eq!(done.unwrap().map(|d| d.0), Some(ActivityKind::Deleted));
    assert_eq!(server.count("DELETE", "/api/folders/docs1"), 1, "one item, so a restore brings it back whole: {:?}", server.seen());
    assert_eq!(trashes(&server), 1);
    assert!(state.files.is_empty() && folders.is_empty());
}

#[test]
fn a_folder_with_a_file_added_in_the_cloud_is_not_trashed() {
    let server = folder_server(
        listing("Docs", &[("a.txt", Some("e1")), ("added.txt", Some("e5"))], &["Sub"]),
        listing("Docs/Sub", &[("b.txt", Some("e1"))], &[]),
    );
    let (done, state, folders) = delete_folder(&server);
    assert_eq!(done.unwrap(), None);
    assert_eq!(trashes(&server), 0, "{:?}", server.seen());
    assert_eq!(state.files.len(), 2, "nothing forgotten");
    assert_eq!(folders.len(), 2);
}

#[test]
fn a_folder_with_a_file_changed_in_the_cloud_is_not_trashed() {
    // The changed file is one level down: every level is listed again.
    let server = folder_server(listing("Docs", &[("a.txt", Some("e1"))], &["Sub"]), listing("Docs/Sub", &[("b.txt", Some("e2"))], &[]));
    let (done, _, folders) = delete_folder(&server);
    assert_eq!(done.unwrap(), None);
    assert_eq!(trashes(&server), 0, "{:?}", server.seen());
    assert_eq!(folders.len(), 2);
}

#[test]
fn a_folder_with_a_file_listed_without_an_etag_is_not_trashed() {
    // The plan's ETag is empty too (a scan that listed the file without one plans it so); the
    // file still can't be shown unchanged, so the folder stays.
    let server = folder_server(listing("Docs", &[("a.txt", None)], &["Sub"]), listing("Docs/Sub", &[("b.txt", Some("e1"))], &[]));
    let dir = tempfile::tempdir().unwrap();
    let mut engine = docs_engine(&server, dir.path());
    let mut state = one_file_state();
    let mut folders: BTreeSet<String> = ["Docs".to_owned(), "Docs/Sub".to_owned()].into();
    let files = vec![("Docs/a.txt".to_owned(), String::new()), ("Docs/Sub/b.txt".to_owned(), "\"e1\"".to_owned())];
    let done = apply(&mut engine, &Action::DeleteRemoteFolder { path: "Docs".into(), files }, &mut state, &mut folders);
    assert_eq!(done.unwrap(), None);
    assert_eq!(trashes(&server), 0, "{:?}", server.seen());
    assert_eq!(folders.len(), 2);
}

#[test]
fn a_folder_with_a_subfolder_that_cant_be_listed_is_not_trashed() {
    let server = folder_server(listing("Docs", &[("a.txt", Some("e1"))], &["Sub"]), (500, "{}".into()));
    let (done, state, _) = delete_folder(&server);
    assert!(done.is_err(), "a failed listing is unknown, not empty: {done:?}");
    assert_eq!(trashes(&server), 0, "{:?}", server.seen());
    assert_eq!(state.files.len(), 2);
}

// --- An empty folder removed in the cloud: only when a fresh listing says empty (T-064) --------

fn remove_folder(server: &FakeServer) -> BTreeSet<String> {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = docs_engine(server, dir.path());
    let mut folders: BTreeSet<String> = ["Docs".to_owned()].into();
    let done = apply(&mut engine, &Action::RemoveRemoteFolder("Docs".into()), &mut State::default(), &mut folders);
    assert_eq!(done, Ok(None));
    folders
}

#[test]
fn a_folder_whose_listing_fails_is_kept() {
    for status in [500, 403, 404] {
        let server = FakeServer::start(move |r| rest(r).unwrap_or((status, "{}".into())));
        let folders = remove_folder(&server);
        assert_eq!(trashes(&server), 0, "listing answered {status}: {:?}", server.seen());
        assert!(folders.contains("Docs"), "listing answered {status}");
    }
}

#[test]
fn a_folder_that_is_not_empty_in_the_cloud_is_kept() {
    let server = FakeServer::start(|r| rest(r).unwrap_or_else(|| listing("Docs", &[("late.txt", Some("e1"))], &[])));
    let folders = remove_folder(&server);
    assert_eq!(trashes(&server), 0);
    assert!(folders.contains("Docs"));
    let server = FakeServer::start(|r| rest(r).unwrap_or_else(|| listing("Docs", &[], &["Sub"])));
    remove_folder(&server);
    assert_eq!(trashes(&server), 0, "a subfolder counts too");
}

#[test]
fn an_empty_folder_goes_to_the_trash() {
    let server = FakeServer::start(|r| rest(r).unwrap_or_else(|| listing("Docs", &[], &[])));
    let folders = remove_folder(&server);
    assert_eq!(server.count("DELETE", "/api/folders/docs1"), 1, "{:?}", server.seen());
    assert!(folders.is_empty());
}

// --- The sign-in during a cycle ----------------------------------------------------------------

#[test]
fn a_cycle_whose_refresh_is_refused_ends_signed_out() {
    let server =
        FakeServer::start(|r| if r.is("POST", "/api/auth/refresh") { (404, r#"{"error":"Session not found"}"#.into()) } else { (401, "{}".into()) });
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(server.server(), dir.path(), &[]);
    engine.cycle(Deletes::Hold);
    assert_eq!(engine.progress.lock().unwrap_or_else(PoisonError::into_inner).state, SyncState::SignedOut);
    assert_eq!(server.seen().len(), 2, "one listing, one refresh, then it waits for a new sign-in: {:?}", server.seen());
}

#[test]
fn a_token_that_runs_out_mid_scan_is_refreshed_once() {
    // The first listing goes through; then the access token runs out while four folders are listed
    // at once. One refresh, and every folder is listed with the new token.
    let used = Arc::new(AtomicUsize::new(0));
    let first = Arc::clone(&used);
    let server = FakeServer::start(move |r| {
        if r.is("POST", "/api/auth/refresh") {
            return if r.body.contains("\"r1\"") { (200, r#"{"access_token":"a2","refresh_token":"r2"}"#.into()) } else { (401, "{}".into()) };
        }
        let accepted = match r.bearer() {
            Some("a1") => first.fetch_add(1, Ordering::SeqCst) == 0,
            Some("a2") => true,
            _ => false,
        };
        if !accepted {
            return (401, "{}".into());
        }
        match r.path.trim_start_matches("/webdav/").trim_end_matches('/') {
            "" => listing("", &[], &["F1", "F2", "F3", "F4", "F5"]),
            folder => listing(folder, &[("x.txt", Some("e1"))], &[]),
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(server.server(), dir.path(), &[]);
    let tree = engine.scan_remote().unwrap();
    assert_eq!(tree.files.keys().collect::<Vec<_>>(), ["F1/x.txt", "F2/x.txt", "F3/x.txt", "F4/x.txt", "F5/x.txt"]);
    assert_eq!(server.count("POST", "/api/auth/refresh"), 1, "{:?}", server.seen());
    assert_eq!(engine.storage.session.tokens.access_token, "a2");
    assert!(!engine.storage.session.signed_out);
}

// --- Modes: what the engine does with a server that allows less --------------------------------

/// OxiCloud's driver with other capabilities, so the engine's mode follows them (no server kind
/// but OxiCloud reaches the engine yet).
struct WithCapabilities {
    inner: OxiCloud,
    caps: Capabilities,
}

impl RemoteStorage for WithCapabilities {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn begin_pass(&mut self) {
        self.inner.begin_pass();
    }
    fn take_offline(&mut self) -> Option<String> {
        self.inner.take_offline()
    }
    fn signed_out(&self) -> bool {
        self.inner.signed_out()
    }
    fn list(&mut self, folder: &str) -> Result<Vec<Entry>, StorageError> {
        self.inner.list(folder)
    }
    fn download(&mut self, path: &str, from: Option<u64>) -> Result<Download, StorageError> {
        self.inner.download(path, from)
    }
    fn create(&mut self, path: &str, file: &Path) -> Result<Written, StorageError> {
        self.inner.create(path, file)
    }
    fn replace(&mut self, path: &str, if_match: &str, file: &Path) -> Result<Written, StorageError> {
        self.inner.replace(path, if_match, file)
    }
    fn make_folder(&mut self, path: &str) -> Result<(), StorageError> {
        self.inner.make_folder(path)
    }
    fn move_to(&mut self, from: &str, to: &str, guard: MoveGuard) -> Result<Moved, StorageError> {
        self.inner.move_to(from, to, guard)
    }
    fn trash(&mut self, path: &str, guard: TrashGuard) -> Result<Trashed, StorageError> {
        self.inner.trash(path, guard)
    }
    fn trash_list(&mut self) -> Result<Vec<TrashItem>, StorageError> {
        self.inner.trash_list()
    }
    fn restore(&mut self, item: &TrashItem) -> Result<(), StorageError> {
        self.inner.restore(item)
    }
    fn keep_version(&mut self, path: &str) -> Result<(), StorageError> {
        self.inner.keep_version(path)
    }
    fn move_versions(&mut self, from: &str, to: &str) -> Result<(), StorageError> {
        self.inner.move_versions(from, to)
    }
    fn free_space(&mut self, needed: u64) -> Result<Option<u64>, StorageError> {
        self.inner.free_space(needed)
    }
    fn purge_own(&mut self, path: &OwnPath) -> Result<(), StorageError> {
        self.inner.purge_own(path)
    }
}

/// [`engine`] over a server with these capabilities.
fn engine_with(server: ServerUrl, root: &Path, caps: Capabilities) -> Engine<WithCapabilities> {
    let Engine {
        plan,
        storage,
        cancel,
        device,
        progress,
        waker,
        paused,
        settled,
        reported,
        no_room,
        refused,
        no_folder_move,
        move_tries,
        now,
        platform,
        parts_swept,
        two_way_off,
        _watcher,
    } = engine(server, root, &[]);
    Engine {
        plan,
        storage: WithCapabilities { inner: storage, caps },
        cancel,
        device,
        progress,
        waker,
        paused,
        settled,
        reported,
        no_room,
        refused,
        no_folder_move,
        move_tries,
        now,
        platform,
        parts_swept,
        two_way_off,
        _watcher,
    }
}

/// A server that ignores `If-None-Match: *` (as OpenCloud does, docs/WEBDAV-STORAGE.md Q1): the
/// sync only downloads.
fn download_only() -> Capabilities {
    Capabilities { create_if_absent: Guard::Ignored, ..Capabilities::oxicloud() }
}

/// The local folder after a sync with the cloud holding `Docs/kept.txt` and `Docs/edited.txt`
/// (both as last synced) and `Moved/a.txt`: since then, here, `edited.txt` was changed,
/// `renamed.txt` is `old.txt` renamed, `new.txt` and the folder `Moved` (renamed from `Folder`)
/// are new; in the cloud, `gone.txt` was deleted and `old.txt` and `Folder` are still there.
fn download_only_scene(root: &Path) -> State {
    let mut state = State::default();
    for name in ["kept", "edited", "gone", "old"] {
        let file = write(root, &format!("Docs/{name}.txt"), "abc");
        state.files.insert(format!("Docs/{name}.txt"), as_synced(&file));
    }
    let moved = write(root, "Folder/a.txt", "abc");
    state.files.insert("Folder/a.txt".into(), as_synced(&moved));
    state.folders.extend(["Docs".to_owned(), "Folder".to_owned()]);
    std::fs::write(root.join("Docs/edited.txt"), "abcd").unwrap();
    std::fs::rename(root.join("Docs/old.txt"), root.join("Docs/renamed.txt")).unwrap();
    std::fs::rename(root.join("Folder"), root.join("Moved")).unwrap();
    write(root, "Docs/new.txt", "mine");
    state
}

fn download_only_server() -> FakeServer {
    FakeServer::start(|r| match r.path.trim_start_matches("/webdav/").trim_end_matches('/') {
        "" => listing("", &[], &["Docs", "Folder"]),
        "Docs" => listing("Docs", &[("kept.txt", Some("e1")), ("edited.txt", Some("e1")), ("old.txt", Some("e1"))], &[]),
        "Folder" => listing("Folder", &[("a.txt", Some("e1"))], &[]),
        _ => (404, String::new()),
    })
}

/// Download only (docs/WEBDAV-STORAGE.md § 9), one whole cycle: nothing but listings reaches the
/// server (no upload, folder, move or delete there, a folder renamed here included); the file
/// deleted in the cloud is moved into the local `.craftcloud/trash/`, never deleted; a change
/// made here, a file new here and a file renamed here stay as they are.
#[test]
fn download_only_writes_nothing_to_the_cloud_and_trashes_a_cloud_delete_here() {
    let server = download_only_server();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state = download_only_scene(root);
    let gone_before = std::fs::read_to_string(root.join("Docs/gone.txt")).unwrap();
    let mut engine = engine_with(server.server(), root, download_only());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    engine.cycle(Deletes::Hold);

    let writes: Vec<String> = server.seen().iter().filter(|r| r.method != "PROPFIND").map(|r| format!("{} {}", r.method, r.path)).collect();
    assert!(writes.is_empty(), "only listings reach the server: {writes:?}");
    assert!(!root.join("Docs/gone.txt").exists(), "deleted in the cloud: not here any more");
    let trashed: Vec<std::path::PathBuf> = walkdir(&root.join(".craftcloud/trash"));
    assert_eq!(trashed.len(), 1, "{trashed:?}");
    assert!(trashed[0].ends_with("Docs/gone.txt"), "{trashed:?}");
    let place = trashed[0].strip_prefix(root.join(".craftcloud/trash")).unwrap();
    let stamp = place.components().next().unwrap().as_os_str().to_string_lossy().into_owned();
    assert!(stamp.ends_with("Z-Laptop") && stamp.len() == "20261117T143205Z-Laptop".len(), "<time>-<device>: {stamp}");
    assert_eq!(std::fs::read_to_string(&trashed[0]).unwrap(), gone_before, "kept whole in the trash here");
    assert_eq!(std::fs::read_to_string(root.join("Docs/edited.txt")).unwrap(), "abcd", "a change made here stays");
    assert_eq!(std::fs::read_to_string(root.join("Docs/new.txt")).unwrap(), "mine", "a file new here stays");
    assert!(root.join("Docs/renamed.txt").exists() && root.join("Moved/a.txt").exists(), "renamed here: stays");
    let saved = synced_state(root);
    assert!(!saved.files.contains_key("Docs/gone.txt"), "the trashed file is forgotten");
    assert!(!saved.files.contains_key("Docs/old.txt"), "renamed here: forgotten, so it comes down again next cycle");
    assert!(saved.files.contains_key("Docs/edited.txt"), "still synced as it was, the change not sent");
}

/// The same scene both ways (OxiCloud's capabilities): the contrast that shows the cut is what
/// keeps the writes away. The folder renamed here is moved in the cloud first thing; the file
/// deleted in the cloud leaves its place here and is kept in the local trash, as under download
/// only (never removed for good here).
#[test]
fn the_same_scene_both_ways_writes_to_the_cloud() {
    let server = download_only_server();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state = download_only_scene(root);
    let mut engine = engine_with(server.server(), root, Capabilities::oxicloud());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    engine.cycle(Deletes::Hold);

    assert!(server.seen().iter().any(|r| r.is("MOVE", "/webdav/Folder")), "the folder is moved in the cloud: {:?}", server.seen());
    assert!(!root.join("Docs/gone.txt").exists());
    let trashed = walkdir(&root.join(".craftcloud/trash"));
    assert_eq!(trashed.len(), 1, "{trashed:?}");
    assert!(trashed[0].ends_with("Docs/gone.txt"), "{trashed:?}");
    assert_eq!(std::fs::read_to_string(&trashed[0]).unwrap(), "abc", "kept whole");
}

/// A file deleted in the cloud that is saved here after the scan, before its delete runs: the
/// last look before the delete sees the change, and the file stays (the next pass uploads it as
/// new). Nothing is removed or trashed.
#[test]
fn a_file_saved_here_after_the_scan_survives_its_cloud_delete() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = write(root, "Docs/f.txt", "abc");
    let mut state = State::default();
    state.files.insert("Docs/f.txt".into(), as_synced(&file));
    // The plan was made from the scan (`DeleteLocal`); then the person saves.
    std::fs::write(&file, "saved after the scan").unwrap();
    let mut engine = engine(ServerUrl::parse("http://127.0.0.1:9").unwrap(), root, &[]);
    let root_path = engine.root();
    let mut folders = BTreeSet::new();
    let done = engine.apply(&root_path, &Action::DeleteLocal("Docs/f.txt".into()), &mut state, &mut folders, &Tree::default());
    assert!(done.is_ok(), "{done:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "saved after the scan", "never removed");
    assert!(!root.join(".craftcloud/trash").exists(), "nor trashed");
    assert!(state.files.contains_key("Docs/f.txt"), "the next pass sees it as this computer's");
}

/// A server whose ETags don't change with the bytes is refused: nothing is listed, nothing
/// changes here.
#[test]
fn a_refused_server_syncs_nothing() {
    let server = download_only_server();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state = download_only_scene(root);
    let mut engine = engine_with(server.server(), root, Capabilities { etag_changes_on_write: false, ..Capabilities::oxicloud() });
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    engine.cycle(Deletes::Hold);
    assert!(server.seen().is_empty(), "{:?}", server.seen());
    assert!(root.join("Docs/gone.txt").exists() && !root.join(".craftcloud/trash").exists());
    assert!(matches!(engine.progress.lock().unwrap_or_else(PoisonError::into_inner).state, SyncState::Failed(_)));
}

// --- The generic WebDAV driver: the same engine, the same guards, over a plain WebDAV server ---

use craftcloud_core::storage::probe::fake::Flags;

use crate::storage::webdav::WebDav;
use crate::storage::webdav::tests::{scripted, session as webdav_session, two_way};

/// [`engine`] over the generic WebDAV driver on `server` (a scripted WebDAV server under a
/// Nextcloud-shaped base), with these capabilities.
fn webdav_engine(server: &FakeServer, root: &Path, caps: Capabilities) -> Engine<WebDav> {
    let session = webdav_session(server);
    Engine {
        plan: plan(&server.url, root, &[]),
        cancel: Arc::clone(&session.cancel),
        storage: WebDav::new(session, caps, "Laptop"),
        device: "Laptop".into(),
        progress: Arc::default(),
        waker: Arc::new(|| {}),
        paused: false,
        settled: BTreeSet::new(),
        reported: BTreeSet::new(),
        no_room: BTreeMap::new(),
        refused: BTreeMap::new(),
        no_folder_move: BTreeSet::new(),
        move_tries: BTreeMap::new(),
        now: test_now,
        platform: Platform::current(),
        parts_swept: None,
        two_way_off: false,
        _watcher: None,
    }
}

/// The cloud holds `Docs/{kept,edited,gone}.txt`, all synced; since then `edited.txt` was changed
/// here, `gone.txt` deleted here and `new.txt` made here. The state carries the cloud's ETags.
fn webdav_scene(server: &FakeServer, dav: &Arc<std::sync::Mutex<craftcloud_core::storage::probe::fake::FakeDav>>, root: &Path) -> State {
    {
        let mut dav = dav.lock().unwrap();
        dav.add_folder("Docs", None);
        for name in ["kept", "edited", "gone"] {
            dav.add_file(&format!("Docs/{name}.txt"), b"abc");
        }
    }
    let listed = webdav_engine(server, root, two_way()).storage.list("Docs").unwrap();
    let etag = |name: &str| listed.iter().find(|e| e.path == format!("Docs/{name}.txt")).unwrap().etag.clone().unwrap();
    let mut state = State::default();
    for name in ["kept", "edited"] {
        let file = write(root, &format!("Docs/{name}.txt"), "abc");
        state.files.insert(format!("Docs/{name}.txt"), Known { etag: etag(name), ..as_synced(&file) });
    }
    state.files.insert("Docs/gone.txt".into(), Known { etag: etag("gone"), size: 3, mtime: 1, hash: None });
    state.folders.insert("Docs".into());
    // Written a while ago (a file written in the last two seconds is held as being saved).
    let an_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::write(root.join("Docs/edited.txt"), "abcd").unwrap();
    let new = write(root, "Docs/new.txt", "mine");
    for file in [root.join("Docs/edited.txt"), new] {
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(an_hour_ago).unwrap();
    }
    state
}

/// The methods the server saw, in order, without listings and downloads.
fn writes(server: &FakeServer) -> Vec<String> {
    server.seen().iter().filter(|r| !matches!(r.method.as_str(), "PROPFIND" | "GET")).map(|r| format!("{} {}", r.method, r.path)).collect()
}

/// Two-way over the generic driver, one whole cycle: the new file is created with its guard, the
/// change overwrites with its guard after a `COPY` kept the old bytes, the file deleted here goes
/// to `.craftcloud/trash/` by one `MOVE`, and nothing is ever deleted outside `.craftcloud/`.
#[test]
fn webdav_two_way_uploads_keeps_versions_and_trashes_by_move() {
    let (server, dav) = scripted(Flags::default());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state = webdav_scene(&server, &dav, root);
    let mut engine = webdav_engine(&server, root, two_way());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    engine.cycle(Deletes::Hold);

    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.state, SyncState::UpToDate, "{:?}", p.activity);
    assert_eq!(p.mode, Some(craftcloud_core::storage::Mode::TwoWay));
    for r in &server.seen() {
        match r.method.as_str() {
            "PUT" => assert!(r.header("if-match").is_some() || r.header("if-none-match") == Some("*"), "an unguarded PUT: {r:?}"),
            "MOVE" | "COPY" => assert_eq!(r.header("overwrite"), Some("F"), "{r:?}"),
            "DELETE" => panic!("a DELETE: {r:?}"),
            _ => {}
        }
    }
    let dav = dav.lock().unwrap();
    assert_eq!(dav.body("Docs/new.txt"), Some(b"mine".as_slice()), "{:?}", writes(&server));
    assert_eq!(dav.body("Docs/edited.txt"), Some(b"abcd".as_slice()));
    assert!(!dav.has("Docs/gone.txt"), "gone to the trash");
    let folders = dav.folders();
    let trashed: Vec<&String> = folders.iter().filter(|f| f.starts_with(".craftcloud/trash/") && f.ends_with("/Docs")).collect();
    assert_eq!(trashed.len(), 1, "{folders:?}");
    assert_eq!(dav.body(&format!("{}/gone.txt", trashed[0])), Some(b"abc".as_slice()), "kept whole in the trash");
    let versions: Vec<&String> = folders.iter().filter(|f| f.starts_with(".craftcloud/versions/Docs/edited.txt")).collect();
    assert_eq!(versions.len(), 1, "{folders:?}");
    assert_eq!(server.seen().iter().filter(|r| r.method == "COPY").count(), 1, "one version kept");
    let said: Vec<String> = p.activity.iter().map(|e| format!("{:?} {} {}", e.kind, e.path, e.detail)).collect();
    assert!(said.iter().any(|s| s.starts_with("Deleted Docs/gone.txt")), "{said:?}");
    assert!(said.iter().any(|s| s == "Up Docs/edited.txt previous version kept"), "{said:?}");
}

/// Download only over the generic driver (docs/WEBDAV-STORAGE.md § 9): no `PUT`, `MKCOL`,
/// `MOVE`, `COPY` or `DELETE` reaches the server in any cycle; the change and the new file made
/// here stay, are said once ("not uploaded") and listed as not synced; the file deleted in the
/// cloud goes to the local `.craftcloud/trash/`.
#[test]
fn webdav_download_only_writes_nothing_and_says_so_once() {
    let (server, dav) = scripted(Flags { ignores_if_none_match: true, ..Flags::default() });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state = webdav_scene(&server, &dav, root);
    // Deleted in the cloud since the last sync.
    let deleted_here = write(root, "Docs/cloud-deleted.txt", "abc");
    let mut state = state;
    state.files.insert("Docs/cloud-deleted.txt".into(), as_synced(&deleted_here));
    let lowered = Capabilities { create_if_absent: Guard::Ignored, ..two_way() };
    let mut engine = webdav_engine(&server, root, lowered);
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    for cycle in 1..=2 {
        engine.cycle(Deletes::Hold);
        assert!(writes(&server).is_empty(), "cycle {cycle}: only listings and downloads reach the server: {:?}", writes(&server));
        assert!(
            server.seen().iter().all(|r| !r.path.contains("/.craftcloud/trash")),
            "cycle {cycle}: the client trash isn't even looked at under download only (no retention sweep): {:?}",
            server.seen()
        );
        let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
        assert_eq!(p.state, SyncState::UpToDate, "cycle {cycle}: {:?}", p.activity);
        assert_eq!(p.mode, Some(craftcloud_core::storage::Mode::DownloadOnly));
        let not_uploaded: Vec<&String> =
            p.left_out.iter().filter(|(_, why)| **why == craftcloud_core::sync::LeftOut::NotUploaded).map(|(p, _)| p).collect();
        assert_eq!(not_uploaded, ["Docs/edited.txt", "Docs/new.txt"], "cycle {cycle}");
        assert!(p.not_synced.contains(&"Docs/new.txt".to_owned()), "cycle {cycle}: {:?}", p.not_synced);
        let said: Vec<&str> = p.activity.iter().filter(|e| e.kind == ActivityKind::Skipped).map(|e| e.detail.as_str()).collect();
        assert_eq!(
            said,
            [
                "changed on this computer, not uploaded: this server only syncs downwards",
                "new on this computer, not uploaded: this server only syncs downwards"
            ],
            "cycle {cycle}: said once, in the first cycle"
        );
    }
    assert_eq!(std::fs::read_to_string(root.join("Docs/edited.txt")).unwrap(), "abcd", "the change stays here");
    assert_eq!(std::fs::read_to_string(root.join("Docs/new.txt")).unwrap(), "mine");
    assert!(!root.join("Docs/cloud-deleted.txt").exists());
    let trashed = walkdir(&root.join(".craftcloud/trash"));
    assert_eq!(trashed.len(), 1, "{trashed:?}");
    assert!(trashed[0].ends_with("Docs/cloud-deleted.txt"));
    let dav = dav.lock().unwrap();
    assert!(!dav.has("Docs/new.txt") && dav.body("Docs/edited.txt") == Some(b"abc".as_slice()), "nothing written to the server");
    assert!(dav.has("Docs/gone.txt"), "deleted here, download only: the cloud keeps it (it comes back down)");
    assert!(root.join("Docs/gone.txt").exists(), "and it came back down");
}

/// Q13 (docs/WEBDAV-STORAGE.md § 16): on a server that reuses a file's version tag within one
/// second, another client's write right behind ours lands under our tag. Without the checksum
/// check the engine records our upload as done while the cloud holds theirs (and our next edit
/// would overwrite it). With the Nextcloud profile the write is a conflict: theirs comes down,
/// ours is kept as a conflict copy, nothing is lost, and the activity list says so.
#[test]
fn webdav_a_write_landing_under_our_tag_becomes_a_conflict_copy_with_the_checksum_check() {
    use crate::storage::profile::nextcloud::Nextcloud;
    let flags = Flags { nextcloud: true, etag_reused_within_second: true, other_writes_after_put: Some(b"theirs".to_vec()), ..Flags::default() };
    let caps = Capabilities { trash: craftcloud_core::storage::Trash::Server, versions: craftcloud_core::storage::Versions::Server, ..two_way() };
    // The scene: Docs/edited.txt synced, then changed here (the other files play no part).
    let scene = |server: &FakeServer, dav: &Arc<std::sync::Mutex<craftcloud_core::storage::probe::fake::FakeDav>>, root: &Path| {
        let mut state = webdav_scene(server, dav, root);
        state.files.remove("Docs/gone.txt");
        std::fs::remove_file(root.join("Docs/new.txt")).unwrap();
        state
    };

    // Without the check: "uploaded", and the cloud holds someone else's bytes under our tag.
    let (server, dav) = scripted(flags.clone());
    dav.lock().unwrap().origin = server.url.clone();
    let dir = tempfile::tempdir().unwrap();
    let state = scene(&server, &dav, dir.path());
    let mut engine = webdav_engine(&server, dir.path(), caps.clone());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let said: Vec<String> = p.activity.iter().map(|e| format!("{:?} {}", e.kind, e.path)).collect();
    assert!(said.contains(&"Up Docs/edited.txt".to_owned()) && !said.iter().any(|s| s.starts_with("Conflict")), "taken as done: {said:?}");
    assert_eq!(dav.lock().unwrap().body("Docs/edited.txt"), Some(b"theirs".as_slice()), "but the cloud holds theirs");
    assert_eq!(std::fs::read_to_string(dir.path().join("Docs/edited.txt")).unwrap(), "abcd", "and nothing here knows");

    // With it: a conflict copy, both kept.
    let (server, dav) = scripted(flags);
    dav.lock().unwrap().origin = server.url.clone();
    let dir = tempfile::tempdir().unwrap();
    let state = scene(&server, &dav, dir.path());
    let mut engine = webdav_engine(&server, dir.path(), caps);
    engine.storage = WebDav::new(webdav_session(&server), engine.storage.capabilities().clone(), "Laptop")
        .with_profile(Box::new(Nextcloud::new(&server.url, "john")));
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    engine.cycle(Deletes::Hold);
    let events = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).activity.clone();
    let conflict = events.iter().find(|e| e.kind == ActivityKind::Conflict).unwrap_or_else(|| panic!("{events:?}"));
    assert_eq!(conflict.path, "Docs/edited.txt");
    assert!(conflict.detail.starts_with("yours kept as "), "{conflict:?}");
    assert_eq!(std::fs::read_to_string(dir.path().join("Docs/edited.txt")).unwrap(), "theirs", "theirs came down");
    let names: Vec<String> =
        std::fs::read_dir(dir.path().join("Docs")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    let copy = names.iter().find(|n| n.starts_with("edited (") && n.contains("Laptop")).unwrap_or_else(|| panic!("{names:?}"));
    assert_eq!(std::fs::read_to_string(dir.path().join("Docs").join(copy)).unwrap(), "abcd", "ours kept whole");
    for r in &server.seen() {
        if r.method == "PUT" {
            assert!(r.header("oc-checksum").is_some_and(|c| c.starts_with("SHA1:")), "every PUT carries its checksum: {r:?}");
        }
    }
}

/// A Nextcloud-shaped scripted server that reuses a file's ETag within one second, holding
/// `Docs/f.txt` as this client uploaded it ("ours", synced), over which another client then wrote
/// "theirs!" within the same second, without a checksum: the ETag is the one this client saved.
/// The engine (with the Nextcloud profile, or the generic one for `verified` false), its folder
/// and the server.
fn same_second_scene(
    verified: bool,
) -> (Engine<WebDav>, tempfile::TempDir, FakeServer, Arc<std::sync::Mutex<craftcloud_core::storage::probe::fake::FakeDav>>) {
    use crate::storage::profile::nextcloud::Nextcloud;
    let (server, dav) = scripted(Flags { nextcloud: true, etag_reused_within_second: true, ..Flags::default() });
    dav.lock().unwrap().origin = server.url.clone();
    dav.lock().unwrap().add_folder("Docs", None);
    let dir = tempfile::tempdir().unwrap();
    let caps = Capabilities { trash: craftcloud_core::storage::Trash::Server, versions: craftcloud_core::storage::Versions::Server, ..two_way() };
    let mut engine = webdav_engine(&server, dir.path(), caps);
    if verified {
        engine.storage = WebDav::new(webdav_session(&server), engine.storage.capabilities().clone(), "Laptop")
            .with_profile(Box::new(Nextcloud::new(&server.url, "john")));
    }
    let file = write(dir.path(), "Docs/f.txt", "ours");
    let an_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(an_hour_ago).unwrap();
    let Written::Saved { etag } = engine.storage.create("Docs/f.txt", &file).unwrap() else { panic!("not saved") };
    // Another client, right behind us: the same second, so the same ETag; no checksum.
    let theirs = dav.lock().unwrap().answer("PUT", "/remote.php/dav/files/john/Docs/f.txt", &[], b"theirs!");
    assert_eq!(
        theirs.header("ETag").map(|e| e.trim_matches('"')),
        Some(crate::storage::webdav::tests::bare_etag(&etag).trim_matches('"')),
        "the ETag was reused"
    );
    let mut state = State::default();
    state.files.insert("Docs/f.txt".into(), Known { etag, ..as_synced(&file) });
    state.folders.insert("Docs".into());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    (engine, dir, server, dav)
}

/// Q13, the other direction (docs/WEBDAV-STORAGE.md § 16): another client's write that landed
/// within the same second as ours sits under the ETag this client saved. With no local change, a
/// sync must still bring it down: on a profile that verifies writes the file's version tag is its
/// ETag plus the listed checksum, which that write changed (cleared). Without it (the generic
/// profile on the same server) the cloud looks unchanged and the write never arrives.
#[test]
fn webdav_a_write_under_our_etag_comes_down_when_nothing_changed_here() {
    let (mut blind, dir, _server, _dav) = same_second_scene(false);
    blind.cycle(Deletes::Hold);
    assert_eq!(std::fs::read_to_string(dir.path().join("Docs/f.txt")).unwrap(), "ours", "bare ETags: the write is never seen");

    let (mut engine, dir, server, dav) = same_second_scene(true);
    let before = server.seen().len();
    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.state, SyncState::UpToDate, "{:?}", p.activity);
    assert_eq!(std::fs::read_to_string(dir.path().join("Docs/f.txt")).unwrap(), "theirs!", "their bytes came down");
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Down && e.path == "Docs/f.txt"), "{:?}", p.activity);
    assert_eq!(dav.lock().unwrap().body("Docs/f.txt"), Some(b"theirs!".as_slice()));
    assert!(server.seen()[before..].iter().all(|r| r.method != "PUT"), "nothing was written: {:?}", &server.seen()[before..]);
    // And it settles: the next pass downloads nothing again.
    let before = server.seen().len();
    engine.cycle(Deletes::Hold);
    assert!(server.seen()[before..].iter().all(|r| r.method != "GET" && r.method != "PUT"), "{:?}", &server.seen()[before..]);
}

/// The same with a local edit: a conflict copy, never an overwrite. Their bytes stay in the cloud
/// and come down, ours are kept under the conflict copy's name, and no `PUT` lands on their
/// version. Without the tag (the generic profile) our `If-Match` on the reused ETag passes and
/// their bytes are overwritten.
#[test]
fn webdav_a_write_under_our_etag_with_a_local_edit_is_a_conflict_copy() {
    let edit = |dir: &Path| {
        let file = dir.join("Docs/f.txt");
        std::fs::write(&file, "ours, edited").unwrap();
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(std::time::SystemTime::now() - Duration::from_secs(1800)).unwrap();
    };
    let (mut blind, dir, _server, dav) = same_second_scene(false);
    edit(dir.path());
    blind.cycle(Deletes::Hold);
    assert_eq!(dav.lock().unwrap().body("Docs/f.txt"), Some(b"ours, edited".as_slice()), "bare ETags: their bytes are overwritten");

    let (mut engine, dir, server, dav) = same_second_scene(true);
    edit(dir.path());
    let before = server.seen().len();
    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.state, SyncState::UpToDate, "{:?}", p.activity);
    assert_eq!(dav.lock().unwrap().body("Docs/f.txt"), Some(b"theirs!".as_slice()), "their bytes stay in the cloud");
    assert_eq!(std::fs::read_to_string(dir.path().join("Docs/f.txt")).unwrap(), "theirs!", "and came down");
    let conflict = p.activity.iter().find(|e| e.kind == ActivityKind::Conflict).unwrap_or_else(|| panic!("{:?}", p.activity));
    assert!(conflict.path == "Docs/f.txt" && conflict.detail.starts_with("yours kept as "), "{conflict:?}");
    let names: Vec<String> =
        std::fs::read_dir(dir.path().join("Docs")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    let copy = names.iter().find(|n| n.starts_with("f (") && n.contains("Laptop")).unwrap_or_else(|| panic!("{names:?}"));
    assert_eq!(std::fs::read_to_string(dir.path().join("Docs").join(copy)).unwrap(), "ours, edited", "ours kept whole");
    let puts: Vec<String> = server.seen()[before..].iter().filter(|r| r.method == "PUT").map(|r| r.path.clone()).collect();
    assert!(puts.iter().all(|p| !p.ends_with("/Docs/f.txt")), "no PUT landed on their version: {puts:?}");
}

/// The guards before a trash hold over the generic driver too: a file whose version tag moved
/// since the plan stays, a folder that gained a file stays, and a folder whose listing fails is
/// unknown (an error, nothing moved).
#[test]
fn webdav_trash_guards_hold_a_changed_file_a_grown_folder_and_a_failed_listing() {
    let (server, dav) = scripted(Flags::default());
    {
        let mut dav = dav.lock().unwrap();
        dav.add_folder("Docs/Sub", None);
        dav.add_file("Docs/a.txt", b"a");
        dav.add_file("Docs/Sub/b.txt", b"b");
    }
    let dir = tempfile::tempdir().unwrap();
    let mut engine = webdav_engine(&server, dir.path(), two_way());
    let etag = |engine: &mut Engine<WebDav>, folder: &str, path: &str| {
        engine.storage.list(folder).unwrap().into_iter().find(|e| e.path == path).unwrap().etag.unwrap()
    };
    let (a, b) = (etag(&mut engine, "Docs", "Docs/a.txt"), etag(&mut engine, "Docs/Sub", "Docs/Sub/b.txt"));
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), Known { etag: a.clone(), size: 1, mtime: 1, hash: None });
    state.files.insert("Docs/Sub/b.txt".into(), Known { etag: b.clone(), size: 1, mtime: 1, hash: None });
    let mut folders: BTreeSet<String> = ["Docs".to_owned(), "Docs/Sub".to_owned()].into();
    let root = engine.root();

    // Changed in the cloud since the plan: stays, forgotten so the next cycle brings it down.
    let stale = Action::DeleteRemote { path: "Docs/a.txt".into(), etag: "\"stale\"".into() };
    assert_eq!(engine.apply(&root, &stale, &mut state, &mut folders, &Tree::default()), Ok(None));
    assert!(dav.lock().unwrap().has("Docs/a.txt") && !state.files.contains_key("Docs/a.txt"));
    state.files.insert("Docs/a.txt".into(), Known { etag: a.clone(), size: 1, mtime: 1, hash: None });

    // A file added in the cloud meanwhile keeps the folder whole.
    dav.lock().unwrap().add_file("Docs/late.txt", b"late");
    let folder =
        Action::DeleteRemoteFolder { path: "Docs".into(), files: vec![("Docs/a.txt".into(), a.clone()), ("Docs/Sub/b.txt".into(), b.clone())] };
    assert_eq!(engine.apply(&root, &folder, &mut state, &mut folders, &Tree::default()), Ok(None));
    assert!(dav.lock().unwrap().has("Docs") && state.files.len() == 2 && folders.len() == 2);
    assert!(writes(&server).is_empty(), "{:?}", writes(&server));

    // A subfolder whose listing fails: unknown, an error, nothing moved.
    let inner = Arc::clone(&dav);
    let failing = FakeServer::start_full(move |seen: &Seen| {
        if seen.path.contains("/Docs/Sub/") {
            return crate::fake_server::Answer::new(500, "");
        }
        let mut dav = inner.lock().unwrap_or_else(PoisonError::into_inner);
        let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
        let mut answer = crate::fake_server::Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
        for (name, value) in resp.headers {
            answer = answer.header(&name, value);
        }
        answer
    });
    let mut engine = webdav_engine(&failing, dir.path(), two_way());
    let late = etag(&mut engine, "Docs", "Docs/late.txt");
    let folder = Action::DeleteRemoteFolder {
        path: "Docs".into(),
        files: vec![("Docs/a.txt".into(), a), ("Docs/Sub/b.txt".into(), b), ("Docs/late.txt".into(), late)],
    };
    let done = engine.apply(&root, &folder, &mut state, &mut folders, &Tree::default());
    assert!(done.is_err(), "a failed listing is unknown, not empty: {done:?}");
    assert!(writes(&failing).is_empty(), "{:?}", writes(&failing));
    assert!(dav.lock().unwrap().has("Docs/Sub/b.txt"));
}

/// A refused app password (401) ends the cycle signed out, with no retry; a server that doesn't
/// answer leaves it offline.
#[test]
fn webdav_a_refused_password_ends_signed_out() {
    let server = FakeServer::start(|_| (401, String::new()));
    let dir = tempfile::tempdir().unwrap();
    let mut engine = webdav_engine(&server, dir.path(), two_way());
    engine.cycle(Deletes::Hold);
    assert_eq!(engine.progress.lock().unwrap_or_else(PoisonError::into_inner).state, SyncState::SignedOut);
    assert_eq!(server.seen().len(), 1, "{:?}", server.seen());
    drop(server);
    let gone = FakeServer::start(|_| (200, String::new()));
    let mut engine = webdav_engine(&gone, dir.path(), two_way());
    drop(gone);
    engine.cycle(Deletes::Hold);
    assert!(matches!(engine.progress.lock().unwrap_or_else(PoisonError::into_inner).state, SyncState::Offline(_)));
}

/// A listing that fails mid-scan ends the cycle with nothing done, over the generic driver too:
/// the folder that couldn't be listed is unknown, so nothing here is deleted or overwritten.
#[test]
fn webdav_a_failed_listing_ends_the_cycle_with_nothing_done() {
    let (server, dav) = scripted(Flags::default());
    let root_dir = tempfile::tempdir().unwrap();
    let root = root_dir.path();
    let state = webdav_scene(&server, &dav, root);
    let failing = FakeServer::start_full(move |seen: &Seen| {
        if seen.path.ends_with("/Docs/") {
            return crate::fake_server::Answer::new(500, "");
        }
        let mut dav = dav.lock().unwrap_or_else(PoisonError::into_inner);
        let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
        crate::fake_server::Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned())
    });
    let mut engine = webdav_engine(&failing, root, two_way());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    engine.cycle(Deletes::Hold);
    assert!(matches!(engine.progress.lock().unwrap_or_else(PoisonError::into_inner).state, SyncState::Failed(_)));
    assert!(writes(&failing).is_empty(), "{:?}", writes(&failing));
    assert_eq!(std::fs::read_to_string(root.join("Docs/edited.txt")).unwrap(), "abcd");
    assert!(root.join("Docs/new.txt").exists() && root.join("Docs/kept.txt").exists());
}

/// The runtime watch (docs/WEBDAV-STORAGE.md § 7.5): a create the server answers 201 to while it
/// keeps the other file is caught by the stat right after the write — two-way sync stops for the
/// run (the mode drops to download only), the probe re-runs, the activity list says so once, the
/// file isn't recorded as synced, and nothing more is written to the server in that session.
#[test]
fn the_runtime_watch_stops_two_way_after_a_create_replaced_someone_elses_file() {
    use craftcloud_core::storage::Mode;
    let (server, dav) = scripted(Flags { create_keeps_other: true, ..Flags::default() });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // A different file is already at Docs/new.txt; the engine, told to create it, gets a 201 but
    // the server keeps the other file.
    {
        let mut d = dav.lock().unwrap();
        d.add_folder("Docs", None);
        d.add_file("Docs/new.txt", b"someone-elses");
    }
    let mut engine = webdav_engine(&server, root, two_way());
    write(root, "Docs/new.txt", "mine");
    let mut state = State::default();
    state.folders.insert("Docs".into());
    let root_path = engine.root();
    let mut folders: BTreeSet<String> = ["Docs".to_owned()].into();
    let done = engine.apply(&root_path, &Action::UploadNew("Docs/new.txt".into()), &mut state, &mut folders, &Tree::default());
    assert!(done.is_err(), "the write that didn't land is an error, in words: {done:?}");
    assert!(engine.two_way_off, "tripped in the same step as the write");
    assert!(!state.files.contains_key("Docs/new.txt"), "not recorded as synced");
    assert!(server.seen().iter().any(|r| r.path.contains("/.craftcloud/probe/")), "the probe re-ran");
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Warning && e.detail.contains("leave alone")), "{:?}", p.activity);

    // Nothing more is written to the server for the rest of the run, and the local file stays.
    write(root, "Docs/another.txt", "x");
    let before = writes(&server).len();
    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.mode, Some(Mode::DownloadOnly), "{:?}", p.activity);
    assert_eq!(writes(&server).len(), before, "download only now: nothing written: {:?}", writes(&server));
    assert_eq!(p.activity.iter().filter(|e| e.kind == ActivityKind::Warning).count(), 1, "said once");
    assert!(
        std::fs::read_to_string(root.join("Docs/new.txt")).is_ok_and(|s| s == "mine") || root.join("Docs").read_dir().unwrap().count() >= 2,
        "this computer's version is never overwritten by what the server kept"
    );
}

/// A later edit by another device is not a misbehaving server: A uploads, B lists the new tag,
/// edits and uploads with an honoured `If-Match` (another ETag, another size), A lists again. The
/// stat right after A's write saw A's file, so nothing trips: two-way stays on, no warning, no
/// re-probe, and B's version comes down as an ordinary change.
#[test]
fn the_runtime_watch_lets_another_devices_later_edit_through() {
    use craftcloud_core::storage::Mode;
    let (server, dav) = scripted(Flags::default());
    dav.lock().unwrap().add_folder("Docs", None);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut engine = webdav_engine(&server, root, two_way());
    write(root, "Docs/new.txt", "mine");
    let mut state = State::default();
    state.folders.insert("Docs".into());
    let root_path = engine.root();
    let mut folders: BTreeSet<String> = ["Docs".to_owned()].into();
    let done = engine.apply(&root_path, &Action::UploadNew("Docs/new.txt".into()), &mut state, &mut folders, &Tree::default());
    assert!(done.is_ok() && !engine.two_way_off, "{done:?}");
    assert!(state.files.contains_key("Docs/new.txt"));
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    // B: a longer file under an honoured guard (the scripted server honours every guard).
    dav.lock().unwrap().add_file("Docs/new.txt", b"the other device's longer edit");
    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.mode, Some(Mode::TwoWay), "{:?}", p.activity);
    assert!(!engine.two_way_off);
    assert!(p.activity.iter().all(|e| e.kind != ActivityKind::Warning), "{:?}", p.activity);
    assert!(server.seen().iter().all(|r| !r.path.contains("/.craftcloud/probe/")), "no re-probe");
    assert_eq!(std::fs::read_to_string(root.join("Docs/new.txt")).unwrap(), "the other device's longer edit", "an ordinary change, downloaded");
}

/// The runtime watch never applies to CraftCloud's own server, whose guards are known rather than
/// measured: after an upload there is no stat, and whatever the server would list for the file
/// (here another ETag and size, as another device's edit would show) is never taken for a
/// misbehaving server. The mode stays two-way, nothing is re-probed, no warning is said.
#[test]
fn the_runtime_watch_leaves_craftclouds_own_server_alone() {
    use craftcloud_core::storage::Mode;
    let server = FakeServer::start_full(|r: &Seen| match r.method.as_str() {
        "PUT" => crate::fake_server::Answer::new(201, "").header("ETag", "\"w1\""),
        _ => crate::fake_server::Answer::new(
            207,
            r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/webdav/Docs/new.txt</d:href><d:propstat><d:prop><d:resourcetype/><d:getetag>"someone-elses"</d:getetag><d:getcontentlength>99</d:getcontentlength></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#,
        ),
    });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut engine = engine(server.server(), root, &[]);
    write(root, "Docs/new.txt", "mine");
    let mut state = State::default();
    state.folders.insert("Docs".into());
    let root_path = engine.root();
    let mut folders: BTreeSet<String> = ["Docs".to_owned()].into();
    let done = engine.apply(&root_path, &Action::UploadNew("Docs/new.txt".into()), &mut state, &mut folders, &Tree::default());
    assert!(done.is_ok(), "{done:?}");
    assert!(!engine.storage.measured());
    assert!(server.seen().iter().all(|r| r.method != "PROPFIND"), "no stat after the write on CraftCloud's own server: {:?}", server.seen());
    assert!(!engine.two_way_off && engine.effective_mode() == Mode::TwoWay);
    assert!(engine.progress.lock().unwrap_or_else(PoisonError::into_inner).activity.iter().all(|e| e.kind != ActivityKind::Warning));
    assert!(server.seen().iter().all(|r| !r.path.contains("/.craftcloud/probe/")), "nothing re-probed");
}

/// The runtime watch's other trigger: a replace that succeeded on a guard the server ignored (it
/// answered 204 yet kept the newer file). Same result: two-way off, the probe re-run, said once.
#[test]
fn the_runtime_watch_stops_two_way_after_a_replace_ignored_its_guard() {
    use craftcloud_core::storage::Mode;
    let (server, dav) = scripted(Flags { replace_keeps_other: true, ..Flags::default() });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let mut d = dav.lock().unwrap();
        d.add_folder("Docs", None);
        d.add_file("Docs/a.txt", b"the-newer-one");
    }
    let mut engine = webdav_engine(&server, root, two_way());
    write(root, "Docs/a.txt", "x");
    let mut state = State::default();
    state.files.insert("Docs/a.txt".into(), Known { etag: "\"stale\"".into(), size: 1, mtime: 1, hash: None });
    state.folders.insert("Docs".into());
    let root_path = engine.root();
    let mut folders: BTreeSet<String> = ["Docs".to_owned()].into();
    let change = Action::UploadChange { path: "Docs/a.txt".into(), etag: "\"stale\"".into() };
    assert!(engine.apply(&root_path, &change, &mut state, &mut folders, &Tree::default()).is_err(), "caught at the write");
    assert!(engine.two_way_off);

    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.mode, Some(Mode::DownloadOnly), "{:?}", p.activity);
    assert_eq!(p.activity.iter().filter(|e| e.kind == ActivityKind::Warning).count(), 1, "{:?}", p.activity);
    assert!(server.seen().iter().any(|r| r.path.contains("/.craftcloud/probe/")), "the probe re-ran");
}

// --- A server that can't right now: a move it couldn't do is tried again, never rebuilt ---------

type Dav = Arc<std::sync::Mutex<craftcloud_core::storage::probe::fake::FakeDav>>;

/// The cloud holds `Proj/` (eight files and `Sub/b.txt`) and `keep.txt`, all synced; since then
/// the folder was renamed to `Proj2` here, and `Proj/0.txt` was edited in the cloud by another
/// device. More files than a plan may delete at once without asking.
fn renamed_folder_scene(engine: &mut Engine<WebDav>, dav: &Dav) {
    let root = engine.root();
    {
        let mut dav = dav.lock().unwrap();
        dav.add_folder("Proj/Sub", None);
        for n in 0..8 {
            dav.add_file(&format!("Proj/{n}.txt"), b"abc");
        }
        dav.add_file("Proj/Sub/b.txt", b"abc");
        dav.add_file("keep.txt", b"abc");
    }
    let mut synced: Vec<(String, String)> = (0..8).map(|n| (format!("Proj/{n}.txt"), format!("Proj2/{n}.txt"))).collect();
    synced.push(("Proj/Sub/b.txt".into(), "Proj2/Sub/b.txt".into()));
    synced.push(("keep.txt".into(), "keep.txt".into()));
    let mut state = State::default();
    for (cloud, here) in synced {
        let (folder, _) = cloud.rsplit_once('/').unwrap_or(("", &cloud));
        let etag = engine.storage.list(folder).unwrap().into_iter().find(|e| e.path == cloud).unwrap().etag.unwrap();
        let file = write(&root, &here, "abc");
        state.files.insert(cloud, Known { etag, ..as_synced(&file) });
    }
    state.folders.extend(["Proj".to_owned(), "Proj/Sub".to_owned()]);
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();
    dav.lock().unwrap().add_file("Proj/0.txt", b"edited elsewhere");
}

/// What a pass that couldn't move the folder must leave: one folder in the cloud (the old one,
/// whole), nothing uploaded under the new name, nothing in any trash, nothing brought down under
/// the old name, and the saved state as it was (so the next pass plans the same move).
fn assert_folder_move_waits(engine: &Engine<WebDav>, server: &FakeServer, dav: &Dav) {
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.state, SyncState::UpToDate, "{:?}", p.activity);
    let wrote = writes(server);
    assert!(wrote.len() == 1 && wrote[0].starts_with("MOVE ") && wrote[0].ends_with("/Proj/"), "only the move that failed: {wrote:?}");
    {
        let dav = dav.lock().unwrap();
        assert!(!dav.has("Proj2"), "nothing was built under the new name: {:?}", dav.folders());
        assert_eq!(dav.body("Proj/Sub/b.txt"), Some(b"abc".as_slice()), "the old folder is whole");
        assert_eq!(dav.body("Proj/0.txt"), Some(b"edited elsewhere".as_slice()));
        assert!(!dav.folders().iter().any(|f| f.starts_with(".craftcloud/trash")), "nothing was trashed: {:?}", dav.folders());
    }
    let root = engine.root();
    assert!(!root.join("Proj").exists(), "nothing came down under the old name");
    assert_eq!(std::fs::read_to_string(root.join("Proj2/0.txt")).unwrap(), "abc");
    let saved = synced_state(&root);
    assert_eq!(saved.files.len(), 10, "the state is as it was: {:?}", saved.files.keys());
    assert!(saved.files.contains_key("Proj/Sub/b.txt") && !saved.files.keys().any(|f| f.starts_with("Proj2/")), "{:?}", saved.files.keys());
    assert!(saved.folders.contains("Proj") && saved.folders.contains("Proj/Sub") && !saved.folders.contains("Proj2"), "{:?}", saved.folders);
    assert!(p.pending_deletes.is_empty(), "no mass delete to confirm: {:?}", p.pending_deletes);
    let said: Vec<&ActivityEvent> = p.activity.iter().filter(|e| e.path == "Proj2").collect();
    assert!(said.len() == 1 && said[0].kind == ActivityKind::Error && said[0].detail.contains("tried again"), "said once: {:?}", p.activity);
    assert!(p.unsynced.contains_key("Proj2"), "the folder shows as not synced yet: {:?}", p.unsynced);
}

/// The next pass, the server well again: the same folder moved by one `MOVE`, nothing uploaded,
/// nothing trashed; the edit made elsewhere comes down under the new name.
fn assert_folder_moved_whole(engine: &Engine<WebDav>, server: &FakeServer, dav: &Dav, before: usize) {
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.state, SyncState::UpToDate, "{:?}", p.activity);
    let wrote = writes(server)[before..].to_vec();
    assert_eq!(wrote.iter().filter(|w| !w.contains("/.craftcloud/")).count(), 1, "one request outside .craftcloud/: {wrote:?}");
    assert!(wrote.iter().any(|w| w.starts_with("MOVE ") && w.ends_with("/Proj/")), "{wrote:?}");
    assert!(!wrote.iter().any(|w| w.starts_with("PUT ") || w.starts_with("DELETE ") || w.contains("/trash")), "{wrote:?}");
    let dav = dav.lock().unwrap();
    assert!(!dav.has("Proj"), "one folder, under its new name: {:?}", dav.folders());
    assert_eq!(dav.body("Proj2/Sub/b.txt"), Some(b"abc".as_slice()));
    let root = engine.root();
    assert_eq!(std::fs::read_to_string(root.join("Proj2/0.txt")).unwrap(), "edited elsewhere", "{:?}", p.activity);
    assert!(!root.join("Proj").exists());
    let saved = synced_state(&root);
    assert!(saved.files.contains_key("Proj2/Sub/b.txt") && !saved.files.keys().any(|f| f.starts_with("Proj/")), "{:?}", saved.files.keys());
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Moved && e.path == "Proj2"), "{:?}", p.activity);
    assert!(p.unsynced.is_empty(), "{:?}", p.unsynced);
}

/// CI's Nextcloud run: the server answered 500 ("locked") to the `MOVE` of a renamed folder,
/// and the client built the folder again beside the old one, file by file. A server that can't
/// right now (500, 423 Locked, 429, 503) is asked again next pass instead; until then nothing
/// under either name is uploaded, downloaded, trashed or forgotten.
#[test]
fn webdav_a_folder_move_the_server_cant_do_right_now_waits_and_is_one_move_next_pass() {
    for status in [500, 423, 429, 503] {
        let (server, dav) = scripted(Flags::default());
        let dir = tempfile::tempdir().unwrap();
        let mut engine = webdav_engine(&server, dir.path(), two_way());
        renamed_folder_scene(&mut engine, &dav);
        dav.lock().unwrap().flags.answers_once.push(("MOVE".into(), status));
        engine.cycle(Deletes::Hold);
        assert_folder_move_waits(&engine, &server, &dav);
        let before = writes(&server).len();
        engine.cycle(Deletes::Hold);
        assert_folder_moved_whole(&engine, &server, &dav, before);
    }
}

/// A second folder moved into the new name of one whose move waits: it waits too. Done now, its
/// move would make the new name in the cloud (as its parent), and the first folder could never
/// be moved there as one any more.
#[test]
fn webdav_a_folder_moved_into_one_whose_move_waits_waits_with_it() {
    let (server, dav) = scripted(Flags::default());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut engine = webdav_engine(&server, root, two_way());
    renamed_folder_scene(&mut engine, &dav);
    {
        let mut dav = dav.lock().unwrap();
        dav.add_folder("X", None);
        dav.add_file("X/x.txt", b"abcde");
    }
    let etag = engine.storage.list("X").unwrap().into_iter().find(|e| e.path == "X/x.txt").unwrap().etag.unwrap();
    let mut state = synced_state(root);
    let moved = write(root, "Proj2/Y/x.txt", "abcde");
    state.files.insert("X/x.txt".into(), Known { etag, ..as_synced(&moved) });
    state.folders.insert("X".into());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();

    dav.lock().unwrap().flags.answers_once.push(("MOVE".into(), 500));
    engine.cycle(Deletes::Hold);
    let wrote = writes(&server);
    assert!(wrote.len() == 1 && wrote[0].starts_with("MOVE ") && wrote[0].ends_with("/Proj/"), "only the move that failed: {wrote:?}");
    assert!(!dav.lock().unwrap().has("Proj2") && dav.lock().unwrap().has("X/x.txt"), "{:?}", dav.lock().unwrap().folders());
    let saved = synced_state(root);
    assert!(saved.files.contains_key("X/x.txt") && saved.folders.contains("X") && saved.folders.contains("Proj"), "{saved:?}");

    engine.cycle(Deletes::Hold);
    let wrote = writes(&server)[1..].to_vec();
    assert!(!wrote.iter().any(|w| w.starts_with("PUT ") || w.starts_with("DELETE ") || w.contains("/trash")), "{wrote:?}");
    let dav = dav.lock().unwrap();
    assert_eq!(dav.body("Proj2/Y/x.txt"), Some(b"abcde".as_slice()), "{wrote:?}");
    assert!(!dav.has("Proj") && !dav.has("X") && dav.has("Proj2/Sub/b.txt"), "{:?}", dav.folders());
}

/// The same when the server never answers the `MOVE` at all (the connection closes): no answer
/// says nothing about the folder, so it is not rebuilt.
#[test]
fn webdav_a_folder_move_that_gets_no_answer_waits() {
    let dav: Dav =
        Arc::new(std::sync::Mutex::new(craftcloud_core::storage::probe::fake::FakeDav::new("/remote.php/dav/files/john", Flags::default())));
    let (inner, down) = (Arc::clone(&dav), Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let is_down = Arc::clone(&down);
    let server = FakeServer::start_full(move |seen: &Seen| {
        if seen.method == "MOVE" && is_down.load(Ordering::SeqCst) {
            return crate::fake_server::Answer::new(crate::fake_server::Answer::DROP, "");
        }
        let mut dav = inner.lock().unwrap_or_else(PoisonError::into_inner);
        let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
        let mut answer = crate::fake_server::Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
        for (name, value) in resp.headers {
            answer = answer.header(&name, value);
        }
        answer
    });
    let dir = tempfile::tempdir().unwrap();
    let mut engine = webdav_engine(&server, dir.path(), two_way());
    renamed_folder_scene(&mut engine, &dav);
    down.store(true, Ordering::SeqCst);
    engine.cycle(Deletes::Hold);
    {
        let dav = dav.lock().unwrap();
        assert!(!dav.has("Proj2") && dav.has("Proj/Sub/b.txt"), "{:?}", dav.folders());
        assert!(!dav.folders().iter().any(|f| f.starts_with(".craftcloud/trash")), "{:?}", dav.folders());
    }
    assert!(!writes(&server).iter().any(|w| !w.starts_with("MOVE ")), "{:?}", writes(&server));
    assert_eq!(synced_state(dir.path()).files.len(), 10);
    down.store(false, Ordering::SeqCst);
    let before = writes(&server).len();
    engine.cycle(Deletes::Hold);
    assert_folder_moved_whole(&engine, &server, &dav, before);
}

/// A folder move the server keeps answering 500 to is asked again for as long as it takes and
/// never rebuilt as a copy. The person is told after the first try (an error line), louder after
/// the third (a warning line, and the move counts as stuck: the window, the tray and a
/// notification follow that), and a last time after the fifth; the tries after that say nothing
/// more. The fourth try waits 2 minutes, the fifth and every later one 5. When the server is well
/// again the folder moves by one `MOVE` and the count is gone.
#[test]
fn webdav_a_folder_move_that_keeps_failing_is_told_three_times_spaced_out_and_never_given_up() {
    let dav: Dav =
        Arc::new(std::sync::Mutex::new(craftcloud_core::storage::probe::fake::FakeDav::new("/remote.php/dav/files/john", Flags::default())));
    let (inner, down) = (Arc::clone(&dav), Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let is_down = Arc::clone(&down);
    let server = FakeServer::start_full(move |seen: &Seen| {
        if seen.method == "MOVE" && is_down.load(Ordering::SeqCst) {
            return crate::fake_server::Answer::new(500, "locked");
        }
        let mut dav = inner.lock().unwrap_or_else(PoisonError::into_inner);
        let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
        let mut answer = crate::fake_server::Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
        for (name, value) in resp.headers {
            answer = answer.header(&name, value);
        }
        answer
    });
    let dir = tempfile::tempdir().unwrap();
    let mut engine = webdav_engine(&server, dir.path(), two_way());
    renamed_folder_scene(&mut engine, &dav);
    down.store(true, Ordering::SeqCst);

    // One pass; then: how often the server was asked so far, the lines said about the folder
    // (their kinds), and the try count shown. Every pass leaves the folder as it was.
    let pass = |engine: &mut Engine<WebDav>| {
        engine.cycle(Deletes::Hold);
        let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
        assert_eq!(p.state, SyncState::UpToDate, "{:?}", p.activity);
        let wrote = writes(&server);
        assert!(wrote.iter().all(|w| w.starts_with("MOVE ") && w.ends_with("/Proj/")), "nothing but the move is ever sent: {wrote:?}");
        {
            let dav = dav.lock().unwrap();
            assert!(!dav.has("Proj2") && dav.has("Proj/Sub/b.txt"), "never rebuilt as a copy: {:?}", dav.folders());
            assert!(!dav.folders().iter().any(|f| f.starts_with(".craftcloud/trash")), "{:?}", dav.folders());
        }
        let saved = synced_state(dir.path());
        assert!(saved.files.len() == 10 && saved.folders.contains("Proj/Sub"), "the state is kept: {:?}", saved.files.keys());
        assert!(p.pending_deletes.is_empty() && p.unsynced.contains_key("Proj2"));
        let said: Vec<ActivityKind> = p.activity.iter().filter(|e| e.path == "Proj2").map(|e| e.kind).collect();
        assert_eq!(p.waiting_moves.len(), 1, "{:?}", p.waiting_moves);
        assert_eq!((p.waiting_moves[0].from.as_str(), p.waiting_moves[0].to.as_str()), ("Proj", "Proj2"));
        assert_eq!(p.stuck_move().is_some(), p.waiting_moves[0].attempts >= 3);
        (wrote.len(), said, p.waiting_moves[0].attempts)
    };
    use ActivityKind::{Error, Warning};
    let (minute, second) = (Duration::from_secs(60), Duration::from_secs(1));

    // Tries 1 to 3, one a pass: told after the first, a warning after the third.
    assert_eq!(pass(&mut engine), (1, vec![Error], 1));
    assert_eq!(pass(&mut engine), (2, vec![Error], 2));
    assert_eq!(pass(&mut engine), (3, vec![Error, Warning], 3));
    // Try 4 not before 2 minutes are over, however many passes run meanwhile.
    assert_eq!(pass(&mut engine), (3, vec![Error, Warning], 3));
    time_passes(2 * minute - second);
    assert_eq!(pass(&mut engine), (3, vec![Error, Warning], 3));
    time_passes(2 * second);
    assert_eq!(pass(&mut engine), (4, vec![Error, Warning], 4));
    // Try 5 not before 5 more minutes: the last notice.
    time_passes(5 * minute - 2 * second);
    assert_eq!(pass(&mut engine), (4, vec![Error, Warning], 4));
    time_passes(3 * second);
    assert_eq!(pass(&mut engine), (5, vec![Error, Warning, Warning], 5));
    let last = activity_of(&engine, "Proj2").pop().unwrap();
    assert!(last.contains("for as long as it takes"), "{last}");
    // Tries 6 and 7: still asked, every 5 minutes, and nothing more is said.
    assert_eq!(pass(&mut engine), (5, vec![Error, Warning, Warning], 5));
    time_passes(5 * minute + second);
    assert_eq!(pass(&mut engine), (6, vec![Error, Warning, Warning], 6));
    time_passes(5 * minute + second);
    assert_eq!(pass(&mut engine), (7, vec![Error, Warning, Warning], 7));
    // "Sync now" asks at the next pass whatever the wait (what the engine's loop does on it).
    engine.move_tries.values_mut().for_each(|(_, at)| *at = None);
    assert_eq!(pass(&mut engine), (8, vec![Error, Warning, Warning], 8));

    // The server is well again: one `MOVE`, and the count is gone with it.
    down.store(false, Ordering::SeqCst);
    time_passes(5 * minute + second);
    let before = writes(&server).len();
    engine.cycle(Deletes::Hold);
    assert_folder_moved_whole(&engine, &server, &dav, before);
    assert!(engine.move_tries.is_empty(), "a success resets the count");
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(p.waiting_moves.is_empty() && p.stuck_move().is_none());

    // The same folder renamed again, the server down again: counted from one, told again.
    std::fs::rename(dir.path().join("Proj2"), dir.path().join("Proj3")).unwrap();
    down.store(true, Ordering::SeqCst);
    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.waiting_moves.iter().map(|m| (m.to.as_str(), m.attempts)).collect::<Vec<_>>(), [("Proj3", 1)]);
    assert_eq!(p.activity.iter().filter(|e| e.path == "Proj3").map(|e| e.kind).collect::<Vec<_>>(), [Error]);
}

/// A scripted WebDAV server that answers 500 to every `MOVE` while the switch is on.
fn server_that_cant_move() -> (FakeServer, Dav, Arc<std::sync::atomic::AtomicBool>) {
    let dav: Dav =
        Arc::new(std::sync::Mutex::new(craftcloud_core::storage::probe::fake::FakeDav::new("/remote.php/dav/files/john", Flags::default())));
    let (inner, down) = (Arc::clone(&dav), Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let is_down = Arc::clone(&down);
    let server = FakeServer::start_full(move |seen: &Seen| {
        if seen.method == "MOVE" && is_down.load(Ordering::SeqCst) {
            return crate::fake_server::Answer::new(500, "locked");
        }
        let mut dav = inner.lock().unwrap_or_else(PoisonError::into_inner);
        let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
        let mut answer = crate::fake_server::Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
        for (name, value) in resp.headers {
            answer = answer.header(&name, value);
        }
        answer
    });
    (server, dav, down)
}

/// The person goes on working in the renamed folder while its move waits: a file edited, one
/// deleted and one made there. The move stays planned (the folder is no longer "the same files
/// under a new name", but it is still under its old name in the cloud only and its new name here
/// only), nothing of it syncs meanwhile, the count goes on, and when the server is well the
/// folder moves by one `MOVE` and only then do the changes sync, inside it.
#[test]
fn webdav_a_folder_changed_here_while_its_move_waits_is_still_moved_as_one() {
    let (server, dav, down) = server_that_cant_move();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut engine = webdav_engine(&server, root, two_way());
    renamed_folder_scene(&mut engine, &dav);
    down.store(true, Ordering::SeqCst);
    engine.cycle(Deletes::Hold);

    let long_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    for (path, bytes) in [("Proj2/1.txt", "edited while waiting"), ("Proj2/new.txt", "made while waiting")] {
        let file = write(root, path, bytes);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(long_ago).unwrap();
    }
    std::fs::remove_file(root.join("Proj2/2.txt")).unwrap();
    for tries in 2..=3 {
        engine.cycle(Deletes::Hold);
        let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
        assert_eq!(p.waiting_moves.iter().map(|m| (m.from.as_str(), m.to.as_str(), m.attempts)).collect::<Vec<_>>(), [("Proj", "Proj2", tries)]);
        let wrote = writes(&server);
        assert!(wrote.iter().all(|w| w.starts_with("MOVE ") && w.ends_with("/Proj/")), "still only the move: {wrote:?}");
        let dav = dav.lock().unwrap();
        assert!(!dav.has("Proj2") && dav.body("Proj/2.txt") == Some(b"abc".as_slice()) && dav.body("Proj/1.txt") == Some(b"abc".as_slice()));
        assert_eq!(synced_state(root).files.len(), 10);
    }

    down.store(false, Ordering::SeqCst);
    time_passes(Duration::from_secs(121));
    let before = writes(&server).len();
    engine.cycle(Deletes::Hold);
    let wrote = writes(&server)[before..].to_vec();
    assert!(wrote[0].starts_with("MOVE ") && wrote[0].ends_with("/Proj/"), "the folder first, as one: {wrote:?}");
    assert_eq!(wrote.iter().filter(|w| w.starts_with("MOVE ") && w.ends_with("/john/Proj/")).count(), 1, "{wrote:?}");
    assert_eq!(wrote.iter().filter(|w| w.starts_with("PUT ")).count(), 2, "only the edited and the new file go up: {wrote:?}");
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(p.waiting_moves.is_empty() && engine.move_tries.is_empty(), "{:?}", p.activity);
    let dav = dav.lock().unwrap();
    assert!(!dav.has("Proj"), "{:?}", dav.folders());
    assert_eq!(dav.body("Proj2/1.txt"), Some(b"edited while waiting".as_slice()), "{:?}", p.activity);
    assert_eq!(dav.body("Proj2/new.txt"), Some(b"made while waiting".as_slice()));
    assert!(!dav.has("Proj2/2.txt"), "the delete made while waiting went through after the move");
    assert_eq!(dav.body("Proj2/Sub/b.txt"), Some(b"abc".as_slice()));
}

/// What the activity list says about `path`, oldest first.
fn activity_of(engine: &Engine<WebDav>, path: &str) -> Vec<String> {
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner);
    p.activity.iter().filter(|e| e.path == path).map(|e| e.detail.clone()).collect()
}

/// A server that has no `MOVE` (405) refuses for what it is, not for now: the folder still
/// syncs file by file, as before (nothing waits for ever on a server that will never move it).
#[test]
fn webdav_a_folder_move_refused_for_good_still_syncs_file_by_file() {
    let (server, dav) = scripted(Flags { no_move: true, ..Flags::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut engine = webdav_engine(&server, dir.path(), two_way());
    renamed_folder_scene(&mut engine, &dav);
    engine.cycle(Deletes::Confirm);
    let dav = dav.lock().unwrap();
    assert_eq!(dav.body("Proj2/Sub/b.txt"), Some(b"abc".as_slice()), "{:?}", writes(&server));
    assert_eq!(dav.body("Proj2/7.txt"), Some(b"abc".as_slice()));
    assert_eq!(engine.no_folder_move.len(), 1, "not asked again this run");
}

/// One renamed file: a `MOVE` the server can't do right now is an error tried again next pass.
/// It is not uploaded under the new name with the old one left to come down again (two files
/// where there was one).
#[test]
fn webdav_a_file_move_the_server_cant_do_right_now_is_tried_again() {
    let (server, dav) = scripted(Flags::default());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut engine = webdav_engine(&server, root, two_way());
    {
        let mut dav = dav.lock().unwrap();
        dav.add_folder("Docs", None);
        dav.add_file("Docs/a.txt", b"abc");
        dav.add_file("Docs/keep.txt", b"abcd");
    }
    let listed = engine.storage.list("Docs").unwrap();
    let etag = |path: &str| listed.iter().find(|e| e.path == path).unwrap().etag.clone().unwrap();
    let mut state = State::default();
    let renamed = write(root, "Docs/b.txt", "abc");
    state.files.insert("Docs/a.txt".into(), Known { etag: etag("Docs/a.txt"), ..as_synced(&renamed) });
    let keep = write(root, "Docs/keep.txt", "abcd");
    state.files.insert("Docs/keep.txt".into(), Known { etag: etag("Docs/keep.txt"), ..as_synced(&keep) });
    state.folders.insert("Docs".into());
    save_saved(&engine.state_path(), &Saved { server: engine.plan.server.clone(), state, ..Saved::default() }).unwrap();

    dav.lock().unwrap().flags.answers_once.push(("MOVE".into(), 500));
    engine.cycle(Deletes::Hold);
    assert_eq!(writes(&server).len(), 1, "only the move that failed: {:?}", writes(&server));
    assert!(dav.lock().unwrap().has("Docs/a.txt") && !dav.lock().unwrap().has("Docs/b.txt"));
    let saved = synced_state(root);
    assert!(saved.files.contains_key("Docs/a.txt") && !saved.files.contains_key("Docs/b.txt"), "{:?}", saved.files.keys());
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Error && e.path == "Docs/b.txt"), "{:?}", p.activity);

    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Moved && e.path == "Docs/b.txt"), "{:?}", p.activity);
    assert!(!writes(&server).iter().any(|w| w.starts_with("PUT ")), "{:?}", writes(&server));
    let dav = dav.lock().unwrap();
    assert!(!dav.has("Docs/a.txt") && dav.body("Docs/b.txt") == Some(b"abc".as_slice()));
    assert!(!root.join("Docs/a.txt").exists(), "the old name didn't come down again");
}

/// CI's Nextcloud run: a `PUT` into a folder made a moment before answered 500. That is an
/// error tried again next pass: the file is not marked refused (which would wait until it
/// changes), and nothing is tried twice within the pass.
#[test]
fn webdav_an_upload_answered_500_is_tried_again_next_pass_and_not_marked_refused() {
    let (server, dav) = scripted(Flags::default());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut engine = webdav_engine(&server, root, two_way());
    let file = write(root, "sf/a.txt", "inside");
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(std::time::SystemTime::now() - Duration::from_secs(3600)).unwrap();
    dav.lock().unwrap().flags.answers_once.push(("PUT".into(), 500));
    engine.cycle(Deletes::Hold);
    let puts = |server: &FakeServer| writes(server).iter().filter(|w| w.starts_with("PUT ")).count();
    assert_eq!(puts(&server), 1, "asked once in the pass: {:?}", writes(&server));
    assert!(engine.refused.is_empty() && engine.no_room.is_empty(), "an error, not a refusal");
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Error && e.path == "sf/a.txt"), "{:?}", p.activity);
    assert_eq!(p.unsynced.get("sf/a.txt"), Some(&craftcloud_core::sync::FileStatus::Error));
    assert!(!dav.lock().unwrap().has("sf/a.txt"));

    engine.cycle(Deletes::Hold);
    assert_eq!(puts(&server), 2);
    assert_eq!(dav.lock().unwrap().body("sf/a.txt"), Some(b"inside".as_slice()));
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(p.unsynced.is_empty() && p.activity.iter().any(|e| e.kind == ActivityKind::Up && e.path == "sf/a.txt"), "{:?}", p.activity);
}

/// Every file under `dir`.
fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() { out.extend(walkdir(&path)) } else { out.push(path) }
    }
    out
}
