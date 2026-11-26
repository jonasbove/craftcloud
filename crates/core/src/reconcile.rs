//! Two-way sync, the decisions: given what was true at the last sync, what is on disk now and
//! what is in the cloud now, what to do for each path. No I/O: craftcloud-native carries the
//! actions out and records each one as it succeeds.
//!
//! Rules (one test each, below):
//! - Changed on one side → copy it to the other. A local change uploads with `If-Match`, so a
//!   cloud change in the meantime turns into a conflict instead of an overwrite.
//! - Changed on both sides → keep both: the local file is renamed to a conflict copy and
//!   uploaded, the cloud version comes down in its place.
//! - Deleted on one side and unchanged on the other → delete it there (OxiCloud keeps it in the
//!   trash). Deleted on one side but changed on the other → the change wins and is copied back.
//! - New on both sides → compared first: the same bytes are just recorded, different bytes are a
//!   conflict.
//! - A rename or move is one move on the other side, never a delete plus a new file: a file is
//!   paired by size and time (here) or ETag (in the cloud); a whole folder by its files'
//!   relative paths, see [`folder_moves`] (done before the plan, so the folder keeps its id).
//!   The one exception: a file moved to where its own folder was, or into a folder in its own
//!   place, which no single move can do.
//! - Folders are created on whichever side lacks them, and removed only once empty.
//!
//! That plan is two-way. [`restrict`] then cuts it to what the server allows
//! ([`crate::storage::Mode`]): unchanged for two-way sync, nothing written to the server for
//! download only, nothing at all when the server is refused.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::storage::Mode;

/// What was true for a file right after it was last synced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    /// The cloud's ETag (quotes included, as `If-Match` needs it).
    pub etag: String,
    pub size: u64,
    /// The local file's modification time, in milliseconds since the Unix epoch.
    pub mtime: i64,
    /// SHA-256 of these bytes (hex), when known: a file whose time changed but whose bytes didn't
    /// (rewritten with the same content) isn't a change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// A local file now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalFile {
    pub size: u64,
    pub mtime: i64,
}

/// A cloud file now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    pub etag: String,
    pub size: u64,
}

/// One side of a sync: files by cloud path (`Documents/notes.md`) and folders.
#[derive(Clone, Debug)]
pub struct Tree<F> {
    pub files: BTreeMap<String, F>,
    pub folders: BTreeSet<String>,
}

impl<F> Default for Tree<F> {
    fn default() -> Self {
        Self { files: BTreeMap::new(), folders: BTreeSet::new() }
    }
}

/// The sync state: what both sides agreed on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub files: BTreeMap<String, Known>,
    #[serde(default)]
    pub folders: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Create this folder in the cloud (parents come first in the plan).
    MakeRemoteFolder(String),
    /// Create this folder locally.
    MakeLocalFolder(String),
    /// Upload a new file (`If-None-Match: *`).
    UploadNew(String),
    /// Upload a change over this cloud version (`If-Match: etag`), keeping a version snapshot.
    UploadChange {
        path: String,
        etag: String,
    },
    Download(String),
    /// Both sides changed, or both are new: compare, then keep both if they differ.
    Conflict(String),
    DeleteRemote {
        path: String,
        etag: String,
    },
    /// Deleted in the cloud, unchanged here. The engine moves it into `.craftcloud/trash/` in the
    /// local folder, as [`Action::TrashLocal`] (never removed for good, and never a version
    /// changed here since the scan). Its own kind so that a mass delete can put the cloud's
    /// trash back first (it was deleted there, not here).
    DeleteLocal(String),
    /// A whole synced folder deleted here, everything in it unchanged in the cloud: one move to
    /// the cloud's trash (so restoring it brings it back whole), unless the cloud no longer holds
    /// exactly these `files` (path, ETag).
    DeleteRemoteFolder {
        path: String,
        files: Vec<(String, String)>,
    },
    /// Renamed or moved here: move the cloud file too (WebDAV `MOVE`, which keeps its id, shares
    /// and ETag), unless it changed since `etag`.
    MoveRemote {
        from: String,
        to: String,
        etag: String,
    },
    /// Renamed or moved in the cloud: rename the local file to match.
    MoveLocal {
        from: String,
        to: String,
    },
    /// A file on one side and a folder on the other have this path: the cloud's is kept, the
    /// local one is renamed to a conflict copy (next to it) and both sync from the next cycle.
    LocalClash(String),
    /// Gone on both sides: drop it from the state.
    Forget(String),
    /// Remove an empty folder in the cloud (deepest first).
    RemoveRemoteFolder(String),
    /// Remove an empty local folder (deepest first).
    RemoveLocalFolder(String),
    /// Download only ([`restrict`]): deleted in the cloud, unchanged here. The local file is
    /// moved into `.craftcloud/trash/` in the local folder (never deleted for good: the server
    /// may have no trash, and this may be the last copy) and the state forgets it.
    TrashLocal(String),
    /// Download only ([`restrict`]): changed on both sides, or new on both. Compared first; when
    /// they differ, the local file is set aside as a conflict copy that stays here (it isn't
    /// uploaded) and the cloud version comes down in its place.
    LocalConflict(String),
}

