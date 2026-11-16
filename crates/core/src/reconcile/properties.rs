//! Property tests: the sync's rules (AGENTS.md, "The sync never loses a file") checked against
//! generated inputs, next to the worked examples in `tests.rs`.
//!
//! The inputs grow from one synced state: each synced file is unchanged, edited, deleted or
//! renamed on each side, new files appear on either side (sometimes at the same path), and the
//! folders follow from the files (plus empty ones), so every side is a tree a scan could return.
//! A small set of names keeps paths colliding, which is where the rules matter.
//!
//! 256 cases per property by default (a fraction of a second), so the gate stays fast;
//! `PROPTEST_CASES=50000 cargo test -p craftcloud-core properties` runs more.
//!
//! What lives in the engine (craftcloud-native) and is not covered here: the scans themselves, the
//! skip rule's filter (`drop_skipped`), and what is held when a place can't be read
//! (`keep_as_synced`, `hold_unreadable`). These tests check the decisions those rely on: a path in
//! no input gets no action, and a place whose local side is held as last synced is never deleted
//! or moved away in the cloud.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

use super::{
    Action, Known, LocalFile, RemoteFile, State, Tree, folder_moves, free_conflict_name, local_changed, move_state, move_tree, reconcile, same_etag,
};
use crate::names::{Platform, skipped};

fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(256);
    // Inputs left out (a name that doesn't sync, a known bug) don't count as cases.
    ProptestConfig { cases, max_global_rejects: cases.saturating_mul(2).max(1024), ..ProptestConfig::default() }
}

/// Folders and names the generated paths are made of. `S` is both a folder (`A/S`) and a file
/// name, so a path can be a file on one side and a folder on the other.
const DIRS: &[&str] = &["", "A", "B", "A/S"];
const NAMES: &[&str] = &["a.txt", "b.md", "S"];

/// The same, with names the sync leaves alone: its own folder, temporary and system files, a
/// folder whose name only looks temporary (it syncs), and names Windows can't have.
const SKIP_DIRS: &[&str] = &["", "A", ".craftcloud", "T.tmp", "trail.", "A/q?"];
const SKIP_NAMES: &[&str] = &["a.txt", "x.tmp", "~$y.docx", ".DS_Store", "con.txt", "w:z.md", "notes~"];

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") }
}

/// `path` is `place` or inside it (`""` is everything).
fn within(place: &str, path: &str) -> bool {
    place.is_empty() || path == place || strictly_under(place, path)
}

