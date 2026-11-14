//! Two-way sync against a real CraftCloud server, with two local folders playing two computers.
//! Skipped unless these are set (never in CI):
//!
//! ```text
//! CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8093 CRAFTCLOUD_TEST_USER=john \
//! CRAFTCLOUD_TEST_PASSWORD=… CRAFTCLOUD_HTTP_IDLE_SECS=5 \
//! cargo test -p craftcloud-native --test live -- --test-threads 1
//! ```
//!
//! On a fresh server it creates the owner with those details first. Use a throwaway container.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::{self, Tokens};
use craftcloud_core::api::resources::Kind;
use craftcloud_core::api::{admin, dav, favorites, folders, trash};
use craftcloud_core::sync::{ActivityKind, Keep, SyncPlan, SyncState};
use craftcloud_native::http::{ThreadBackend, execute};
use craftcloud_native::sync::{Command, Handle};

fn env() -> Option<(ServerUrl, String, String)> {
    let server = ServerUrl::parse(&std::env::var("CRAFTCLOUD_TEST_SERVER").ok()?).ok()?;
    Some((server, std::env::var("CRAFTCLOUD_TEST_USER").ok()?, std::env::var("CRAFTCLOUD_TEST_PASSWORD").ok()?))
}

/// Signs in once per test run (the server limits sign-ins per minute).
fn sign_in(server: &ServerUrl, user: &str, password: &str) -> Tokens {
    static TOKENS: std::sync::Mutex<Option<Tokens>> = std::sync::Mutex::new(None);
    let mut kept = TOKENS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(tokens) = kept.as_ref() {
        return tokens.clone();
    }
    let status = auth::parse_status(&execute(&auth::status(server)).unwrap()).unwrap();
    if !status.initialized {
        auth::parse_setup(&execute(&auth::setup(server, user, &format!("{user}@example.org"), password)).unwrap()).unwrap();
    }
    let tokens = auth::parse_login(&execute(&auth::login(server, user, password)).unwrap()).unwrap();
    *kept = Some(tokens.clone());
    tokens
}

fn computer(server: &ServerUrl, tokens: &Tokens, local: &Path, name: &str) -> Handle {
    // Setup makes the folder; the sync never does.
    std::fs::create_dir_all(local).unwrap();
    let plan =
        SyncPlan { server: server.to_string(), local_folder: local.display().to_string(), excluded: vec![], folders: vec![], sync_root_files: false };
    craftcloud_native::sync::start(plan, tokens.clone(), name.into(), Box::new(|_| {}), Arc::new(|| {}))
}