/// Why a plan was not made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// More deletions than `limit`: probably a missing drive or a mistake, not intent.
    TooManyDeletes { deletes: usize, limit: usize },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyDeletes { deletes, limit } => {
                write!(f, "Paused: this would delete {deletes} files (more than {limit}). Check the folder, then resume.")
            }
        }
    }
}

/// The plan, or a refusal when it would delete too much at once. `confirm_deletes` lifts the limit.
/// Order: folders to create (parents first), then file actions, then empty folders to remove
/// (deepest first); a path that is a file on one side and a folder on the other changes that a
/// little (see `settle_clashes`), so that every action can be done in one pass, in plan order.
pub fn reconcile(state: &State, local: &Tree<LocalFile>, remote: &Tree<RemoteFile>, confirm_deletes: bool) -> Result<Vec<Action>, Refusal> {
    let paths: BTreeSet<&String> = state.files.keys().chain(local.files.keys()).chain(remote.files.keys()).collect();
    let files: Vec<Action> =
        paths.into_iter().filter_map(|path| decide(state.files.get(path), local.files.get(path), remote.files.get(path), path)).collect();
    let files = pair_moves(state, local, remote, files);
    let files = group_folder_deletes(state, local, remote, files);

    let deletes: usize = files
        .iter()
        .map(|a| match a {
            Action::DeleteLocal(_) | Action::DeleteRemote { .. } => 1,
            Action::DeleteRemoteFolder { files, .. } => files.len(),
            _ => 0,
        })
        .sum();
    let limit = (state.files.len() / 2).max(5);
    if deletes > limit && !confirm_deletes {
        return Err(Refusal::TooManyDeletes { deletes, limit });
    }

    let mut actions = folders_needed(state, local, remote, &files);
    let removals = remove_empty_folders(state, local, remote, &actions, &files);
    actions.extend(files);
    actions.extend(removals);
    Ok(settle_clashes(local, remote, actions))
}

