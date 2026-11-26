//! What the desktop client syncs and how far it has got. The platform code (craftcloud-native)
//! does the transfers; these types are shared with the UI.

use serde::{Deserialize, Serialize};

/// The client's saved choices (stored with craft-settings; the tokens live in the keychain).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientSettings {
    /// Server base URL; empty until signed in.
    pub server: String,
    /// The local folder that mirrors the cloud, e.g. `/home/john/CraftCloud`.
    pub local_folder: String,
    /// Top-level cloud folders that are NOT synced (switched off in the Folders tab). Every other
    /// folder syncs, new ones too, whichever side makes them.
    pub excluded: Vec<String>,
    /// Version 0.1.0's list of the folders that sync (only those). Still honoured while it is
    /// non-empty; [`ClientSettings::migrate`] turns it into `excluded` once the cloud's folders
    /// are known. Empty means everything syncs. Always written (as `[]` once migrated): the
    /// settings store merges into the file, so a key left out would keep its old value.
    pub folders: Vec<String>,
    /// Also sync the files at the top level of the cloud.
    pub sync_root_files: bool,
    /// What this computer is called in conflict copies; empty means the computer's own name.
    pub device_name: String,
    /// Light, dark, or as the system is.
    pub appearance: Appearance,
    /// The colour theme's id (`craft.nord`); empty for CraftCloud's own look, which a file from
    /// before this field reads as. An id the client doesn't know shows CraftCloud's look. Always
    /// written, since the settings store merges into the file.
    pub theme: String,
    /// The kind of server (docs/WEBDAV-STORAGE.md § 11): CraftCloud's own unless said otherwise.
    /// A file from before this field reads as `oxicloud`. Always written, since the settings
    /// store merges into the file.
    pub storage: Storage,
    /// The WebDAV base for another kind of server (`https://cloud.example.org/remote.php/dav/files/john/`);
    /// empty for OxiCloud, whose base is `<server>/webdav/`.
    pub dav_url: String,
    /// Look once a day for a newer CraftCloud release and say so in Settings › About (it is
    /// never downloaded or installed). On unless switched off; a file from before this field
    /// reads as on.
    pub check_for_updates: bool,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            server: String::new(),
            local_folder: String::new(),
            excluded: Vec::new(),
            folders: Vec::new(),
            sync_root_files: false,
            device_name: String::new(),
            appearance: Appearance::default(),
            storage: Storage::default(),
            dav_url: String::new(),
            check_for_updates: true,
            theme: String::new(),
        }
    }
}

/// The kind of server the client syncs with (docs/WEBDAV-STORAGE.md § 7.4). Only `oxicloud` and
/// `webdav` have a driver so far; the other three are reserved for their profiles, and a client
/// without one refuses them rather than treating them as another kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Storage {
    /// CraftCloud's own server.
    #[default]
    Oxicloud,
    /// Any other WebDAV server, with CraftCloud's own trash and versions in `.craftcloud/`.
    Webdav,
    Nextcloud,
    Owncloud,
    Opencloud,
}

impl Storage {
    /// The word in settings, `--kind` and `--json`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Oxicloud => "oxicloud",
            Self::Webdav => "webdav",
            Self::Nextcloud => "nextcloud",
            Self::Owncloud => "owncloud",
            Self::Opencloud => "opencloud",
        }
    }

    /// The kind a word names (`as_str`'s, any case).
    pub fn parse(word: &str) -> Option<Self> {
        [Self::Oxicloud, Self::Webdav, Self::Nextcloud, Self::Owncloud, Self::Opencloud]
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(word.trim()))
    }

    /// In plain words: "CraftCloud (OxiCloud)", "another WebDAV server" …
    pub const fn words(self) -> &'static str {
        match self {
            Self::Oxicloud => "CraftCloud (OxiCloud)",
            Self::Webdav => "another WebDAV server",
            Self::Nextcloud => "Nextcloud",
            Self::Owncloud => "ownCloud",
            Self::Opencloud => "OpenCloud",
        }
    }
}

impl ClientSettings {
    /// What a sign-out on the desktop keeps: the server (to sign in again), this computer's
    /// choices (its name, the appearance and colour theme, checking for updates); the sync's
    /// folder and choices go.
    pub fn after_sign_out(&self) -> Self {
        Self {
            server: self.server.clone(),
            device_name: self.device_name.clone(),
            appearance: self.appearance,
            theme: self.theme.clone(),
            check_for_updates: self.check_for_updates,
            ..Self::default()
        }
    }

