//! Two-way sync: scans both sides, asks `core::reconcile` what to do, does it, and records each
//! step as it succeeds in `<local>/.craftcloud/state.json` (so a crash never turns into fake
//! conflicts). Runs a cycle at start, then every [`INTERVAL`], or at once when asked.
//!
//! Safety rails: a missing local folder pauses (an unmounted drive must not read as "everything
//! deleted"); a cycle that would delete more than half of the known files pauses until confirmed;
//! a file written in the last two seconds waits for the next cycle; a changed file never
//! overwrites a cloud version it didn't start from (412 → conflict copy); a delete goes to the
//! cloud's trash (over REST: WebDAV's `DELETE` deletes for good), never past a newer version.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SendError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::{self, Tokens};
use craftcloud_core::api::dav::{self, Uploaded, Write};
use craftcloud_core::api::resources::Kind;
use craftcloud_core::api::{admin, files, folders, trash};
use craftcloud_core::http::HttpResponse;
use craftcloud_core::names::{self, Platform};
use craftcloud_core::reconcile::{self, Action, Known, LocalFile, RemoteFile, State, Tree};
use craftcloud_core::sync::{
    ActivityEvent, ActivityKind, ConflictCopy, FileStatus, Keep, LeftOut, SyncPlan, SyncProgress, SyncState, folder_stats, in_plan, local_relative,
};
use serde::{Deserialize, Serialize};

use crate::http::{self, Waker};
pub use crate::session::Session;

/// What the sync says while the server can't be reached (the error itself goes to the log).
pub const OFFLINE: &str = "Can't reach the server. Trying again…";

/// How often the cloud is checked when nothing asks for a sync sooner.
pub const INTERVAL: Duration = Duration::from_secs(15);
/// The wait after a cycle that moved something: changes come in bursts (an app saving several
/// files, another computer still uploading), so look again soon.
const BUSY_INTERVAL: Duration = Duration::from_secs(4);
/// Folders listed at the same time during a scan.
const LISTERS: usize = 4;
/// How long an upload refused for the quota waits before it's tried again anyway (the cloud's
/// count of the storage use lags behind, e.g. after emptying the trash).
const NO_ROOM_RETRY: Duration = Duration::from_secs(5 * 60);
/// How often an admin's client asks the cloud to count the storage use again.
const RECOUNT_EVERY: Duration = Duration::from_secs(60);
/// Files written more recently than this wait for the next cycle (an app may still be saving).
pub const SETTLE_MS: i64 = 2_000;
/// How long the folder watcher waits after the last change (a bit over `SETTLE_MS`, so the
/// changed files count as settled when the cycle runs).
const SETTLE: Duration = Duration::from_millis(2_500);

/// Called with new tokens after a refresh, so they can be saved.
pub use crate::session::TokenSink;

/// Shared with the UI.
pub type Progress = Arc<Mutex<SyncProgress>>;

/// Requests to the sync thread.
pub enum Command {
    /// Run a cycle now.
    SyncNow,
    /// Run a cycle now; its number shows up in `SyncProgress::served` once that cycle is done.
    SyncFor(u64),
    /// Something changed in the local folder (from the watcher): sync once it settles.
    LocalChange,
    /// Run a cycle now and carry out deletions that were paused.
    ConfirmDeletes,
    /// Run a cycle now and bring back the files whose deletion was paused instead (downloaded
    /// again where they were deleted here, put back from the cloud's trash where they were deleted
    /// there, or uploaded again when they aren't in the trash any more).
    DiscardDeletes,
    /// Stop (`true`) or resume (`false`) syncing.
    Pause(bool),
    /// Sync other folders: local copies of folders no longer chosen are removed where unchanged.
    Replan(SyncPlan),
    /// Settle a conflict copy.
    Resolve(ConflictCopy, Keep),
    /// Name new conflict copies after this device name.
    Rename(String),
    Stop,
}

pub struct Handle {
    pub progress: Progress,
    commands: Commands,
}

impl Handle {
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Another way in to the sync thread (the command line's control channel). Unlike the
    /// handle, dropping it doesn't stop the sync.
    pub fn sender(&self) -> Commands {
        self.commands.clone()
    }
}

/// The way in to the sync thread. Pause and Stop also stop a transfer under way at once (the
/// thread reads commands only between cycles).
#[derive(Clone)]
pub struct Commands {
    to_engine: Sender<Command>,
    cancel: Arc<AtomicBool>,
}

impl Commands {
    /// Commands that go to `to_engine`, stopping transfers through `cancel`.
    pub fn new(to_engine: Sender<Command>, cancel: Arc<AtomicBool>) -> Self {
        Self { to_engine, cancel }
    }

    pub fn send(&self, command: Command) -> Result<(), SendError<Command>> {
        if matches!(command, Command::Pause(true) | Command::Stop) {
            self.cancel.store(true, Ordering::Relaxed);
        }
        self.to_engine.send(command)
    }
}

impl From<Sender<Command>> for Commands {
    fn from(to_engine: Sender<Command>) -> Self {
        Self::new(to_engine, Arc::default())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
    }
}

/// Starts syncing on its own thread; refreshed tokens go to `on_tokens`.
pub fn start(plan: SyncPlan, tokens: Tokens, device: String, on_tokens: TokenSink, waker: Waker) -> Handle {
    match ServerUrl::parse(&plan.server) {
        Ok(server) => start_with(plan, Session::new(server, tokens, on_tokens), device, waker),
        Err(e) => failed(&e.to_string()),
    }
}

/// Starts syncing on its own thread with the caller's session (e.g. [`Session::shared`], so the
/// command line and the desktop client share one sign-in).
pub fn start_with(plan: SyncPlan, session: Session, device: String, waker: Waker) -> Handle {
    let progress: Progress = Arc::default();
    let (to_engine, inbox) = channel();
    let commands = Commands::new(to_engine.clone(), Arc::clone(&session.cancel));
    let shared = Arc::clone(&progress);
    let to_self = to_engine;
    let spawned = std::thread::Builder::new().name("craftcloud-sync".into()).spawn(move || {
        let watcher = watch(Path::new(&plan.local_folder), to_self);
        let mut engine = Engine {
            plan,
            session,
            device,
            progress: shared,
            waker,
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
            _watcher: watcher,
        };
        engine.run(&inbox);
    });
    if let Err(e) = spawned {
        progress.lock().unwrap_or_else(PoisonError::into_inner).state = SyncState::Failed(format!("couldn't start syncing: {e}"));
    }
    Handle { progress, commands }
}

/// Why a cycle stopped early.
enum Stop {
    /// Waits for the person (a mass delete, a missing folder).
    Hold(SyncState),
    Error(String),
}

impl From<String> for Stop {
    fn from(e: String) -> Self {
        Self::Error(e)
    }
}

/// A handle on a sync that couldn't start.
fn failed(why: &str) -> Handle {
    let (commands, _) = channel();
    let progress: Progress = Arc::default();
    progress.lock().unwrap_or_else(PoisonError::into_inner).state = SyncState::Failed(why.to_owned());
    Handle { progress, commands: commands.into() }
}

/// What `state.json` holds.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    server: String,
    #[serde(flatten)]
    state: State,
    /// Conflict copies the person chose to keep as files of their own.
    #[serde(default)]
    kept_conflicts: BTreeSet<String>,
}

struct Engine {
    plan: SyncPlan,
    session: Session,
    device: String,
    progress: Progress,
    waker: Waker,
    paused: bool,
    /// Local files this client itself just changed (a settled conflict): no 2-second wait for them.
    settled: BTreeSet<String>,
    /// REST folder ids by cloud path (`""` is the drive's root), learnt this cycle: WebDAV has no
    /// ids, and moving to the trash goes by id.
    ids: BTreeMap<String, String>,
    /// Local things outside the plan already reported in the activity list (once per run).
    reported: BTreeSet<String>,
    /// Uploads the cloud refused for its quota, with the file's size and time then: not tried
    /// again while unchanged, until there is room (or Sync now).
    no_room: BTreeMap<String, (LocalFile, Instant)>,
    /// Uploads the cloud refused outright (a 4xx for the name or the file), with the file's size
    /// and time then: not tried again until the file changes.
    refused: BTreeMap<String, LocalFile>,
    /// When the cloud was last asked to count the storage use again (admins only).
    recounted: Option<Instant>,
    /// Folder moves (from, to) that failed: their files are synced one by one instead.
    no_folder_move: BTreeSet<(String, String)>,
    /// The name rules of this computer's file system (`names::Platform`).
    platform: Platform,
    /// When leftover part files were last looked for ([`sweep_stale_parts`]).
    parts_swept: Option<Instant>,
    /// Keeps the folder watcher alive (`None` where watching isn't possible: then the timer).
    _watcher: Option<notify::RecommendedWatcher>,
}

/// Watches the local folder and sends [`Command::LocalChange`] for changes to synced files.
fn watch(root: &Path, to_engine: Sender<Command>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    let handler = move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        // Our own bookkeeping and temporary files don't count.
        // Folders are only skipped by the always-ignored names; the temporary-file rules are for the
        // file itself (T-062: a folder called `Backup~` syncs).
        let relevant = event.paths.iter().any(|p| {
            let parts: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let Some((last, parents)) = parts.split_last() else { return false };
            !(parents.iter().any(|c| names::ignored_always(c)) || ignored(last))
        });
        if relevant {
            let _ = to_engine.send(Command::LocalChange);
        }
    };
    let mut watcher =
        notify::recommended_watcher(handler).map_err(|e| log::warn!("no folder watching ({e}); syncing every {}s", INTERVAL.as_secs())).ok()?;
    watcher.watch(root, notify::RecursiveMode::Recursive).map_err(|e| log::warn!("can't watch {}: {e}", root.display())).ok()?;
    Some(watcher)
}