/// Paths that are a file on one side and a folder on the other (top-most only).
///
/// - A replacement (T-052): the state knows the path as the other kind, and the side that didn't
///   change is as it was at the last sync (a synced folder replaced by a file here while the cloud
///   folder is unchanged, a synced file replaced by a folder in the cloud while the local file is
///   unchanged, …). The plan then already removes the old one on the unchanged side (a folder as
///   one trash item, or once empty); it only has to go before the new one takes its place, so the
///   plan keeps its order (folders to make, files, removals) with two changes. A file replaced by
///   a folder is deleted first of all (a delete needs nothing before it). A folder replaced by a
///   file is emptied with the rest of the files (moves out of it included, each after the folder
///   it moves into is made), then removed, its subfolders first, ahead of the other removals; the
///   file then takes its place, still ahead of the other removals, so a file moved there from a
///   folder that goes leaves it before it is removed. (The parents are there on both sides, or
///   there would be no clash.)
/// - Otherwise both sides changed it (T-029): nothing can be copied across there (a download onto
///   a folder, an upload under a file): the local one is set aside as a conflict copy first
///   ([`Action::LocalClash`]), and everything at or under the path waits for the next cycle.
fn settle_clashes(local: &Tree<LocalFile>, remote: &Tree<RemoteFile>, actions: Vec<Action>) -> Vec<Action> {
    let mut clashes: Vec<&String> =
        local.files.keys().filter(|p| remote.folders.contains(*p)).chain(local.folders.iter().filter(|f| remote.files.contains_key(*f))).collect();
    clashes.sort();
    let mut top: Vec<&String> = Vec::new();
    for c in clashes {
        if !top.iter().any(|t| c.starts_with(&format!("{t}/"))) {
            top.push(c);
        }
    }
    if top.is_empty() {
        return actions;
    }
    // The old one goes on the side that didn't change: the plan removes exactly it there.
    let replaced = |t: &str| {
        actions.iter().any(|a| match a {
            Action::DeleteRemoteFolder { path, .. } | Action::DeleteRemote { path, .. } => path == t,
            Action::RemoveRemoteFolder(p) | Action::DeleteLocal(p) | Action::RemoveLocalFolder(p) => p == t,
            _ => false,
        })
    };
    let (replacements, clashes): (Vec<&String>, Vec<&String>) = top.into_iter().partition(|t| replaced(t));
    let at = |set: &[&String], p: &str| set.iter().any(|t| p == t.as_str() || p.starts_with(&format!("{t}/")));
    let touches = |set: &[&String], a: &Action| match a {
        Action::MoveRemote { from, to, .. } | Action::MoveLocal { from, to } => at(set, from) || at(set, to),
        other => at(set, action_path(other)),
    };
    // A replaced folder goes as a folder (removed once empty, or trashed whole); a replaced file
    // as a file.
    let folder_gone = |t: &str| {
        actions.iter().any(|a| match a {
            Action::RemoveRemoteFolder(p) | Action::RemoveLocalFolder(p) | Action::DeleteRemoteFolder { path: p, .. } => p == t,
            _ => false,
        })
    };
    let (folders, files): (Vec<&String>, Vec<&String>) = replacements.iter().copied().partition(|t| folder_gone(t));
    // The file that takes a replaced folder's place: whatever puts a file there.
    let takes = |a: &Action| match a {
        Action::UploadNew(p)
        | Action::UploadChange { path: p, .. }
        | Action::Download(p)
        | Action::Conflict(p)
        | Action::MoveRemote { to: p, .. }
        | Action::MoveLocal { to: p, .. } => at(&folders, p),
        _ => false,
    };
    let rank = |a: &Action| match a {
        Action::DeleteRemote { path, .. } | Action::DeleteLocal(path) if files.contains(&path) => 0,
        Action::MakeRemoteFolder(_) | Action::MakeLocalFolder(_) => 1,
        Action::RemoveRemoteFolder(f) | Action::RemoveLocalFolder(f) if at(&folders, f) => 3,
        _ if takes(a) => 4,
        Action::RemoveRemoteFolder(_) | Action::RemoveLocalFolder(_) => 5,
        _ => 2,
    };
    let mut out: Vec<Action> = clashes.iter().map(|t| Action::LocalClash((*t).clone())).collect();
    let (dropped, kept): (Vec<Action>, Vec<Action>) = actions.into_iter().partition(|a| touches(&clashes, a));
    // A folder that a set-aside path was going to leave (a move out of it, a delete in it) isn't
    // emptied this cycle: its removal waits too.
    let left = |f: &str| {
        dropped.iter().any(|a| match a {
            Action::MoveRemote { from: p, .. }
            | Action::MoveLocal { from: p, .. }
            | Action::DeleteRemote { path: p, .. }
            | Action::DeleteLocal(p) => p.starts_with(&format!("{f}/")),
            _ => false,
        })
    };
    let mut rest: Vec<Action> =
        kept.into_iter().filter(|a| !matches!(a, Action::RemoveRemoteFolder(f) | Action::RemoveLocalFolder(f) if left(f))).collect();
    // Stable: each kind keeps its order (parents made before children, the deepest folder removed
    // first).
    rest.sort_by_key(rank);
    let first = rest.iter().position(|a| rank(a) >= 3).unwrap_or(rest.len());
    let after = rest.iter().position(|a| rank(a) >= 5).unwrap_or(rest.len());
    let tail = rest.split_off(after);
    let swaps = rest.split_off(first);
    out.extend(rest);
    out.extend(swap_in(swaps));
    out.extend(tail);
    out
}

/// The removals of folders replaced by a file, and the files that take their places, in an order
/// where each folder is removed after everything moves out of it (and after its subfolders), and
/// each file comes after the folder at its place is gone: a file can move from one replaced folder
/// to where another one was. Where no order works (two files that swap places with each other's
/// folder), the rest stays as it was, removals first, and the next cycle finishes it.
fn swap_in(mut pending: Vec<Action>) -> Vec<Action> {
    fn removed(a: &Action) -> Option<&str> {
        match a {
            Action::RemoveRemoteFolder(f) | Action::RemoveLocalFolder(f) => Some(f),
            _ => None,
        }
    }
    fn under(p: &str, f: &str) -> bool {
        p.strip_prefix(f).is_some_and(|rest| rest.starts_with('/'))
    }
    fn ready(a: &Action, pending: &[Action]) -> bool {
        match removed(a) {
            Some(f) => !pending.iter().any(|b| match b {
                Action::MoveRemote { from, .. } | Action::MoveLocal { from, .. } => under(from, f),
                other => removed(other).is_some_and(|g| under(g, f)),
            }),
            None => {
                let at = action_path(a);
                !pending.iter().filter_map(removed).any(|f| at == f || under(at, f))
            }
        }
    }
    let mut out = Vec::with_capacity(pending.len());
    while let Some(next) = pending.iter().position(|a| ready(a, &pending)) {
        out.push(pending.remove(next));
    }
    out.extend(pending);
    out
}