    /// Whether these are 0.1.0's settings, with a list of included folders.
    pub fn needs_migration(&self) -> bool {
        !self.folders.is_empty()
    }

    /// 0.1.0's "only these folders" as "all but these", given the cloud's top-level folders.
    /// `None` when there is nothing to migrate.
    ///
    /// A file with both lists was migrated before by a build that didn't write `folders` back, so
    /// it migrated again on every start and switched off every folder made in between. Which of
    /// those exclusions were meant can't be told apart, so such a file gets nothing excluded:
    /// everything syncs again (nothing is stranded), and folders can be switched off again.
    pub fn migrate(&self, cloud_folders: &[String]) -> Option<Self> {
        if !self.needs_migration() {
            return None;
        }
        if !self.excluded.is_empty() {
            return Some(Self { excluded: Vec::new(), folders: Vec::new(), ..self.clone() });
        }
        let mut excluded: Vec<String> = cloud_folders.iter().filter(|f| !self.folders.contains(f)).cloned().collect();
        for f in &self.excluded {
            if !excluded.contains(f) {
                excluded.push(f.clone());
            }
        }
        Some(Self { excluded, folders: Vec::new(), ..self.clone() })
    }
}

/// The window's colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Dark,
    Light,
}

/// One pass of the initial download.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncPlan {
    pub server: String,
    pub local_folder: String,
    /// Top-level folders that don't sync.
    pub excluded: Vec<String>,
    /// 0.1.0's included folders; while non-empty only these sync (see [`ClientSettings::folders`]).
    pub folders: Vec<String>,
    pub sync_root_files: bool,
}

impl From<&ClientSettings> for SyncPlan {
    fn from(s: &ClientSettings) -> Self {
        Self {
            server: s.server.clone(),
            local_folder: s.local_folder.clone(),
            excluded: s.excluded.clone(),
            folders: s.folders.clone(),
            sync_root_files: s.sync_root_files,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SyncState {
    #[default]
    Idle,
    /// Walking the cloud folders.
    Scanning,
    Downloading,
    /// Everything is here.
    UpToDate,
    /// Syncing switched off by the person (tray or Settings).
    Paused,
    /// The server didn't answer (no network, or it's down); tried again every cycle.
    Offline(String),
    /// The sign-in ran out or this device was removed: sign in again. Nothing was deleted.
    SignedOut,
    /// Held: this cycle would delete `deletes` files at once, which is more likely a mistake
    /// (or a missing drive) than intent. Only the deletes wait (for a confirmation, or for the
    /// files to be brought back, or a change); everything else keeps syncing.
    MassDelete {
        deletes: usize,
    },
    /// The local folder is gone (moved, deleted, or on a drive that isn't plugged in).
    FolderMissing,
    Failed(String),
}

impl SyncState {
    /// A cycle is running.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Scanning | Self::Downloading)
    }
}

/// What happened to a file, for the activity list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityKind {
    /// Uploaded from this computer.
    Up,
    /// Downloaded from the cloud.
    Down,
    /// Deleted here or in the cloud.
    Deleted,
    /// Renamed or moved, on one side and then on the other (no upload or download).
    Moved,
    /// Changed in two places; both versions kept.
    Conflict,
    /// Couldn't be synced; tried again next cycle.
    Error,
    /// Left out on purpose (a folder switched off, top-level files off): said once, so nothing
    /// is skipped silently.
    Skipped,
    /// Synced, but worth knowing about (a name a Windows computer can't have): said once.
    Warning,
}

/// One line of the activity list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEvent {
    /// Seconds since 1970 (UTC).
    pub at: i64,
    /// Local wall-clock time, "14:32".
    pub time: String,
    /// Local date, "2026-11-13".
    pub date: String,
    pub kind: ActivityKind,
    /// The cloud path, `Documents/notes.md`.
    pub path: String,
    /// Bytes moved, where known.
    pub size: Option<u64>,
    /// A few words more: "to the cloud trash", "kept both", an error.
    pub detail: String,
}

/// A conflict copy next to its original: both sides changed `original`; `copy` holds this
/// computer's version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictCopy {
    pub original: String,
    pub copy: String,
}

/// How to settle a conflict copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Keep the copy as its own file (stop asking).
    Both,
    /// This computer's version replaces the original; the copy goes.
    Mine,
    /// The cloud's version stays; the copy goes.
    Cloud,
}