impl Engine {
    fn run(&mut self, inbox: &Receiver<Command>) {
        let mut confirm = Deletes::Hold;
        let mut ticket = 0;
        loop {
            if self.paused {
                self.update(|p| p.state = SyncState::Paused);
            } else {
                // A pause asked for before this cycle has been read already (it would have stopped
                // the last cycle's transfer, not this one's).
                self.session.cancel.store(false, Ordering::Relaxed);
                self.cycle(confirm);
            }
            // Commands are read between cycles, so a ticket taken before this cycle is served by it.
            self.update(|p| p.served = p.served.max(ticket));
            confirm = Deletes::Hold;
            let busy = !self.paused && self.progress.lock().unwrap_or_else(PoisonError::into_inner).files_done > 0;
            let mut next = inbox.recv_timeout(if busy { BUSY_INTERVAL } else { INTERVAL });
            if let Ok(Command::LocalChange) = next {
                // Let a burst of changes (an app saving) settle into one cycle. Anything else that
                // arrives meanwhile is kept and handled below, not dropped.
                next = loop {
                    match inbox.recv_timeout(SETTLE) {
                        Ok(Command::LocalChange) => {}
                        Err(RecvTimeoutError::Timeout) => break Ok(Command::LocalChange),
                        other => break other,
                    }
                };
            }
            match next {
                // Asked for by the person: try what didn't fit again too.
                Ok(Command::SyncNow) => self.no_room.clear(),
                Ok(Command::LocalChange) | Err(RecvTimeoutError::Timeout) => {}
                Ok(Command::SyncFor(n)) => ticket = n,
                Ok(Command::ConfirmDeletes) => confirm = Deletes::Confirm,
                Ok(Command::DiscardDeletes) => confirm = Deletes::Discard,

                Ok(Command::Pause(on)) => self.paused = on,
                Ok(Command::Replan(plan)) => self.replan(plan),
                Ok(Command::Resolve(conflict, keep)) => self.resolve(&conflict, keep),
                Ok(Command::Rename(device)) => self.device = device,
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn state_path(&self) -> PathBuf {
        self.root().join(dav::CRAFTCLOUD_DIR).join("state.json")
    }

    fn load(&self) -> Option<Saved> {
        std::fs::read(self.state_path()).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// Switches to another set of folders. Local files of folders no longer chosen are removed
    /// only where they are exactly as last synced; anything else is left and reported.
    fn replan(&mut self, plan: SyncPlan) {
        let old = std::mem::replace(&mut self.plan, plan);
        self.reported.clear();
        let Some(mut saved) = self.load() else { return };
        let root = self.root();
        let mut kept = 0usize;
        // Whatever synced under the old plan and doesn't under the new one.
        let dropped: Vec<String> =
            saved.state.folders.iter().filter(|f| !f.contains('/') && in_plan(&old, f, true) && !in_plan(&self.plan, f, true)).cloned().collect();
        let gone: Vec<String> = saved.state.files.keys().filter(|p| in_plan(&old, p, false) && !in_plan(&self.plan, p, false)).cloned().collect();
        for path in gone {
            let Some(known) = saved.state.files.remove(&path) else { continue };
            let Some(file) = local_relative(&path).map(|r| root.join(r)) else { continue };
            match local_meta(&file) {
                Some(m) if m.size == known.size && m.mtime == known.mtime => {
                    let _ = std::fs::remove_file(&file);
                }
                Some(_) => kept += 1,
                None => {}
            }
        }
        for folder in &dropped {
            if let Some(dir) = local_relative(folder).map(|r| root.join(r)) {
                remove_empty_dirs(&dir);
            }
            saved.state.folders.retain(|f| f != folder && !f.starts_with(&format!("{folder}/")));
        }
        let _ = save_saved(&self.state_path(), &saved);
        self.update(|p| {
            for folder in &dropped {
                p.record(event(ActivityKind::Deleted, folder, None, "stopped syncing; the cloud keeps it".into()));
            }
            if kept > 0 {
                p.record(event(ActivityKind::Error, "", None, format!("kept {kept} changed files that are no longer synced")));
            }
        });
    }

    /// Settles a conflict copy, then the next cycle syncs the result.
    fn resolve(&mut self, conflict: &ConflictCopy, keep: Keep) {
        let root = self.root();
        let (Some(original), Some(copy)) =
            (local_relative(&conflict.original).map(|r| root.join(r)), local_relative(&conflict.copy).map(|r| root.join(r)))
        else {
            return;
        };
        let result = match keep {
            Keep::Both => {
                let mut saved = self.load().unwrap_or_default();
                saved.kept_conflicts.insert(conflict.copy.clone());
                save_saved(&self.state_path(), &saved)
            }
            Keep::Mine => {
                self.settled.insert(conflict.original.clone());
                std::fs::rename(&copy, &original).map_err(|e| e.to_string())
            }
            Keep::Cloud => std::fs::remove_file(&copy).map_err(|e| e.to_string()),
        };
        let path = conflict.original.clone();
        self.update(|p| match &result {
            Ok(()) => p.record(event(
                ActivityKind::Conflict,
                &path,
                None,
                match keep {
                    Keep::Both => "kept both".into(),
                    Keep::Mine => "kept yours".into(),
                    Keep::Cloud => "kept the cloud's".into(),
                },
            )),
            Err(e) => p.record(event(ActivityKind::Error, &path, None, format!("couldn't settle the conflict: {e}"))),
        });
    }

    fn update(&self, f: impl FnOnce(&mut SyncProgress)) {
        f(&mut self.progress.lock().unwrap_or_else(PoisonError::into_inner));
        (self.waker)();
    }

    fn root(&self) -> PathBuf {
        PathBuf::from(&self.plan.local_folder)
    }

    fn cycle(&mut self, confirm: Deletes) {
        self.session.offline = None;
        self.update(|p| {
            p.files_done = 0;
            p.files_total = 0;
        });
        let result = self.try_cycle(confirm);
        let state = match result {
            Ok(()) => SyncState::UpToDate,
            Err(Stop::Hold(state)) => state,
            Err(Stop::Error(_)) if self.session.signed_out => SyncState::SignedOut,
            Err(Stop::Error(e)) => match self.session.offline.take() {
                Some(why) => {
                    log::warn!("offline: {why}");
                    SyncState::Offline(OFFLINE.into())
                }
                None => SyncState::Failed(e),
            },
        };
        self.update(|p| {
            p.current.clear();
            p.cycles += 1;
            // What this cycle didn't get to isn't syncing any more (it's tried again next time).
            p.unsynced.retain(|_, s| *s != FileStatus::Syncing);
            if state == SyncState::UpToDate {
                p.last_synced = Some(jiff::Timestamp::now().as_second());
            }
            p.state = state;
        });
    }

    fn try_cycle(&mut self, confirm: Deletes) -> Result<(), Stop> {
        self.ids.clear();
        let root = self.root();
        let state_path = self.state_path();
        // Never made here: a folder that's gone was moved, deleted, or is on a drive that isn't
        // plugged in, and making it again would download everything into the wrong place. (Setup
        // makes it, and so does the person's "Download everything again".)
        if !root.is_dir() {
            return Err(Stop::Hold(SyncState::FolderMissing));
        }
        if self.parts_swept.is_none_or(|at| at.elapsed() >= SWEEP_PARTS_EVERY) {
            self.parts_swept = Some(Instant::now());
            let removed = sweep_stale_parts(&root, STALE_PART);
            if removed > 0 {
                log::info!("removed {removed} leftover part files of downloads that never finished");
            }
        }
        let saved = self.load().unwrap_or_default();
        let (mut state, kept_conflicts) = (saved.state, saved.kept_conflicts);

        self.update(|p| p.state = SyncState::Scanning);
        let mut remote = self.scan_remote()?;
        let (mut local, unreadable) = scan_local(&root, &self.plan, &state, &std::mem::take(&mut self.settled));
        self.report_unreadable(&unreadable);
        self.report_unsynced(&root);
        let refused_names = self.screen_names(&mut state, &mut local, &mut remote);
        // Whatever the sync leaves alone is left alone on both sides and in the state (T-063).
        drop_skipped(&mut state, &mut local, &mut remote, self.platform);
        if same_bytes_again(&root, &mut state, &local) {
            save_saved(&state_path, &Saved { server: self.plan.server.clone(), state: state.clone(), kept_conflicts: kept_conflicts.clone() })?;
        }
        let left_out: BTreeMap<String, LeftOut> = refused_names
            .into_iter()
            .map(|p| (p, LeftOut::Name))
            .chain(unreadable.iter().filter(|u| u.why != MID_SAVE).map(|u| {
                let why = match u.why.as_str() {
                    SYMLINK => LeftOut::Symlink,
                    TEMPORARY => LeftOut::Temporary,
                    _ => LeftOut::Unreadable,
                };
                (u.path.clone(), why)
            }))
            .collect();
        let not_synced: Vec<String> = left_out.keys().cloned().collect();
        let waiting = mid_save(&unreadable);
        self.move_folders(&root, &mut state, &mut local, &mut remote, &kept_conflicts, &unreadable)?;
        let (actions, held) = match reconcile::reconcile(&state, &local, &remote, confirm == Deletes::Confirm) {
            Ok(actions) => (actions, Vec::new()),
            Err(reconcile::Refusal::TooManyDeletes { .. }) => {
                let all = reconcile::reconcile(&state, &local, &remote, true).map_err(|e| e.to_string())?;
                if confirm == Deletes::Discard {
                    // What was deleted in the cloud comes back from its trash, keeping its id (and
                    // with it links, stars and versions); only what isn't there any more is
                    // uploaded again (T-038).
                    let restored = self.restore_trashed(&all)?;
                    if !restored.is_empty() {
                        let again = self.scan_remote()?;
                        for path in &restored {
                            if let Some(f) = again.files.get(path) {
                                remote.files.insert(path.clone(), f.clone());
                            }
                            let mut folder = path.as_str();
                            while let Some((parent, _)) = folder.rsplit_once('/') {
                                if again.folders.contains(parent) {
                                    remote.folders.insert(parent.to_owned());
                                }
                                folder = parent;
                            }
                        }
                    }
                    let back = bring_back(&mut state, &all, &restored);
                    save_saved(
                        &state_path,
                        &Saved { server: self.plan.server.clone(), state: state.clone(), kept_conflicts: kept_conflicts.clone() },
                    )?;
                    let detail = if restored.is_empty() {
                        format!("kept {back} files instead of deleting them")
                    } else {
                        format!("kept {back} files instead of deleting them ({} put back from the cloud's trash)", restored.len())
                    };
                    self.update(|p| p.record(event(ActivityKind::Skipped, "", None, detail)));
                    (reconcile::reconcile(&state, &local, &remote, true).map_err(|e| e.to_string())?, Vec::new())
                } else {
                    // Only the deletes wait; everything else syncs.
                    split_deletes(all)
                }
            }
        };
        let actions = self.hold_moved_out(&root, &state, actions);
        let actions = self.hold_no_room(&local, actions);
        let actions = hold_unreadable(actions, &unreadable);
        let transfers: Vec<&str> = actions.iter().filter(|a| is_transfer(a)).map(action_path).collect();
        self.update(|p| {
            p.state = SyncState::Downloading;
            p.files_total = transfers.len() as u64;
            p.files_done = 0;
            // Each file this cycle moves is syncing until it's done; an error stays until it works.
            p.unsynced.retain(|path, s| *s == FileStatus::Error && transfers.contains(&path.as_str()));
            for path in &transfers {
                p.unsynced.entry((*path).to_owned()).or_insert(FileStatus::Syncing);
            }
        });

        let mut folders: BTreeSet<String> = local.folders.intersection(&remote.folders).cloned().collect();
        folders.extend(held_folders(&state, &held));
        let mut saved = Saved { server: self.plan.server.clone(), state: State::default(), kept_conflicts };
        for action in actions {
            if self.session.cancel.load(Ordering::Relaxed) {
                // Paused or quitting: the rest waits (what was done so far is saved).
                return Err(Stop::Hold(SyncState::Paused));
            }
            let (label, path) = (describe(&action), action_path(&action).to_owned());
            self.update(|p| p.current = label.clone());
            match self.apply(&root, &action, &mut state, &mut folders, &remote) {
                Ok(Some((kind, detail))) => {
                    let moved = matches!(kind, ActivityKind::Up | ActivityKind::Down | ActivityKind::Conflict | ActivityKind::Moved);
                    let size = if moved { state.files.get(&path).map(|k| k.size) } else { None };
                    self.update(|p| {
                        p.files_done += 1;
                        p.unsynced.remove(&path);
                        p.record(event(kind, &path, size, detail));
                    });
                }
                Ok(None) => self.update(|p| {
                    p.unsynced.remove(&path);
                }),
                // One failed file doesn't stop the others; it is tried again next cycle (or, when it
                // doesn't fit in the cloud, once there's room).
                Err(e) if self.session.cancel.load(Ordering::Relaxed) => {
                    log::info!("{label}: {e}");
                    self.update(|p| {
                        p.unsynced.remove(&path);
                    });
                }
                Err(e) => {
                    let skipped = self.no_room.contains_key(&path) || self.refused.contains_key(&path);
                    let kind = if skipped { ActivityKind::Skipped } else { ActivityKind::Error };
                    self.update(|p| {
                        p.unsynced.insert(path.clone(), FileStatus::Error);
                        p.record(event(kind, &path, None, format!("couldn't {label}: {e}")));
                    });
                }
            }
            state.folders = folders.clone();
            saved.state = state.clone();
            save_saved(&state_path, &saved)?;
        }
        state.folders = folders;
        saved.state = state;
        // After this cycle's downloads, so a conflict copy that just arrived is listed at once.
        let conflicts = find_conflicts(&scan_local(&root, &self.plan, &saved.state, &BTreeSet::new()).0, &saved.kept_conflicts);
        save_saved(&state_path, &saved)?;
        let files = &saved.state.files;
        let no_room: Vec<String> = self.no_room.keys().cloned().collect();
        self.update(|p| {
            p.unsynced.retain(|_, s| *s == FileStatus::Error);
            for path in &no_room {
                p.unsynced.insert(path.clone(), FileStatus::Error);
            }
            p.no_room = no_room;
            for c in &conflicts {
                p.unsynced.insert(c.copy.clone(), FileStatus::Conflict);
                p.unsynced.insert(c.original.clone(), FileStatus::Conflict);
            }
            p.folders = folder_stats(files.iter().map(|(path, k)| (path.as_str(), k.size)), &p.unsynced);
            p.conflicts = conflicts;
            p.pending_deletes.clone_from(&held);
            p.not_synced.clone_from(&not_synced);
            p.left_out.clone_from(&left_out);
            p.waiting.clone_from(&waiting);
        });
        if held.is_empty() { Ok(()) } else { Err(Stop::Hold(SyncState::MassDelete { deletes: held.len() })) }
    }

    /// Does the folder renames and moves of one side on the other, each as one move (see
    /// `reconcile::folder_moves`), and updates the state and the scanned trees to match, so the
    /// plan sees each folder in its new place on both sides. A move that fails is not tried again
    /// this run: its files then sync one by one.
    fn move_folders(
        &mut self,
        root: &Path,
        state: &mut State,
        local: &mut Tree<LocalFile>,
        remote: &mut Tree<RemoteFile>,
        kept_conflicts: &BTreeSet<String>,
        unreadable: &[Unreadable],
    ) -> Result<(), Stop> {
        for m in reconcile::folder_moves(state, local, remote) {
            if self.no_folder_move.contains(&(m.from.clone(), m.to.clone())) {
                continue;
            }
            // Nothing moves from, into or around a place that can't be read.
            let touches = |place: &str| unreadable.iter().any(|u| within(&u.path, place) || within(place, &u.path));
            if touches(&m.from) || touches(&m.to) {
                continue;
            }
            self.update(|p| p.current = format!("move folder {} to {}", m.from, m.to));
            let done = match m.side {
                reconcile::Side::Local => self.move_remote_folder(&m.from, &m.to, remote),
                reconcile::Side::Remote => move_local_folder(root, &m.from, &m.to),
            };
            match done {
                Ok(()) => {
                    reconcile::move_state(state, &m.from, &m.to);
                    match m.side {
                        reconcile::Side::Local => reconcile::move_tree(remote, &m.from, &m.to),
                        reconcile::Side::Remote => reconcile::move_tree(local, &m.from, &m.to),
                    }
                    let saved = Saved { server: self.plan.server.clone(), state: state.clone(), kept_conflicts: kept_conflicts.clone() };
                    save_saved(&self.state_path(), &saved)?;
                    let detail = match m.side {
                        reconcile::Side::Local => format!("folder, from {}", m.from),
                        reconcile::Side::Remote => format!("folder moved in the cloud, from {}", m.from),
                    };
                    self.update(|p| {
                        p.files_done += 1;
                        p.record(event(ActivityKind::Moved, &m.to, None, detail));
                    });
                }
                Err(e) => {
                    log::warn!("moving folder {} to {}: {e}; syncing its files one by one", m.from, m.to);
                    self.no_folder_move.insert((m.from.clone(), m.to.clone()));
                }
            }
        }
        Ok(())
    }

    /// One `MOVE` of a folder in the cloud (its new parents made first), with everything in it.
    fn move_remote_folder(&mut self, from: &str, to: &str, remote: &mut Tree<RemoteFile>) -> Result<(), String> {
        let parents: Vec<&str> = to.match_indices('/').map(|(i, _)| &to[..i]).collect();
        for parent in parents {
            if remote.folders.contains(parent) {
                continue;
            }
            let resp = self.session.call(&dav::create_folder(&self.session.server, parent))?;
            if !dav::folder_exists_after(&resp) {
                return Err(format!("making {parent}: the server answered {}", resp.status));
            }
            remote.folders.insert(parent.to_owned());
        }
        let resp = self.session.call(&dav::move_to(&self.session.server, from, to, true))?;
        if !resp.is_success() {
            return Err(format!("the server answered {}", resp.status));
        }
        if let Err(e) = self.session.move_versions(from, to) {
            log::warn!("{e}");
        }
        Ok(())
    }

    /// Leaves out uploads that didn't fit in the cloud before, while the file is unchanged and the
    /// cloud still has no room for it (asked once per cycle, only when there are such files).
    ///
    /// The cloud's count of the storage use lags (it doesn't drop when the trash is emptied until
    /// its next recount, up to 10 minutes): an admin's client asks for a recount, and everyone's
    /// tries the files again after [`NO_ROOM_RETRY`] whatever the count says.
    fn hold_no_room(&mut self, local: &Tree<LocalFile>, actions: Vec<Action>) -> Vec<Action> {
        // A changed or vanished file is a new try, and so is one refused a while ago.
        self.no_room.retain(|path, (then, at)| local.files.get(path) == Some(then) && at.elapsed() < NO_ROOM_RETRY);
        self.refused.retain(|path, then| local.files.get(path) == Some(then));
        if self.no_room.is_empty() && self.refused.is_empty() {
            return actions;
        }
        if self.no_room.is_empty() {
            return self.leave_out(actions);
        }
        let me = |s: &mut Session| s.call(&auth::me(&s.server)).ok().and_then(|r| auth::parse_me(&r).ok().flatten());
        let free = |me: &auth::Me| if me.storage_quota_bytes == 0 { u64::MAX } else { me.storage_quota_bytes.saturating_sub(me.storage_used_bytes) };
        let needed = self.no_room.values().map(|(f, _)| f.size).min().unwrap_or(0);
        let mut now = me(&mut self.session);
        if let Some(m) = &now
            && free(m) < needed
            && m.user.is_admin()
            && self.recounted.is_none_or(|at| at.elapsed() >= RECOUNT_EVERY)
        {
            self.recounted = Some(Instant::now());
            let recount = self.session.call(&admin::trigger_job(&self.session.server, admin::USAGE_RECONCILE));
            if recount.is_ok_and(|r| r.is_success()) {
                now = me(&mut self.session);
            }
        }
        if let Some(free) = now.as_ref().map(free) {
            self.no_room.retain(|_, (then, _)| then.size > free);
        }
        self.leave_out(actions)
    }

    /// The plan without the uploads of files that didn't fit or were refused.
    fn leave_out(&self, actions: Vec<Action>) -> Vec<Action> {
        let held = |p: &str| self.no_room.contains_key(p) || self.refused.contains_key(p);
        actions
            .into_iter()
            .filter(|a| match a {
                Action::UploadNew(p) | Action::UploadChange { path: p, .. } | Action::MoveRemote { to: p, .. } => !held(p),
                _ => true,
            })
            .collect()
    }

    /// Leaves out the cloud deletes of files that were moved to where nothing syncs (a folder
    /// switched off, the top level with its files off): the cloud keeps them, and the activity
    /// list says so once.
    fn hold_moved_out(&mut self, root: &Path, state: &State, actions: Vec<Action>) -> Vec<Action> {
        if !actions.iter().any(|a| matches!(a, Action::DeleteRemote { .. })) {
            return actions;
        }
        let outside = scan_outside(root, &self.plan);
        let mut notes = Vec::new();
        let kept = actions
            .into_iter()
            .filter(|a| {
                let Action::DeleteRemote { path, .. } = a else { return true };
                let Some(size) = state.files.get(path).map(|k| k.size) else { return true };
                let Some(now) = moved_to(path, size, &outside) else { return true };
                if self.reported.insert(format!("moved:{path}")) {
                    notes.push(event(ActivityKind::Skipped, path, Some(size), format!("moved to {now}, which doesn't sync; the cloud keeps it")));
                }
                false
            })
            .collect();
        if !notes.is_empty() {
            self.update(|p| notes.into_iter().for_each(|e| p.record(e)));
        }
        kept
    }

    /// Says once (per run and place) that part of the folder can't be read, so it isn't synced.
    fn report_unreadable(&mut self, unreadable: &[Unreadable]) {
        let mut notes = Vec::new();
        let root = self.root();
        for place in unreadable.iter().filter(|u| u.why != TEMPORARY && u.why != MID_SAVE) {
            if self.reported.insert(format!("unreadable:{}", place.path)) {
                let shown = if place.path.is_empty() { "the CraftCloud folder" } else { place.path.as_str() };
                let is_file = !place.path.is_empty() && local_relative(&place.path).is_some_and(|rel| root.join(rel).is_file());
                let detail = if place.why == SYMLINK {
                    "a symbolic link: not followed, not synced".to_owned()
                } else if is_file {
                    format!("can't read {shown}: {}; it isn't synced or deleted", place.why)
                } else {
                    format!("can't read {shown}: {}; nothing in it is synced or deleted", place.why)
                };
                notes.push(event(ActivityKind::Skipped, &place.path, None, detail));
            }
        }
        if !notes.is_empty() {
            self.update(|p| notes.into_iter().for_each(|e| p.record(e)));
        }
    }

    /// Leaves out the names the cloud or this computer can't have, and the cloud names this
    /// computer can't keep apart (see the free [`screen_names`]), and says so once.
    fn screen_names(&mut self, state: &mut State, local: &mut Tree<LocalFile>, remote: &mut Tree<RemoteFile>) -> Vec<String> {
        let (notes, left_out) = screen_names(state, local, remote, self.platform, &mut self.reported);
        if !notes.is_empty() {
            self.update(|p| notes.into_iter().for_each(|e| p.record(e)));
        }
        left_out
    }

    /// Says once (per run) which top-level things in the local folder don't sync, so nothing is
    /// left out silently.
    fn report_unsynced(&mut self, root: &Path) {
        let Ok(entries) = std::fs::read_dir(root) else { return };
        let mut new = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            let skipped = names::ignored_always(&name) || (kind.is_file() && names::temporary(&name));
            if skipped || !(kind.is_dir() || kind.is_file()) || in_plan(&self.plan, &name, kind.is_dir()) || self.reported.contains(&name) {
                continue;
            }
            self.reported.insert(name.clone());
            let why = if kind.is_dir() {
                "switched off in Folders; it stays on this computer only"
            } else {
                "files at the top level are off; it stays here only"
            };
            new.push(event(ActivityKind::Skipped, &name, None, why.into()));
        }
        if !new.is_empty() {
            self.update(|p| new.into_iter().for_each(|e| p.record(e)));
        }
    }

    /// The cloud side: every file and folder in the plan, listed level by level with up to
    /// [`LISTERS`] folders at a time. (Skipping folders by ETag isn't safe: OxiCloud doesn't always
    /// change a folder's ETag when a file is added to it.)
    fn scan_remote(&mut self) -> Result<Tree<RemoteFile>, String> {
        let mut tree = Tree::default();
        let mut level = Vec::new();
        for e in self.list("")? {
            if e.is_folder {
                if in_plan(&self.plan, &e.path, true) {
                    level.push(e.path);
                }
            } else if in_plan(&self.plan, &e.path, false) {
                tree.files.insert(e.path.clone(), RemoteFile { etag: e.etag.unwrap_or_default(), size: e.size });
            }
        }
        while !level.is_empty() {
            let mut next = Vec::new();
            for (folder, entries) in self.list_many(&level)? {
                tree.folders.insert(folder);
                for e in entries {
                    if !in_plan(&self.plan, &e.path, e.is_folder) {
                        continue;
                    }
                    if e.is_folder {
                        next.push(e.path);
                    } else {
                        tree.files.insert(e.path.clone(), RemoteFile { etag: e.etag.unwrap_or_default(), size: e.size });
                    }
                }
            }
            level = next;
        }
        Ok(tree)
    }

    /// Lists several folders at once. A folder gone meanwhile (404) is left out; an expired token
    /// is refreshed once and those folders are listed again.
    fn list_many(&mut self, folders: &[String]) -> Result<Vec<(String, Vec<dav::Entry>)>, String> {
        let mut out = Vec::new();
        for chunk in folders.chunks(LISTERS) {
            let (server, tokens) = (&self.session.server, &self.session.tokens);
            let answers: Vec<(String, Result<HttpResponse, String>)> = std::thread::scope(|scope| {
                let jobs: Vec<_> = chunk
                    .iter()
                    .map(|folder| (folder.clone(), scope.spawn(move || http::execute(&tokens.authorize(dav::list(server, folder))))))
                    .collect();
                jobs.into_iter().map(|(f, job)| (f, job.join().unwrap_or_else(|_| Err("listing thread failed".into())))).collect()
            });
            for (folder, answer) in answers {
                // No answer at all: offline (the pass ends here, and is tried again).
                let answer = answer.inspect_err(|e| self.session.offline = Some(e.clone()));
                let resp = match answer? {
                    r if auth::is_expired(&r) => self.session.call(&dav::list(&self.session.server, &folder))?,
                    r => r,
                };
                if resp.status == 404 {
                    continue;
                }
                let mut entries = dav::parse_list(&resp).map_err(|e| format!("listing {folder:?}: {e}"))?;
                entries.retain(|e| e.path != folder);
                out.push((folder, entries));
            }
        }
        Ok(out)
    }

    fn list(&mut self, folder: &str) -> Result<Vec<dav::Entry>, String> {
        let resp = self.session.call(&dav::list(&self.session.server, folder))?;
        if resp.status == 404 {
            return Err(format!("listing {folder:?}: 404"));
        }
        let mut entries = dav::parse_list(&resp).map_err(|e| format!("listing {folder:?}: {e}"))?;
        let me = folder.trim_matches('/');
        entries.retain(|e| e.path != me);
        Ok(entries)
    }

    /// Does one action and records it. `Ok(Some(…))` is a line for the activity list.
    fn apply(
        &mut self,
        root: &Path,
        action: &Action,
        state: &mut State,
        folders: &mut BTreeSet<String>,
        remote: &Tree<RemoteFile>,
    ) -> Result<Option<Done>, String> {
        let local_path = |p: &str| local_relative(p).map(|rel| root.join(rel)).ok_or_else(|| format!("{p}: name not allowed here"));
        match action {
            Action::MakeRemoteFolder(f) => {
                let resp = self.session.call(&dav::create_folder(&self.session.server, f))?;
                if !dav::folder_exists_after(&resp) {
                    return Err(format!("the server answered {}", resp.status));
                }
                if local_path(f)?.is_dir() {
                    folders.insert(f.clone());
                }
                Ok(None)
            }
            Action::MakeLocalFolder(f) => {
                std::fs::create_dir_all(local_path(f)?).map_err(|e| e.to_string())?;
                if remote.folders.contains(f) {
                    folders.insert(f.clone());
                }
                Ok(None)
            }
            Action::UploadNew(p) => match self.put(root, p, &Write::Create, state)? {
                true => Ok(Some((ActivityKind::Up, String::new()))),
                false => self.conflict(root, p, state),
            },
            Action::UploadChange { path, etag } => {
                self.snapshot(path)?;
                match self.put(root, path, &Write::Replace { etag: etag.clone() }, state)? {
                    true => Ok(Some((ActivityKind::Up, "previous version kept".into()))),
                    false => self.conflict(root, path, state),
                }
            }
            Action::Download(p) => {
                let resumed = self.download(root, p, state, remote.files.get(p))?;
                Ok(Some((ActivityKind::Down, resumed.map(|at| format!("resumed at {}", human_bytes(at))).unwrap_or_default())))
            }
            Action::Conflict(p) => self.conflict(root, p, state),
            Action::DeleteRemote { path, etag } => {
                if !self.trash_file(path, etag)? {
                    // Changed in the cloud meanwhile: the next cycle downloads it again.
                    state.files.remove(path);
                    return Ok(None);
                }
                state.files.remove(path);
                Ok(Some((ActivityKind::Deleted, "to the cloud trash".into())))
            }
            Action::DeleteRemoteFolder { path, files } => {
                if !self.cloud_holds(path, files)? {
                    // Changed in the cloud meanwhile: the next cycle sees what (and brings it back).
                    return Ok(None);
                }
                if !self.trash_folder(path)? {
                    return Err("the folder is still in the cloud".into());
                }
                let prefix = format!("{path}/");
                state.files.retain(|p, _| !p.starts_with(&prefix));
                folders.retain(|f| f != path && !f.starts_with(&prefix));
                let n = files.len();
                Ok(Some((ActivityKind::Deleted, format!("folder with {n} file{}, to the cloud trash", if n == 1 { "" } else { "s" }))))
            }
            Action::DeleteLocal(p) => {
                match std::fs::remove_file(local_path(p)?) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
                state.files.remove(p);
                Ok(Some((ActivityKind::Deleted, "deleted in the cloud".into())))
            }
            Action::MoveRemote { from, to, etag } => self.move_remote(root, from, to, etag, state),
            Action::MoveLocal { from, to } => {
                let (src, dest) = (local_path(from)?, local_path(to)?);
                if dest.exists() {
                    return Err(format!("{to} is already here"));
                }
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::rename(&src, &dest).map_err(|e| e.to_string())?;
                let known = state.files.remove(from);
                let meta = local_meta(&dest).ok_or("the file disappeared")?;
                let etag = remote.files.get(to).map(|r| r.etag.clone()).or(known.as_ref().map(|k| k.etag.clone())).unwrap_or_default();
                // As it was last synced (not as it is now): a change made here meanwhile then goes
                // up under the new name next cycle.
                let known = match known {
                    Some(k) => Known { etag, ..k },
                    None => Known { etag, size: meta.size, mtime: meta.mtime, hash: None },
                };
                state.files.insert(to.clone(), known);
                Ok(Some((ActivityKind::Moved, format!("moved in the cloud, from {from}"))))
            }
            Action::Forget(p) => {
                state.files.remove(p);
                Ok(None)
            }
            Action::LocalClash(p) => {
                let src = local_path(p)?;
                let was_folder = src.is_dir();
                let when = jiff::Zoned::now().strftime("%Y-%m-%d %H.%M").to_string();
                let copy = reconcile::free_conflict_name(p, &when, &self.device, |c| {
                    local_relative(c).is_none_or(|rel| std::fs::symlink_metadata(root.join(rel)).is_ok())
                });
                let dest = local_path(&copy)?;
                std::fs::rename(&src, &dest).map_err(|e| e.to_string())?;
                let prefix = format!("{p}/");
                state.files.retain(|f, _| f != p && !f.starts_with(&prefix));
                folders.retain(|f| f != p && !f.starts_with(&prefix));
                let name = copy.rsplit('/').next().unwrap_or(&copy).to_owned();
                let (yours, cloud) = if was_folder { ("folder", "file") } else { ("file", "folder") };
                Ok(Some((ActivityKind::Conflict, format!("the cloud has a {cloud} of this name; your {yours} is kept as {name}"))))
            }
            Action::RemoveRemoteFolder(f) => {
                // Only an empty folder: a DELETE on a folder is recursive, so the cloud must say it's
                // empty right now. A listing that fails says nothing: keep the folder (T-064).
                match self.list(f) {
                    Ok(entries) if entries.is_empty() => {}
                    _ => return Ok(None),
                }
                if self.trash_folder(f)? {
                    folders.remove(f);
                }
                Ok(None)
            }
            Action::RemoveLocalFolder(f) => {
                if std::fs::remove_dir(local_path(f)?).is_ok() {
                    folders.remove(f);
                }
                Ok(None)
            }
        }
    }

    /// Whether the cloud file at `path` is still at version `etag` (`None`: it's gone). Neither
    /// REST's delete nor OxiCloud's MOVE takes If-Match, so this is checked first (the window is
    /// one request). A file listed without an ETag can't be shown to be at `etag`, so it counts
    /// as changed (never trashed or moved on that guess), and the activity list says so once.
    fn still_at(&mut self, path: &str, etag: &str) -> Result<Option<bool>, String> {
        let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
        match self.list(parent).map(|entries| entries.into_iter().find(|e| e.path == path)) {
            Ok(Some(e)) => {
                let now = e.etag.as_deref().map(|now| now.trim_matches('"')).filter(|now| !now.is_empty());
                if now.is_none() && self.reported.insert(format!("no-etag:{path}")) {
                    let note = event(ActivityKind::Skipped, path, None, "the cloud lists no version for it, so it is left as it is there".into());
                    self.update(|p| p.record(note));
                }
                Ok(Some(now.is_some_and(|now| now == etag.trim_matches('"'))))
            }
            Ok(None) => Ok(None),
            Err(e) if e.ends_with(": 404") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A file renamed or moved here: `MOVE` in the cloud, which keeps its id, shares and versions.
    /// When the cloud file changed meanwhile, or the move fails, this is an upload of the new name
    /// instead (and the old one comes down again next cycle): nothing is lost either way.
    fn move_remote(&mut self, root: &Path, from: &str, to: &str, etag: &str, state: &mut State) -> Result<Option<Done>, String> {
        let file = local_relative(to).map(|rel| root.join(rel)).ok_or("name not allowed")?;
        let meta = local_meta(&file).ok_or("the file disappeared")?;
        if self.still_at(from, etag)? == Some(true) {
            let resp = self.session.call(&dav::move_to(&self.session.server, from, to, false))?;
            if resp.is_success() {
                // The version last synced (a change made in the cloud meanwhile then comes down
                // under the new name next cycle).
                let old = state.files.remove(from);
                let hash = old.as_ref().and_then(|k| k.hash.clone());
                let synced = old.map_or_else(|| etag.to_owned(), |k| k.etag);
                state.files.insert(to.to_owned(), Known { etag: synced, size: meta.size, mtime: meta.mtime, hash });
                if let Err(e) = self.session.move_versions(from, to) {
                    log::warn!("{e}");
                }
                return Ok(Some((ActivityKind::Moved, format!("from {from}"))));
            }
            log::warn!("moving {from} to {to} in the cloud: the server answered {}; uploading instead", resp.status);
        }
        state.files.remove(from);
        match self.put(root, to, &Write::Create, state)? {
            true => Ok(Some((ActivityKind::Up, format!("moved here from {from}")))),
            false => self.conflict(root, to, state),
        }
    }

    /// Whether the cloud folder `path` holds exactly `files` (path, ETag), nothing more or newer,
    /// listed again now: a folder goes to the trash whole only then. A file listed without an
    /// ETag can't be shown unchanged, so it keeps the folder.
    fn cloud_holds(&mut self, path: &str, files: &[(String, String)]) -> Result<bool, String> {
        let mut found: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut level = vec![path.to_owned()];
        while !level.is_empty() {
            let mut next = Vec::new();
            for (_, entries) in self.list_many(&level)? {
                for e in entries {
                    if e.is_folder {
                        next.push(e.path);
                    } else {
                        found.insert(e.path, e.etag);
                    }
                }
            }
            level = next;
        }
        Ok(found.len() == files.len()
            && files.iter().all(|(p, etag)| {
                let now = found.get(p).and_then(Option::as_deref).map(|now| now.trim_matches('"')).filter(|now| !now.is_empty());
                now.is_some_and(|now| now == etag.trim_matches('"'))
            }))
    }

    /// Moves a cloud file to the trash, unless it changed since `etag` (then `false`: it stays).
    /// One already gone counts as done.
    fn trash_file(&mut self, path: &str, etag: &str) -> Result<bool, String> {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        match self.still_at(path, etag)? {
            None => return Ok(true),
            Some(false) => return Ok(false),
            Some(true) => {}
        }
        let Some(folder) = self.folder_id(parent)? else { return Ok(true) };
        let Some(id) = self.child_id(&folder, name, Kind::File)? else { return Ok(true) };
        let resp = self.session.call(&files::trash(&self.session.server, &id))?;
        if resp.is_success() || resp.status == 404 { Ok(true) } else { Err(format!("moving it to the trash: the server answered {}", resp.status)) }
    }

    /// Moves an (empty) cloud folder to the trash; `true` when it's gone.
    fn trash_folder(&mut self, path: &str) -> Result<bool, String> {
        let Some(id) = self.folder_id(path)? else { return Ok(true) };
        let resp = self.session.call(&folders::trash(&self.session.server, &id))?;
        self.ids.retain(|p, _| p != path && !p.starts_with(&format!("{path}/")));
        if resp.is_success() || resp.status == 404 { Ok(true) } else { Err(format!("moving it to the trash: the server answered {}", resp.status)) }
    }

    /// The REST id of the cloud folder at `path` (`""`: the drive's root), or `None` when there is
    /// no such folder.
    fn folder_id(&mut self, path: &str) -> Result<Option<String>, String> {
        if let Some(id) = self.ids.get(path) {
            return Ok(Some(id.clone()));
        }
        let id = if path.is_empty() {
            let resp = self.session.call(&folders::roots(&self.session.server))?;
            let roots = folders::parse_roots(&resp).map_err(|e| format!("the drive's root folder: {e}"))?;
            roots.into_iter().find(|f| f.parent_id.is_none()).map(|f| f.id)
        } else {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            match self.folder_id(parent)? {
                Some(parent) => self.child_id(&parent, name, Kind::Folder)?,
                None => None,
            }
        };
        if let Some(id) = &id {
            self.ids.insert(path.to_owned(), id.clone());
        }
        Ok(id)
    }

    /// The id of `name` (a file or a folder, as `kind` says) in the folder with id `folder`.
    fn child_id(&mut self, folder: &str, name: &str, kind: Kind) -> Result<Option<String>, String> {
        let mut cursor: Option<String> = None;
        loop {
            let resp = self.session.call(&folders::contents(&self.session.server, folder, Some(200), cursor.as_deref()))?;
            if resp.status == 404 {
                return Ok(None);
            }
            let page = folders::parse_contents_all(&resp).map_err(|e| format!("listing a folder: {e}"))?;
            if let Some(found) = page.items.iter().find(|r| r.kind() == kind && r.name() == name) {
                return Ok(Some(found.id().to_owned()));
            }
            match page.next_cursor {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => return Ok(None),
            }
        }
    }

    /// Uploads a local file; `false` when the cloud said 412 (someone else wrote it first).
    fn put(&mut self, root: &Path, path: &str, write: &Write, state: &mut State) -> Result<bool, String> {
        let file = local_relative(path).map(|rel| root.join(rel)).ok_or("name not allowed")?;
        let before = local_meta(&file).ok_or("the file disappeared")?;
        let hash = hash_of(&file, &before);
        let req = dav::upload(&self.session.server, path, Vec::new(), write);
        let resp = self.session.upload(&req, &file)?;
        if resp.status == 507 {
            self.no_room.insert(path.to_owned(), (before, Instant::now()));
            // In our own words and units: the server's message counts in binary units (T-049).
            return Err(format!(
                "no room in the cloud for this file ({}): the storage is full. Storage use updates within a few minutes after emptying the trash (earlier versions count too); it's tried again then.",
                craftcloud_core::size::text(before.size)
            ));
        }
        if matches!(resp.status, 400 | 403 | 414 | 415 | 422) {
            // Refused for what it is, not for now: tried again once the file changes.
            self.refused.insert(path.to_owned(), before);
            let why = serde_json::from_slice::<serde_json::Value>(&resp.body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("the server answered {}", resp.status));
            return Err(format!("the cloud refused it: {}. Not tried again until it changes.", why.trim_end_matches('.')));
        }
        match dav::parse_upload(&resp).map_err(|e| e.to_string())? {
            Uploaded::Conflict => Ok(false),
            Uploaded::Saved { etag } => {
                let etag = match etag {
                    Some(e) => e,
                    None => self.remote_etag(path)?,
                };
                state.files.insert(path.to_owned(), Known { etag, size: before.size, mtime: before.mtime, hash });
                Ok(true)
            }
        }
    }

    fn remote_etag(&mut self, path: &str) -> Result<String, String> {
        let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
        self.list(parent)?.into_iter().find(|e| e.path == path).and_then(|e| e.etag).ok_or_else(|| format!("{path}: no ETag after upload"))
    }

    /// Puts back from the cloud's trash what the held deletes in `actions` would delete here
    /// because it was deleted in the cloud. Returns the files that are back.
    fn restore_trashed(&mut self, actions: &[Action]) -> Result<BTreeSet<String>, String> {
        let gone: Vec<&str> = actions.iter().filter_map(|a| if let Action::DeleteLocal(p) = a { Some(p.as_str()) } else { None }).collect();
        let mut back = BTreeSet::new();
        if gone.is_empty() {
            return Ok(back);
        }
        let mut listed = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let resp = self.session.call(&trash::list(&self.session.server, Some(200), cursor.as_deref()))?;
            let page = trash::parse_list(&resp).map_err(|e| format!("listing the trash: {e}"))?;
            for item in page.items {
                if let Some(place) = item.resource.cloud_path() {
                    listed.push((item.resource.id().to_owned(), place, item.trashed_at));
                }
            }
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        for (id, place, covers) in trash_to_restore(&gone, &listed) {
            let resp = self.session.call(&trash::restore(&self.session.server, &id))?;
            if resp.is_success() {
                back.extend(covers);
            } else {
                log::warn!("putting {place} back from the trash failed ({}); its files are uploaded again", resp.status);
            }
        }
        Ok(back)
    }

    /// Keeps the cloud version about to be replaced in `.craftcloud/versions/<path>/`, newest 5.
    fn snapshot(&mut self, path: &str) -> Result<(), String> {
        self.session.snapshot(path)
    }

    /// Streams a cloud file into place (temporary name, then rename) and records it. Where it
    /// resumed an interrupted download, how many bytes were there already.
    fn download(&mut self, root: &Path, path: &str, state: &mut State, listed: Option<&RemoteFile>) -> Result<Option<u64>, String> {
        let dest = local_relative(path).map(|rel| root.join(rel)).ok_or("name not allowed")?;
        let (part, etag, resumed) = self.fetch(path, &dest, listed)?;
        let meta = put_in_place(part, &dest, state.files.get(path))?;
        let hash = hash_of(&dest, &meta);
        state.files.insert(path.to_owned(), Known { etag, size: meta.size, mtime: meta.mtime, hash });
        Ok(resumed)
    }

    /// Downloads to a hidden part file next to `dest`; returns it, the cloud ETag and where it
    /// resumed (if it did).
    ///
    /// With the version the listing showed (`listed`), the part file is named after its ETag and
    /// kept when the transfer breaks off: the next try asks only for the rest (`Range`), and uses
    /// it only when the server's answer is that same version from that byte on (OxiCloud ignores
    /// `If-Range`, docs/UPSTREAM.md C13, so the ETag and `Content-Range` are checked here).
    ///
    /// A part file that is no use any more is removed on every way out: only one named after its
    /// version, whose transfer broke off (or never started), is kept to resume from; past that the
    /// returned [`PartFile`] removes it unless it is moved into place.
    fn fetch(&mut self, path: &str, dest: &Path, listed: Option<&RemoteFile>) -> Result<(PartFile, String, Option<u64>), String> {
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let part = dest.with_file_name(part_name(&name, listed.map(|r| r.etag.as_str())));
        if let Some(parent) = part.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        remove_other_parts(dest, &part);
        let have = listed.and_then(|r| resume_from(std::fs::metadata(&part).ok().map(|m| m.len()), r.size));
        let mut req = dav::download(&self.session.server, path);
        if let Some(from) = have {
            req = req.header("Range", format!("bytes={from}-"));
        }
        let mut resp = self.session.open(&req)?;
        let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let resumed = match (resp.status().as_u16(), have, listed) {
            (206, Some(from), Some(r)) => {
                let range = resp.headers().get("content-range").and_then(|v| v.to_str().ok()).unwrap_or_default();
                if !range_continues(range, from, r.size) || !etag.as_deref().is_some_and(|e| e.trim_matches('"') == r.etag.trim_matches('"')) {
                    // Another version now: start over next time.
                    let _ = std::fs::remove_file(&part);
                    return Err("changed in the cloud while downloading; trying again".into());
                }
                Some(from)
            }
            (200, ..) => None,
            (status, ..) => return Err(format!("the server answered {status}")),
        };
        let written = (|| {
            let mut out = if resumed.is_some() { std::fs::OpenOptions::new().append(true).open(&part) } else { std::fs::File::create(&part) }
                .map_err(|e| format!("can't write {}: {e}", part.display()))?;
            http::stream_body(&mut resp, &mut out, &self.session.cancel)?;
            out.sync_all().map_err(|e| e.to_string())
        })();
        if let Err(e) = written {
            // Kept to resume from when it's named after its version; else no use.
            if listed.is_none() {
                let _ = std::fs::remove_file(&part);
            }
            return Err(e);
        }
        // Whole now: from here on any failure removes it.
        let part = PartFile(part);
        if let Some(r) = listed
            && std::fs::metadata(&part.0).map(|m| m.len()).ok() != Some(r.size)
            && etag.as_deref().is_some_and(|e| e.trim_matches('"') == r.etag.trim_matches('"'))
        {
            return Err("the download came out the wrong size; trying again".into());
        }
        let etag = match etag {
            Some(e) => e,
            None => self.remote_etag(path)?,
        };
        Ok((part, etag, resumed))
    }

    /// Both sides have the file and it isn't known to be the same: compare, and keep both if not.
    fn conflict(&mut self, root: &Path, path: &str, state: &mut State) -> Result<Option<Done>, String> {
        let local = local_relative(path).map(|rel| root.join(rel)).ok_or("name not allowed")?;
        let (part, etag, _) = self.fetch(path, &local, None)?;
        if same_bytes(&part.0, &local) {
            drop(part);
            let meta = local_meta(&local).ok_or("the file disappeared")?;
            state.files.insert(path.to_owned(), Known { etag, size: meta.size, mtime: meta.mtime, hash: None });
            return Ok(None);
        }
        let when = jiff::Zoned::now().strftime("%Y-%m-%d %H.%M").to_string();
        let copy = reconcile::free_conflict_name(path, &when, &self.device, |c| {
            local_relative(c).is_none_or(|rel| std::fs::symlink_metadata(root.join(rel)).is_ok())
        });
        let copy_local = local_relative(&copy).map(|rel| root.join(rel)).ok_or("name not allowed")?;
        std::fs::rename(&local, &copy_local).map_err(|e| e.to_string())?;
        std::fs::rename(&part.0, &local).map_err(|e| e.to_string())?;
        let meta = local_meta(&local).ok_or("the file disappeared")?;
        state.files.insert(path.to_owned(), Known { etag, size: meta.size, mtime: meta.mtime, hash: None });
        // The local edit goes up under its new name; a failure here is retried as a new file.
        let _ = self.put(root, &copy, &Write::Create, state);
        let name = copy.rsplit('/').next().unwrap_or(&copy).to_owned();
        Ok(Some((ActivityKind::Conflict, format!("yours kept as {name}"))))
    }
}

/// A whole download in its part file, removed when dropped unless it was moved into place first
/// (a failed download never leaves its part file behind).
struct PartFile(PathBuf);

impl Drop for PartFile {
    fn drop(&mut self) {
        // Gone already when it was moved into place.
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Moves a downloaded part file to `dest`, after the last look before replacing a local file:
/// it must still be what was last synced (`known`), or not be there. A change made here meanwhile
/// is never overwritten; the next pass sees both (#36). On any failure the part file goes.
fn put_in_place(part: PartFile, dest: &Path, known: Option<&Known>) -> Result<LocalFile, String> {
    if !replaceable(dest, known) {
        return Err("changed here while it was downloading; it syncs in the next pass".into());
    }
    std::fs::rename(&part.0, dest).map_err(|e| e.to_string())?;
    local_meta(dest).ok_or_else(|| "the file disappeared".into())
}

/// Part files left this long are left over (a crash, a file gone from the cloud before its
/// download could resume): [`sweep_stale_parts`] removes them.
const STALE_PART: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the local folder is looked through for them.
const SWEEP_PARTS_EVERY: Duration = Duration::from_secs(60 * 60);

/// Whether a file name is one of this client's own download part files ([`part_name`]).
fn is_part_file(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".craftcloud-part")
}

/// Removes this client's own part files (and nothing else) last written more than `older_than`
/// ago, anywhere in the local folder but `.craftcloud/`; symbolic links are never followed.
/// Returns how many went.
fn sweep_stale_parts(root: &Path, older_than: Duration) -> usize {
    let mut removed = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() && name != dav::CRAFTCLOUD_DIR {
                stack.push(entry.path());
            } else if kind.is_file()
                && is_part_file(&name)
                && entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > older_than)
                && std::fs::remove_file(entry.path()).is_ok()
            {
                removed += 1;
            }
        }
    }
    removed
}

/// The part file a download goes to: `.<name>.<version>.craftcloud-part` when the version is
/// known (so an interrupted one can be resumed, and never from another version's bytes), else
/// `.<name>.craftcloud-part`.
fn part_name(name: &str, etag: Option<&str>) -> String {
    let version: String = etag.unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let full = if version.is_empty() { format!(".{name}.craftcloud-part") } else { format!(".{name}.{version}.craftcloud-part") };
    if full.len() <= MAX_NAME_BYTES {
        return full;
    }
    // A long name would make the part file's name too long for the file system (T-059): a short,
    // stable name from the file's name and version instead, still ignored as a part file.
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{name}\u{0}{version}").as_bytes());
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!(".ccpart-{hex}.craftcloud-part")
}

/// The longest file name most file systems take (ext4, APFS, NTFS: 255 bytes or units).
const MAX_NAME_BYTES: usize = 255;

/// Where to resume a download of `size` bytes from a part file holding `have`: only part-way.
fn resume_from(have: Option<u64>, size: u64) -> Option<u64> {
    have.filter(|h| *h > 0 && *h < size)
}

/// Whether a 206's `Content-Range` (`bytes 50000-99999/100000`) carries on from `from` to the
/// end of a `size`-byte file.
fn range_continues(range: &str, from: u64, size: u64) -> bool {
    let Some((span, total)) = range.trim().strip_prefix("bytes ").and_then(|r| r.split_once('/')) else { return false };
    let Some((start, end)) = span.split_once('-') else { return false };
    start.parse() == Ok(from) && end.parse() == Ok(size.saturating_sub(1)) && total.parse() == Ok(size)
}

/// Removes leftover part files of `dest` (of other versions) but `keep`.
fn remove_other_parts(dest: &Path, keep: &Path) {
    let (Some(dir), Some(name)) = (dest.parent(), dest.file_name().map(|n| n.to_string_lossy().into_owned())) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let prefix = format!(".{name}.");
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with(&prefix) && file.ends_with(".craftcloud-part") && entry.path() != keep {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `12.3 MB`, for the activity list.
fn human_bytes(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1} GB", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1} kB", n as f64 / 1e3),
        n => format!("{n} bytes"),
    }
}

/// What an action did, for the activity list: its kind and a few words more.
type Done = (ActivityKind, String);

/// An activity event that happened now.
fn event(kind: ActivityKind, path: &str, size: Option<u64>, detail: String) -> ActivityEvent {
    let now = jiff::Zoned::now();
    ActivityEvent {
        at: now.timestamp().as_second(),
        time: now.strftime("%H:%M").to_string(),
        date: now.strftime("%Y-%m-%d").to_string(),
        kind,
        path: path.to_owned(),
        size,
        detail: cloud_paths_only(&detail),
    }
}

/// The drive's root folder as OxiCloud names it in server paths (`Personal/Docs/a.txt`).
const DRIVE_ROOT: &str = "Personal/";

/// A message with the server's own paths (`Personal/Docs/a.txt`, from its error texts) written
/// as the cloud paths the person knows (`Docs/a.txt`).
fn cloud_paths_only(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(DRIVE_ROOT) {
        let before = &rest[..i];
        // Only at the start of a path: after a space, a quote, a slash or the start of the text.
        let starts_path = before.is_empty() || before.ends_with([' ', '\'', '"', '/', '(', ':']);
        out.push_str(before.strip_suffix('/').filter(|_| starts_path).unwrap_or(before));
        if !starts_path {
            out.push_str(DRIVE_ROOT);
        }
        rest = &rest[i + DRIVE_ROOT.len()..];
    }
    out.push_str(rest);
    out
}

/// Renames a local folder the way it was moved in the cloud; never over something already there.
fn move_local_folder(root: &Path, from: &str, to: &str) -> Result<(), String> {
    let (Some(src), Some(dest)) = (local_relative(from).map(|r| root.join(r)), local_relative(to).map(|r| root.join(r))) else {
        return Err("name not allowed here".into());
    };
    if std::fs::symlink_metadata(&dest).is_ok() {
        return Err(format!("{to} is already here"));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&src, &dest).map_err(|e| e.to_string())
}

/// Files hashed for [`Known::hash`] are at most this big (bigger ones count as changed whenever
/// their time changes).
const HASH_LIMIT: u64 = 256 * 1024 * 1024;

/// SHA-256 (hex) of a local file that is still `expect` (same size and time) after reading it.
fn hash_of(file: &Path, expect: &LocalFile) -> Option<String> {
    use sha2::Digest;
    if expect.size > HASH_LIMIT {
        return None;
    }
    let mut hasher = sha2::Sha256::new();
    let mut reader = std::io::BufReader::new(std::fs::File::open(file).ok()?);
    std::io::copy(&mut reader, &mut hasher).ok()?;
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (local_meta(file).as_ref() == Some(expect)).then_some(hash)
}

/// Files rewritten with the same bytes (only their time changed): their new time is recorded
/// as synced, so they aren't uploaded again (and no version is kept). Whether any was.
fn same_bytes_again(root: &Path, state: &mut State, local: &Tree<LocalFile>) -> bool {
    let mut any = false;
    for (path, now) in &local.files {
        let Some(k) = state.files.get_mut(path) else { continue };
        if k.size != now.size || k.mtime == now.mtime || k.hash.is_none() {
            continue;
        }
        let Some(file) = local_relative(path).map(|rel| root.join(rel)) else { continue };
        if hash_of(&file, now).is_some_and(|h| Some(&h) == k.hash.as_ref()) {
            k.mtime = now.mtime;
            any = true;
        }
    }
    any
}

/// What a cycle does with deletions that are too many at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Deletes {
    /// Hold them (and sync the rest) until the person decides.
    Hold,
    /// Carry them out.
    Confirm,
    /// Bring the files back instead.
    Discard,
}

/// Whether an action deletes something.
fn is_delete(action: &Action) -> bool {
    matches!(
        action,
        Action::DeleteRemote { .. }
            | Action::DeleteLocal(_)
            | Action::DeleteRemoteFolder { .. }
            | Action::RemoveRemoteFolder(_)
            | Action::RemoveLocalFolder(_)
    )
}

/// A plan without its deletions, and the files those would delete.
fn split_deletes(actions: Vec<Action>) -> (Vec<Action>, Vec<String>) {
    // What takes the place of something whose removal waits (a file replacing a synced folder,
    // T-052) waits with it: it can't be put there while the old one is still in the way.
    let removed: Vec<String> = actions.iter().filter(|a| is_delete(a)).map(|a| action_path(a).to_owned()).collect();
    let mut held = Vec::new();
    let mut rest = Vec::new();
    for action in actions {
        match action {
            Action::DeleteRemote { path, .. } | Action::DeleteLocal(path) => held.push(path),
            Action::DeleteRemoteFolder { files, .. } => held.extend(files.into_iter().map(|(p, _)| p)),
            a if is_delete(&a) => {}
            a if action_paths(&a).iter().any(|p| removed.iter().any(|r| within(r, p))) => {}
            a => rest.push(a),
        }
    }
    (rest, held)
}

/// The synced folders that files whose deletion is `held` are in: still known as synced while the
/// hold lasts, although one side no longer has them. Forgotten, the next pass would make such a
/// folder again (empty) on the side it was deleted from, and a confirmed delete would then go file
/// by file instead of as one folder to the trash (T-042).
fn held_folders(state: &State, held: &[String]) -> Vec<String> {
    state.folders.iter().filter(|f| held.iter().any(|p| p.starts_with(&format!("{f}/")))).cloned().collect()
}

/// Forgets that the files `actions` would delete were ever synced, so the next plan copies them
/// back to the side they are missing from instead; the `restored` ones are back in the cloud
/// already and stay as they were. Returns how many are kept.
fn bring_back(state: &mut State, actions: &[Action], restored: &BTreeSet<String>) -> usize {
    let (_, held) = split_deletes(actions.to_vec());
    for path in held.iter().filter(|p| !restored.contains(*p)) {
        state.files.remove(path);
    }
    held.len()
}

/// The trash items to put back so that the files in `gone` (deleted in the cloud) are there
/// again: newest first, one per place (a trashed folder brings back everything in it). Each is
/// (trash id, its cloud path, the files of `gone` it brings back).
fn trash_to_restore(gone: &[&str], trash: &[(String, String, String)]) -> Vec<(String, String, Vec<String>)> {
    let mut items: Vec<&(String, String, String)> = trash.iter().collect();
    // Newest first (RFC 3339 sorts as text): a path trashed twice comes back as its latest.
    items.sort_by(|a, b| b.2.cmp(&a.2));
    let mut taken: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::new();
    for (id, place, _) in items {
        if place.is_empty() || out.iter().any(|(_, p, _): &(String, String, Vec<String>)| within(p, place) || within(place, p)) {
            continue;
        }
        let covers: Vec<String> = gone.iter().filter(|p| !taken.contains(**p) && within(place, p)).map(|p| (*p).to_owned()).collect();
        if covers.is_empty() {
            continue;
        }
        taken.extend(gone.iter().filter(|p| within(place, p)));
        out.push((id.clone(), place.clone(), covers));
    }
    out
}

/// Whether an action moves a file (counted in progress, and its file shows as syncing).
fn is_transfer(action: &Action) -> bool {
    !matches!(
        action,
        Action::Forget(_) | Action::MakeLocalFolder(_) | Action::MakeRemoteFolder(_) | Action::RemoveLocalFolder(_) | Action::RemoveRemoteFolder(_)
    )
}

/// The cloud path an action is about.
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
        | Action::RemoveLocalFolder(p) => p,
    }
}