/// Runs one more cycle (one that starts after this call) and waits for it to finish.
fn sync(h: &Handle) {
    static TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = TICKET.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    h.send(Command::SyncFor(n));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let p = h.progress.lock().unwrap().clone();
        if p.served >= n {
            assert!(matches!(p.state, SyncState::UpToDate), "{:?}: {:?}", p.state, p.activity);
            return;
        }
        assert!(Instant::now() < deadline, "no sync finished: {p:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Writes a file and waits until it counts as settled (the client waits 2 s after a write).
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
}

/// The folder's counts after a cycle: `files` synced with `bytes`, nothing pending.
fn counted(h: &Handle, folder: &str, files: u64, bytes: u64) {
    let p = h.progress.lock().unwrap();
    let stats = p.folders.iter().find(|f| f.folder == folder).cloned().unwrap_or_else(|| panic!("{folder} isn't counted: {:?}", p.folders));
    assert_eq!((stats.files, stats.bytes, stats.errors, stats.syncing), (files, bytes, 0, 0));
    assert!(p.unsynced.is_empty(), "nothing is left syncing after a cycle: {:?}", p.unsynced);
}

/// The trash id of the newest trashed item that was at `cloud_path`.
fn in_trash(server: &ServerUrl, tokens: &Tokens, cloud_path: &str) -> Option<String> {
    let page = trash::parse_list(&execute(&tokens.authorize(trash::list(server, Some(200), None))).unwrap()).unwrap();
    page.items.iter().rev().find(|i| i.resource.cloud_path().as_deref() == Some(cloud_path)).map(|i| i.resource.id().to_owned())
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn two_computers_stay_in_sync() {
    let Some((server, user, password)) = env() else {
        eprintln!("skipped: CRAFTCLOUD_TEST_SERVER not set");
        return;
    };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    sync(&a);
    sync(&b);

    // A new file on A reaches B.
    write(&a_root.join("Shared/notes.md"), "one");
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("Shared/notes.md")), "one");
    counted(&b, "Shared", 1, 3);

    // An edit on A reaches B, and the cloud keeps the previous version.
    write(&a_root.join("Shared/notes.md"), "two");
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("Shared/notes.md")), "two");
    let versions = execute(&tokens.authorize(dav::list(&server, &dav::versions_folder("Shared/notes.md")))).unwrap();
    let kept: Vec<_> = dav::parse_list(&versions).unwrap().into_iter().filter(|e| !e.is_folder).collect();
    assert_eq!(kept.len(), 1, "{kept:?}");

    // Both edit the same file: one keeps its version as a conflict copy, on both sides.
    write(&a_root.join("Shared/notes.md"), "from A");
    write(&b_root.join("Shared/notes.md"), "from B, longer");
    sync(&a);
    sync(&b);
    sync(&a);
    assert_eq!(read(&a_root.join("Shared/notes.md")), "from A");
    assert_eq!(read(&b_root.join("Shared/notes.md")), "from A");
    let in_a = names_in(&a_root.join("Shared"));
    let in_b = names_in(&b_root.join("Shared"));
    assert_eq!(in_a, in_b);
    let copies: Vec<&String> = in_a.iter().filter(|n| n.contains("(conflict ")).collect();
    assert_eq!(copies.len(), 1, "{in_a:?}");
    assert!(copies[0].contains("Beta") && copies[0].ends_with(".md"), "{copies:?}");
    assert_eq!(read(&a_root.join("Shared").join(copies[0])), "from B, longer");

    // Settling it on A: "keep mine" (the copy's text) replaces the original everywhere.
    let conflict = a.progress.lock().unwrap().conflicts.clone();
    assert_eq!(conflict.len(), 1, "{conflict:?}");
    assert_eq!(conflict[0].original, "Shared/notes.md");
    a.send(Command::Resolve(conflict[0].clone(), Keep::Mine));
    sync(&a);
    sync(&b);
    assert_eq!(read(&a_root.join("Shared/notes.md")), "from B, longer");
    assert_eq!(read(&b_root.join("Shared/notes.md")), "from B, longer");
    assert!(names_in(&b_root.join("Shared")).iter().all(|n| !n.contains("(conflict ")), "{:?}", names_in(&b_root.join("Shared")));
    assert!(a.progress.lock().unwrap().conflicts.is_empty());

    // A delete on A removes it on B, and the cloud keeps it in the trash (never deleted for good).
    std::fs::remove_file(a_root.join("Shared/notes.md")).unwrap();
    sync(&a);
    sync(&b);
    assert!(!b_root.join("Shared/notes.md").exists());
    let trashed = in_trash(&server, &tokens, "Shared/notes.md").expect("the deleted file is in the cloud's trash");
    execute(&tokens.authorize(trash::restore(&server, &trashed))).unwrap();
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("Shared/notes.md")), "from B, longer", "restored from the trash, it syncs back");
    std::fs::remove_file(b_root.join("Shared/notes.md")).unwrap();
    sync(&b);
    sync(&a);
    assert!(!a_root.join("Shared/notes.md").exists());

    // An emptied folder goes to the trash too.
    write(&a_root.join("Shared/Old/gone.md"), "x");
    sync(&a);
    std::fs::remove_file(a_root.join("Shared/Old/gone.md")).unwrap();
    std::fs::remove_dir(a_root.join("Shared/Old")).unwrap();
    sync(&a);
    sync(&a);
    assert!(in_trash(&server, &tokens, "Shared/Old").is_some(), "the emptied folder is in the trash");
    let listed = execute(&tokens.authorize(dav::list(&server, "Shared"))).unwrap();
    assert!(dav::parse_list(&listed).unwrap().iter().all(|e| e.path != "Shared/Old"), "the empty folder is gone from the cloud");

    // The folder watcher: a change on A is synced without asking.
    let before = a.progress.lock().unwrap().cycles;
    std::fs::write(a_root.join("Shared/watched.md"), "seen").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while a.progress.lock().unwrap().cycles == before {
        assert!(Instant::now() < deadline, "the watcher didn't start a sync");
        std::thread::sleep(Duration::from_millis(100));
    }
    // (The first cycle may come before the 2 s settle; the next one uploads it.)
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("Shared/watched.md")), "seen");

    // A save in progress (CraftCloud's own temporary names, the Craft apps' .craft_tmp) is never uploaded.
    write(&a_root.join("Shared/.report.99-1.craft_tmp"), "half a save");
    sync(&a);
    sync(&b);
    assert!(!b_root.join("Shared/.report.99-1.craft_tmp").exists());

    // Switching a folder off on B removes B's copy; the cloud (and A) keep it. What B still has
    // there is reported, not silently skipped.
    b.send(Command::Replan(SyncPlan {
        server: server.to_string(),
        local_folder: b_root.display().to_string(),
        excluded: vec!["Shared".into()],
        folders: vec![],
        sync_root_files: false,
    }));
    sync(&b);
    assert!(!b_root.join("Shared/watched.md").exists());
    sync(&a);
    assert_eq!(read(&a_root.join("Shared/watched.md")), "seen");
    std::fs::create_dir_all(b_root.join("Shared")).unwrap();
    write(&b_root.join("Shared/local only.md"), "here");
    sync(&b);
    let skipped = b.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Skipped && e.path == "Shared");
    assert!(skipped, "the switched-off folder is reported");
    sync(&a);
    assert!(!a_root.join("Shared/local only.md").exists());
}

#[test]
fn folders_made_anywhere_join_the_sync() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    sync(&a);
    sync(&b);

    // Folders made on one computer join the sync (only folders switched off stay out), however
    // many files they hold and whatever their names.
    write(&a_root.join("Été/fichier.txt"), "x");
    for i in 0..300 {
        std::fs::write(a_root.join("Été").join(format!("{i:03}.txt")), i.to_string()).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("Été/fichier.txt")), "x");
    assert_eq!(names_in(&b_root.join("Été")).len(), 301);
    let ups = a.progress.lock().unwrap().activity.iter().filter(|e| e.path.starts_with("Été/")).count();
    // Each upload is in the activity list (which keeps the newest 200).
    assert_eq!(ups, craftcloud_core::sync::ACTIVITY_KEPT);

    // Moving a file into a new folder moves it everywhere (it is never lost on the way).
    std::fs::create_dir_all(b_root.join("sub/deep")).unwrap();
    std::fs::rename(b_root.join("Été/000.txt"), b_root.join("sub/deep/000.txt")).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&b);
    sync(&a);
    assert_eq!(read(&a_root.join("sub/deep/000.txt")), "0");
    assert!(!a_root.join("Été/000.txt").exists());

    // Moving one into a folder switched off on that computer keeps it in the cloud, and says so.
    b.send(Command::Replan(SyncPlan {
        server: server.to_string(),
        local_folder: b_root.display().to_string(),
        excluded: vec!["Private".into()],
        folders: vec![],
        sync_root_files: false,
    }));
    std::fs::create_dir_all(b_root.join("Private")).unwrap();
    std::fs::rename(b_root.join("Été/001.txt"), b_root.join("Private/001.txt")).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&b);
    sync(&a);
    assert_eq!(read(&a_root.join("Été/001.txt")), "1", "the cloud keeps it");
    let said = b.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Skipped && e.path == "Été/001.txt");
    assert!(said, "the move out of the synced folders is reported");
}