/// Shown in the main window and the tray.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncProgress {
    pub state: SyncState,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    /// Cloud path being transferred.
    pub current: String,
    /// Local files that already existed and differ from the cloud: kept, not overwritten.
    pub kept_local: Vec<String>,
    /// Recent activity, newest last, at most [`ACTIVITY_KEPT`].
    pub activity: Vec<ActivityEvent>,
    /// When the last cycle finished without an error (seconds since 1970).
    pub last_synced: Option<i64>,
    /// Files that aren't simply synced, by cloud path; every other known file is
    /// [`FileStatus::Synced`].
    pub unsynced: std::collections::BTreeMap<String, FileStatus>,
    /// The chosen top-level folders (and top-level files), counted after each cycle.
    pub folders: Vec<FolderStats>,
    /// Files that don't fit in the cloud (its storage quota is full): not synced, and not tried
    /// again until there is room or the person asks (Sync now).
    pub no_room: Vec<String>,
    /// Finished sync cycles since start (lets callers wait for "the next sync").
    pub cycles: u64,
    /// Conflict copies waiting for a decision.
    pub conflicts: Vec<ConflictCopy>,
    /// The highest `SyncFor` ticket whose cycle has finished.
    pub served: u64,
    /// While [`SyncState::MassDelete`] holds: the files whose deletion waits for a decision
    /// (everything else keeps syncing).
    pub pending_deletes: Vec<String>,
    /// Local places left out of the last sync although they're in a synced folder: one that
    /// can't be read, a symbolic link, a name the cloud can't take (the activity says why).
    pub not_synced: Vec<String>,
    /// Why each of [`Self::not_synced`] is left out.
    pub left_out: std::collections::BTreeMap<String, LeftOut>,
    /// Files the last cycle held because they were written moments before (an app may still be
    /// saving them): untouched by that cycle, synced by the next one.
    pub waiting: Vec<String>,
    /// What the sync may do with this server (docs/WEBDAV-STORAGE.md § 7.3), once the engine knows
    /// (`None` before its first cycle). Two-way on CraftCloud's own server.
    pub mode: Option<crate::storage::Mode>,
    /// Folders renamed or moved here whose move in the cloud the server couldn't do yet: asked
    /// again, never given up and never rebuilt as a copy; nothing in them syncs until it is done.
    pub waiting_moves: Vec<WaitingMove>,
}

/// A folder move that waits for the server ([`SyncProgress::waiting_moves`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingMove {
    pub from: String,
    pub to: String,
    /// How often the server was asked and couldn't (0: it waits for another move, not asked yet).
    pub attempts: u32,
}

/// From this many failed tries on, a waiting folder move is a warning in the window, the tray
/// and a notification, and the tries after it are spaced out ([`move_retry_wait`]).
pub const MOVE_WARNS_AFTER: u32 = 3;
/// After this many failed tries the person is told a last time; the tries go on.
pub const MOVE_LAST_NOTICE_AFTER: u32 = 5;

/// How long the client waits before asking for a folder move again, after `failed` tries: the
/// next pass for the first three, 2 minutes before the fourth, 5 minutes before the fifth and
/// before every one after it (it never stops asking).
pub fn move_retry_wait(failed: u32) -> std::time::Duration {
    match failed {
        0..MOVE_WARNS_AFTER => std::time::Duration::ZERO,
        MOVE_WARNS_AFTER => std::time::Duration::from_secs(2 * 60),
        _ => std::time::Duration::from_secs(5 * 60),
    }
}

impl SyncProgress {
    /// The waiting folder move tried most often, once one has reached [`MOVE_WARNS_AFTER`]: what
    /// the window and the tray warn about.
    pub fn stuck_move(&self) -> Option<&WaitingMove> {
        self.waiting_moves.iter().filter(|m| m.attempts >= MOVE_WARNS_AFTER).max_by_key(|m| m.attempts)
    }
}

/// Why a local place is left out of the sync ([`SyncProgress::left_out`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftOut {
    /// It can't be read (permissions, a broken disk): its contents are unknown, so nothing under
    /// it is synced or deleted until it can be read again. Needs the person's attention.
    Unreadable,
    /// A symbolic link: never followed, on purpose.
    Symlink,
    /// A name the cloud or this computer can't have, or a cloud name this computer can't keep
    /// apart from another (differing only in case on Windows); it syncs once renamed.
    Name,
    /// A file whose name looks like an app's temporary, lock or backup file; never synced.
    Temporary,
    /// New or changed on this computer, and not uploaded: the server only syncs downwards
    /// (download only, docs/WEBDAV-STORAGE.md § 9). It stays here as it is.
    NotUploaded,
}