fn describe(action: &Action) -> String {
    match action {
        Action::MakeRemoteFolder(f) | Action::MakeLocalFolder(f) => format!("create folder {f}"),
        Action::UploadNew(p) | Action::UploadChange { path: p, .. } => format!("upload {p}"),
        Action::Download(p) => format!("download {p}"),
        Action::Conflict(p) => format!("compare {p}"),
        Action::DeleteRemote { path, .. } => format!("delete {path} in the cloud"),
        Action::DeleteLocal(p) => format!("remove {p}"),
        Action::DeleteRemoteFolder { path, .. } => format!("delete folder {path} in the cloud"),
        Action::MoveRemote { from, to, .. } => format!("move {from} to {to} in the cloud"),
        Action::MoveLocal { from, to } => format!("move {from} to {to}"),
        Action::LocalClash(p) => format!("set {p} aside"),
        Action::Forget(p) => format!("forget {p}"),
        Action::RemoveRemoteFolder(f) | Action::RemoveLocalFolder(f) => format!("remove folder {f}"),
    }
}

pub(crate) fn local_meta(path: &Path) -> Option<LocalFile> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_millis();
    Some(LocalFile { size: meta.len(), mtime: i64::try_from(mtime).ok()? })
}

/// Names that are never synced (the list is in core: `names::ignored`).
pub use craftcloud_core::names::ignored;

