//! Opening synced files: a cloud path is turned into its local copy only when that stays inside
//! the sync folder, and then handed to the system (which opens it with its app, or shows it in
//! the file manager). `craftcloud open <path>`, `craftcloud://` links and the desktop client's
//! Activity list all come through here.

use std::path::{Path, PathBuf};

use craftcloud_core::deeplink::{self, DeepLink};
use craftcloud_core::sync::{SyncPlan, in_plan, local_relative};
use serde::{Deserialize, Serialize};

/// Why a path or link wasn't opened. The command line turns each into an exit code (docs/CLI.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum OpenError {
    /// Not a `craftcloud://` link CraftCloud reads, or a path that can't be inside the folder.
    BadLink(String),
    /// No sync folder is set up on this computer.
    NoFolder(String),
    /// The path is in a folder this computer doesn't sync, or leads outside the sync folder.
    NotSynced(String),
    /// It should be here but isn't (not downloaded yet, or deleted).
    NotHere(String),
    /// The system couldn't open it.
    Failed(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadLink(m) | Self::NoFolder(m) | Self::NotSynced(m) | Self::NotHere(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for OpenError {}

/// The local copy of the synced file or folder at `cloud_path`, checked: no `..`, no absolute or
/// odd names, and after following links it must still be inside `root` and exist.
pub fn resolve(root: &Path, cloud_path: &str) -> Result<PathBuf, OpenError> {
    let rel = local_relative(cloud_path).ok_or_else(|| OpenError::BadLink(format!("{cloud_path:?} isn't a path inside the CraftCloud folder")))?;
    let root = root.canonicalize().map_err(|e| OpenError::NotHere(format!("the CraftCloud folder {}: {e}", root.display())))?;
    let path = root.join(rel).canonicalize().map_err(|_| OpenError::NotHere(format!("{cloud_path} isn't on this computer (yet): has it synced?")))?;
    if !path.starts_with(&root) || path == root {
        return Err(OpenError::NotSynced(format!("{cloud_path} leads outside the CraftCloud folder")));
    }
    Ok(path)
}

/// [`resolve`], after checking there is a sync folder and `cloud_path` is in a folder it syncs
/// (not an excluded one, not CraftCloud's own `.craftcloud`).
pub fn locate(plan: &SyncPlan, cloud_path: &str) -> Result<PathBuf, OpenError> {
    if plan.local_folder.is_empty() {
        return Err(OpenError::NoFolder("No sync folder yet: set one up in the desktop client first.".into()));
    }
    let found = resolve(Path::new(&plan.local_folder), cloud_path);
    let synced = match &found {
        Err(e @ OpenError::BadLink(_)) => return Err(e.clone()),
        Ok(path) => in_plan(plan, cloud_path, path.is_dir()),
        // Not here: either kind would do.
        Err(_) => in_plan(plan, cloud_path, false) || in_plan(plan, cloud_path, true),
    };
    if !synced {
        return Err(OpenError::NotSynced(format!("{cloud_path} isn't in a folder this computer syncs")));
    }
    found
}

/// Acts on a `craftcloud://` link: opens the local copy with its app, or shows it in the file
/// manager. Returns the local path it handed to the system.
pub fn follow(plan: &SyncPlan, link: &str) -> Result<PathBuf, OpenError> {
    let link = deeplink::parse(link).map_err(|e| OpenError::BadLink(e.to_string()))?;
    let path = locate(plan, link.path())?;
    match link {
        DeepLink::Open(_) => open(&path),
        DeepLink::Reveal(_) => reveal(&path),
    }
    .map_err(OpenError::Failed)?;
    Ok(path)
}

/// Tells the user something when there may be no terminal to print it to (a link clicked in a
/// browser): the log, plus a desktop notification where `notify-send` exists (Linux).
pub fn tell_user(message: &str) {
    log::warn!("{message}");
    if cfg!(all(unix, not(target_os = "macos"))) {
        // Arguments, not a shell line: the message can't run anything.
        let _ = std::process::Command::new("notify-send").args(["--app-name=CraftCloud", "CraftCloud", message]).spawn();
    }
}

/// Opens a file with its default app (or a folder in the file manager).
pub fn open(path: &Path) -> Result<(), String> {
    spawn(opener(), &[path.as_os_str()])
}

/// Shows a file in the file manager, selected where the platform can do that.
pub fn reveal(path: &Path) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        spawn("open", &["-R".as_ref(), path.as_os_str()])
    } else if cfg!(windows) {
        let mut select = std::ffi::OsString::from("/select,");
        select.push(path.as_os_str());
        spawn("explorer", &[select.as_os_str()])
    } else {
        // xdg-open has no "select": open the folder it is in.
        spawn(opener(), &[path.parent().unwrap_or(path).as_os_str()])
    }
}