fn decide(known: Option<&Known>, local: Option<&LocalFile>, remote: Option<&RemoteFile>, path: &str) -> Option<Action> {
    let p = || path.to_owned();
    match (known, local, remote) {
        (None, None, None) => None,
        (None, Some(_), None) => Some(Action::UploadNew(p())),
        (None, None, Some(_)) => Some(Action::Download(p())),
        (None, Some(_), Some(_)) => Some(Action::Conflict(p())),
        (Some(_), None, None) => Some(Action::Forget(p())),
        (Some(k), Some(l), Some(r)) => match (local_changed(k, l), r.etag != k.etag) {
            (false, false) => None,
            (true, false) => Some(Action::UploadChange { path: p(), etag: k.etag.clone() }),
            (false, true) => Some(Action::Download(p())),
            (true, true) => Some(Action::Conflict(p())),
        },
        // Deleted locally: delete in the cloud, unless the cloud copy changed meanwhile.
        (Some(k), None, Some(r)) => {
            Some(if r.etag == k.etag { Action::DeleteRemote { path: p(), etag: k.etag.clone() } } else { Action::Download(p()) })
        }
        // Deleted in the cloud: delete locally, unless the local copy changed meanwhile.
        (Some(k), Some(l), None) => Some(if local_changed(k, l) { Action::UploadNew(p()) } else { Action::DeleteLocal(p()) }),
    }
}

/// ETags as OxiCloud sends them, with or without quotes.
fn same_etag(a: &str, b: &str) -> bool {
    a.trim_matches('"') == b.trim_matches('"')
}

/// Turns a delete plus a new file that are one file moved into a move.
///
/// - Here: a synced file is gone and a new one has its exact size and modification time (a
///   rename keeps both) → [`Action::MoveRemote`].
/// - In the cloud: a synced file is gone and a new one has its ETag (OxiCloud's ETag is the
///   content hash and time, and a `MOVE` keeps it) → [`Action::MoveLocal`].
///
/// Only one-to-one matches pair; anything ambiguous stays a delete plus a new file. Each file is
/// in one move at most: a local file that is where a synced file moved to here is that file, not
/// an edit of the one synced at its path, so that one's rename in the cloud is not followed here
/// (its new name then comes down as a new file, and both are kept). The same the other way round:
/// a cloud file with the ETag of another synced file (gone from its own place) is that file moved
/// onto it, not an edit, so it isn't moved along with a rename made here.
/// A file moved to where the folder holding it was, or into a folder that took its own place, also
/// stays a delete plus a new file.
fn pair_moves(state: &State, local: &Tree<LocalFile>, remote: &Tree<RemoteFile>, files: Vec<Action>) -> Vec<Action> {
    // A file moved to where the folder holding it was (`a/b` → `a`), or into a folder that took its
    // own place (`a` → `a/b`), can't be one move on the other side: the folder (or file) in the way
    // goes only once the file is out of it. It stays a delete plus a new file.
    let nested = |from: &str, to: &str| from.starts_with(&format!("{to}/")) || to.starts_with(&format!("{from}/"));
    let unique = |mut v: Vec<String>| if v.len() == 1 { v.pop() } else { None };
    let mut moves: BTreeMap<String, Action> = BTreeMap::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    // Moves here first, so that their new names are known before a cloud rename claims one.
    for action in &files {
        match action {
            Action::DeleteRemote { path: from, etag } => {
                let Some(k) = state.files.get(from) else { continue };
                let same =
                    |a: &Action| matches!(a, Action::UploadNew(p) if local.files.get(p).is_some_and(|l| l.size == k.size && l.mtime == k.mtime));
                let Some(to) = unique(files.iter().filter(|a| same(a)).map(|a| action_path(a).to_owned()).collect()) else { continue };
                if !nested(from, &to) && taken.insert(to.clone()) {
                    moves.insert(from.clone(), Action::MoveRemote { from: from.clone(), to, etag: etag.clone() });
                }
            }
            // Renamed here while it changed in the cloud: moved there too (the cloud's version, by
            // its ETag now), and the change then comes down under the new name. Not when the cloud
            // file there is another synced file moved onto it (its ETag, gone from its own place).
            Action::Download(from) if !local.files.contains_key(from) => {
                let (Some(k), Some(r)) = (state.files.get(from), remote.files.get(from)) else { continue };
                if state.files.iter().any(|(p, other)| p != from && same_etag(&other.etag, &r.etag) && !remote.files.contains_key(p)) {
                    continue;
                }
                let same =
                    |a: &Action| matches!(a, Action::UploadNew(p) if local.files.get(p).is_some_and(|l| l.size == k.size && l.mtime == k.mtime));
                let Some(to) = unique(files.iter().filter(|a| same(a)).map(|a| action_path(a).to_owned()).collect()) else { continue };
                if !nested(from, &to) && taken.insert(to.clone()) {
                    moves.insert(from.clone(), Action::MoveRemote { from: from.clone(), to, etag: r.etag.clone() });
                }
            }
            _ => {}
        }
    }
    for action in &files {
        match action {
            // Renamed in the cloud; here it's unchanged (a delete) or changed meanwhile (an upload
            // of a file that's gone there): either way it's renamed here too, and a change then
            // goes up under the new name. Not when the file here is one moved there from elsewhere.
            Action::DeleteLocal(from) | Action::UploadNew(from) if !remote.files.contains_key(from) && !taken.contains(from) => {
                let Some(k) = state.files.get(from) else { continue };
                let same = |a: &Action| matches!(a, Action::Download(p) if !state.files.contains_key(p) && remote.files.get(p).is_some_and(|r| same_etag(&r.etag, &k.etag)));
                let Some(to) = unique(files.iter().filter(|a| same(a)).map(|a| action_path(a).to_owned()).collect()) else { continue };
                if !nested(from, &to) && taken.insert(to.clone()) {
                    moves.insert(from.clone(), Action::MoveLocal { from: from.clone(), to });
                }
            }
            _ => {}
        }
    }
    if moves.is_empty() {
        return files;
    }
    files
        .into_iter()
        .filter_map(|a| match &a {
            Action::DeleteRemote { path, .. } | Action::DeleteLocal(path) => Some(moves.remove(path).unwrap_or(a)),
            Action::UploadNew(p) | Action::Download(p) if moves.contains_key(p) => moves.remove(p),
            Action::UploadNew(p) | Action::Download(p) if taken.contains(p) => None,
            _ => Some(a),
        })
        .collect()
}