/// Leaves names the cloud can't take, names this computer can't have and cloud names it can't
/// keep apart out of both sides and the state (they stay where they are, never uploaded,
/// downloaded or deleted), each said once (`reported`: once per version of a local file, else
/// once per run), and flags names a Windows computer can't have where they still sync (once per
/// run). Returns the notes and the places left out.
///
/// Such a name in the cloud (another client put it there) is left out on both sides, so it's
/// never taken for one deleted here and trashed in the cloud.
fn screen_names(
    state: &mut State,
    local: &mut Tree<LocalFile>,
    remote: &mut Tree<RemoteFile>,
    platform: Platform,
    reported: &mut BTreeSet<String>,
) -> (Vec<ActivityEvent>, Vec<String>) {
    let mut notes = Vec::new();
    let left_out = leave_out_names(state, local, remote, platform);
    for (place, (why, file)) in &left_out {
        let key = match file {
            Some(f) => format!("refused:{place}:{}:{}", f.size, f.mtime),
            None => format!("refused:{place}"),
        };
        if reported.insert(key) {
            notes.push(event(ActivityKind::Skipped, place, None, why.clone()));
        }
    }
    // What still syncs here but can't on Windows (on Windows itself, all of it was left out above).
    let mut flagged: BTreeSet<String> = BTreeSet::new();
    for path in local.files.keys().chain(&local.folders).chain(remote.files.keys()).chain(&remote.folders) {
        if let Some((place, why)) = names::local_refused_in(path, Platform::Windows)
            && flagged.insert(place.to_owned())
            && reported.insert(format!("windows:{place}"))
        {
            notes.push(event(ActivityKind::Warning, place, None, format!("{why}; it syncs, but not to a Windows computer")));
        }
    }
    (notes, left_out.into_keys().collect())
}

