//! The sync folder as this computer has it: which synced files are here, where, and whether each
//! is in step with the cloud. Read from the disk and the sync's `state.json` only (no network), so
//! a launcher or a Craft app can ask "which WordCraft files are in CraftCloud?" offline and fast,
//! through the control channel (`craftcloud files --app wordcraft`) or the command line's
//! listings (`local_path`, `sync` in `--json`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use craftcloud_core::reconcile::State;
use craftcloud_core::sync::{FileStatus, SyncPlan, in_plan, local_relative};
use serde::{Deserialize, Serialize};

use crate::sync::{ignored, local_meta, synced_state};

/// A synced file in the local folder, as `craftcloud files --json` prints it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedFile {
    /// Cloud path, `Documents/Q3 report.docx`.
    pub path: String,
    pub name: String,
    /// The local copy, absolute.
    pub local_path: String,
    pub size: u64,
    /// The local copy's last write, RFC 3339 (UTC, whole seconds).
    pub modified: String,
    /// One of [`SyncWord`]'s words.
    pub sync: String,
}

/// Where a file stands, as one word in `--json` output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncWord {
    /// The same here and in the cloud.
    Synced,
    /// Changed or new on one side; the next sync moves it.
    Pending,
    /// In a transfer now.
    Syncing,
    /// The last try failed; tried again next cycle.
    Error,
    /// A conflict copy, or the file it was made from, waiting for a decision.
    Conflict,
    /// In a folder this computer doesn't sync.
    Excluded,
}

impl SyncWord {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::Pending => "pending",
            Self::Syncing => "syncing",
            Self::Error => "error",
            Self::Conflict => "conflict",
            Self::Excluded => "excluded",
        }
    }
}

/// The sync folder and what the last sync agreed on, plus (from a running client) the files
/// that aren't simply synced.
pub struct LocalView {
    pub plan: SyncPlan,
    root: PathBuf,
    state: State,
    unsynced: BTreeMap<String, FileStatus>,
}