/// Turns the deletes of every file in a folder deleted here into one delete of the folder (the
/// top-most one): a synced folder gone here and still in the cloud, where each synced file in it
/// is a plain delete (unchanged in the cloud) and the cloud holds nothing else in it (no new file
/// or folder). Otherwise the files go one by one, as before.
fn group_folder_deletes(state: &State, local: &Tree<LocalFile>, remote: &Tree<RemoteFile>, files: Vec<Action>) -> Vec<Action> {
    let deleted: BTreeMap<&str, &str> = files
        .iter()
        .filter_map(|a| match a {
            Action::DeleteRemote { path, etag } => Some((path.as_str(), etag.as_str())),
            _ => None,
        })
        .collect();
    if deleted.is_empty() {
        return files;
    }
    let whole = |folder: &str| {
        let prefix = format!("{folder}/");
        let under = |p: &&String| p.starts_with(&prefix);
        state.folders.contains(folder)
            && !local.folders.contains(folder)
            && remote.folders.contains(folder)
            && state.files.keys().filter(under).all(|p| deleted.contains_key(p.as_str()))
            && remote.files.keys().filter(under).all(|p| state.files.contains_key(p))
            && remote.folders.iter().filter(under).all(|f| state.folders.contains(f))
            && state.files.keys().any(|p| under(&p))
    };
    let mut groups: Vec<String> = Vec::new();
    for folder in state.folders.iter().filter(|f| whole(f)) {
        // Top-most only: BTreeSet order puts `a` before `a/b`.
        if !groups.iter().any(|g| folder.starts_with(&format!("{g}/"))) {
            groups.push(folder.clone());
        }
    }
    if groups.is_empty() {
        return files;
    }
    let group_of = |p: &str| groups.iter().find(|g| p.starts_with(&format!("{g}/"))).cloned();
    let mut grouped: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut out = Vec::new();
    for action in files {
        match &action {
            Action::DeleteRemote { path, etag } => match group_of(path) {
                Some(g) => grouped.entry(g).or_default().push((path.clone(), etag.clone())),
                None => out.push(action),
            },
            _ => out.push(action),
        }
    }
    out.extend(grouped.into_iter().map(|(path, files)| Action::DeleteRemoteFolder { path, files }));
    out
}

/// The plan cut to what the server allows (docs/WEBDAV-STORAGE.md § 9).
///
/// - Two-way ([`Mode::TwoWay`]): the plan as it is.
/// - Refused ([`Mode::Refused`]): nothing.
/// - Download only ([`Mode::DownloadOnly`]): nothing that writes, moves or deletes on the server, and nothing
///   that deletes a file here for good:
///   - made, changed or new here (a folder to make, an upload, a removal there): left out; the
///     change stays here as it is;
///   - deleted in the cloud, unchanged here ([`Action::DeleteLocal`]): moved into the local
///     `.craftcloud/trash/` ([`Action::TrashLocal`]); a plan past the mass-delete limit is
///     refused before it gets here, so this comes only after the person's say;
///   - changed on both sides ([`Action::Conflict`]): the local file is set aside here, never
///     uploaded ([`Action::LocalConflict`]);
///   - deleted, renamed or moved here (a cloud delete, a cloud folder to trash, a cloud move):
///     the state forgets the synced file ([`Action::Forget`]), so the next plan downloads it
///     again (the local folder mirrors the cloud), and a renamed file stays here under its new
///     name as a file new here.
///
/// Downloads, local folders made or removed once empty, local renames that follow the cloud and
/// files forgotten stay as they are.
pub fn restrict(plan: Vec<Action>, mode: Mode) -> Vec<Action> {
    match mode {
        Mode::TwoWay => plan,
        Mode::Refused => Vec::new(),
        Mode::DownloadOnly => plan.into_iter().flat_map(download_only).collect(),
    }
}