/// The top-most place in a path the cloud can't take or this computer can't have, with why.
fn name_trouble(path: &str, platform: Platform) -> Option<(&str, String)> {
    let mut end = 0;
    for segment in path.split('/') {
        end += segment.len();
        if let Some(why) = names::server_refuses(segment) {
            return Some((&path[..end], why.to_owned()));
        }
        if let Some(why) = names::local_refuses(segment, platform) {
            return Some((&path[..end], format!("{why}; it isn't synced to or from this computer (rename it to sync it here)")));
        }
        end += 1;
    }
    None
}

/// Takes the places [`screen_names`] leaves out out of both sides and the state (the top-most
/// such place of each path: `a\b/c.txt` leaves out all of `a\b`), and returns them with why
/// (and the local file, if it is one, to say it once per version).
fn leave_out_names(
    state: &mut State,
    local: &mut Tree<LocalFile>,
    remote: &mut Tree<RemoteFile>,
    platform: Platform,
) -> BTreeMap<String, (String, Option<LocalFile>)> {
    let mut out: BTreeMap<String, (String, Option<LocalFile>)> = BTreeMap::new();
    for path in local.files.keys().chain(&local.folders).chain(remote.files.keys()).chain(&remote.folders) {
        if let Some((place, why)) = name_trouble(path, platform) {
            out.entry(place.to_owned()).or_insert((why, local.files.get(place).copied()));
        }
    }
    // Two cloud names that would be one file here (Windows: `Report.txt`, `report.txt`): the one
    // this computer has keeps syncing, the other is held, so neither is downloaded over the other.
    let left = |p: &str| out.keys().any(|place| within(place, p));
    let cloud: Vec<&str> = remote.files.keys().chain(&remote.folders).map(String::as_str).filter(|p| !left(p)).collect();
    let here = |p: &str| local.files.contains_key(p) || local.folders.contains(p);
    for (place, other) in names::clashes(cloud, here, platform) {
        let why = format!(
            "this computer can't keep it apart from {other} (the names differ only in upper and lower case); it stays in the cloud until one is renamed"
        );
        let file = local.files.get(&place).copied();
        out.entry(place).or_insert((why, file));
    }
    for place in out.keys() {
        local.files.retain(|p, _| !within(place, p));
        local.folders.retain(|p| !within(place, p));
        remote.files.retain(|p, _| !within(place, p));
        remote.folders.retain(|p| !within(place, p));
        state.files.retain(|p, _| !within(place, p));
        state.folders.retain(|p| !within(place, p));
    }
    out
}

/// Why a symbolic link isn't synced (it is never followed).
const SYMLINK: &str = "a symbolic link";
const TEMPORARY: &str = "looks like a temporary file";
/// A file written in the last moments (an app may still be saving it): held, untouched by this pass, and
/// synced by the next one. Not reported as unreadable.
const MID_SAVE: &str = "being saved";

/// The files held because they're being saved (#15: reported as waiting, not as up to date).
fn mid_save(unreadable: &[Unreadable]) -> Vec<String> {
    unreadable.iter().filter(|u| u.why == MID_SAVE).map(|u| u.path.clone()).collect()
}

/// Takes every path the sync leaves alone (`names::skipped`) out of the state and both sides, so
/// nothing is planned for it: no download, no upload, no delete either way (T-063). A file that
/// another device uploads under such a name stays in the cloud; one here stays here.
fn drop_skipped(state: &mut State, local: &mut Tree<LocalFile>, remote: &mut Tree<RemoteFile>, platform: Platform) {
    state.files.retain(|p, _| !names::skipped(p, true, platform));
    state.folders.retain(|p| !names::skipped(p, false, platform));
    local.files.retain(|p, _| !names::skipped(p, true, platform));
    local.folders.retain(|p| !names::skipped(p, false, platform));
    remote.files.retain(|p, _| !names::skipped(p, true, platform));
    remote.folders.retain(|p| !names::skipped(p, false, platform));
}

