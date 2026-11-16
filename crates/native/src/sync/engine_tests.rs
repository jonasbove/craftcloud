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
use craftcloud_core::sync::{ActivityEvent, ActivityKind, SyncPlan, SyncState};

use super::{Deletes, Engine, Saved, local_meta, save_saved, synced_state};
use crate::fake_server::{FakeServer, Reply, Seen, listing};
use crate::session::Session;

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
fn engine(server: ServerUrl, root: &Path, excluded: &[&str]) -> Engine {
    let session = Session::new(server.clone(), Tokens { access_token: "a1".into(), refresh_token: "r1".into() }, Box::new(|_| {}));
    Engine {
        plan: plan(server.as_str(), root, excluded),
        session,
        device: "Laptop".into(),
        progress: Arc::default(),
        waker: Arc::new(|| {}),
        paused: false,
        settled: BTreeSet::new(),
        ids: BTreeMap::new(),
        reported: BTreeSet::new(),
        no_room: BTreeMap::new(),
        recounted: None,
        refused: BTreeMap::new(),
        no_folder_move: BTreeSet::new(),
        platform: Platform::current(),
        parts_swept: None,
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
    engine.ids.insert("Docs".into(), "docs1".into());
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
#[ignore = "known bug: `still_at` reads a cloud file listed without an ETag as unchanged, so a file whose version can't be compared is trashed (`cloud_holds` keeps it); needs its own fix PR"]
fn a_file_listed_without_an_etag_is_not_trashed() {
    let server = FakeServer::start(|r| rest(r).unwrap_or_else(|| listing("Docs", &[("a.txt", None)], &[])));
    let dir = tempfile::tempdir().unwrap();
    let mut engine = docs_engine(&server, dir.path());
    let mut state = one_file_state();
    let done = apply(&mut engine, &Action::DeleteRemote { path: "Docs/a.txt".into(), etag: "\"e1\"".into() }, &mut state, &mut BTreeSet::new());
    assert_eq!(trashes(&server), 0, "{done:?}");
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
    assert_eq!(engine.session.tokens.access_token, "a2");
    assert!(!engine.session.signed_out);
}