/// The folder moves ([`folder_moves`]) the server allows: all of them both ways; under download
/// only the ones made in the cloud (a local rename to match), never one made here (a cloud
/// `MOVE`); none when refused.
pub fn restrict_folder_moves(moves: Vec<FolderMove>, mode: Mode) -> Vec<FolderMove> {
    match mode {
        Mode::TwoWay => moves,
        Mode::DownloadOnly => moves.into_iter().filter(|m| m.side == Side::Remote).collect(),
        Mode::Refused => Vec::new(),
    }
}

/// One two-way action, cut to download only (see [`restrict`]).
fn download_only(action: Action) -> Vec<Action> {
    match action {
        Action::MakeRemoteFolder(_) | Action::UploadNew(_) | Action::UploadChange { .. } | Action::RemoveRemoteFolder(_) => vec![],
        Action::Conflict(p) => vec![Action::LocalConflict(p)],
        Action::DeleteLocal(p) => vec![Action::TrashLocal(p)],
        Action::DeleteRemote { path, .. } | Action::MoveRemote { from: path, .. } => vec![Action::Forget(path)],
        Action::DeleteRemoteFolder { files, .. } => files.into_iter().map(|(p, _)| Action::Forget(p)).collect(),
        keep @ (Action::MakeLocalFolder(_)
        | Action::Download(_)
        | Action::MoveLocal { .. }
        | Action::LocalClash(_)
        | Action::Forget(_)
        | Action::RemoveLocalFolder(_)
        | Action::TrashLocal(_)
        | Action::LocalConflict(_)) => vec![keep],
    }
}

/// Where a folder was renamed or moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Here: the cloud folder is moved too (one WebDAV `MOVE`, which keeps its id, shares and
    /// stars, and everything in it).
    Local,
    /// In the cloud: the local folder is renamed to match.
    Remote,
}

/// A synced folder that was renamed or moved as a whole on one side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderMove {
    pub from: String,
    pub to: String,
    pub side: Side,
}

/// Folders renamed or moved as a whole on one side, to be done as one move on the other side
/// before [`reconcile`] runs (which then sees the folder in its new place on both sides; see
/// [`move_state`] and [`move_tree`]).
///
/// A synced folder `from` vanished on one side (and is still on the other), a new folder `to`
/// appeared on that side only, and every file synced under `from` is under `to` at the same
/// relative path, unchanged: same size and modification time here (a rename keeps both), same
/// ETag in the cloud (a `MOVE` keeps it). More files under `to` are fine (new files, uploaded or
/// downloaded after the move), and so are changes on the other side under `from` (they move along
/// and are synced after, by the usual rules). Each folder pairs one-to-one; anything else (a file
/// missing or changed under `to`, two candidates) is no folder move, and the files are handled
/// one by one as before. Only the top-most folder of a moved tree is listed.
pub fn folder_moves(state: &State, local: &Tree<LocalFile>, remote: &Tree<RemoteFile>) -> Vec<FolderMove> {
    let mut found = Vec::new();
    for side in [Side::Local, Side::Remote] {
        let (gone_from, other): (&BTreeSet<String>, &BTreeSet<String>) = match side {
            Side::Local => (&local.folders, &remote.folders),
            Side::Remote => (&remote.folders, &local.folders),
        };
        let vanished: Vec<&String> = state.folders.iter().filter(|f| !gone_from.contains(*f) && other.contains(*f)).collect();
        let fresh: Vec<&String> = gone_from.iter().filter(|f| !state.folders.contains(*f) && !other.contains(*f)).collect();
        if vanished.is_empty() || fresh.is_empty() {
            continue;
        }
        let same = |known: &Known, to: &str| match side {
            Side::Local => local.files.get(to).is_some_and(|l| l.size == known.size && l.mtime == known.mtime),
            Side::Remote => remote.files.get(to).is_some_and(|r| same_etag(&r.etag, &known.etag)),
        };
        let mut pairs: Vec<(&String, &String)> = Vec::new();
        for from in vanished {
            let prefix = format!("{from}/");
            let synced: Vec<(&str, &Known)> = state.files.iter().filter_map(|(p, k)| p.strip_prefix(&prefix).map(|rel| (rel, k))).collect();
            if synced.is_empty() {
                continue;
            }
            let candidates: Vec<&String> = fresh
                .iter()
                .filter(|to| !to.starts_with(&prefix) && !from.starts_with(&format!("{to}/")))
                .filter(|to| synced.iter().all(|(rel, k)| same(k, &format!("{to}/{rel}"))))
                .copied()
                .collect();
            if let [to] = candidates[..] {
                pairs.push((from, to));
            }
        }
        for (from, to) in &pairs {
            let one_to_one = pairs.iter().filter(|(_, t)| t == to).count() == 1;
            // A folder inside one that moves goes along with it.
            let inside_another = pairs.iter().any(|(f, _)| from.starts_with(&format!("{f}/")));
            if one_to_one && !inside_another {
                found.push(FolderMove { from: (*from).clone(), to: (*to).clone(), side });
            }
        }
    }
    found
}