/// The local side. Symlinks are not followed. A file written in the last two seconds counts as
/// it was at the last sync (or as absent when new), so a save in progress waits a cycle.
fn scan_local(root: &Path, plan: &SyncPlan, state: &State, settled: &BTreeSet<String>) -> (Tree<LocalFile>, Vec<Unreadable>) {
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let mut tree = Tree::default();
    let mut unreadable: Vec<Unreadable> = Vec::new();
    // A rename keeps size and time: a "new" file that matches a synced one isn't mid-save.
    let synced: BTreeSet<(u64, i64)> = state.files.values().map(|k| (k.size, k.mtime)).collect();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        // Anything that can't be read is unknown, never gone (T-033): keep what was synced under it.
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                unreadable.push(Unreadable { path: prefix.clone(), why: e.to_string() });
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    // A listing that breaks off: the rest of this folder is unknown.
                    unreadable.push(Unreadable { path: prefix.clone(), why: e.to_string() });
                    break;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if names::ignored_always(&name) {
                continue;
            }
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(e) => {
                    unreadable.push(Unreadable { path, why: e.to_string() });
                    continue;
                }
            };
            if kind.is_dir() {
                if in_plan(plan, &path, true) {
                    tree.folders.insert(path.clone());
                    stack.push((entry.path(), path));
                }
            } else if kind.is_file() && names::temporary(&name) {
                // An app's temporary, lock or backup file: not synced, but said in `status`, and
                // held like a symlink so a synced file of that name is never deleted (T-062).
                if in_plan(plan, &path, false) {
                    unreadable.push(Unreadable { path, why: TEMPORARY.into() });
                }
            } else if kind.is_symlink() && (in_plan(plan, &path, true) || in_plan(plan, &path, false)) {
                // Not followed (it may lead out of the folder, or in a circle); kept out of the
                // sync like a place that can't be read, so nothing overwrites or deletes it.
                unreadable.push(Unreadable { path, why: SYMLINK.into() });
            } else if kind.is_file() && in_plan(plan, &path, false) {
                // Its size and time come without reading it (T-053): a file without read permission
                // is found here, by opening it, not on upload, where it would fail every cycle.
                let meta = match file_meta(&entry.path()).and_then(|meta| std::fs::File::open(entry.path()).map(|_| meta).map_err(|e| e.to_string()))
                {
                    Ok(meta) => meta,
                    Err(why) => {
                        unreadable.push(Unreadable { path, why });
                        continue;
                    }
                };
                // Mid-save only if written in the last moments; a time in the future counts as settled (T-060).
                // Held for the next pass like a place that can't be read (#36): kept as last synced
                // and left out of every action, so a cloud change can't be downloaded over it.
                if (0..SETTLE_MS).contains(&(now - meta.mtime)) && !settled.contains(&path) && !synced.contains(&(meta.size, meta.mtime)) {
                    unreadable.push(Unreadable { path, why: MID_SAVE.into() });
                    continue;
                }
                tree.files.insert(path, meta);
            }
        }
    }
    for place in &unreadable {
        keep_as_synced(&mut tree, state, &place.path);
    }
    (tree, unreadable)
}

/// A place in the local folder the scan couldn't read (cloud-style path; "" is the folder itself).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unreadable {
    path: String,
    why: String,
}

/// Whether `path` is `place` or inside it.
fn within(place: &str, path: &str) -> bool {
    place.is_empty() || path == place || path.strip_prefix(place).is_some_and(|rest| rest.starts_with('/'))
}

/// Puts everything last synced at or under `place` back into `tree` unchanged, so an unreadable
/// folder or file plans no delete, upload or download.
fn keep_as_synced(tree: &mut Tree<LocalFile>, state: &State, place: &str) {
    for (path, k) in &state.files {
        if within(place, path) {
            tree.files.insert(path.clone(), LocalFile { size: k.size, mtime: k.mtime });
        }
    }
    for folder in &state.folders {
        if within(place, folder) {
            tree.folders.insert(folder.clone());
        }
    }
}

/// Every path an action touches (both ends of a move).
fn action_paths(action: &Action) -> Vec<&str> {
    match action {
        Action::MoveRemote { from, to, .. } | Action::MoveLocal { from, to } => vec![from, to],
        other => vec![action_path(other)],
    }
}

/// Drops every action that touches an unreadable place: no deletes, downloads or moves there.
fn hold_unreadable(actions: Vec<Action>, unreadable: &[Unreadable]) -> Vec<Action> {
    if unreadable.is_empty() {
        return actions;
    }
    actions.into_iter().filter(|a| !action_paths(a).iter().any(|p| unreadable.iter().any(|u| within(&u.path, p)))).collect()
}

/// Whether a download may replace the local file at `dest`: it isn't there, or it is still the
/// version last synced (same size and time). Anything else is a change made here (#36).
fn replaceable(dest: &Path, known: Option<&Known>) -> bool {
    match file_meta(dest) {
        Ok(now) => known.is_some_and(|k| k.size == now.size && k.mtime == now.mtime),
        Err(_) => std::fs::symlink_metadata(dest).is_err(),
    }
}

/// A local file's size and time, or why they can't be read.
fn file_meta(path: &Path) -> Result<LocalFile, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let mtime = meta.modified().map_err(|e| e.to_string())?;
    let mtime = mtime.duration_since(SystemTime::UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis();
    Ok(LocalFile { size: meta.len(), mtime: i64::try_from(mtime).map_err(|e| e.to_string())? })
}

/// The files in the local folder that are outside the plan, as (cloud-style path, size).
fn scan_outside(root: &Path, plan: &SyncPlan) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), false)];
    while let Some((dir, prefix, outside)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if names::ignored_always(&name) {
                continue;
            }
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_file() && names::temporary(&name) {
                continue;
            }
            if kind.is_dir() {
                // Inside the plan only the top level can hold something that doesn't sync.
                let out_here = outside || !in_plan(plan, &path, true);
                if out_here || prefix.is_empty() {
                    stack.push((entry.path(), path, out_here));
                }
            } else if kind.is_file()
                && (outside || !in_plan(plan, &path, false))
                && let Some(meta) = local_meta(&entry.path())
            {
                out.push((path, meta.size));
            }
        }
    }
    out
}

/// Where a synced file that vanished went, if a file of the same name and size sits outside the
/// plan.
fn moved_to<'a>(path: &str, size: u64, outside: &'a [(String, u64)]) -> Option<&'a str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    outside.iter().find(|(p, s)| *s == size && p.rsplit('/').next() == Some(name)).map(|(p, _)| p.as_str())
}