#[test]
fn renames_and_moves_are_moves_on_both_sides() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    write(&a_root.join("Moves/c1.txt"), "rename me");
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("Moves/c1.txt")), "rename me");
    let moved = |h: &Handle, path: &str| h.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Moved && e.path == path);

    // A rename here is one MOVE in the cloud (no upload, nothing in the trash) and a rename there.
    std::fs::rename(a_root.join("Moves/c1.txt"), a_root.join("Moves/c1-renamed.txt")).unwrap();
    sync(&a);
    assert!(moved(&a, "Moves/c1-renamed.txt"), "{:?}", a.progress.lock().unwrap().activity.last());
    assert!(in_trash(&server, &tokens, "Moves/c1.txt").is_none(), "nothing went to the trash");
    sync(&b);
    assert!(moved(&b, "Moves/c1-renamed.txt"), "{:?}", b.progress.lock().unwrap().activity.last());
    assert_eq!(read(&b_root.join("Moves/c1-renamed.txt")), "rename me");
    assert!(!b_root.join("Moves/c1.txt").exists());

    // A move into new folders, the same way.
    std::fs::create_dir_all(b_root.join("Moves/sub/deep")).unwrap();
    std::fs::rename(b_root.join("Moves/c1-renamed.txt"), b_root.join("Moves/sub/deep/c1-renamed.txt")).unwrap();
    sync(&b);
    assert!(moved(&b, "Moves/sub/deep/c1-renamed.txt"), "{:#?}", b.progress.lock().unwrap().activity.iter().rev().take(4).collect::<Vec<_>>());
    sync(&a);
    assert!(moved(&a, "Moves/sub/deep/c1-renamed.txt"));
    assert_eq!(read(&a_root.join("Moves/sub/deep/c1-renamed.txt")), "rename me");
    assert!(!a_root.join("Moves/c1-renamed.txt").exists());
}

/// The REST id of the top-level cloud folder `name`.
fn folder_id(server: &ServerUrl, tokens: &Tokens, name: &str) -> Option<String> {
    let roots = folders::parse_roots(&execute(&tokens.authorize(folders::roots(server))).unwrap()).unwrap();
    let root = roots.into_iter().find(|f| f.parent_id.is_none()).unwrap().id;
    let page = folders::parse_contents_all(&execute(&tokens.authorize(folders::contents(server, &root, Some(200), None))).unwrap()).unwrap();
    page.items.iter().find(|r| r.kind() == Kind::Folder && r.name() == name).map(|r| r.id().to_owned())
}

#[test]
fn a_renamed_folder_is_one_move_and_keeps_its_id() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    write(&a_root.join("Proj/a.txt"), "one");
    write(&a_root.join("Proj/Sub/b.txt"), "two");
    sync(&a);
    sync(&b);
    let id = folder_id(&server, &tokens, "Proj").expect("the folder is in the cloud");
    execute(&tokens.authorize(favorites::add(&server, Kind::Folder, &id))).unwrap();
    let moved = |h: &Handle, path: &str| h.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Moved && e.path == path);

    // Renamed on A: one MOVE in the cloud. The folder keeps its id and its star; nothing is trashed.
    std::fs::rename(a_root.join("Proj"), a_root.join("Proj2")).unwrap();
    sync(&a);
    assert!(moved(&a, "Proj2"), "{:#?}", a.progress.lock().unwrap().activity.iter().rev().take(4).collect::<Vec<_>>());
    assert_eq!(folder_id(&server, &tokens, "Proj2").as_deref(), Some(id.as_str()), "same folder, new name");
    assert_eq!(folder_id(&server, &tokens, "Proj"), None);
    let stars = favorites::parse_list_all(&execute(&tokens.authorize(favorites::list(&server, Some(200), None))).unwrap()).unwrap();
    assert!(stars.items.iter().any(|f| f.resource.id() == id), "still starred");
    assert!(in_trash(&server, &tokens, "Proj/a.txt").is_none() && in_trash(&server, &tokens, "Proj").is_none(), "nothing went to the trash");

    // B renames its local folder to match.
    sync(&b);
    assert!(moved(&b, "Proj2"));
    assert_eq!(read(&b_root.join("Proj2/Sub/b.txt")), "two");
    assert!(!b_root.join("Proj").exists());

    // B renames it while offline, A edits a file in it meanwhile: one folder in the end, holding the edit.
    std::fs::rename(b_root.join("Proj2"), b_root.join("Proj3")).unwrap();
    write(&a_root.join("Proj2/a.txt"), "edited on A");
    sync(&a);
    sync(&b);
    sync(&a);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join("Proj3/a.txt")), "edited on A", "{}", root.display());
        assert_eq!(read(&root.join("Proj3/Sub/b.txt")), "two");
        assert!(!root.join("Proj2").exists(), "{:?}", names_in(root));
    }
    assert_eq!(folder_id(&server, &tokens, "Proj3").as_deref(), Some(id.as_str()));
}