/// `path` with its leading folder `from` replaced by `to` (`None` when it isn't in `from`).
fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_owned());
    }
    path.strip_prefix(from).and_then(|rest| rest.strip_prefix('/')).map(|rel| format!("{to}/{rel}"))
}

fn move_keys<V>(map: &mut BTreeMap<String, V>, from: &str, to: &str) {
    let moving: Vec<String> = map.keys().filter(|p| moved_path(p, from, to).is_some()).cloned().collect();
    for old in moving {
        if let (Some(value), Some(new)) = (map.remove(&old), moved_path(&old, from, to)) {
            map.insert(new, value);
        }
    }
}

fn move_set(set: &mut BTreeSet<String>, from: &str, to: &str) {
    let moving: Vec<String> = set.iter().filter(|p| moved_path(p, from, to).is_some()).cloned().collect();
    for old in moving {
        set.remove(&old);
        set.extend(moved_path(&old, from, to));
    }
}

/// The sync state after folder `from` was moved to `to` on both sides: what was synced there is
/// synced at the new place (the ETags stay: a `MOVE` keeps them).
pub fn move_state(state: &mut State, from: &str, to: &str) {
    move_keys(&mut state.files, from, to);
    move_set(&mut state.folders, from, to);
}

/// A scanned side after folder `from` was moved to `to` on it.
pub fn move_tree<F>(tree: &mut Tree<F>, from: &str, to: &str) {
    move_keys(&mut tree.files, from, to);
    move_set(&mut tree.folders, from, to);
}

/// The path an action is about (a move's destination).
fn action_path(action: &Action) -> &str {
    match action {
        Action::MakeRemoteFolder(p)
        | Action::MakeLocalFolder(p)
        | Action::UploadNew(p)
        | Action::UploadChange { path: p, .. }
        | Action::Download(p)
        | Action::Conflict(p)
        | Action::DeleteRemote { path: p, .. }
        | Action::DeleteLocal(p)
        | Action::DeleteRemoteFolder { path: p, .. }
        | Action::MoveRemote { to: p, .. }
        | Action::MoveLocal { to: p, .. }
        | Action::LocalClash(p)
        | Action::Forget(p)
        | Action::RemoveRemoteFolder(p)
        | Action::RemoveLocalFolder(p)
        | Action::TrashLocal(p)
        | Action::LocalConflict(p) => p,
    }
}

fn local_changed(known: &Known, local: &LocalFile) -> bool {
    known.size != local.size || known.mtime != local.mtime
}

/// Every folder a path needs, parents first: `a/b/c.txt` → `a`, `a/b`.
fn parents(path: &str) -> impl Iterator<Item = String> + '_ {
    let parts: Vec<&str> = path.split('/').collect();
    (1..parts.len()).map(move |n| parts[..n].join("/"))
}

/// Folders to create, parents first: new empty folders on one side, and the parents of files
/// about to cross over.
fn folders_needed(state: &State, local: &Tree<LocalFile>, remote: &Tree<RemoteFile>, files: &[Action]) -> Vec<Action> {
    let mut make_remote = BTreeSet::new();
    let mut make_local = BTreeSet::new();
    for f in local.folders.iter().filter(|f| !remote.folders.contains(*f) && !state.folders.contains(*f)) {
        make_remote.extend(parents(&format!("{f}/x")));
    }
    for f in remote.folders.iter().filter(|f| !local.folders.contains(*f) && !state.folders.contains(*f)) {
        make_local.extend(parents(&format!("{f}/x")));
    }
    for action in files {
        match action {
            Action::UploadNew(p) | Action::UploadChange { path: p, .. } => make_remote.extend(parents(p)),
            Action::Download(p) => make_local.extend(parents(p)),
            Action::MoveRemote { to, .. } => make_remote.extend(parents(to)),
            Action::MoveLocal { to, .. } => make_local.extend(parents(to)),
            Action::Conflict(p) => {
                make_remote.extend(parents(p));
                make_local.extend(parents(p));
            }
            _ => {}
        }
    }
    make_remote.retain(|f| !remote.folders.contains(f));
    make_local.retain(|f| !local.folders.contains(f));
    // BTreeSet order puts `a` before `a/b`.
    make_remote.into_iter().map(Action::MakeRemoteFolder).chain(make_local.into_iter().map(Action::MakeLocalFolder)).collect()
}

