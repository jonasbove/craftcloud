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
        platform: Platform::current(),
        parts_swept: None,
        watched: BTreeMap::new(),
        two_way_off: false,
        _watcher: None,
    }
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
        platform,
        parts_swept,
        watched,
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
        platform,
        parts_swept,
        watched,
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
/// keeps the writes away. The folder renamed here is moved in the cloud first thing, and the
/// file deleted in the cloud is removed here, not kept in a local trash.
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
    assert!(!root.join(".craftcloud/trash").exists(), "both ways, the cloud's trash keeps it");
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
        platform: Platform::current(),
        parts_swept: None,
        watched: BTreeMap::new(),
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

/// The runtime watch (docs/WEBDAV-STORAGE.md § 7.5): a create that comes back as someone else's
/// file (the server answered 201 yet kept the other file) stops two-way sync for the run — the
/// mode drops to download only, the probe re-runs, the activity list says so once, and nothing
/// more is written to the server afterwards in that session.
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
    assert!(done.is_ok(), "{done:?}");

    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.mode, Some(Mode::DownloadOnly), "two-way sync stopped: {:?}", p.activity);
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Warning && e.detail.contains("leave alone")), "{:?}", p.activity);
    assert!(server.seen().iter().any(|r| r.path.contains("/.craftcloud/probe/")), "the probe re-ran");

    // Nothing more is written to the server for the rest of the run.
    write(root, "Docs/another.txt", "x");
    let before = writes(&server).len();
    engine.cycle(Deletes::Hold);
    assert_eq!(writes(&server).len(), before, "download only now: nothing written: {:?}", writes(&server));
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
    assert!(engine.apply(&root_path, &change, &mut state, &mut folders, &Tree::default()).is_ok());

    engine.cycle(Deletes::Hold);
    let p = engine.progress.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(p.mode, Some(Mode::DownloadOnly), "{:?}", p.activity);
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Warning), "{:?}", p.activity);
    assert!(server.seen().iter().any(|r| r.path.contains("/.craftcloud/probe/")), "the probe re-ran");
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