#[test]
fn a_deleted_folder_goes_to_the_trash_whole() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    // (Few enough files not to pause for a mass delete.)
    for i in 0..2 {
        std::fs::create_dir_all(a_root.join("Gone/Sub")).unwrap();
        std::fs::write(a_root.join(format!("Gone/{i}.txt")), i.to_string()).unwrap();
        std::fs::write(a_root.join(format!("Gone/Sub/{i}.txt")), i.to_string()).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("Gone/Sub/1.txt")), "1");

    // Deleted on A: one item in the trash (the folder), not one per file and folder.
    std::fs::remove_dir_all(a_root.join("Gone")).unwrap();
    sync(&a);
    let trashed = in_trash(&server, &tokens, "Gone").expect("the folder is in the trash");
    assert!(in_trash(&server, &tokens, "Gone/0.txt").is_none() && in_trash(&server, &tokens, "Gone/Sub").is_none(), "only the folder");
    sync(&b);
    assert!(!b_root.join("Gone").exists());

    // Restored, it comes back whole.
    execute(&tokens.authorize(trash::restore(&server, &trashed))).unwrap();
    sync(&a);
    sync(&b);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join("Gone/Sub/1.txt")), "1");
        assert_eq!(read(&root.join("Gone/0.txt")), "0");
    }
}

#[test]
fn a_file_and_a_folder_of_one_name_both_survive() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    write(&a_root.join("t5/keep.txt"), "k");
    sync(&a);
    sync(&b);

    // A makes a file `t5/clash`, B (not synced meanwhile) a folder of that name.
    write(&a_root.join("t5/clash"), "a file");
    write(&b_root.join("t5/clash/inner.txt"), "in a folder");
    sync(&a);
    sync(&b);
    sync(&b);
    sync(&a);
    let copy = names_in(&b_root.join("t5")).into_iter().find(|n| n.starts_with("clash (conflict ")).expect("B's folder is set aside");
    assert!(copy.contains("Beta"), "{copy}");
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join("t5/clash")), "a file", "the cloud's file is kept");
        assert_eq!(read(&root.join("t5").join(&copy).join("inner.txt")), "in a folder");
    }
    let p = b.progress.lock().unwrap();
    assert_eq!(p.activity.iter().filter(|e| e.kind == ActivityKind::Conflict && e.path == "t5/clash").count(), 1, "said once");
    assert!(!p.activity.iter().any(|e| e.kind == ActivityKind::Error), "{:?}", p.activity);
    assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
}

/// T-052: a synced folder replaced by a file of its name on one computer (and a synced file by a
/// folder) is a replacement, not a conflict: the cloud and the other computer follow.
#[test]
fn a_folder_replaced_by_a_file_replaces_it_everywhere() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Laptop");
    let b = computer(&server, &tokens, &b_root, "Studio");
    write(&a_root.join("t17/sw/in.txt"), "in");
    write(&a_root.join("t17/sf"), "a file");
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join("t17/sw/in.txt")), "in");

    // On A: `rm -r t17/sw; echo x > t17/sw`, and the file `t17/sf` becomes a folder.
    std::fs::remove_dir_all(a_root.join("t17/sw")).unwrap();
    std::fs::remove_file(a_root.join("t17/sf")).unwrap();
    std::fs::create_dir_all(a_root.join("t17/sf")).unwrap();
    std::fs::write(a_root.join("t17/sf/a.txt"), "inside").unwrap();
    write(&a_root.join("t17/sw"), "x\n");
    sync(&a);
    assert!(in_trash(&server, &tokens, "t17/sw").is_some(), "the folder is one trash item");
    assert!(in_trash(&server, &tokens, "t17/sw/in.txt").is_none());
    sync(&b);
    sync(&a);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join("t17/sw")), "x\n", "{}", root.display());
        assert_eq!(read(&root.join("t17/sf/a.txt")), "inside", "{}", root.display());
        assert_eq!(names_in(&root.join("t17")), ["sf", "sw"], "no conflict copies");
    }
    for h in [&a, &b] {
        let p = h.progress.lock().unwrap();
        assert!(!p.activity.iter().any(|e| matches!(e.kind, ActivityKind::Error | ActivityKind::Conflict)), "{:?}", p.activity);
    }

    // And back, the other way round on B: the file becomes a folder again, the folder a file.
    std::fs::remove_file(b_root.join("t17/sw")).unwrap();
    std::fs::create_dir_all(b_root.join("t17/sw")).unwrap();
    std::fs::write(b_root.join("t17/sw/again.txt"), "again").unwrap();
    std::fs::remove_dir_all(b_root.join("t17/sf")).unwrap();
    write(&b_root.join("t17/sf"), "a file again");
    sync(&b);
    sync(&a);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join("t17/sw/again.txt")), "again", "{}", root.display());
        assert_eq!(read(&root.join("t17/sf")), "a file again", "{}", root.display());
        assert_eq!(names_in(&root.join("t17")), ["sf", "sw"], "no conflict copies");
    }
    for h in [&a, &b] {
        let p = h.progress.lock().unwrap();
        assert!(!p.activity.iter().any(|e| matches!(e.kind, ActivityKind::Error | ActivityKind::Conflict)), "{:?}", p.activity);
    }
}