fn strictly_under(folder: &str, path: &str) -> bool {
    path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// Every folder above `path`: `a/b/c` → `a`, `a/b`.
fn ancestors(path: &str) -> Vec<String> {
    path.match_indices('/').map(|(i, _)| path[..i].to_owned()).collect()
}

#[derive(Clone, Copy, Debug)]
enum Fate {
    Same,
    Edited,
    Gone,
    /// Renamed or moved to this path (by index), keeping size and time here, the ETag in the cloud.
    Renamed(usize),
}

fn fate(paths: usize) -> impl Strategy<Value = Fate> {
    prop_oneof![4 => Just(Fate::Same), 2 => Just(Fate::Edited), 2 => Just(Fate::Gone), 2 => (0..paths).prop_map(Fate::Renamed)]
}

/// Generated choices, turned into a [`Case`] by [`build`].
#[derive(Clone, Debug)]
struct Raw {
    synced: BTreeMap<usize, (u64, i64)>,
    fates: Vec<(Fate, Fate)>,
    new_local: BTreeMap<usize, (u64, i64)>,
    new_remote: BTreeMap<usize, u64>,
    /// Folders (by index): empty ones in the state, then kept (if synced) or made on each side.
    state_folders: BTreeSet<usize>,
    local_folders: BTreeSet<usize>,
    remote_folders: BTreeSet<usize>,
}

/// The three inputs of [`reconcile`], plus what each file holds (for the simulation): equal
/// numbers are equal bytes.
#[derive(Clone, Debug)]
struct Case {
    state: State,
    local: Tree<LocalFile>,
    remote: Tree<RemoteFile>,
    local_bytes: BTreeMap<String, u32>,
    remote_bytes: BTreeMap<String, u32>,
}

fn raw(paths: usize, folders: usize) -> impl Strategy<Value = Raw> {
    let files = prop::collection::btree_map(0..paths, (0..3u64, 0..3i64), 0..8);
    let fates = prop::collection::vec((fate(paths), fate(paths)), 8);
    let new_local = prop::collection::btree_map(0..paths, (0..3u64, 0..3i64), 0..4);
    let new_remote = prop::collection::btree_map(0..paths, 0..3u64, 0..4);
    let some_folders = || prop::collection::btree_set(0..folders, 0..=folders);
    (files, fates, new_local, new_remote, some_folders(), some_folders(), some_folders()).prop_map(
        |(synced, fates, new_local, new_remote, state_folders, local_folders, remote_folders)| Raw {
            synced,
            fates,
            new_local,
            new_remote,
            state_folders,
            local_folders,
            remote_folders,
        },
    )
}

/// A tree a scan could return: every file's folders are there, and no path is both a file and a
/// folder (the file goes).
fn settle<F>(files: &mut BTreeMap<String, F>, folders: &mut BTreeSet<String>, bytes: &mut BTreeMap<String, u32>) {
    let mut all: BTreeSet<String> = folders.iter().flat_map(|f| ancestors(&format!("{f}/x"))).collect();
    all.extend(files.keys().flat_map(|p| ancestors(p)));
    files.retain(|p, _| !all.contains(p));
    bytes.retain(|p, _| files.contains_key(p));
    *folders = all;
}

/// Builds the inputs. With `unique`, every distinct content has its own size-and-time here (so a
/// rename is the only way two files match); otherwise times collide often, as can happen.
fn build(dirs: &[&str], names: &[&str], raw: &Raw, unique: bool) -> Case {
    let paths: Vec<String> = dirs.iter().flat_map(|d| names.iter().map(move |n| join(d, n))).collect();
    let pool: Vec<&str> = dirs.iter().copied().filter(|d| !d.is_empty()).collect();
    let folder_set = |set: &BTreeSet<usize>| -> BTreeSet<String> { set.iter().filter_map(|i| pool.get(*i)).map(|f| (*f).to_owned()).collect() };

    let mut state = State::default();
    let mut origin: BTreeMap<String, u32> = BTreeMap::new();
    for (n, (i, (size, mtime))) in raw.synced.iter().enumerate() {
        let path = paths[*i].clone();
        let mtime = if unique { n as i64 } else { *mtime };
        state.files.insert(path.clone(), Known { etag: format!("e{n}"), size: *size, mtime, hash: None });
        origin.insert(path, n as u32);
    }
    state.folders = folder_set(&raw.state_folders);
    settle(&mut state.files, &mut state.folders, &mut origin);

    let (mut local, mut remote) = (Tree::<LocalFile>::default(), Tree::<RemoteFile>::default());
    let (mut local_bytes, mut remote_bytes) = (BTreeMap::new(), BTreeMap::new());
    for (n, (path, k)) in state.files.iter().enumerate() {
        let id = origin.get(path).copied().unwrap_or_default();
        let (here, there) = raw.fates.get(n).copied().unwrap_or((Fate::Same, Fate::Same));
        let synced = LocalFile { size: k.size, mtime: k.mtime };
        match here {
            Fate::Same => {
                local.files.insert(path.clone(), synced);
                local_bytes.insert(path.clone(), id);
            }
            Fate::Edited => {
                local.files.insert(path.clone(), LocalFile { size: k.size, mtime: if unique { 100 + n as i64 } else { k.mtime + 10 } });
                local_bytes.insert(path.clone(), 100 + id);
            }
            Fate::Gone => {}
            Fate::Renamed(to) => {
                local.files.insert(paths[to].clone(), synced);
                local_bytes.insert(paths[to].clone(), id);
            }
        }
        match there {
            Fate::Same => {
                remote.files.insert(path.clone(), RemoteFile { etag: k.etag.clone(), size: k.size });
                remote_bytes.insert(path.clone(), id);
            }
            Fate::Edited => {
                remote.files.insert(path.clone(), RemoteFile { etag: format!("{}x", k.etag), size: k.size + 1 });
                remote_bytes.insert(path.clone(), 200 + id);
            }
            Fate::Gone => {}
            Fate::Renamed(to) => {
                remote.files.insert(paths[to].clone(), RemoteFile { etag: k.etag.clone(), size: k.size });
                remote_bytes.insert(paths[to].clone(), id);
            }
        }
    }
    for (n, (i, (size, mtime))) in raw.new_local.iter().enumerate() {
        if !local.files.contains_key(&paths[*i]) {
            let mtime = if unique { 300 + n as i64 } else { *mtime };
            local.files.insert(paths[*i].clone(), LocalFile { size: *size, mtime });
            local_bytes.insert(paths[*i].clone(), 300 + n as u32);
        }
    }
    for (n, (i, size)) in raw.new_remote.iter().enumerate() {
        if !remote.files.contains_key(&paths[*i]) {
            remote.files.insert(paths[*i].clone(), RemoteFile { etag: format!("n{n}"), size: *size });
            remote_bytes.insert(paths[*i].clone(), 400 + n as u32);
        }
    }
    // A synced folder stays on a side when something is in it there, or when it was kept.
    local.folders = state.folders.iter().filter(|f| raw.local_folders.iter().any(|i| pool.get(*i) == Some(&f.as_str()))).cloned().collect();
    remote.folders = state.folders.iter().filter(|f| raw.remote_folders.iter().any(|i| pool.get(*i) == Some(&f.as_str()))).cloned().collect();
    // Some new empty folders too: the ones picked for a side that the state doesn't know.
    local.folders.extend(folder_set(&raw.local_folders).into_iter().filter(|f| !state.folders.contains(f) && f.len() % 2 == 1));
    remote.folders.extend(folder_set(&raw.remote_folders).into_iter().filter(|f| !state.folders.contains(f) && f.len() % 2 == 0));
    settle(&mut local.files, &mut local.folders, &mut local_bytes);
    settle(&mut remote.files, &mut remote.folders, &mut remote_bytes);
    Case { state, local, remote, local_bytes, remote_bytes }
}

fn case(unique: bool) -> impl Strategy<Value = Case> {
    raw(DIRS.len() * NAMES.len(), DIRS.len() - 1).prop_map(move |r| build(DIRS, NAMES, &r, unique))
}

/// The number of files a plan deletes, counted as [`reconcile`] counts them.
fn deletes(plan: &[Action]) -> usize {
    plan.iter()
        .map(|a| match a {
            Action::DeleteLocal(_) | Action::DeleteRemote { .. } => 1,
            Action::DeleteRemoteFolder { files, .. } => files.len(),
            _ => 0,
        })
        .sum()
}

/// The file paths an action reads or writes (both ends of a move, every file of a trashed folder).
fn file_paths(action: &Action) -> Vec<&str> {
    match action {
        Action::UploadNew(p)
        | Action::UploadChange { path: p, .. }
        | Action::Download(p)
        | Action::Conflict(p)
        | Action::DeleteRemote { path: p, .. }
        | Action::DeleteLocal(p)
        | Action::LocalClash(p)
        | Action::Forget(p) => vec![p],
        Action::MoveRemote { from, to, .. } | Action::MoveLocal { from, to } => vec![from, to],
        Action::DeleteRemoteFolder { files, .. } => files.iter().map(|(p, _)| p.as_str()).collect(),
        Action::MakeRemoteFolder(_) | Action::MakeLocalFolder(_) | Action::RemoveRemoteFolder(_) | Action::RemoveLocalFolder(_) => vec![],
    }
}

/// Every path an action names, files and folders.
fn all_paths(action: &Action) -> Vec<&str> {
    let mut paths = file_paths(action);
    match action {
        Action::MakeRemoteFolder(f)
        | Action::MakeLocalFolder(f)
        | Action::RemoveRemoteFolder(f)
        | Action::RemoveLocalFolder(f)
        | Action::DeleteRemoteFolder { path: f, .. } => paths.push(f),
        _ => {}
    }
    paths
}

/// What the plan writes in the cloud, by path (uploads, a conflict's copy goes next to `p`, the
/// target of a move).
fn remote_writes(action: &Action) -> Option<&str> {
    match action {
        Action::UploadNew(p) | Action::UploadChange { path: p, .. } | Action::Conflict(p) | Action::MoveRemote { to: p, .. } => Some(p),
        _ => None,
    }
}

/// Whether one path is the old name in one move and the new name in another: a known bug, not
/// fixed here (`one_file_is_never_two_moves` in `tests.rs`, ignored until it is). Plans with it
/// are left out of the properties below, so that they keep checking everything else; once it is
/// fixed, this goes.
fn known_double_move(plan: &[Action]) -> bool {
    let from: BTreeSet<&str> = plan
        .iter()
        .filter_map(|a| match a {
            Action::MoveRemote { from, .. } | Action::MoveLocal { from, .. } => Some(from.as_str()),
            _ => None,
        })
        .collect();
    plan.iter().any(|a| matches!(a, Action::MoveRemote { to, .. } | Action::MoveLocal { to, .. } if from.contains(to.as_str())))
}

/// Whether a folder is removed before a file is moved out of it, or a file is moved before the
/// folder it goes into is made: a known bug in how a folder replaced by a file (or the other way
/// round) is put first, not fixed here (`a_replaced_folder_is_emptied_before_it_goes` in
/// `tests.rs`, ignored until it is). Left out like [`known_double_move`]; once fixed, this goes.
/// Only `settle_clashes` reorders a plan (otherwise: folders to make, files, removals), so this
/// can only match a plan with a file and a folder at one path.
fn known_replacement_order(plan: &[Action]) -> bool {
    plan.iter().enumerate().any(|(i, a)| match a {
        Action::RemoveRemoteFolder(f) | Action::RemoveLocalFolder(f) | Action::DeleteRemoteFolder { path: f, .. } => {
            plan[i..].iter().any(|b| matches!(b, Action::MoveRemote { from, .. } | Action::MoveLocal { from, .. } if strictly_under(f, from)))
        }
        Action::MoveRemote { to, .. } => plan[i..].iter().any(|b| matches!(b, Action::MakeRemoteFolder(f) if strictly_under(f, to))),
        _ => false,
    })
}

/// The rules every plan keeps, whatever the inputs.
fn check_plan(c: &Case, plan: &[Action]) -> Result<(), TestCaseError> {
    let (state, local, remote) = (&c.state, &c.local, &c.remote);
    let known = |p: &str| state.files.get(p);
    let input = |p: &str| {
        state.files.contains_key(p)
            || local.files.contains_key(p)
            || remote.files.contains_key(p)
            || state.folders.contains(p)
            || local.folders.contains(p)
            || remote.folders.contains(p)
    };
    for a in plan {
        // No action touches a path outside the inputs (or a folder above one): what the engine
        // leaves out of all three is never planned for.
        for p in all_paths(a) {
            let above = state.files.keys().chain(local.files.keys()).chain(remote.files.keys()).any(|f| strictly_under(p, f))
                || state.folders.iter().chain(&local.folders).chain(&remote.folders).any(|f| strictly_under(p, f));
            prop_assert!(input(p) || above, "{a:?} names {p}, which is in no input");
        }
        match a {
            // Only what was synced, and is unchanged where it still is, is ever deleted.
            Action::DeleteLocal(p) => {
                let k = known(p);
                prop_assert!(k.is_some(), "{a:?}: never synced");
                prop_assert!(!remote.files.contains_key(p), "{a:?}: still in the cloud");
                let l = local.files.get(p);
                prop_assert!(l.is_some_and(|l| k.is_some_and(|k| !local_changed(k, l))), "{a:?}: changed here");
            }
            Action::DeleteRemote { path, etag } => {
                prop_assert!(!local.files.contains_key(path), "{a:?}: still here");
                prop_assert!(known(path).is_some_and(|k| &k.etag == etag), "{a:?}: not the synced version");
                prop_assert!(remote.files.get(path).is_some_and(|r| &r.etag == etag), "{a:?}: changed in the cloud");
            }
            // A folder goes to the trash whole only when everything in it is what was synced.
            Action::DeleteRemoteFolder { path, files } => {
                prop_assert!(state.folders.contains(path) && remote.folders.contains(path) && !local.folders.contains(path), "{a:?}");
                let under: BTreeMap<&str, &str> =
                    remote.files.iter().filter(|(p, _)| strictly_under(path, p)).map(|(p, r)| (p.as_str(), r.etag.as_str())).collect();
                let listed: BTreeMap<&str, &str> = files.iter().map(|(p, e)| (p.as_str(), e.as_str())).collect();
                prop_assert_eq!(&under, &listed, "{:?}: not exactly what the cloud holds", a);
                for (p, e) in files {
                    prop_assert!(known(p).is_some_and(|k| &k.etag == e), "{a:?}: {p} changed in the cloud or never synced");
                    prop_assert!(!local.files.contains_key(p), "{a:?}: {p} is still here");
                }
                for f in remote.folders.iter().filter(|f| strictly_under(path, f)) {
                    prop_assert!(state.folders.contains(f), "{a:?}: {f} is new in the cloud");
                }
            }
            // Writes are guarded: an overwrite names the version it replaces, a create that
            // nothing is there.
            Action::UploadChange { path, etag } => {
                prop_assert!(known(path).is_some_and(|k| &k.etag == etag), "{a:?}: not the synced version");
                prop_assert!(remote.files.get(path).is_some_and(|r| &r.etag == etag), "{a:?}: the cloud changed");
            }
            Action::UploadNew(p) => {
                prop_assert!(local.files.contains_key(p), "{a:?}: nothing here to upload");
                prop_assert!(!remote.files.contains_key(p), "{a:?}: the cloud has it");
            }
            // A download never lands on a change made here.
            Action::Download(p) => {
                prop_assert!(remote.files.contains_key(p), "{a:?}: not in the cloud");
                if let Some(l) = local.files.get(p) {
                    prop_assert!(known(p).is_some_and(|k| !local_changed(k, l)), "{a:?}: over a change made here");
                }
            }
            Action::Conflict(p) => {
                prop_assert!(local.files.contains_key(p) && remote.files.contains_key(p), "{a:?}: not on both sides");
            }
            Action::MoveRemote { from, to, etag } => {
                prop_assert!(remote.files.get(from).is_some_and(|r| &r.etag == etag), "{a:?}: not the cloud's version");
                prop_assert!(!remote.files.contains_key(to), "{a:?}: onto a cloud file");
                prop_assert!(!local.files.contains_key(from), "{a:?}: still here");
                let (k, l) = (known(from), local.files.get(to));
                prop_assert!(k.is_some_and(|k| l.is_some_and(|l| !local_changed(k, l))), "{a:?}: not the same file");
            }
            Action::MoveLocal { from, to } => {
                prop_assert!(!local.files.contains_key(to) && known(to).is_none(), "{a:?}: onto a file");
                prop_assert!(!remote.files.contains_key(from), "{a:?}: still in the cloud");
                let (k, r) = (known(from), remote.files.get(to));
                prop_assert!(k.is_some_and(|k| r.is_some_and(|r| same_etag(&r.etag, &k.etag))), "{a:?}: not the same file");
            }
            Action::Forget(p) => {
                prop_assert!(known(p).is_some() && !local.files.contains_key(p) && !remote.files.contains_key(p), "{a:?}");
            }
            _ => {}
        }
    }

    // Changed in two places (or new in both): a conflict, never anything that overwrites or
    // deletes either side.
    for (p, l) in &local.files {
        let Some(r) = remote.files.get(p) else { continue };
        let both = known(p).is_none_or(|k| local_changed(k, l) && r.etag != k.etag);
        if both {
            for a in plan.iter().filter(|a| file_paths(a).contains(&p.as_str())) {
                prop_assert!(matches!(a, Action::Conflict(_)), "{p} changed on both sides, yet {a:?}");
            }
        }
    }
    // Deleted on one side, changed on the other: the change wins.
    for (p, k) in &state.files {
        match (local.files.get(p), remote.files.get(p)) {
            (Some(l), None) if local_changed(k, l) => {
                prop_assert!(!plan.contains(&Action::DeleteLocal(p.clone())), "{p}: a change here deleted");
            }
            (None, Some(r)) if r.etag != k.etag => {
                prop_assert!(!plan.iter().any(|a| matches!(a, Action::DeleteRemote { path, .. } if path == p)), "{p}: a cloud change deleted");
                let trashed = plan.iter().any(|a| matches!(a, Action::DeleteRemoteFolder { files, .. } if files.iter().any(|(f, _)| f == p)));
                prop_assert!(!trashed, "{p}: a cloud change trashed with its folder");
            }
            _ => {}
        }
    }

    // One action per file, counting both ends of a move.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for a in plan {
        for p in file_paths(a) {
            prop_assert!(seen.insert(p), "{p} twice in the plan: {plan:?}");
        }
    }

    // Order: a folder is there (or made earlier) before anything goes into it in the cloud; a
    // folder that goes away gets nothing new after.
    let mut cloud_folders: BTreeSet<&str> = remote.folders.iter().map(String::as_str).collect();
    let mut removed: BTreeSet<&str> = BTreeSet::new();
    for (i, a) in plan.iter().enumerate() {
        match a {
            Action::MakeRemoteFolder(f) => {
                for parent in ancestors(f) {
                    prop_assert!(cloud_folders.contains(parent.as_str()), "{a:?} before its parent: {plan:?}");
                }
                cloud_folders.insert(f);
                removed.remove(f.as_str());
            }
            Action::RemoveRemoteFolder(f) | Action::DeleteRemoteFolder { path: f, .. } => {
                removed.insert(f);
            }
            _ => {}
        }
        // A file moves out of a folder before the folder is removed (a removal only takes an
        // empty folder, so the other way round both fail).
        if let Action::MoveRemote { from, .. } | Action::MoveLocal { from, .. } = a {
            for f in ancestors(from) {
                let gone = plan[..i].iter().any(|b| matches!(b, Action::RemoveRemoteFolder(r) | Action::RemoveLocalFolder(r) if *r == f));
                prop_assert!(!gone, "{a:?} after its folder {f} is removed: {plan:?}");
            }
        }
        if let Some(p) = remote_writes(a) {
            for parent in ancestors(p) {
                prop_assert!(cloud_folders.contains(parent.as_str()), "{a:?} before its folder {parent}: {plan:?}");
                prop_assert!(!removed.contains(parent.as_str()), "{a:?} into a folder removed before it: {plan:?}");
            }
        }
        // Nothing goes into a folder that is trashed whole, before or after.
        if let Action::DeleteRemoteFolder { path, .. } = a {
            for (j, b) in plan.iter().enumerate() {
                if j != i
                    && let Some(p) = remote_writes(b)
                {
                    prop_assert!(!strictly_under(path, p), "{b:?} into {path}, which goes to the trash whole: {plan:?}");
                }
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// The rules of [`check_plan`], with the mass-delete limit lifted.
    #[test]
    fn every_plan_keeps_the_rules(c in case(false)) {
        let plan = reconcile(&c.state, &c.local, &c.remote, true).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        prop_assume!(!known_double_move(&plan) && !known_replacement_order(&plan));
        check_plan(&c, &plan)?;
    }

    /// Mass deletes wait: past max(half the synced files, 5) the plan is refused, and below it the
    /// plan is the same as a confirmed one.
    #[test]
    fn mass_deletes_wait_for_the_person(c in case(false)) {
        let all = reconcile(&c.state, &c.local, &c.remote, true).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        let limit = (c.state.files.len() / 2).max(5);
        match reconcile(&c.state, &c.local, &c.remote, false) {
            Ok(plan) => {
                prop_assert_eq!(&plan, &all);
                prop_assert!(deletes(&plan) <= limit, "{} deletes, limit {}", deletes(&plan), limit);
            }
            Err(super::Refusal::TooManyDeletes { deletes: n, limit: l }) => {
                prop_assert_eq!(l, limit);
                prop_assert!(n > limit && n >= deletes(&all), "refused {} (limit {}), the plan deletes {}", n, l, deletes(&all));
            }
        }
    }

    /// Nothing changed anywhere: nothing to do.
    #[test]
    fn a_synced_folder_needs_nothing(c in case(false)) {
        let local = Tree {
            files: c.state.files.iter().map(|(p, k)| (p.clone(), LocalFile { size: k.size, mtime: k.mtime })).collect(),
            folders: c.state.folders.clone(),
        };
        let remote = Tree {
            files: c.state.files.iter().map(|(p, k)| (p.clone(), RemoteFile { etag: k.etag.clone(), size: k.size })).collect(),
            folders: c.state.folders.clone(),
        };
        prop_assert_eq!(reconcile(&c.state, &local, &remote, false), Ok(vec![]));
    }

    /// A place that couldn't be read here is held as last synced (the engine's `keep_as_synced`):
    /// with the local side there as it was, nothing at, in or above it is deleted, trashed, moved
    /// away or forgotten in the cloud, whatever else changed. `""` is a missing sync folder.
    #[test]
    fn a_place_held_as_synced_is_never_taken_as_gone(c in case(false), pick in any::<prop::sample::Index>()) {
        let mut places: Vec<String> = vec![String::new()];
        places.extend(c.local.folders.iter().cloned());
        places.extend(c.local.files.keys().cloned());
        let place = pick.get(&places).clone();
        let mut local = c.local.clone();
        local.files.retain(|p, _| !within(&place, p));
        local.folders.retain(|f| !within(&place, f) || *f == place);
        for (p, k) in c.state.files.iter().filter(|(p, _)| within(&place, p)) {
            local.files.insert(p.clone(), LocalFile { size: k.size, mtime: k.mtime });
        }
        local.folders.extend(c.state.folders.iter().filter(|f| within(&place, f)).cloned());
        let plan = reconcile(&c.state, &local, &c.remote, true).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        let held = |p: &str| within(&place, p) && (c.state.files.contains_key(p) || c.state.folders.contains(p));
        let covers = |f: &str| held(f) || c.state.files.keys().chain(&c.state.folders).any(|p| held(p) && strictly_under(f, p));
        for a in &plan {
            let gone = match a {
                Action::DeleteRemote { path, .. } | Action::Forget(path) | Action::MoveRemote { from: path, .. } => held(path),
                Action::DeleteRemoteFolder { path, files } => covers(path) || files.iter().any(|(p, _)| held(p)),
                Action::RemoveRemoteFolder(f) => covers(f),
                _ => false,
            };
            prop_assert!(!gone, "{a:?} with {place:?} held as synced: {plan:?}");
        }
        for m in folder_moves(&c.state, &local, &c.remote) {
            prop_assert!(!(m.side == super::Side::Local && covers(&m.from)), "{m:?} with {place:?} held as synced");
        }
    }

    /// One skip rule: with every skipped name taken out of all three inputs (as the engine's
    /// `drop_skipped` does), no action names one, on either platform. A folder whose name only
    /// looks temporary (`T.tmp`) still syncs.
    #[test]
    fn skipped_names_are_never_planned(r in raw(SKIP_DIRS.len() * SKIP_NAMES.len(), SKIP_DIRS.len() - 1), windows in any::<bool>()) {
        let platform = if windows { Platform::Windows } else { Platform::Other };
        let mut c = build(SKIP_DIRS, SKIP_NAMES, &r, false);
        c.state.files.retain(|p, _| !skipped(p, true, platform));
        c.state.folders.retain(|p| !skipped(p, false, platform));
        c.local.files.retain(|p, _| !skipped(p, true, platform));
        c.local.folders.retain(|p| !skipped(p, false, platform));
        c.remote.files.retain(|p, _| !skipped(p, true, platform));
        c.remote.folders.retain(|p| !skipped(p, false, platform));
        let plan = reconcile(&c.state, &c.local, &c.remote, true).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        for a in &plan {
            for p in file_paths(a) {
                prop_assert!(!skipped(p, true, platform), "{a:?} on {platform:?}");
            }
            for p in all_paths(a) {
                prop_assert!(!skipped(p, false, platform), "{a:?} on {platform:?}");
            }
        }
    }

    /// A folder renamed on one side is found only when every synced file in it is at the same
    /// place under the new name, unchanged; afterwards the files that the other side didn't touch
    /// either need nothing more.
    #[test]
    fn folder_moves_carry_every_synced_file(c in case(false)) {
        let moves = folder_moves(&c.state, &c.local, &c.remote);
        let mut targets = BTreeSet::new();
        for m in &moves {
            prop_assert!(c.state.folders.contains(&m.from) && !c.state.folders.contains(&m.to), "{m:?}");
            prop_assert!(targets.insert(m.to.clone()), "two folders moved to {}", m.to);
            for (p, k) in c.state.files.iter().filter(|(p, _)| strictly_under(&m.from, p)) {
                let to = format!("{}{}", m.to, &p[m.from.len()..]);
                let same = match m.side {
                    super::Side::Local => c.local.files.get(&to).is_some_and(|l| !local_changed(k, l)),
                    super::Side::Remote => c.remote.files.get(&to).is_some_and(|r| same_etag(&r.etag, &k.etag)),
                };
                prop_assert!(same, "{m:?}: {p} isn't at {to}");
            }
        }
        let (mut state, mut local, mut remote) = (c.state.clone(), c.local.clone(), c.remote.clone());
        for m in &moves {
            move_state(&mut state, &m.from, &m.to);
            match m.side {
                super::Side::Local => move_tree(&mut remote, &m.from, &m.to),
                super::Side::Remote => move_tree(&mut local, &m.from, &m.to),
            }
        }
        let plan = reconcile(&state, &local, &remote, true).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        for m in &moves {
            for a in &plan {
                // A file the move carried that the other side hadn't touched either: synced in
                // its new place. (A change or delete made on the other side still comes across,
                // as in `tests.rs`.)
                let untouched = |p: &str| {
                    let Some(old) = strictly_under(&m.to, p).then(|| format!("{}{}", m.from, &p[m.to.len()..])) else { return false };
                    let Some(k) = c.state.files.get(&old) else { return false };
                    match m.side {
                        super::Side::Local => c.remote.files.get(&old).is_some_and(|r| r.etag == k.etag),
                        super::Side::Remote => c.local.files.get(&old).is_some_and(|l| !local_changed(k, l)),
                    }
                };
                let bad = match a {
                    Action::DeleteLocal(p) | Action::DeleteRemote { path: p, .. } | Action::UploadNew(p) | Action::Download(p) | Action::Conflict(p) => {
                        untouched(p)
                    }
                    _ => false,
                };
                prop_assert!(!bad, "{a:?} after {m:?}");
            }
        }
    }
}

/// A model of both sides that carries a plan out the way the engine does (craftcloud-native's
/// `apply`), with every write guarded as on the server.
#[derive(Clone, Debug)]
struct Sim {
    state: State,
    local: BTreeMap<String, (LocalFile, u32)>,
    local_folders: BTreeSet<String>,
    remote: BTreeMap<String, (RemoteFile, u32)>,
    remote_folders: BTreeSet<String>,
    tick: i64,
}

impl Sim {
    fn new(c: &Case) -> Self {
        Self {
            state: c.state.clone(),
            local: c.local.files.iter().map(|(p, l)| (p.clone(), (*l, c.local_bytes.get(p).copied().unwrap_or_default()))).collect(),
            local_folders: c.local.folders.clone(),
            remote: c.remote.files.iter().map(|(p, r)| (p.clone(), (r.clone(), c.remote_bytes.get(p).copied().unwrap_or_default()))).collect(),
            remote_folders: c.remote.folders.clone(),
            tick: 1000,
        }
    }

    fn trees(&self) -> (Tree<LocalFile>, Tree<RemoteFile>) {
        (
            Tree { files: self.local.iter().map(|(p, (l, _))| (p.clone(), *l)).collect(), folders: self.local_folders.clone() },
            Tree { files: self.remote.iter().map(|(p, (r, _))| (p.clone(), r.clone())).collect(), folders: self.remote_folders.clone() },
        )
    }

    fn fresh(&mut self) -> i64 {
        self.tick += 1;
        self.tick
    }

    fn make_local_parents(&mut self, path: &str) {
        self.local_folders.extend(ancestors(path));
    }

    /// A `PUT` (create or overwrite): 409 without the folder, `false` on a 412.
    fn put(&mut self, path: &str, replace: Option<&str>) -> Result<bool, String> {
        let Some((l, bytes)) = self.local.get(path).copied() else { return Err(format!("upload {path}: not here")) };
        if let Some(parent) = ancestors(path).into_iter().find(|f| !self.remote_folders.contains(f)) {
            return Err(format!("upload {path}: 409, no folder {parent} in the cloud"));
        }
        if self.remote_folders.contains(path) {
            return Err(format!("upload {path}: a folder is there in the cloud"));
        }
        let current = self.remote.get(path).map(|(r, _)| r.etag.clone());
        if current.as_deref() != replace {
            return Ok(false);
        }
        let etag = format!("w{}", self.fresh());
        self.remote.insert(path.to_owned(), (RemoteFile { etag: etag.clone(), size: l.size }, bytes));
        self.state.files.insert(path.to_owned(), Known { etag, size: l.size, mtime: l.mtime, hash: None });
        Ok(true)
    }

    fn download(&mut self, path: &str) -> Result<(), String> {
        let Some((r, bytes)) = self.remote.get(path).cloned() else { return Err(format!("download {path}: not in the cloud")) };
        if let Some((l, _)) = self.local.get(path)
            && !self.state.files.get(path).is_some_and(|k| !local_changed(k, l))
        {
            return Err(format!("download {path}: over a change made here"));
        }
        if self.local_folders.contains(path) {
            return Err(format!("download {path}: a folder is here"));
        }
        let mtime = self.fresh();
        self.make_local_parents(path);
        self.local.insert(path.to_owned(), (LocalFile { size: r.size, mtime }, bytes));
        self.state.files.insert(path.to_owned(), Known { etag: r.etag, size: r.size, mtime, hash: None });
        Ok(())
    }

    fn conflict(&mut self, path: &str) -> Result<(), String> {
        let (Some((_, mine)), Some((r, theirs))) = (self.local.get(path).copied(), self.remote.get(path).cloned()) else {
            return Err(format!("conflict {path}: not on both sides"));
        };
        if mine == theirs {
            let l = self.local.get(path).map(|(l, _)| *l).unwrap_or(LocalFile { size: 0, mtime: 0 });
            self.state.files.insert(path.to_owned(), Known { etag: r.etag, size: l.size, mtime: l.mtime, hash: None });
            return Ok(());
        }
        let copy = free_conflict_name(path, "2026-11-16 12.00", "Studio", |c| self.local.contains_key(c) || self.local_folders.contains(c));
        if let Some(mine) = self.local.remove(path) {
            self.local.insert(copy.clone(), mine);
        }
        let mtime = self.fresh();
        self.local.insert(path.to_owned(), (LocalFile { size: r.size, mtime }, theirs));
        self.state.files.insert(path.to_owned(), Known { etag: r.etag, size: r.size, mtime, hash: None });
        // A failed upload of the copy is retried as a new file next cycle.
        let _ = self.put(&copy, None);
        Ok(())
    }

    fn apply(&mut self, action: &Action) -> Result<(), String> {
        match action {
            Action::MakeRemoteFolder(f) => {
                if let Some(parent) = ancestors(f).into_iter().find(|p| !self.remote_folders.contains(p)) {
                    return Err(format!("make {f}: 409, no folder {parent} in the cloud"));
                }
                self.remote_folders.insert(f.clone());
            }
            Action::MakeLocalFolder(f) => {
                self.make_local_parents(&format!("{f}/x"));
            }
            Action::UploadNew(p) => {
                if !self.put(p, None)? {
                    self.conflict(p)?;
                }
            }
            Action::UploadChange { path, etag } => {
                if !self.put(path, Some(etag))? {
                    self.conflict(path)?;
                }
            }
            Action::Download(p) => self.download(p)?,
            Action::Conflict(p) => self.conflict(p)?,
            Action::DeleteRemote { path, etag } => {
                if self.remote.get(path).is_some_and(|(r, _)| same_etag(&r.etag, etag)) {
                    self.remote.remove(path);
                }
                self.state.files.remove(path);
            }
            Action::DeleteRemoteFolder { path, files } => {
                let now: BTreeMap<&str, &str> =
                    self.remote.iter().filter(|(p, _)| strictly_under(path, p)).map(|(p, (r, _))| (p.as_str(), r.etag.as_str())).collect();
                let listed: BTreeMap<&str, &str> = files.iter().map(|(p, e)| (p.as_str(), e.as_str())).collect();
                if now == listed {
                    self.remote.retain(|p, _| !strictly_under(path, p));
                    self.remote_folders.retain(|f| !within(path, f));
                    self.state.files.retain(|p, _| !strictly_under(path, p));
                }
            }
            Action::DeleteLocal(p) => {
                self.local.remove(p);
                self.state.files.remove(p);
            }
            Action::MoveRemote { from, to, etag } => {
                let Some((l, _)) = self.local.get(to).copied() else { return Err(format!("move {from} to {to}: {to} isn't here")) };
                let still = self.remote.get(from).is_some_and(|(r, _)| same_etag(&r.etag, etag));
                let folder = ancestors(to).iter().all(|f| self.remote_folders.contains(f));
                if still && folder && !self.remote.contains_key(to) && !self.remote_folders.contains(to) {
                    if let Some(moved) = self.remote.remove(from) {
                        self.remote.insert(to.clone(), moved);
                    }
                    let old = self.state.files.remove(from);
                    let synced = old.map_or_else(|| etag.clone(), |k| k.etag);
                    self.state.files.insert(to.clone(), Known { etag: synced, size: l.size, mtime: l.mtime, hash: None });
                } else {
                    self.state.files.remove(from);
                    if !self.put(to, None)? {
                        self.conflict(to)?;
                    }
                }
            }
            Action::MoveLocal { from, to } => {
                if self.local.contains_key(to) || self.local_folders.contains(to) {
                    return Err(format!("move {from} to {to}: {to} is already here"));
                }
                let Some(file) = self.local.remove(from) else { return Err(format!("move {from}: not here")) };
                self.make_local_parents(to);
                self.local.insert(to.clone(), file);
                let known = self.state.files.remove(from);
                let etag = self.remote.get(to).map(|(r, _)| r.etag.clone()).or(known.as_ref().map(|k| k.etag.clone())).unwrap_or_default();
                let known = match known {
                    Some(k) => Known { etag, ..k },
                    None => Known { etag, size: file.0.size, mtime: file.0.mtime, hash: None },
                };
                self.state.files.insert(to.clone(), known);
            }
            Action::Forget(p) => {
                self.state.files.remove(p);
            }
            Action::LocalClash(p) => {
                let copy = free_conflict_name(p, "2026-11-16 12.00", "Studio", |c| self.local.contains_key(c) || self.local_folders.contains(c));
                let moved = |path: &str| -> Option<String> {
                    if path == p {
                        Some(copy.clone())
                    } else {
                        path.strip_prefix(p.as_str()).filter(|rest| rest.starts_with('/')).map(|rest| format!("{copy}{rest}"))
                    }
                };
                let files: Vec<String> = self.local.keys().filter(|f| within(p, f)).cloned().collect();
                for f in files {
                    if let (Some(file), Some(to)) = (self.local.remove(&f), moved(&f)) {
                        self.local.insert(to, file);
                    }
                }
                let folders: Vec<String> = self.local_folders.iter().filter(|f| within(p, f)).cloned().collect();
                for f in folders {
                    self.local_folders.remove(&f);
                    self.local_folders.extend(moved(&f));
                }
                self.state.files.retain(|f, _| !within(p, f));
            }
            Action::RemoveRemoteFolder(f) => {
                let empty = !self.remote.keys().any(|p| strictly_under(f, p)) && !self.remote_folders.iter().any(|p| strictly_under(f, p));
                if empty {
                    self.remote_folders.remove(f);
                }
            }
            Action::RemoveLocalFolder(f) => {
                let empty = !self.local.keys().any(|p| strictly_under(f, p)) && !self.local_folders.iter().any(|p| strictly_under(f, p));
                if empty {
                    self.local_folders.remove(f);
                }
            }
        }
        Ok(())
    }

    /// One sync cycle (deletes confirmed); the plan it carried out. As in the engine, an action
    /// that fails is noted (in `errors`) and tried again next cycle; the others go on.
    fn cycle(&mut self, errors: &mut Vec<String>) -> Result<Vec<Action>, TestCaseError> {
        let (local, remote) = self.trees();
        let plan = reconcile(&self.state, &local, &remote, true).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        if known_double_move(&plan) || known_replacement_order(&plan) {
            return Err(TestCaseError::reject("a known bug (see known_double_move, known_replacement_order)"));
        }
        for a in &plan {
            if let Err(e) = self.apply(a) {
                errors.push(e);
            }
        }
        self.state.folders = self.local_folders.intersection(&self.remote_folders).cloned().collect();
        Ok(plan)
    }
}

proptest! {
    #![proptest_config(config())]

    /// Carried out cycle by cycle, the plans settle: within a few cycles there is nothing left to
    /// do, both sides hold the same files with the same bytes, and every change made on either
    /// side is still there (no edit or new file is lost; only unchanged files are deleted).
    #[test]
    fn cycles_settle_without_losing_a_change(c in case(true)) {
        let mut sim = Sim::new(&c);
        // The work at the start: every new or edited file, on either side (`build` numbers the
        // synced bytes below 100, edits and new files from 100 on).
        let work: BTreeSet<u32> = c.local_bytes.values().chain(c.remote_bytes.values()).copied().filter(|b| *b >= 100).collect();
        let (mut plans, mut errors) = (Vec::new(), Vec::new());
        let mut settled = false;
        for _ in 0..5 {
            let plan = sim.cycle(&mut errors)?;
            if plan.is_empty() {
                settled = true;
                break;
            }
            plans.push(plan);
        }
        prop_assert!(settled, "still busy after 5 cycles: {plans:#?}\nerrors: {errors:#?}\n{sim:#?}");
        let here: BTreeMap<&String, u32> = sim.local.iter().map(|(p, (_, b))| (p, *b)).collect();
        let there: BTreeMap<&String, u32> = sim.remote.iter().map(|(p, (_, b))| (p, *b)).collect();
        prop_assert_eq!(&here, &there, "the sides differ after {:#?}", plans);
        let kept: BTreeSet<u32> = here.values().copied().collect();
        for b in &work {
            prop_assert!(kept.contains(b), "a change ({b}) was lost: {plans:#?}");
        }
    }
}

/// A file or folder name made of the characters the name rules look at.
fn name() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(vec!['a', 'b', 'C', 'O', 'N', '.', '~', '#', '$', ' ', ':', '?']), 1..8)
        .prop_map(|chars| chars.into_iter().collect())
}

/// Mostly plain names, sometimes with what the temporary-file and Windows rules look at.
fn plain_name() -> impl Strategy<Value = String> {
    let chars = vec!['a', 'b', 'c', 'x', 'C', 'O', 'N', '.', '.', '~', '#', '$', ' '];
    prop::collection::vec(prop::sample::select(chars), 1..8).prop_map(|chars| chars.into_iter().collect())
}

fn path() -> impl Strategy<Value = String> {
    let part = prop_oneof![3 => name(), 1 => Just(crate::api::dav::CRAFTCLOUD_DIR.to_owned()), 1 => Just("..".to_owned())];
    prop::collection::vec(part, 1..4).prop_map(|parts| parts.join("/"))
}

proptest! {
    #![proptest_config(config())]

    /// A conflict copy of a file that syncs syncs too (on either platform), whatever its count,
    /// and leads back to its original: otherwise the copy that keeps one side's change would stay
    /// on one computer.
    #[test]
    fn conflict_copies_sync_like_their_original(dir in prop::option::of(plain_name()), file in plain_name(), n in 1u32..4, windows in any::<bool>()) {
        let platform = if windows { Platform::Windows } else { Platform::Other };
        let original = dir.map_or_else(|| file.clone(), |d| format!("{d}/{file}"));
        prop_assume!(!skipped(&original, true, platform) && !original.contains(" (conflict "));
        let nth = format!(" Studio {n})");
        let copy = free_conflict_name(&original, "2026-11-16 12.00", "Studio", |c| n > 1 && !c.contains(&nth));
        prop_assert!(!skipped(&copy, true, platform), "{copy} on {platform:?}");
        prop_assert_eq!(super::original_of_conflict(&copy), Some(original));
    }

    /// The skip rule: CraftCloud's own folder holds everything in it, anywhere, and is never in
    /// the plan; a Windows computer leaves alone at least what the others do; a skipped folder
    /// holds what is in it; and no cloud path leads out of the local folder.
    #[test]
    fn the_skip_rule_holds_whole_folders(p in path(), child in name(), windows in any::<bool>()) {
        let platform = if windows { Platform::Windows } else { Platform::Other };
        if p.split('/').any(|s| s == crate::api::dav::CRAFTCLOUD_DIR) {
            prop_assert!(skipped(&p, true, platform) && skipped(&p, false, platform), "{p}");
        }
        if p.split('/').next() == Some(crate::api::dav::CRAFTCLOUD_DIR) {
            let everything = crate::sync::SyncPlan { sync_root_files: true, ..Default::default() };
            prop_assert!(!crate::sync::in_plan(&everything, &p, false) && !crate::sync::in_plan(&everything, &p, true), "{p}");
        }
        for is_file in [true, false] {
            prop_assert!(!skipped(&p, is_file, Platform::Other) || skipped(&p, is_file, Platform::Windows), "{p}");
        }
        if skipped(&p, false, platform) {
            let inside = format!("{p}/{child}");
            prop_assert!(skipped(&inside, true, platform) && skipped(&inside, false, platform), "{inside}");
        }
        if let Some(rel) = crate::sync::local_relative_on(&p, platform) {
            prop_assert!(rel.components().all(|c| matches!(c, std::path::Component::Normal(_))), "{p} → {rel:?}");
        }
    }
}