fn opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    }
}

fn spawn(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    std::process::Command::new(program).args(args).spawn().map(|_| ()).map_err(|e| format!("couldn't start {program}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{OpenError, follow, locate, resolve};
    use craftcloud_core::sync::SyncPlan;

    #[test]
    fn only_files_inside_the_sync_folder_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("CraftCloud");
        std::fs::create_dir_all(root.join("Documents")).unwrap();
        std::fs::write(root.join("Documents/notes.md"), "x").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();

        let ok = resolve(&root, "Documents/notes.md").unwrap();
        assert!(ok.ends_with("Documents/notes.md"));
        assert!(resolve(&root, "/Documents/notes.md").is_ok(), "a leading slash is the cloud root");
        for bad in ["../secret.txt", "Documents/../../secret.txt", "", "/", "Documents/missing.md", "a\\..\\..\\secret.txt"] {
            assert!(resolve(&root, bad).is_err(), "{bad:?} must not resolve");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_sync_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("CraftCloud");
        std::fs::create_dir_all(root.join("Documents")).unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), root.join("Documents/link.txt")).unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("Documents/up")).unwrap();
        assert!(matches!(resolve(&root, "Documents/link.txt"), Err(OpenError::NotSynced(_))));
        let plan = SyncPlan { local_folder: root.display().to_string(), ..SyncPlan::default() };
        assert!(matches!(locate(&plan, "Documents/up/secret.txt"), Err(OpenError::NotSynced(_))), "a linked folder");
        assert!(matches!(follow(&plan, "craftcloud://open?path=Documents/link.txt"), Err(OpenError::NotSynced(_))));
    }

    #[test]
    fn only_synced_folders_are_located() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["Documents/a.md", "Private/b.md", "top.md", ".craftcloud/state.json"] {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let plan = SyncPlan { local_folder: dir.path().display().to_string(), excluded: vec!["Private".into()], ..SyncPlan::default() };
        assert!(locate(&plan, "Documents/a.md").is_ok());
        assert!(locate(&plan, "Documents").is_ok(), "a synced folder");
        assert!(matches!(locate(&plan, "Private/b.md"), Err(OpenError::NotSynced(_))), "an excluded folder");
        assert!(matches!(locate(&plan, "top.md"), Err(OpenError::NotSynced(_))), "top-level files don't sync in this plan");
        assert!(matches!(locate(&plan, ".craftcloud/state.json"), Err(OpenError::NotSynced(_))));
        assert!(matches!(locate(&plan, "Documents/new.md"), Err(OpenError::NotHere(_))), "not downloaded yet");
        let only = SyncPlan { folders: vec!["Private".into()], ..plan.clone() };
        assert!(matches!(locate(&only, "Documents/a.md"), Err(OpenError::NotSynced(_))), "0.1.0's included folders");
        let none = SyncPlan { local_folder: String::new(), ..plan };
        assert!(matches!(locate(&none, "Documents/a.md"), Err(OpenError::NoFolder(_))));
    }

    #[test]
    fn a_bad_link_opens_nothing() {
        let plan = SyncPlan { local_folder: "/nonexistent".into(), ..SyncPlan::default() };
        for link in ["craftcloud://open?path=../etc/passwd", "craftcloud://run?path=a.md", "https://cloud.example.org/a.md"] {
            assert!(matches!(follow(&plan, link), Err(OpenError::BadLink(_))), "{link}");
        }
    }
}