#[test]
fn a_vanished_sync_folder_is_never_made_again_on_its_own() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("A");
    let a = computer(&server, &tokens, &root, "Alpha");
    write(&root.join("Vanish/a.txt"), "still here");
    sync(&a);

    // Moved away (or its drive unplugged): held, not made again, nothing deleted in the cloud.
    std::fs::rename(&root, dirs.path().join("Elsewhere")).unwrap();
    a.send(Command::SyncNow);
    let deadline = Instant::now() + Duration::from_secs(30);
    while a.progress.lock().unwrap().state != SyncState::FolderMissing {
        assert!(Instant::now() < deadline, "{:?}", a.progress.lock().unwrap().state);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!root.exists(), "the folder is not made again");
    let listed = execute(&tokens.authorize(dav::list(&server, "Vanish"))).unwrap();
    assert!(dav::parse_list(&listed).unwrap().iter().any(|e| e.path == "Vanish/a.txt"), "nothing deleted in the cloud");

    // Put back, it syncs again.
    std::fs::rename(dirs.path().join("Elsewhere"), &root).unwrap();
    sync(&a);
    assert_eq!(read(&root.join("Vanish/a.txt")), "still here");
}

/// The kept versions of a cloud file.
fn versions_of(server: &ServerUrl, tokens: &Tokens, path: &str) -> Vec<craftcloud_core::api::versions::Version> {
    use craftcloud_core::api::versions;
    versions::parse_list(&execute(&tokens.authorize(versions::list(server, path))).unwrap()).unwrap()
}

#[test]
fn versions_follow_renames_and_are_kept_once() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("A");
    let a = computer(&server, &tokens, &root, "Alpha");
    write(&root.join("Hist/doc.txt"), "one");
    sync(&a);
    write(&root.join("Hist/doc.txt"), "two");
    sync(&a);
    let kept = versions_of(&server, &tokens, "Hist/doc.txt");
    assert_eq!(kept.len(), 1);
    // T-019: the version says when its content was written, before it was replaced.
    let (modified, replaced) = (kept[0].modified_at().expect("the content's time is kept"), kept[0].replaced_at().unwrap());
    assert!(modified < replaced, "{modified} < {replaced}");

    // A rename takes the history along.
    std::fs::rename(root.join("Hist/doc.txt"), root.join("Hist/renamed.txt")).unwrap();
    sync(&a);
    assert_eq!(versions_of(&server, &tokens, "Hist/renamed.txt").len(), 1, "the history moved");
    assert!(versions_of(&server, &tokens, "Hist/doc.txt").is_empty(), "nothing left behind");

    // So does a folder rename.
    std::fs::rename(root.join("Hist"), root.join("Hist2")).unwrap();
    sync(&a);
    assert_eq!(versions_of(&server, &tokens, "Hist2/renamed.txt").len(), 1);
    assert!(versions_of(&server, &tokens, "Hist/renamed.txt").is_empty());

    // Keeping the same version twice (an overwrite tried again) adds nothing.
    let mut session = craftcloud_native::session::Session::new(server.clone(), tokens.clone(), Box::new(|_| {}));
    session.snapshot("Hist2/renamed.txt").unwrap();
    session.snapshot("Hist2/renamed.txt").unwrap();
    assert_eq!(versions_of(&server, &tokens, "Hist2/renamed.txt").len(), 2, "the current bytes once, not twice");
}

#[test]
fn a_file_too_big_for_the_quota_waits_without_holding_up_the_rest() {
    let Some((server, user, password)) = env() else { return };
    let owner = sign_in(&server, &user, &password);
    // A new user each run, so the test can run again on the same server.
    let name = format!("tiny{}", std::process::id());
    let created = execute(&owner.authorize(admin::create(
        &server,
        &name,
        &format!("{name}@example.org"),
        "correct-horse-tiny",
        Some(100_000),
        admin::Role::User,
    )))
    .unwrap();
    let tiny = admin::parse_create(&created).unwrap();
    let tokens = auth::parse_login(&execute(&auth::login(&server, &name, "correct-horse-tiny")).unwrap()).unwrap();
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("T");
    let t = computer(&server, &tokens, &root, "Tiny");
    sync(&t);

    std::fs::create_dir_all(root.join("Big")).unwrap();
    std::fs::write(root.join("Big/big.bin"), vec![b'x'; 200_000]).unwrap();
    write(&root.join("Big/small.txt"), "fits");
    sync(&t);
    {
        let p = t.progress.lock().unwrap();
        assert_eq!(p.no_room, ["Big/big.bin"], "{:?}", p.activity.last());
        assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Skipped && e.path == "Big/big.bin"));
        assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Up && e.path == "Big/small.txt"), "the rest syncs");
    }
    // Not tried again while there is no room: the next cycles have nothing to do.
    sync(&t);
    assert_eq!(t.progress.lock().unwrap().files_total, 0);

    // Room again: it goes up without being asked.
    execute(&owner.authorize(admin::set_quota(&server, &tiny.id, 1_000_000))).unwrap();
    sync(&t);
    let p = t.progress.lock().unwrap();
    assert!(p.no_room.is_empty(), "{:?}", p.activity.last());
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Up && e.path == "Big/big.bin"));
}