/// Folders gone on one side (and known before) are removed on the other once nothing will be
/// left in them, deepest first.
fn remove_empty_folders(state: &State, local: &Tree<LocalFile>, remote: &Tree<RemoteFile>, folders: &[Action], actions: &[Action]) -> Vec<Action> {
    let created: BTreeSet<&String> = folders
        .iter()
        .filter_map(|a| match a {
            Action::MakeRemoteFolder(f) | Action::MakeLocalFolder(f) => Some(f),
            _ => None,
        })
        .collect();
    // Files that will still exist (on either side) after this plan.
    let staying: BTreeSet<&String> = actions
        .iter()
        .filter_map(|a| match a {
            Action::UploadNew(p)
            | Action::UploadChange { path: p, .. }
            | Action::Download(p)
            | Action::Conflict(p)
            | Action::MoveRemote { to: p, .. }
            | Action::MoveLocal { to: p, .. } => Some(p),
            _ => None,
        })
        .chain(state.files.keys().filter(|p| {
            !actions.iter().any(|a| {
                matches!(a, Action::DeleteLocal(x) | Action::DeleteRemote { path: x, .. } | Action::Forget(x) | Action::MoveRemote { from: x, .. } | Action::MoveLocal { from: x, .. } if x == *p)
            })
        }))
        .collect();
    // Folders going to the trash whole, with everything in them.
    let trashed: Vec<&String> = actions
        .iter()
        .filter_map(|a| match a {
            Action::DeleteRemoteFolder { path, .. } => Some(path),
            _ => None,
        })
        .collect();
    let in_trashed = |f: &str| trashed.iter().any(|t| f == t.as_str() || f.starts_with(&format!("{t}/")));
    let has_files = |folder: &str| staying.iter().any(|p| p.starts_with(&format!("{folder}/")) && !in_trashed(p));
    let has_folders_left = |folder: &str, gone: &dyn Fn(&str) -> bool| {
        local.folders.iter().chain(remote.folders.iter()).any(|f| f.starts_with(&format!("{folder}/")) && !gone(f))
    };
    let mut out = Vec::new();
    // Deepest first: reverse order puts `a/b` before `a`.
    for folder in state.folders.iter().rev() {
        if created.contains(folder) || has_files(folder) || in_trashed(folder) {
            continue;
        }
        let gone_locally = !local.folders.contains(folder);
        let gone_remotely = !remote.folders.contains(folder);
        let gone = |f: &str| state.folders.contains(f) && (!local.folders.contains(f) || !remote.folders.contains(f));
        if has_folders_left(folder, &gone) {
            continue;
        }
        if gone_locally && !gone_remotely {
            out.push(Action::RemoveRemoteFolder(folder.clone()));
        } else if gone_remotely && !gone_locally {
            out.push(Action::RemoveLocalFolder(folder.clone()));
        }
    }
    out
}

/// A conflict copy's name: `Q3 report (conflict 2026-11-13 14.32 Studio).docx`. A conflict copy
/// of a conflict copy is named after the original, never nested.
pub fn conflict_name(path: &str, when: &str, device: &str) -> String {
    numbered_conflict(path, when, device, 1)
}

/// A conflict copy's name that isn't `taken` yet: the plain one, else with a counter after the
/// device, `Q3 report (conflict 2026-11-13 14.32 Studio 2).docx`, 3, …
pub fn free_conflict_name(path: &str, when: &str, device: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..1000)
        .map(|n| numbered_conflict(path, when, device, n))
        .find(|name| !taken(name))
        .unwrap_or_else(|| numbered_conflict(path, when, device, 1000))
}

fn numbered_conflict(path: &str, when: &str, device: &str, n: u32) -> String {
    let original = original_of_conflict(path).unwrap_or_else(|| path.to_owned());
    let (dir, file) = original.rsplit_once('/').map_or(("", original.as_str()), |(d, f)| (d, f));
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (file, String::new()),
    };
    let count = if n > 1 { format!(" {n}") } else { String::new() };
    let name = format!("{stem} (conflict {when} {device}{count}){ext}");
    if dir.is_empty() { name } else { format!("{dir}/{name}") }
}

/// The original a conflict copy belongs to: `Docs/Q3 report (conflict 2026-11-13 14.32 Studio).docx`
/// → `Docs/Q3 report.docx`. `None` for names that aren't conflict copies.
pub fn original_of_conflict(path: &str) -> Option<String> {
    let (dir, file) = path.rsplit_once('/').map_or(("", path), |(d, f)| (d, f));
    let start = file.find(" (conflict ")?;
    let end = start + file[start..].find(')')?;
    let name = format!("{}{}", &file[..start], &file[end + 1..]);
    if name.is_empty() {
        return None;
    }
    Some(if dir.is_empty() { name } else { format!("{dir}/{name}") })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod properties;