impl LeftOut {
    /// The word in `craftcloud status --json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unreadable => "unreadable",
            Self::Symlink => "symlink",
            Self::Name => "name",
            Self::Temporary => "temporary",
            Self::NotUploaded => "not_uploaded",
        }
    }
}

/// Where one file stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// The same here and in the cloud.
    Synced,
    /// Waiting for, or in, a transfer this cycle.
    Syncing,
    /// The last try failed; tried again next cycle.
    Error,
    /// A conflict copy, or the file it was made from, waiting for a decision.
    Conflict,
}

/// One top-level cloud folder as this computer has it (`folder` is empty for the files at the
/// top level).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderStats {
    pub folder: String,
    /// Files synced, and their bytes.
    pub files: u64,
    pub bytes: u64,
    pub syncing: u64,
    pub errors: u64,
    pub conflicts: u64,
}

/// Counts per top-level folder (`""` for the files at the top level): the files known to be
/// synced (`files`, with their sizes) and what isn't synced yet (`unsynced`, which may hold files
/// that aren't known yet, such as new ones on their way). Sorted by folder.
pub fn folder_stats<'a>(
    files: impl IntoIterator<Item = (&'a str, u64)>,
    unsynced: &std::collections::BTreeMap<String, FileStatus>,
) -> Vec<FolderStats> {
    let mut by: std::collections::BTreeMap<String, FolderStats> = std::collections::BTreeMap::new();
    let entry = |by: &mut std::collections::BTreeMap<String, FolderStats>, path: &str| -> String {
        let folder = top_folder(path).unwrap_or("").to_owned();
        by.entry(folder.clone()).or_insert_with(|| FolderStats { folder: folder.clone(), ..FolderStats::default() });
        folder
    };
    for (path, size) in files {
        if unsynced.contains_key(path) {
            continue;
        }
        let key = entry(&mut by, path);
        if let Some(s) = by.get_mut(&key) {
            s.files += 1;
            s.bytes += size;
        }
    }
    for (path, status) in unsynced {
        let key = entry(&mut by, path);
        if let Some(s) = by.get_mut(&key) {
            match status {
                FileStatus::Synced => {}
                FileStatus::Syncing => s.syncing += 1,
                FileStatus::Error => s.errors += 1,
                FileStatus::Conflict => s.conflicts += 1,
            }
        }
    }
    by.into_values().collect()
}

/// How many activity events are kept.
pub const ACTIVITY_KEPT: usize = 200;

impl SyncProgress {
    /// Adds an event, dropping the oldest beyond [`ACTIVITY_KEPT`].
    pub fn record(&mut self, event: ActivityEvent) {
        self.activity.push(event);
        let extra = self.activity.len().saturating_sub(ACTIVITY_KEPT);
        self.activity.drain(..extra);
    }

    /// Where a known file stands.
    pub fn status_of(&self, path: &str) -> FileStatus {
        self.unsynced.get(path).copied().unwrap_or(FileStatus::Synced)
    }

    /// Files synced in all chosen folders.
    pub fn files_synced(&self) -> u64 {
        self.folders.iter().map(|f| f.files).sum()
    }
}

/// Whether a cloud path is in the plan: inside any top-level folder that isn't excluded (new ones
/// included), or a top-level file when those are synced. CraftCloud's own `.craftcloud` folder
/// never is.
pub fn in_plan(plan: &SyncPlan, path: &str, is_folder: bool) -> bool {
    if crate::api::dav::is_craftcloud_internal(path) {
        return false;
    }
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let Some(first) = parts.next() else { return false };
    let top_level = parts.next().is_none();
    if top_level && !is_folder {
        return plan.sync_root_files;
    }
    if !plan.folders.is_empty() {
        return plan.folders.iter().any(|f| f == first);
    }
    !plan.excluded.iter().any(|f| f == first)
}

/// The top-level folder a cloud path is in (`None` for a top-level file).
pub fn top_folder(path: &str) -> Option<&str> {
    path.split_once('/').map(|(first, _)| first)
}

/// A cloud path as a path below the local folder, refusing anything that could escape it, and
/// any name this computer can't have (`names::local_refuses`: on Windows also `C:`-style and
/// stream names, reserved names, a trailing dot).
pub fn local_relative(path: &str) -> Option<std::path::PathBuf> {
    local_relative_on(path, crate::names::Platform::current())
}