fn same_bytes(a: &Path, b: &Path) -> bool {
    let (Ok(fa), Ok(fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    if fa.metadata().map(|m| m.len()).ok() != fb.metadata().map(|m| m.len()).ok() {
        return false;
    }
    let (mut ra, mut rb) = (std::io::BufReader::new(fa), std::io::BufReader::new(fb));
    let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let (Ok(na), Ok(nb)) = (ra.read(&mut ba), rb.read(&mut bb)) else { return false };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

/// Conflict copies in the local folder that still wait for a decision.
fn find_conflicts(local: &Tree<LocalFile>, kept: &BTreeSet<String>) -> Vec<ConflictCopy> {
    local
        .files
        .keys()
        .filter(|p| !kept.contains(*p))
        .filter_map(|copy| reconcile::original_of_conflict(copy).map(|original| ConflictCopy { original, copy: copy.clone() }))
        // A file set aside for a cloud folder of its name isn't a version of anything.
        .filter(|c| !local.folders.contains(&c.original))
        .collect()
}

/// Removes a folder and the empty folders inside it; anything with files stays.
fn remove_empty_dirs(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                remove_empty_dirs(&entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// What the last sync of the local folder `root` agreed on (empty when it never synced): read
/// from `state.json` without changing it, for the command line and the control channel.
pub fn synced_state(root: &Path) -> State {
    let file = root.join(dav::CRAFTCLOUD_DIR).join("state.json");
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice::<Saved>(&b).ok()).map(|s| s.state).unwrap_or_default()
}

fn save_saved(path: &Path, saved: &Saved) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(saved).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{bring_back, leave_out_names, part_name, range_continues, resume_from, same_bytes_again, split_deletes};
    use craftcloud_core::names::Platform;

    #[test]
    fn a_name_the_cloud_cant_take_is_never_deleted_from_it() {
        // Another client put `ctl\u{1}.txt` in the cloud and it was synced: leaving it out here
        // must not read as "deleted here, so trash it there".
        let path = "Docs/ctl\u{1}.txt".to_owned();
        let mut state = State::default();
        state.files.insert(path.clone(), Known { etag: "e".into(), size: 1, mtime: 1, hash: None });
        state.folders.insert("Docs".into());
        let mut local: Tree<LocalFile> = Tree::default();
        local.files.insert(path.clone(), LocalFile { size: 1, mtime: 1 });
        local.folders.insert("Docs".into());
        let mut remote: Tree<RemoteFile> = Tree::default();
        remote.files.insert(path.clone(), RemoteFile { etag: "e".into(), size: 1 });
        remote.folders.insert("Docs".into());
        let out = leave_out_names(&mut state, &mut local, &mut remote, Platform::current());
        assert_eq!(out.keys().collect::<Vec<_>>(), [&path]);
        let plan = reconcile::reconcile(&state, &local, &remote, false).unwrap();
        assert!(!plan.iter().any(super::is_delete), "{plan:?}");
    }

    #[test]
    fn a_file_rewritten_with_the_same_bytes_is_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "base\n").unwrap();
        let then = super::local_meta(&file).unwrap();
        let hash = super::hash_of(&file, &then).expect("hashed");
        let mut state = State::default();
        state.files.insert("f.txt".into(), Known { etag: "e".into(), size: then.size, mtime: then.mtime - 5_000, hash: Some(hash) });
        let mut local: Tree<LocalFile> = Tree::default();
        local.files.insert("f.txt".into(), then);
        assert!(same_bytes_again(dir.path(), &mut state, &local), "same bytes, new time");
        assert_eq!(state.files["f.txt"].mtime, then.mtime);

        // Other bytes of the same size are a change.
        std::fs::write(&file, "BASE\n").unwrap();
        let now = super::local_meta(&file).unwrap();
        local.files.insert("f.txt".into(), now);
        state.files.get_mut("f.txt").unwrap().mtime = now.mtime - 5_000;
        assert!(!same_bytes_again(dir.path(), &mut state, &local));
        assert_eq!(state.files["f.txt"].mtime, now.mtime - 5_000, "still a change to upload");
    }

    #[test]
    fn an_interrupted_download_resumes_only_into_the_same_version() {
        assert_eq!(part_name("a.bin", Some("\"d7fc8c633e524686-1794615066\"")), ".a.bin.d7fc8c633e524686-1794615066.craftcloud-part");
        assert_eq!(part_name("a.bin", None), ".a.bin.craftcloud-part");
        assert_eq!(part_name("a.bin", Some("\"../x\"")), ".a.bin.x.craftcloud-part", "nothing but the version's letters");
        assert!(super::ignored(&part_name("a.bin", Some("\"e-1\""))));
        // T-059: a name near the limit gets a short part name that fits, is stable, and is ignored.
        let long = format!("{}.txt", "é".repeat(110));
        let part = part_name(&long, Some("\"d7fc8c633e524686-1794615066\""));
        assert!(part.len() <= 255, "{}", part.len());
        assert_eq!(part, part_name(&long, Some("\"d7fc8c633e524686-1794615066\"")));
        assert_ne!(part, part_name(&long, Some("\"other-1\"")));
        assert!(super::ignored(&part));
        assert_eq!(resume_from(Some(50_000), 100_000), Some(50_000));
        assert_eq!(resume_from(Some(0), 100_000), None);
        assert_eq!(resume_from(Some(100_000), 100_000), None, "a whole one is fetched again (nothing says it's whole)");
        assert_eq!(resume_from(None, 100_000), None);
        assert!(range_continues("bytes 50000-99999/100000", 50_000, 100_000));
        assert!(!range_continues("bytes 0-99999/100000", 50_000, 100_000));
        assert!(!range_continues("bytes 50000-99999/120000", 50_000, 100_000));
        assert!(!range_continues("", 50_000, 100_000));
    }
    use craftcloud_core::reconcile::LocalFile;

    #[test]
    fn a_mass_delete_holds_only_the_deletes() {
        let mut state = State::default();
        for i in 0..20 {
            state.files.insert(format!("Gone/{i}.txt"), Known { etag: format!("e{i}"), size: 1, mtime: 1, hash: None });
        }
        state.folders.insert("Gone".into());
        let mut remote: Tree<RemoteFile> = Tree::default();
        for (p, k) in &state.files {
            remote.files.insert(p.clone(), RemoteFile { etag: k.etag.clone(), size: 1 });
        }
        remote.folders.insert("Gone".into());
        // The folder is gone here, and a new file waits to go up.
        let mut local: Tree<LocalFile> = Tree::default();
        local.files.insert("new.txt".into(), LocalFile { size: 3, mtime: 5 });
        assert!(reconcile::reconcile(&state, &local, &remote, false).is_err(), "too many deletes");

        let all = reconcile::reconcile(&state, &local, &remote, true).unwrap();
        let (rest, held) = split_deletes(all.clone());
        assert_eq!(rest, [reconcile::Action::UploadNew("new.txt".into())], "the rest still syncs");
        assert_eq!(held.len(), 20);
        assert!(held.contains(&"Gone/7.txt".to_owned()));

        // A file that replaced the folder (T-052) waits with its delete: the folder is in the way.
        local.files.insert("Gone".into(), LocalFile { size: 2, mtime: 9 });
        let (rest, _) = split_deletes(reconcile::reconcile(&state, &local, &remote, true).unwrap());
        assert_eq!(rest, [reconcile::Action::UploadNew("new.txt".into())]);
        local.files.remove("Gone");

        // Brought back instead: they come down again, nothing is deleted.
        assert_eq!(bring_back(&mut state, &all, &BTreeSet::new()), 20);
        let again = reconcile::reconcile(&state, &local, &remote, false).unwrap();
        assert!(again.contains(&reconcile::Action::Download("Gone/7.txt".into())), "{again:?}");
        assert!(!again.iter().any(super::is_delete), "{again:?}");
    }

    /// T-038: files deleted in the cloud come back from its trash (one item per place: a trashed
    /// folder brings back all of it), newest first; only what isn't there is uploaded again.
    #[test]
    fn discarded_deletes_come_back_from_the_trash() {
        use super::trash_to_restore;
        let gone = ["Old/a.txt", "Old/sub/b.txt", "Notes/c.md", "Notes/d.md", "Lost/e.txt"];
        let item = |id: &str, place: &str, at: &str| (id.to_owned(), place.to_owned(), at.to_owned());
        let trash = [
            item("t1", "Old", "2026-11-14T10:00:00Z"),
            item("t2", "Old/a.txt", "2026-11-14T09:00:00Z"),
            item("t3", "Notes/c.md", "2026-11-13T10:00:00Z"),
            item("t4", "Notes/c.md", "2026-11-14T11:00:00Z"),
            item("t5", "Notes/other.md", "2026-11-14T11:00:00Z"),
        ];
        let plan = trash_to_restore(&gone, &trash);
        let ids: Vec<&str> = plan.iter().map(|(id, _, _)| id.as_str()).collect();
        assert_eq!(ids, ["t4", "t1"], "the newest copy of c.md; the folder, not the file inside it; nothing unrelated");
        assert_eq!(plan[1].2, ["Old/a.txt", "Old/sub/b.txt"]);
        let back: BTreeSet<String> = plan.into_iter().flat_map(|(_, _, covers)| covers).collect();
        assert!(!back.contains("Notes/d.md") && !back.contains("Lost/e.txt"), "not in the trash: uploaded again");

        // What is back keeps its record (no new upload); the rest is forgotten and goes up again.
        let mut state = State::default();
        for p in gone {
            state.files.insert(p.to_owned(), reconcile::Known { etag: "e".into(), size: 1, mtime: 1, hash: None });
        }
        let all: Vec<reconcile::Action> = gone.iter().map(|p| reconcile::Action::DeleteLocal((*p).to_owned())).collect();
        assert_eq!(bring_back(&mut state, &all, &back), 5);
        let kept: Vec<&str> = state.files.keys().map(String::as_str).collect();
        assert_eq!(kept, ["Notes/c.md", "Old/a.txt", "Old/sub/b.txt"]);
    }

    /// T-042: while a folder deleted here waits as a held mass delete, the passes in between keep
    /// knowing it was synced: they don't make it again (empty) here, and the confirmed delete is
    /// still one folder to the trash, not one trash item per file.
    #[test]
    fn a_held_folder_delete_stays_one_folder() {
        use super::held_folders;
        let mut state = State::default();
        let mut remote: Tree<RemoteFile> = Tree::default();
        let mut local: Tree<LocalFile> = Tree::default();
        for f in ["t14", "t14/many"] {
            state.folders.insert(f.into());
            remote.folders.insert(f.into());
        }
        local.folders.insert("t14".into());
        for i in 0..50 {
            let p = format!("t14/many/{i}.txt");
            state.files.insert(p.clone(), reconcile::Known { etag: format!("e{i}"), size: 1, mtime: 1, hash: None });
            remote.files.insert(p, RemoteFile { etag: format!("e{i}"), size: 1 });
        }
        let all = reconcile::reconcile(&state, &local, &remote, true).unwrap();
        let (rest, held) = split_deletes(all);
        assert_eq!(held.len(), 50);
        assert!(rest.is_empty(), "{rest:?}");

        // What the held pass saves: the folders on both sides, and those the held deletes are in.
        let mut after = state.clone();
        after.folders = local.folders.intersection(&remote.folders).cloned().collect();
        after.folders.extend(held_folders(&state, &held));
        assert!(after.folders.contains("t14/many"));
        let next = reconcile::reconcile(&after, &local, &remote, true).unwrap();
        assert!(!next.contains(&reconcile::Action::MakeLocalFolder("t14/many".into())), "not made again here: {next:?}");
        let folder_deletes: Vec<&reconcile::Action> = next.iter().filter(|a| matches!(a, reconcile::Action::DeleteRemoteFolder { .. })).collect();
        assert_eq!(folder_deletes.len(), 1, "{next:?}");
        assert!(!next.iter().any(|a| matches!(a, reconcile::Action::DeleteRemote { .. })), "no file goes on its own: {next:?}");
    }

    use super::{cloud_paths_only, drop_skipped, hold_unreadable, ignored, moved_to, scan_local, scan_outside, within};

    #[test]
    fn messages_name_cloud_paths_not_server_paths() {
        assert_eq!(
            cloud_paths_only("the server answered 409: Parent path is a file, not a collection: Personal/t5/clash"),
            "the server answered 409: Parent path is a file, not a collection: t5/clash"
        );
        assert_eq!(cloud_paths_only("'/Personal/a.txt' and \"Personal/b\""), "'a.txt' and \"b\"");
        assert_eq!(cloud_paths_only("MyPersonal/x stays"), "MyPersonal/x stays");
    }
    use craftcloud_core::reconcile::{self, Action, Known, RemoteFile, State, Tree};
    use craftcloud_core::sync::SyncPlan;
    use std::collections::BTreeSet;

    /// T-033: a synced folder the client can't read is unknown, never gone — it must plan no delete
    /// in the cloud (and no download into it), however many files it holds.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_never_taken_as_deleted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut state = State::default();
        let mut remote: Tree<RemoteFile> = Tree::default();
        for i in 0..40 {
            let path = format!("t6/many/f{i}.txt");
            std::fs::create_dir_all(root.join("t6/many")).unwrap();
            std::fs::write(root.join(&path), "x").unwrap();
            let meta = super::file_meta(&root.join(&path)).unwrap();
            let etag = format!("\"e{i}\"");
            // One file changed in the cloud meanwhile: it must not be downloaded into the folder either.
            let remote_etag = if i == 0 { "\"changed\"".to_owned() } else { etag.clone() };
            state.files.insert(path.clone(), Known { etag, size: meta.size, mtime: meta.mtime, hash: None });
            remote.files.insert(path, RemoteFile { etag: remote_etag, size: 1 });
        }
        for folder in ["t6", "t6/many"] {
            state.folders.insert(folder.into());
            remote.folders.insert(folder.into());
        }
        let plan =
            SyncPlan { server: String::new(), local_folder: root.display().to_string(), excluded: vec![], folders: vec![], sync_root_files: true };
        let many = root.join("t6/many");
        std::fs::set_permissions(&many, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&many).is_ok() {
            // Running as root: permissions don't stop the scan, so there's nothing to test.
            std::fs::set_permissions(&many, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let (local, unreadable) = scan_local(root, &plan, &state, &BTreeSet::new());
        std::fs::set_permissions(&many, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].path, "t6/many");
        assert_eq!(local.files.len(), 40, "everything synced under it is kept as it was");
        let actions = reconcile::reconcile(&state, &local, &remote, false).unwrap();
        let actions = hold_unreadable(actions, &unreadable);
        assert!(!actions.iter().any(|a| matches!(a, Action::DeleteRemote { .. } | Action::DeleteLocal(_))), "{actions:?}");
        assert!(!actions.iter().any(|a| matches!(a, Action::Download(p) if p.starts_with("t6/many/"))), "{actions:?}");
    }

    /// T-053: a file without read permission is a place that can't be read, found by the scan (its
    /// size and time read fine): never uploaded (so never failing every cycle), never deleted.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_found_by_the_scan() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("t13")).unwrap();
        let (synced, new) = (root.join("t13/locked.txt"), root.join("t13/new-locked.txt"));
        std::fs::write(&synced, "x").unwrap();
        std::fs::write(&new, "y").unwrap();
        let meta = super::file_meta(&synced).unwrap();
        let mut state = State::default();
        state.files.insert("t13/locked.txt".into(), Known { etag: "\"e1\"".into(), size: meta.size, mtime: meta.mtime, hash: None });
        state.folders.insert("t13".into());
        let mut remote: Tree<RemoteFile> = Tree::default();
        remote.files.insert("t13/locked.txt".into(), RemoteFile { etag: "\"e1\"".into(), size: 1 });
        remote.folders.insert("t13".into());
        let plan =
            SyncPlan { server: String::new(), local_folder: root.display().to_string(), excluded: vec![], folders: vec![], sync_root_files: true };
        for f in [&synced, &new] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        if std::fs::File::open(&synced).is_ok() {
            // Running as root: permissions don't stop a read, so there's nothing to test.
            return;
        }
        let (local, unreadable) = scan_local(root, &plan, &state, &BTreeSet::new());
        let mut places: Vec<&str> = unreadable.iter().map(|u| u.path.as_str()).collect();
        places.sort_unstable();
        assert_eq!(places, ["t13/locked.txt", "t13/new-locked.txt"]);
        assert_eq!(local.files.len(), 1, "the synced one is kept as it was: {:?}", local.files);
        let actions = hold_unreadable(reconcile::reconcile(&state, &local, &remote, false).unwrap(), &unreadable);
        assert_eq!(actions, [], "no upload, no delete");
    }

    /// #36: a file written moments ago (mid-save) is held for the next pass, so a cloud change can't
    /// overwrite it: with the old rule it looked unchanged and the cloud version was downloaded over it.
    #[test]
    fn a_file_being_saved_is_never_overwritten_by_a_cloud_change() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("t36")).unwrap();
        let file = root.join("t36/notes.txt");
        std::fs::write(&file, "synced").unwrap();
        let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(an_hour_ago).unwrap();
        let synced = super::file_meta(&file).unwrap();
        let mut state = State::default();
        state.files.insert("t36/notes.txt".into(), Known { etag: "\"e1\"".into(), size: synced.size, mtime: synced.mtime, hash: None });
        state.folders.insert("t36".into());
        // Edited here just now, and overwritten in the cloud meanwhile.
        std::fs::write(&file, "edited here").unwrap();
        let mut remote: Tree<RemoteFile> = Tree::default();
        remote.files.insert("t36/notes.txt".into(), RemoteFile { etag: "\"e2\"".into(), size: 5 });
        remote.folders.insert("t36".into());
        let plan =
            SyncPlan { server: String::new(), local_folder: root.display().to_string(), excluded: vec![], folders: vec![], sync_root_files: true };
        let (local, unreadable) = scan_local(root, &plan, &state, &BTreeSet::new());
        let actions = hold_unreadable(reconcile::reconcile(&state, &local, &remote, false).unwrap(), &unreadable);
        assert!(!actions.iter().any(|a| super::action_paths(a).contains(&"t36/notes.txt")), "held for the next pass: {actions:?}");
        assert!(unreadable.iter().all(|u| u.why == super::MID_SAVE), "held, not reported as unreadable: {unreadable:?}");
        assert_eq!(super::mid_save(&unreadable), vec!["t36/notes.txt".to_owned()], "#15: reported as waiting");
    }

    /// #36: the download step never replaces a local file that changed since it was last synced.
    #[test]
    fn a_download_replaces_only_the_file_last_synced() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        assert!(super::replaceable(&file, None), "nothing there yet");
        std::fs::write(&file, "synced").unwrap();
        let meta = super::file_meta(&file).unwrap();
        let known = Known { etag: "\"e1\"".into(), size: meta.size, mtime: meta.mtime, hash: None };
        assert!(super::replaceable(&file, Some(&known)), "still the synced version");
        assert!(!super::replaceable(&file, None), "a file that was never synced is someone's work");
        std::fs::write(&file, "edited here").unwrap();
        assert!(!super::replaceable(&file, Some(&known)), "changed here since the last sync");
    }

    /// #36: a new file being saved isn't replaced by a cloud file of the same name either.
    #[test]
    fn a_new_file_being_saved_is_never_overwritten_by_a_cloud_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("t36")).unwrap();
        std::fs::write(root.join("t36/new.txt"), "mine").unwrap();
        let mut state = State::default();
        state.folders.insert("t36".into());
        let mut remote: Tree<RemoteFile> = Tree::default();
        remote.files.insert("t36/new.txt".into(), RemoteFile { etag: "\"c1\"".into(), size: 6 });
        remote.folders.insert("t36".into());
        let plan =
            SyncPlan { server: String::new(), local_folder: root.display().to_string(), excluded: vec![], folders: vec![], sync_root_files: true };
        let (local, unreadable) = scan_local(root, &plan, &state, &BTreeSet::new());
        let actions = hold_unreadable(reconcile::reconcile(&state, &local, &remote, false).unwrap(), &unreadable);
        assert!(!actions.iter().any(|a| super::action_paths(a).contains(&"t36/new.txt")), "held for the next pass: {actions:?}");
    }

    /// T-060: a file whose time is in the future is settled, not mid-save: it must be synced.
    #[test]
    fn a_file_dated_in_the_future_is_synced() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("t33")).unwrap();
        let file = root.join("t33/future.txt");
        std::fs::write(&file, "fut").unwrap();
        let in_2099 = std::time::UNIX_EPOCH + std::time::Duration::from_secs(4_070_952_000);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(in_2099).unwrap();
        let plan =
            SyncPlan { server: String::new(), local_folder: root.display().to_string(), excluded: vec![], folders: vec![], sync_root_files: true };
        let (local, unreadable) = scan_local(root, &plan, &State::default(), &BTreeSet::new());
        assert!(unreadable.is_empty());
        assert!(local.files.contains_key("t33/future.txt"), "a future time isn't held back as mid-save");
    }

    /// T-062: folders whose names look like temporary files sync with everything in them; only
    /// FILES with such names are left out, and they're listed (not silently dropped).
    #[test]
    fn temp_looking_folders_sync_and_temp_files_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (path, text) in [
            ("t37/projekt.tmp/vigtig.txt", "c"),
            ("t37/Backup~/rapport.docx", "d"),
            ("t37/x.part/a.txt", "e"),
            ("t37/notes.tmp", "f"),
            ("t37/.#lock", "g"),
        ] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), text).unwrap();
        }
        std::fs::write(root.join("t37/.x.bin.e-1.craftcloud-part"), "own part").unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        for path in ["t37/projekt.tmp/vigtig.txt", "t37/Backup~/rapport.docx", "t37/x.part/a.txt"] {
            std::fs::File::options().write(true).open(root.join(path)).unwrap().set_modified(old).unwrap();
        }
        let plan =
            SyncPlan { server: String::new(), local_folder: root.display().to_string(), excluded: vec![], folders: vec![], sync_root_files: true };
        let (local, left) = scan_local(root, &plan, &State::default(), &BTreeSet::new());
        for synced in ["t37/projekt.tmp/vigtig.txt", "t37/Backup~/rapport.docx", "t37/x.part/a.txt"] {
            assert!(local.files.contains_key(synced), "{synced} must sync");
        }
        assert!(!local.files.contains_key("t37/notes.tmp"));
        let mut temporary: Vec<&str> = left.iter().filter(|u| u.why == super::TEMPORARY).map(|u| u.path.as_str()).collect();
        temporary.sort_unstable();
        assert_eq!(temporary, ["t37/.#lock", "t37/notes.tmp"], "the client's own part files aren't listed");
    }

    /// T-063: names the sync leaves alone are never downloaded, uploaded or deleted, whichever
    /// side has them and whatever the state remembers (e.g. from a client with other rules).
    #[test]
    fn skipped_names_are_never_downloaded_or_deleted() {
        use craftcloud_core::reconcile::{LocalFile, RemoteFile};
        let mut state = State::default();
        let mut local: Tree<LocalFile> = Tree::default();
        let mut remote: Tree<RemoteFile> = Tree::default();
        let known = |etag: &str| Known { etag: etag.into(), size: 1, mtime: 1, hash: None };
        // In the cloud only (uploaded through the web or the CLI): never downloaded.
        for p in ["t37/notes.tmp", "t37/.DS_Store", ".craftcloud/versions/a/1"] {
            remote.files.insert(p.into(), RemoteFile { etag: "\"r\"".into(), size: 1 });
        }
        // Synced earlier (an older client's state), still in the cloud, gone or held here: never deleted in the cloud.
        state.files.insert("t37/old.tmp".into(), known("\"o\""));
        remote.files.insert("t37/old.tmp".into(), RemoteFile { etag: "\"o\"".into(), size: 1 });
        // Synced earlier and still here, gone in the cloud: never deleted here.
        state.files.insert("t37/here.tmp".into(), known("\"h\""));
        local.files.insert("t37/here.tmp".into(), LocalFile { size: 1, mtime: 1 });
        // A folder with a temporary-looking name is ordinary: it syncs.
        remote.folders.insert("t37/projekt.tmp".into());
        remote.files.insert("t37/projekt.tmp/vigtig.txt".into(), RemoteFile { etag: "\"v\"".into(), size: 1 });
        for folder in ["t37", ".craftcloud", ".craftcloud/versions", ".craftcloud/versions/a"] {
            remote.folders.insert(folder.into());
        }
        drop_skipped(&mut state, &mut local, &mut remote, Platform::current());
        let actions = reconcile::reconcile(&state, &local, &remote, false).unwrap();
        for a in &actions {
            for p in super::action_paths(a) {
                assert!(!p.ends_with(".tmp") || p.starts_with("t37/projekt.tmp"), "nothing planned for {p}: {a:?}");
                assert!(!p.contains(".DS_Store") && !p.starts_with(".craftcloud"), "{a:?}");
            }
        }
        assert!(actions.iter().any(|a| matches!(a, Action::Download(p) if p == "t37/projekt.tmp/vigtig.txt")), "{actions:?}");
        assert!(!actions.iter().any(|a| matches!(a, Action::DeleteRemote { .. } | Action::DeleteLocal(_))), "{actions:?}");
    }

    /// T-064: a synced folder named like a temporary file, with a temporary file next to it, plans
    /// nothing on later passes, on the computer that uploaded it and on the one that downloaded it.
    #[test]
    fn a_temp_named_folder_stays_synced_over_many_passes() {
        use craftcloud_core::reconcile::RemoteFile;
        for side in ["uploader", "downloader"] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            std::fs::create_dir_all(root.join("smoke-2/projekt.tmp")).unwrap();
            std::fs::write(root.join("smoke-2/projekt.tmp/inside.txt"), "p").unwrap();
            std::fs::write(root.join("smoke-2/notes.tmp"), "n").unwrap();
            let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
            for f in ["smoke-2/projekt.tmp/inside.txt", "smoke-2/notes.tmp"] {
                std::fs::File::options().write(true).open(root.join(f)).unwrap().set_modified(old).unwrap();
            }
            let meta = super::file_meta(&root.join("smoke-2/projekt.tmp/inside.txt")).unwrap();
            // After the first sync both sides agree: the folder and its file are known and in the cloud.
            let mut state = State::default();
            state
                .files
                .insert("smoke-2/projekt.tmp/inside.txt".into(), Known { etag: "\"e1\"".into(), size: meta.size, mtime: meta.mtime, hash: None });
            state.folders.extend(["smoke-2".to_owned(), "smoke-2/projekt.tmp".to_owned()]);
            let plan = SyncPlan {
                server: String::new(),
                local_folder: root.display().to_string(),
                excluded: vec![],
                folders: vec![],
                sync_root_files: true,
            };
            for pass in 1..=4 {
                let mut remote: Tree<RemoteFile> = Tree::default();
                remote.folders.extend(["smoke-2".to_owned(), "smoke-2/projekt.tmp".to_owned()]);
                remote.files.insert("smoke-2/projekt.tmp/inside.txt".into(), RemoteFile { etag: "\"e1\"".into(), size: meta.size });
                let (mut local, unreadable) = scan_local(root, &plan, &state, &BTreeSet::new());
                drop_skipped(&mut state, &mut local, &mut remote, Platform::current());
                let actions = hold_unreadable(reconcile::reconcile(&state, &local, &remote, false).unwrap(), &unreadable);
                assert!(actions.is_empty(), "{side}, pass {pass}: {actions:?}");
                assert!(local.folders.contains("smoke-2/projekt.tmp"), "{side}, pass {pass}: the folder is seen here");
            }
        }
    }

    #[test]
    fn places_contain_their_paths_only() {
        assert!(within("", "a/b"));
        assert!(within("t6/many", "t6/many"));
        assert!(within("t6/many", "t6/many/f.txt"));
        assert!(!within("t6/many", "t6/many2/f.txt"));
        assert!(!within("t6/many", "t6"));
    }

    #[test]
    fn a_file_moved_out_of_the_plan_is_found_where_it_went() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (path, text) in [("Shared/a.md", "aaa"), ("Private/deep/b.md", "bb"), ("top.txt", "t")] {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let plan = SyncPlan {
            server: String::new(),
            local_folder: root.display().to_string(),
            excluded: vec!["Private".into()],
            folders: vec![],
            sync_root_files: false,
        };
        let mut outside = scan_outside(root, &plan);
        outside.sort();
        assert_eq!(outside, [("Private/deep/b.md".to_owned(), 2), ("top.txt".to_owned(), 1)]);
        assert_eq!(moved_to("Shared/b.md", 2, &outside), Some("Private/deep/b.md"));
        assert_eq!(moved_to("Shared/b.md", 3, &outside), None, "another size is another file");
        assert_eq!(moved_to("Shared/c.md", 2, &outside), None);
    }

    #[test]
    #[cfg(unix)]
    fn symbolic_links_are_not_followed_and_said_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Docs")).unwrap();
        std::fs::write(root.join("Docs/a.txt"), "a").unwrap();
        std::os::unix::fs::symlink(root.join("Docs/a.txt"), root.join("Docs/link.txt")).unwrap();
        std::os::unix::fs::symlink(root, root.join("Docs/loop")).unwrap();
        std::os::unix::fs::symlink(root.join("Docs"), root.join("Photos")).unwrap();
        let plan = SyncPlan { local_folder: root.display().to_string(), ..SyncPlan::default() };
        let (tree, out) =
            scan_local(root, &plan, &craftcloud_core::reconcile::State::default(), &std::collections::BTreeSet::from(["Docs/a.txt".to_owned()]));
        assert_eq!(tree.files.keys().collect::<Vec<_>>(), ["Docs/a.txt"]);
        let mut links: Vec<(&str, &str)> = out.iter().map(|u| (u.path.as_str(), u.why.as_str())).collect();
        links.sort();
        assert_eq!(links, [("Docs/link.txt", super::SYMLINK), ("Docs/loop", super::SYMLINK), ("Photos", super::SYMLINK)]);
    }

    #[test]
    fn temporary_files_of_saving_apps_are_never_synced() {
        for name in [".craftcloud", ".report.1234-1.craft_tmp", ".a.docx.craftcloud-part", "~$report.docx", ".~lock.report.odt#", "x.part"] {
            assert!(ignored(name), "{name}");
        }
        for name in ["report.docx", ".env", "notes.md", "part.txt"] {
            assert!(!ignored(name), "{name}");
        }
    }

    /// The names a Windows 11 test of 0.1.1 met in the cloud, as the five files of `Weird/`.
    fn weird_cloud() -> Tree<RemoteFile> {
        let mut remote: Tree<RemoteFile> = Tree::default();
        remote.folders.insert("Weird".into());
        for (i, name) in ["a:b.txt", "what?.md", "CON.txt", "trailing.", "trailing"].iter().enumerate() {
            remote.files.insert(format!("Weird/{name}"), RemoteFile { etag: format!("\"e{i}\""), size: 1 });
        }
        remote
    }

    /// One pass's plan the way `try_cycle` makes it: names screened, skipped names dropped.
    fn plan_pass(
        state: &mut State,
        local: &mut Tree<LocalFile>,
        remote: &mut Tree<RemoteFile>,
        platform: Platform,
        reported: &mut BTreeSet<String>,
    ) -> (Vec<Action>, Vec<craftcloud_core::sync::ActivityEvent>) {
        let (notes, _) = super::screen_names(state, local, remote, platform, reported);
        drop_skipped(state, local, remote, platform);
        (reconcile::reconcile(state, local, remote, false).unwrap(), notes)
    }

    /// On Windows, every name the other computers warn about is left alone: not downloaded (no
    /// per-pass error), not deleted on either side, said once. `trailing` itself still syncs, and
    /// never collides with `trailing.`. Elsewhere all five download, with a warning each.
    #[test]
    fn names_windows_cant_have_are_left_alone_on_windows() {
        let bad = ["Weird/a:b.txt", "Weird/what?.md", "Weird/CON.txt", "Weird/trailing."];
        let mut state = State::default();
        let mut reported = BTreeSet::new();
        // An older client made `CON.txt` on Windows 11 and recorded it as synced.
        state.files.insert("Weird/CON.txt".into(), Known { etag: "\"e2\"".into(), size: 1, mtime: 1, hash: None });
        state.folders.insert("Weird".into());
        let mut local: Tree<LocalFile> = Tree::default();
        local.folders.insert("Weird".into());
        local.files.insert("Weird/CON.txt".into(), LocalFile { size: 1, mtime: 1 });
        for pass in 1..=3 {
            let (mut s, mut l, mut r) = (state.clone(), local.clone(), weird_cloud());
            let (actions, notes) = plan_pass(&mut s, &mut l, &mut r, Platform::Windows, &mut reported);
            for a in &actions {
                for p in super::action_paths(a) {
                    assert!(!bad.contains(&p), "pass {pass}: nothing planned for {p}: {a:?}");
                }
                assert!(!matches!(a, Action::DeleteRemote { .. } | Action::DeleteLocal(_) | Action::DeleteRemoteFolder { .. }), "{a:?}");
            }
            assert!(actions.iter().any(|a| matches!(a, Action::Download(p) if p == "Weird/trailing")), "pass {pass}: {actions:?}");
            if pass == 1 {
                let said: BTreeSet<&str> = notes.iter().map(|n| n.path.as_str()).collect();
                assert_eq!(said, bad.into_iter().collect(), "each said once: {notes:?}");
                assert!(notes.iter().all(|n| n.kind == craftcloud_core::sync::ActivityKind::Skipped), "{notes:?}");
            } else {
                assert!(notes.is_empty(), "pass {pass}: said again: {notes:?}");
            }
        }
        // Linux and macOS keep syncing them all, with a warning for the four.
        let (mut s, mut l, mut r) = (State::default(), Tree::default(), weird_cloud());
        let (actions, notes) = plan_pass(&mut s, &mut l, &mut r, Platform::Other, &mut BTreeSet::new());
        let downloads = actions.iter().filter(|a| matches!(a, Action::Download(_))).count();
        assert_eq!(downloads, 5, "{actions:?}");
        assert_eq!(notes.iter().filter(|n| n.kind == craftcloud_core::sync::ActivityKind::Warning).count(), 4, "{notes:?}");
    }

    /// Two cloud names that are one file on Windows: the one this computer has keeps syncing, the
    /// other is held (never downloaded over it), and nothing is deleted on either side.
    #[test]
    fn cloud_names_windows_cant_keep_apart_are_held() {
        let mut state = State::default();
        state.files.insert("Docs/Report.txt".into(), Known { etag: "\"r1\"".into(), size: 1, mtime: 1, hash: None });
        state.folders.insert("Docs".into());
        let mut local: Tree<LocalFile> = Tree::default();
        local.folders.insert("Docs".into());
        local.files.insert("Docs/Report.txt".into(), LocalFile { size: 1, mtime: 1 });
        let mut remote: Tree<RemoteFile> = Tree::default();
        remote.folders.insert("Docs".into());
        remote.files.insert("Docs/Report.txt".into(), RemoteFile { etag: "\"r2\"".into(), size: 2 });
        remote.files.insert("Docs/report.txt".into(), RemoteFile { etag: "\"x\"".into(), size: 3 });
        let mut reported = BTreeSet::new();
        let (actions, notes) = plan_pass(&mut state.clone(), &mut local.clone(), &mut remote.clone(), Platform::Windows, &mut reported);
        assert_eq!(actions, [Action::Download("Docs/Report.txt".into())], "only the one this computer has");
        assert_eq!(notes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), ["Docs/report.txt"]);
        assert!(notes[0].detail.contains("Docs/Report.txt"), "{notes:?}");
        // A new pair, neither here: both held.
        let mut remote2 = remote.clone();
        remote2.files.insert("Docs/Notes.md".into(), RemoteFile { etag: "\"n1\"".into(), size: 1 });
        remote2.files.insert("Docs/NOTES.md".into(), RemoteFile { etag: "\"n2\"".into(), size: 1 });
        let (actions, _) = plan_pass(&mut state.clone(), &mut local.clone(), &mut remote2, Platform::Windows, &mut reported);
        assert!(!actions.iter().any(|a| super::action_paths(a).iter().any(|p| p.eq_ignore_ascii_case("Docs/notes.md"))), "{actions:?}");
        // Elsewhere both sync.
        let (actions, _) = plan_pass(&mut state, &mut local, &mut remote, Platform::Other, &mut BTreeSet::new());
        assert!(actions.contains(&Action::Download("Docs/report.txt".into())), "{actions:?}");
    }

    /// A download that can't be put in place leaves no part file behind (seen as
    /// `Weird/.trailing..<version>.craftcloud-part` on Windows).
    #[test]
    fn a_failed_download_leaves_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("trailing");
        std::fs::write(&dest, "changed here").unwrap();
        let part = dir.path().join(".trailing..44c77418e27569db-1794748821.craftcloud-part");
        std::fs::write(&part, "cloud bytes").unwrap();
        let known = Known { etag: "\"e\"".into(), size: 1, mtime: 1, hash: None };
        let failed = super::put_in_place(super::PartFile(part.clone()), &dest, Some(&known));
        assert!(failed.is_err());
        assert!(!part.exists(), "the part file is gone");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "changed here", "never overwritten");
        // And when it can be put in place, it is.
        std::fs::write(&part, "cloud bytes").unwrap();
        std::fs::remove_file(&dest).unwrap();
        assert!(super::put_in_place(super::PartFile(part.clone()), &dest, None).is_ok());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "cloud bytes");
        assert!(!part.exists());
    }

    /// Leftover part files are cleaned up: only this client's own, only old ones, never inside
    /// `.craftcloud/`.
    #[test]
    fn stale_part_files_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Weird")).unwrap();
        std::fs::create_dir_all(root.join(".craftcloud")).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 60 * 60);
        let files = [
            ("Weird/.trailing..44c77418e27569db-1794748821.craftcloud-part", true, true),
            ("Weird/.fresh.bin.e-1.craftcloud-part", false, false),
            ("Weird/movie.part", true, false),
            ("Weird/notes.craftcloud-part", true, false),
            (".craftcloud/.x.craftcloud-part", true, false),
        ];
        for (f, aged, _) in files {
            std::fs::write(root.join(f), "x").unwrap();
            if aged {
                std::fs::File::options().write(true).open(root.join(f)).unwrap().set_modified(old).unwrap();
            }
        }
        assert_eq!(super::sweep_stale_parts(root, super::STALE_PART), 1);
        for (f, _, gone) in files {
            assert_eq!(root.join(f).exists(), !gone, "{f}");
        }
    }
}

/// Tests that build an [`Engine`] and drive it against `crate::fake_server`.
#[cfg(test)]
mod engine_tests;
/// The requests the engine sends, recorded (`golden/`) and compared on every run.
#[cfg(test)]
mod golden_tests;