#[test]
fn room_freed_by_emptying_the_trash_is_used_once_counted() {
    let Some((server, user, password)) = env() else { return };
    let owner = sign_in(&server, &user, &password);
    let name = format!("roomy{}", std::process::id());
    let created = execute(&owner.authorize(admin::create(
        &server,
        &name,
        &format!("{name}@example.org"),
        "correct-horse-roomy",
        Some(100_000),
        admin::Role::User,
    )))
    .unwrap();
    admin::parse_create(&created).unwrap();
    let tokens = auth::parse_login(&execute(&auth::login(&server, &name, "correct-horse-roomy")).unwrap()).unwrap();
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("R");
    let r = computer(&server, &tokens, &root, "Roomy");
    std::fs::create_dir_all(root.join("Box")).unwrap();
    std::fs::write(root.join("Box/first.bin"), vec![b'1'; 70_000]).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&r);

    // Deleted, the trash emptied: the room is free, but the cloud still counts it.
    std::fs::remove_file(root.join("Box/first.bin")).unwrap();
    sync(&r);
    execute(&tokens.authorize(trash::empty(&server))).unwrap();
    std::fs::write(root.join("Box/second.bin"), vec![b'2'; 70_000]).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&r);
    {
        let p = r.progress.lock().unwrap();
        let said = p.activity.iter().find(|e| e.path == "Box/second.bin").map(|e| e.detail.clone()).unwrap_or_default();
        if p.no_room.is_empty() {
            eprintln!("this server counted the freed room at once");
            return;
        }
        assert!(said.contains("within a few minutes after emptying the trash"), "{said}");
    }
    // Counted again (an admin's client asks for it; here the owner), it goes up.
    let recount = execute(&owner.authorize(admin::trigger_job(&server, admin::USAGE_RECONCILE))).unwrap();
    assert!(recount.is_success(), "{}", recount.status);
    sync(&r);
    let p = r.progress.lock().unwrap();
    assert!(p.no_room.is_empty(), "{:?}", p.activity.last());
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Up && e.path == "Box/second.bin"));
}

#[test]
fn an_interrupted_download_resumes_where_it_stopped() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::create_dir_all(a_root.join("Big")).unwrap();
    std::fs::write(a_root.join("Big/movie.bin"), &bytes).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);

    // B got the first half before the connection dropped: its part file, named after the version.
    let listed = dav::parse_list(&execute(&tokens.authorize(dav::list(&server, "Big"))).unwrap()).unwrap();
    let etag = listed.iter().find(|e| e.path == "Big/movie.bin").and_then(|e| e.etag.clone()).unwrap();
    let version: String = etag.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    std::fs::create_dir_all(b_root.join("Big")).unwrap();
    std::fs::write(b_root.join(format!("Big/.movie.bin.{version}.craftcloud-part")), &bytes[..120_000]).unwrap();
    let b = computer(&server, &tokens, &b_root, "Beta");
    sync(&b);
    assert_eq!(std::fs::read(b_root.join("Big/movie.bin")).unwrap(), bytes, "whole and right");
    let p = b.progress.lock().unwrap();
    let down = p.activity.iter().find(|e| e.path == "Big/movie.bin").unwrap_or_else(|| panic!("{:#?}", p.activity));
    assert!(down.detail.starts_with("resumed at"), "{down:?}");
    assert!(names_in(&b_root.join("Big")).iter().all(|n| !n.ends_with(".craftcloud-part")), "no part file left");
}

/// A TCP forwarder to the test server that can go silent: connections stay open, but no byte
/// passes (a dropped route after a Wi-Fi change or a suspend), until it is let go again.
struct Forwarder {
    url: String,
    silent: Arc<std::sync::atomic::AtomicBool>,
}