impl LocalView {
    /// `None` when no sync folder is set up.
    pub fn new(plan: &SyncPlan, unsynced: BTreeMap<String, FileStatus>) -> Option<Self> {
        if plan.local_folder.trim().is_empty() {
            return None;
        }
        let root = PathBuf::from(&plan.local_folder);
        let state = synced_state(&root);
        Some(Self { plan: plan.clone(), root, state, unsynced })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where a cloud file stands here: its local copy (when it is synced and present) and its
    /// sync word. A file in the plan that isn't here yet is `pending` (it will be downloaded).
    pub fn locate(&self, cloud_path: &str) -> (Option<PathBuf>, SyncWord) {
        if !in_plan(&self.plan, cloud_path, false) {
            return (None, SyncWord::Excluded);
        }
        let Some(rel) = local_relative(cloud_path) else { return (None, SyncWord::Excluded) };
        let local = self.root.join(rel);
        match local_meta(&local) {
            Some(now) => (Some(local), self.word(cloud_path, now.size, now.mtime)),
            None => (None, self.unsynced_word(cloud_path).unwrap_or(SyncWord::Pending)),
        }
    }

    fn unsynced_word(&self, cloud_path: &str) -> Option<SyncWord> {
        self.unsynced.get(cloud_path).map(|s| match s {
            FileStatus::Synced => SyncWord::Synced,
            FileStatus::Syncing => SyncWord::Syncing,
            FileStatus::Error => SyncWord::Error,
            FileStatus::Conflict => SyncWord::Conflict,
        })
    }

    fn word(&self, cloud_path: &str, size: u64, mtime: i64) -> SyncWord {
        if let Some(w) = self.unsynced_word(cloud_path) {
            return w;
        }
        match self.state.files.get(cloud_path) {
            Some(k) if k.size == size && k.mtime == mtime => SyncWord::Synced,
            _ => SyncWord::Pending,
        }
    }

    /// Every synced file present in the local folder whose cloud path `keep` accepts, by path.
    /// CraftCloud's own `.craftcloud`, temporary files and excluded folders are left out;
    /// symbolic links are not followed.
    pub fn files(&self, keep: impl Fn(&str) -> bool) -> Vec<SyncedFile> {
        let mut out = Vec::new();
        let mut stack = vec![(self.root.clone(), String::new())];
        while let Some((dir, prefix)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if craftcloud_core::names::ignored_always(&name) {
                    continue;
                }
                let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_file() && ignored(&name) {
                    continue;
                }
                if kind.is_dir() {
                    if in_plan(&self.plan, &path, true) {
                        stack.push((entry.path(), path));
                    }
                } else if kind.is_file() && in_plan(&self.plan, &path, false) && keep(&path) {
                    let Some(meta) = local_meta(&entry.path()) else { continue };
                    out.push(SyncedFile {
                        sync: self.word(&path, meta.size, meta.mtime).as_str().to_owned(),
                        local_path: entry.path().display().to_string(),
                        size: meta.size,
                        modified: rfc3339(meta.mtime),
                        name,
                        path,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }
}

/// Keeps cloud paths with one of these extensions (lower case; empty keeps everything).
pub fn by_extension(extensions: &[String]) -> impl Fn(&str) -> bool + '_ {
    move |path| {
        extensions.is_empty() || craftcloud_core::catalog::extension(path).is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(&e)))
    }
}

/// Milliseconds since 1970 as RFC 3339, whole seconds, UTC.
fn rfc3339(mtime_ms: i64) -> String {
    jiff::Timestamp::from_second(mtime_ms.div_euclid(1000)).map(|t| t.to_string()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{LocalView, SyncWord, by_extension};
    use craftcloud_core::reconcile::{Known, State};
    use craftcloud_core::sync::{FileStatus, SyncPlan};

    /// A sync folder: Documents (synced), Private (excluded), `.craftcloud` and a temporary file.
    pub(crate) fn folder() -> (tempfile::TempDir, SyncPlan) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (path, bytes) in [
            ("Documents/Q3 report.docx", "report"),
            ("Documents/Notes.MD", "# notes"),
            ("Documents/Sheets/budget.xlsx", "xlsx"),
            ("Documents/~$Q3 report.docx", "lock"),
            ("Private/diary.md", "secret"),
            (".craftcloud/versions/Documents/Notes.MD/1", "old"),
            ("top.md", "top-level file"),
        ] {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let plan = SyncPlan { local_folder: root.display().to_string(), excluded: vec!["Private".into()], ..SyncPlan::default() };
        (dir, plan)
    }

    #[test]
    fn lists_the_synced_files_an_app_opens() {
        let (dir, plan) = folder();
        // The report synced as it is now; the notes changed since.
        let meta = crate::sync::local_meta(&dir.path().join("Documents/Q3 report.docx")).unwrap();
        let mut state = State::default();
        state.files.insert("Documents/Q3 report.docx".into(), Known { etag: "\"1\"".into(), size: meta.size, mtime: meta.mtime, hash: None });
        state.files.insert("Documents/Notes.MD".into(), Known { etag: "\"2\"".into(), size: 1, mtime: 1, hash: None });
        let json = serde_json::json!({ "server": "https://cloud.example.org", "files": state.files, "folders": [] });
        std::fs::write(dir.path().join(".craftcloud/state.json"), json.to_string()).unwrap();

        let view = LocalView::new(&plan, Default::default()).unwrap();
        let word = craftcloud_core::catalog::app("wordcraft").unwrap();
        let files = view.files(|p| word.opens_name(p));
        let paths: Vec<(&str, &str)> = files.iter().map(|f| (f.path.as_str(), f.sync.as_str())).collect();
        // Not Private (excluded), not `.craftcloud`, not the lock file, not top.md (top-level files off).
        assert_eq!(paths, [("Documents/Notes.MD", "pending"), ("Documents/Q3 report.docx", "synced")]);
        let report = &files[1];
        assert_eq!((report.name.as_str(), report.size), ("Q3 report.docx", 6));
        assert!(report.local_path.ends_with("Q3 report.docx"));
        assert!(report.modified.ends_with('Z') && report.modified.len() == 20, "{}", report.modified);

        let exts = vec!["XLSX".to_owned()];
        assert_eq!(view.files(by_extension(&exts)).len(), 1, "extensions match in any case");
        assert_eq!(view.files(by_extension(&[])).len(), 3, "no filter: every synced file");
    }

    #[test]
    fn a_running_client_knows_what_is_moving() {
        let (_dir, plan) = folder();
        let unsynced = [("Documents/Q3 report.docx".to_owned(), FileStatus::Conflict), ("Documents/new.docx".to_owned(), FileStatus::Syncing)];
        let view = LocalView::new(&plan, unsynced.into_iter().collect()).unwrap();
        let (local, word) = view.locate("Documents/Q3 report.docx");
        assert!(local.is_some());
        assert_eq!(word, SyncWord::Conflict);
        assert_eq!(view.locate("Documents/new.docx"), (None, SyncWord::Syncing));
        assert_eq!(view.locate("Documents/elsewhere.docx"), (None, SyncWord::Pending), "in the plan, not here yet");
        assert_eq!(view.locate("Private/diary.md"), (None, SyncWord::Excluded));
        assert_eq!(view.locate(".craftcloud/state.json"), (None, SyncWord::Excluded));
        assert!(LocalView::new(&SyncPlan::default(), Default::default()).is_none(), "no folder set up");
    }
}