/// [`local_relative`] with that computer's name rule.
pub fn local_relative_on(path: &str, platform: crate::names::Platform) -> Option<std::path::PathBuf> {
    let mut out = std::path::PathBuf::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." || part.contains('\\') || part.contains('\0') || crate::names::local_refuses(part, platform).is_some() {
            return None;
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::{Appearance, ClientSettings, SyncPlan, in_plan, local_relative};

    /// Signing out forgets the sync but keeps this computer's choices: an update check switched
    /// off stays off.
    #[test]
    fn a_sign_out_keeps_this_computers_choices() {
        let before = ClientSettings {
            server: "https://cloud.example.org".into(),
            local_folder: "/home/john/CraftCloud".into(),
            excluded: vec!["Photos".into()],
            sync_root_files: true,
            device_name: "Laptop".into(),
            appearance: Appearance::Dark,
            check_for_updates: false,
            theme: "craft.nord".into(),
            ..ClientSettings::default()
        };
        let after = before.after_sign_out();
        assert_eq!(after.theme, "craft.nord", "the colour theme is this computer's choice");
        assert_eq!(after.server, "https://cloud.example.org");
        assert_eq!((after.device_name.as_str(), after.appearance, after.check_for_updates), ("Laptop", Appearance::Dark, false));
        assert!(after.local_folder.is_empty() && after.excluded.is_empty() && !after.sync_root_files);
    }
    use std::path::PathBuf;

    fn plan() -> SyncPlan {
        SyncPlan { server: String::new(), local_folder: String::new(), excluded: vec!["Photos".into()], folders: Vec::new(), sync_root_files: false }
    }

    #[test]
    fn every_folder_syncs_but_the_excluded_ones() {
        let p = plan();
        assert!(in_plan(&p, "Documents", true));
        assert!(in_plan(&p, "Documents/Reports/q3.docx", false));
        assert!(in_plan(&p, "Été/fichier.txt", false), "a folder made since syncs too");
        assert!(in_plan(&p, "Été", true));
        assert!(!in_plan(&p, "Photos/a.jpg", false));
        assert!(!in_plan(&p, "Photos", true));
        assert!(!in_plan(&p, "readme.md", false), "top-level files are off");
        assert!(in_plan(&SyncPlan { sync_root_files: true, ..plan() }, "readme.md", false));
        assert!(!in_plan(&SyncPlan { excluded: vec![], ..plan() }, ".craftcloud/versions/x", false));
        assert!(!in_plan(&SyncPlan { excluded: vec![], ..plan() }, ".craftcloud", true));
    }

    #[test]
    fn folders_are_counted_with_what_is_not_synced_yet() {
        use super::{FileStatus, folder_stats};
        let files = [("Documents/a.md", 10), ("Documents/Reports/b.docx", 20), ("readme.md", 3), ("Design/c.psd", 100)];
        let unsynced: std::collections::BTreeMap<String, FileStatus> = [
            ("Design/c.psd".to_owned(), FileStatus::Error),
            ("Design/new.psd".to_owned(), FileStatus::Syncing),
            ("Documents/a (conflict 2026-11-13 14.32 Laptop).md".to_owned(), FileStatus::Conflict),
        ]
        .into();
        let stats = folder_stats(files, &unsynced);
        let get = |f: &str| stats.iter().find(|s| s.folder == f).unwrap().clone();
        assert_eq!((get("Documents").files, get("Documents").bytes, get("Documents").conflicts), (2, 30, 1));
        assert_eq!((get("Design").files, get("Design").errors, get("Design").syncing), (0, 1, 1));
        assert_eq!((get("").files, get("").bytes), (1, 3));
    }

    #[test]
    fn version_0_1_settings_keep_their_folders_until_migrated() {
        let old = SyncPlan { folders: vec!["Documents".into()], excluded: vec![], ..plan() };
        assert!(in_plan(&old, "Documents/a.md", false));
        assert!(!in_plan(&old, "Design/a.md", false));

        let settings: ClientSettings = serde_json::from_str(
            r#"{"server":"https://cloud.example.org","local_folder":"/home/john/CraftCloud","folders":["Documents"],"sync_root_files":true}"#,
        )
        .unwrap();
        assert!(settings.needs_migration());
        let cloud = ["Documents".to_owned(), "Design".to_owned(), "Photos".to_owned()];
        let new = settings.migrate(&cloud).unwrap();
        assert_eq!(new.excluded, ["Design", "Photos"]);
        assert!(new.folders.is_empty() && new.sync_root_files);
        assert!(serde_json::to_string(&new).unwrap().contains("\"folders\":[]"), "written as empty, so the old list can't survive a save");

        // A file the 0.1.1 build migrated again and again: nothing stays switched off.
        let damaged = ClientSettings { folders: vec!["Work".into()], excluded: vec!["t2".into(), "Été".into()], ..ClientSettings::default() };
        let fixed = damaged.migrate(&["Work".to_owned(), "t2".to_owned(), "Été".to_owned()]).unwrap();
        assert!(fixed.excluded.is_empty() && fixed.folders.is_empty());
        assert!(!fixed.needs_migration());

        // Nothing chosen (folders = [], files at the top level on) means everything.
        let all: ClientSettings = serde_json::from_str(r#"{"folders":[],"sync_root_files":true}"#).unwrap();
        assert!(!all.needs_migration());
        assert!(in_plan(&SyncPlan::from(&all), "Anything/new.txt", false));
    }

    /// A 0.2.0 settings file names no server kind: it reads as CraftCloud's own server with no
    /// WebDAV base, and both are written from then on (docs/WEBDAV-STORAGE.md § 11, § 12).
    #[test]
    fn a_0_2_0_settings_file_reads_as_oxicloud() {
        use super::Storage;
        let old: ClientSettings = serde_json::from_str(
            r#"{"server":"https://cloud.example.org","local_folder":"/home/john/CraftCloud","excluded":["Photos"],"folders":[],"sync_root_files":true,"device_name":"Laptop","appearance":"dark"}"#,
        )
        .unwrap();
        assert_eq!((old.storage, old.dav_url.as_str()), (Storage::Oxicloud, ""));
        assert_eq!(old.server, "https://cloud.example.org");
        assert_eq!(old.excluded, ["Photos"]);
        let written = serde_json::to_string(&old).unwrap();
        assert!(written.contains("\"storage\":\"oxicloud\"") && written.contains("\"dav_url\":\"\""), "{written}");
        let other: ClientSettings =
            serde_json::from_str(r#"{"server":"https://cloud.example.org","storage":"webdav","dav_url":"https://cloud.example.org/dav/john/"}"#)
                .unwrap();
        assert_eq!(other.storage, Storage::Webdav);
        assert_eq!(Storage::parse("WebDAV"), Some(Storage::Webdav));
        assert_eq!(Storage::parse("nextcloud"), Some(Storage::Nextcloud));
        assert_eq!(Storage::parse("dropbox"), None);
        assert_eq!(serde_json::to_string(&Storage::Opencloud).unwrap(), "\"opencloud\"");
    }

    /// A settings file from before colour themes reads as CraftCloud's own look; a chosen theme
    /// is written and read back, and always written (the settings store merges into the file).
    #[test]
    fn the_colour_theme_defaults_to_craftclouds_look_and_round_trips() {
        let old: ClientSettings = serde_json::from_str(r#"{"server":"https://cloud.example.org","appearance":"dark"}"#).unwrap();
        assert_eq!(old.theme, "");
        assert!(serde_json::to_string(&old).unwrap().contains("\"theme\":\"\""));
        let chosen = ClientSettings { theme: "craft.nord".into(), ..old };
        let written = serde_json::to_string(&chosen).unwrap();
        assert!(written.contains("\"theme\":\"craft.nord\""), "{written}");
        assert_eq!(serde_json::from_str::<ClientSettings>(&written).unwrap(), chosen);
    }

    #[test]
    fn cloud_paths_stay_inside_the_local_folder() {
        assert_eq!(local_relative("Documents/notes.md"), Some(PathBuf::from("Documents").join("notes.md")));
        assert_eq!(local_relative("Documents/../../etc/passwd"), None);
        assert_eq!(local_relative("a\\..\\b"), None);
        assert_eq!(local_relative(""), None);
    }

    #[test]
    fn local_paths_follow_this_computers_name_rule() {
        use super::local_relative_on;
        use crate::names::Platform;
        for name in ["Weird/a:b.txt", "Weird/what?.md", "Weird/CON.txt", "Weird/trailing.", "Weird/trailing./in.txt", "C:/x"] {
            assert_eq!(local_relative_on(name, Platform::Windows), None, "{name}");
            assert!(local_relative_on(name, Platform::Other).is_some(), "{name}");
        }
        assert!(local_relative_on("Weird/trailing", Platform::Windows).is_some());
    }
}