impl Forwarder {
    fn to(server: &ServerUrl) -> Self {
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;
        let target = server.to_string().trim_start_matches("http://").trim_end_matches('/').to_owned();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let silent = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let quiet = Arc::clone(&silent);
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(upstream) = std::net::TcpStream::connect(&target) else { continue };
                for (mut from, mut to) in [(client.try_clone().unwrap(), upstream.try_clone().unwrap()), (upstream, client)] {
                    let quiet = Arc::clone(&quiet);
                    std::thread::spawn(move || {
                        let mut buf = [0u8; 16 * 1024];
                        loop {
                            if quiet.load(Ordering::SeqCst) {
                                std::thread::sleep(Duration::from_millis(50));
                                continue;
                            }
                            let _ = from.set_read_timeout(Some(Duration::from_millis(100)));
                            match from.read(&mut buf) {
                                Ok(0) => return,
                                Ok(n) => {
                                    if to.write_all(&buf[..n]).is_err() {
                                        return;
                                    }
                                }
                                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                                Err(_) => return,
                            }
                        }
                    });
                }
            }
        });
        Self { url, silent }
    }

    fn silent(&self, on: bool) {
        self.silent.store(on, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Waits (up to `limit`) until the sync's state is what `want` says.
fn until(h: &Handle, limit: Duration, want: impl Fn(&SyncState) -> bool) {
    let deadline = Instant::now() + limit;
    loop {
        let state = h.progress.lock().unwrap().state.clone();
        if want(&state) {
            return;
        }
        assert!(Instant::now() < deadline, "still {state:?} after {limit:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Needs a short idle limit: `CRAFTCLOUD_HTTP_IDLE_SECS=5` (skipped otherwise).
#[test]
fn a_connection_gone_silent_goes_offline_and_recovers_by_itself() {
    let Some((server, user, password)) = env() else { return };
    let idle = craftcloud_native::http::idle();
    if idle > Duration::from_secs(10) {
        eprintln!("skipped: set CRAFTCLOUD_HTTP_IDLE_SECS=5");
        return;
    }
    let tokens = sign_in(&server, &user, &password);
    let forwarder = Forwarder::to(&server);
    let via = ServerUrl::parse(&forwarder.url).unwrap();
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("S");
    let s = computer(&via, &tokens, &root, "Silent");
    write(&root.join("Quiet/before.txt"), "before");
    sync(&s);

    // The route drops while the client is idle: the kept-alive connections go silent.
    forwarder.silent(true);
    write(&root.join("Quiet/during.txt"), "during");
    s.send(Command::SyncNow);
    until(&s, idle * 3 + Duration::from_secs(10), |st| matches!(st, SyncState::Offline(_)));
    let SyncState::Offline(said) = s.progress.lock().unwrap().state.clone() else { unreachable!() };
    assert_eq!(said, craftcloud_native::sync::OFFLINE, "plain words, not the raw error");

    // Pause takes effect while offline, and resume.
    s.send(Command::Pause(true));
    until(&s, idle * 2 + Duration::from_secs(5), |st| *st == SyncState::Paused);
    s.send(Command::Pause(false));

    // The network is back: it syncs again by itself (no restart, no command).
    forwarder.silent(false);
    until(&s, craftcloud_native::sync::INTERVAL + idle * 3 + Duration::from_secs(10), |st| *st == SyncState::UpToDate);
    let listed = execute(&tokens.authorize(dav::list(&server, "Quiet"))).unwrap();
    assert!(dav::parse_list(&listed).unwrap().iter().any(|e| e.path == "Quiet/during.txt"), "the change made offline went up");
}

#[test]
fn names_the_cloud_cant_take_are_said_once_and_never_sent() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("N");
    let n = computer(&server, &tokens, &root, "Names");
    write(&root.join("Names/back\\slash.txt"), "never sent");
    write(&root.join("Names/a:b.txt"), "fine here, not on Windows");
    write(&root.join("Names/ok.txt"), "ok");
    sync(&n);
    sync(&n);
    sync(&n);
    let listed = dav::parse_list(&execute(&tokens.authorize(dav::list(&server, "Names"))).unwrap()).expect("the listing is whole");
    let names: Vec<&str> = listed.iter().map(dav::Entry::name).collect();
    assert!(names.contains(&"ok.txt") && names.contains(&"a:b.txt"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains('\\')), "{names:?}");
    let p = n.progress.lock().unwrap();
    let about = |path: &str, kind: ActivityKind| p.activity.iter().filter(|e| e.path == path && e.kind == kind).count();
    assert_eq!(about("Names/back\\slash.txt", ActivityKind::Skipped), 1, "said once: {:?}", p.activity);
    assert_eq!(about("Names/a:b.txt", ActivityKind::Warning), 1);
    assert_eq!(p.not_synced, ["Names/back\\slash.txt"], "never just \"up to date\"");
    assert_eq!(p.left_out.get("Names/back\\slash.txt"), Some(&craftcloud_core::sync::LeftOut::Name), "left out for good, not a hold");
    assert!(!p.activity.iter().any(|e| e.kind == ActivityKind::Error), "{:?}", p.activity);
}

#[test]
fn a_deleted_nested_or_renamed_folder_is_one_trash_item() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("A");
    let a = computer(&server, &tokens, &root, "Alpha");
    for dir in ["t11/top", "t11/top/a", "t11/top/a/x", "t11/top/b", "t9/fold"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
        for f in ["f1", "f2", "f3"] {
            std::fs::write(root.join(dir).join(f), format!("{dir}/{f}")).unwrap();
        }
    }
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    std::fs::rename(root.join("t9/fold"), root.join("t9/fold-ren")).unwrap();
    sync(&a);

    // Nested: 12 files in four folders, deleted at once (confirmed, it's a mass delete).
    std::fs::remove_dir_all(root.join("t11/top")).unwrap();
    a.send(Command::ConfirmDeletes);
    sync(&a);
    assert!(in_trash(&server, &tokens, "t11/top").is_some(), "the folder is in the trash");
    for inner in ["t11/top/f1", "t11/top/a", "t11/top/a/x", "t11/top/a/x/f1", "t11/top/b"] {
        assert!(in_trash(&server, &tokens, inner).is_none(), "{inner} is not a trash item of its own");
    }

    // A folder renamed earlier: the same.
    std::fs::remove_dir_all(root.join("t9/fold-ren")).unwrap();
    sync(&a);
    assert!(in_trash(&server, &tokens, "t9/fold-ren").is_some());
    assert!(in_trash(&server, &tokens, "t9/fold-ren/f1").is_none());
}

#[test]
fn the_same_bytes_written_again_are_not_uploaded_again() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    write(&a_root.join("Same/up.txt"), "base\n");
    sync(&a);
    sync(&b);
    let ups = |h: &Handle, path: &str| h.progress.lock().unwrap().activity.iter().filter(|e| e.kind == ActivityKind::Up && e.path == path).count();

    // Uploaded from A, then written again with the same bytes: no upload, no version.
    write(&a_root.join("Same/up.txt"), "base\n");
    sync(&a);
    assert_eq!(ups(&a, "Same/up.txt"), 1);
    assert!(versions_of(&server, &tokens, "Same/up.txt").is_empty());
    // Downloaded on B, the same.
    write(&b_root.join("Same/up.txt"), "base\n");
    sync(&b);
    assert_eq!(ups(&b, "Same/up.txt"), 0);
    // Other bytes still go up.
    write(&b_root.join("Same/up.txt"), "BASE\n");
    sync(&b);
    assert_eq!(ups(&b, "Same/up.txt"), 1);
}

#[test]
fn an_edit_made_offline_follows_a_rename_made_elsewhere() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (dirs.path().join("A"), dirs.path().join("B"));
    let a = computer(&server, &tokens, &a_root, "Alpha");
    let b = computer(&server, &tokens, &b_root, "Beta");
    write(&a_root.join("Follow/f.txt"), "first");
    write(&a_root.join("Follow/g.txt"), "second");
    sync(&a);
    sync(&b);

    // A renames f while B (not syncing) edits it: one file in the end, renamed, with the edit.
    std::fs::rename(a_root.join("Follow/f.txt"), a_root.join("Follow/renamed-A.txt")).unwrap();
    sync(&a);
    write(&b_root.join("Follow/f.txt"), "edited on B");
    sync(&b);
    sync(&b);
    sync(&a);
    // And the other way round: B renames g while A edits it.
    std::fs::rename(b_root.join("Follow/g.txt"), b_root.join("Follow/renamed-B.txt")).unwrap();
    write(&a_root.join("Follow/g.txt"), "edited on A");
    sync(&a);
    sync(&b);
    sync(&b);
    sync(&a);
    for root in [&a_root, &b_root] {
        assert_eq!(names_in(&root.join("Follow")), ["renamed-A.txt", "renamed-B.txt"], "{}", root.display());
        assert_eq!(read(&root.join("Follow/renamed-A.txt")), "edited on B");
        assert_eq!(read(&root.join("Follow/renamed-B.txt")), "edited on A");
    }
}

/// T-038: a mass delete made in the cloud and discarded here comes back from the cloud's trash
/// (the same folder, its id kept), not uploaded again as new files.
#[test]
fn discarded_cloud_deletes_come_back_from_the_trash() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("A");
    std::fs::create_dir_all(root.join("t38/data")).unwrap();
    std::fs::write(root.join("t38/keep.txt"), "keep").unwrap();
    for i in 0..12 {
        std::fs::write(root.join(format!("t38/data/f{i}.txt")), format!("file {i}")).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    let plan = SyncPlan {
        server: server.to_string(),
        local_folder: root.display().to_string(),
        excluded: vec![],
        folders: vec!["t38".into()],
        sync_root_files: false,
    };
    let a = craftcloud_native::sync::start(plan, tokens.clone(), "Alpha".into(), Box::new(|_| {}), Arc::new(|| {}));
    sync(&a);
    let top = folder_id(&server, &tokens, "t38").unwrap();
    let data_id = || {
        let page = folders::parse_contents_all(&execute(&tokens.authorize(folders::contents(&server, &top, Some(200), None))).unwrap()).unwrap();
        page.items.iter().find(|r| r.kind() == Kind::Folder && r.name() == "data").map(|r| r.id().to_owned())
    };
    let id = data_id().unwrap();

    // Deleted in the cloud: 12 of 13 files would go here too, so the sync holds them.
    assert!(execute(&tokens.authorize(folders::trash(&server, &id))).unwrap().is_success());
    a.send(Command::SyncNow);
    until(&a, Duration::from_secs(60), |s| matches!(s, SyncState::MassDelete { .. }));
    a.send(Command::DiscardDeletes);
    sync(&a);

    assert_eq!(data_id().as_deref(), Some(id.as_str()), "the same folder is back");
    assert!(in_trash(&server, &tokens, "t38/data").is_none(), "nothing left in the trash");
    let p = a.progress.lock().unwrap();
    let kept = p.activity.iter().position(|e| e.detail.contains("12 put back from the cloud's trash")).expect("said what came back");
    assert!(!p.activity[kept..].iter().any(|e| e.kind == ActivityKind::Up), "nothing uploaded as new: {:?}", &p.activity[kept..]);
    assert_eq!(names_in(&root.join("t38/data")).len(), 12, "nothing deleted here");
}

/// T-042: a folder deleted here and held as a mass delete isn't made again (empty) by the passes
/// while it waits, and once confirmed it goes to the trash as one item.
#[test]
fn a_held_folder_delete_is_one_trash_item_when_confirmed() {
    let Some((server, user, password)) = env() else { return };
    let tokens = sign_in(&server, &user, &password);
    let dirs = tempfile::tempdir().unwrap();
    let root = dirs.path().join("A");
    std::fs::create_dir_all(root.join("t42/many")).unwrap();
    std::fs::write(root.join("t42/keep.txt"), "keep").unwrap();
    for i in 0..20 {
        std::fs::write(root.join(format!("t42/many/f{i}.txt")), format!("file {i}")).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    let plan = SyncPlan {
        server: server.to_string(),
        local_folder: root.display().to_string(),
        excluded: vec![],
        folders: vec!["t42".into()],
        sync_root_files: false,
    };
    let a = craftcloud_native::sync::start(plan, tokens.clone(), "Alpha".into(), Box::new(|_| {}), Arc::new(|| {}));
    sync(&a);

    std::fs::remove_dir_all(root.join("t42/many")).unwrap();
    for _ in 0..2 {
        // Held, and held again: the pass in between must not bring the folder back here.
        let cycles = a.progress.lock().unwrap().cycles;
        a.send(Command::SyncNow);
        until(&a, Duration::from_secs(60), |s| matches!(s, SyncState::MassDelete { .. }));
        while a.progress.lock().unwrap().cycles == cycles {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!root.join("t42/many").exists(), "not made again while held");
    }
    a.send(Command::ConfirmDeletes);
    sync(&a);
    assert!(!root.join("t42/many").exists());
    assert!(in_trash(&server, &tokens, "t42/many").is_some(), "the folder is one trash item");
    assert!(in_trash(&server, &tokens, "t42/many/f0.txt").is_none(), "its files are not trash items of their own");
    let p = a.progress.lock().unwrap();
    let deleted: Vec<_> = p.activity.iter().filter(|e| e.kind == ActivityKind::Deleted).collect();
    assert_eq!(deleted.len(), 1, "one row for the folder: {deleted:?}");
    assert_eq!(deleted[0].path, "t42/many");
}

#[test]
fn the_ui_transport_answers_on_its_thread() {
    let Some((server, _, _)) = env() else { return };
    let mut backend = ThreadBackend::new(Arc::new(|| {}));
    backend.send(7, auth::status(&server));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some((id, result)) = backend.poll().pop() {
            assert_eq!(id, 7);
            assert!(auth::parse_status(&result.unwrap()).is_ok());
            return;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
}
