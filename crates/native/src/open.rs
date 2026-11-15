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
    // On Windows `canonicalize` answers `\\?\C:\…`: print and hand on the usual form (#5).
    if cfg!(windows)
        && let Some(plain) = path.to_str().and_then(plain_windows_path)
    {
        return Ok(PathBuf::from(plain));
    }
    Ok(path)
}

/// The usual form of a Windows "verbatim" path, as `canonicalize` gives it: `\\?\C:\a` → `C:\a`,
/// `\\?\UNC\server\share\a` → `\\server\share\a`. `None` when the path isn't verbatim, or when it
/// would mean something else (or nothing) without the prefix: as long as `MAX_PATH` or longer, a
/// `/` in it, or a name Windows treats specially (`CON`, `nul.txt`, `COM1`, a trailing dot or
/// space, `.` or `..`). Pure string work, so it is tested on every platform.
pub fn plain_windows_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix(r"\\?\")?;
    let plain = if let Some(unc) = rest.strip_prefix(r"UNC\") {
        let mut parts = unc.splitn(3, '\\');
        let (server, share) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if server.is_empty() || share.is_empty() {
            return None;
        }
        format!(r"\\{unc}")
    } else {
        let b = rest.as_bytes();
        if !(b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') {
            return None;
        }
        rest.to_owned()
    };
    // MAX_PATH (260) counts the closing NUL.
    if plain.encode_utf16().count() >= 260 || plain.contains('/') {
        return None;
    }
    if plain.split('\\').skip(if plain.starts_with(r"\\") { 4 } else { 1 }).filter(|n| !n.is_empty()).any(special_windows_name) {
        return None;
    }
    Some(plain)
}

/// A file name Windows changes or reserves outside a verbatim path.
fn special_windows_name(name: &str) -> bool {
    if name == "." || name == ".." || name.ends_with('.') || name.ends_with(' ') {
        return true;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
    let numbered =
        |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"));
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
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

/// Starts `program` and returns at once. Its stdin, stdout and stderr are null rather than ours:
/// an app left holding the caller's output pipe would keep `out=$(craftcloud open …)` waiting
/// until that app quits (#6). A thread reaps it, so a long-running client keeps no finished child.
fn spawn(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let mut child = command(program, args).spawn().map_err(|e| format!("couldn't start {program}: {e}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn command(program: &str, args: &[&std::ffi::OsStr]) -> std::process::Command {
    use std::process::Stdio;
    let mut command = std::process::Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::{OpenError, follow, locate, plain_windows_path, resolve};
    use craftcloud_core::sync::SyncPlan;

    /// #5: `craftcloud open` prints `C:\…`, not the `\\?\C:\…` that `canonicalize` gives on Windows.
    #[test]
    fn verbatim_windows_paths_get_their_usual_form() {
        let plain = |p: &str| plain_windows_path(p);
        assert_eq!(plain(r"\\?\C:\Users\john\CraftCloud\notes.md").as_deref(), Some(r"C:\Users\john\CraftCloud\notes.md"));
        assert_eq!(plain(r"\\?\d:\").as_deref(), Some(r"d:\"));
        assert_eq!(plain(r"\\?\UNC\server\share\CraftCloud\a.md").as_deref(), Some(r"\\server\share\CraftCloud\a.md"));
        assert_eq!(plain(r"\\?\C:\Docs\console.md").as_deref(), Some(r"C:\Docs\console.md"), "only the exact names are special");
        assert_eq!(plain(r"\\?\C:\Docs\COM10.md").as_deref(), Some(r"C:\Docs\COM10.md"));
        assert_eq!(plain(r"\\?\UNC\con\share\a.md").as_deref(), Some(r"\\con\share\a.md"), "a server may be called that");
        // Already usual, or not safe to shorten: left alone.
        for keep in [
            r"C:\Users\john\notes.md",
            "/home/john/CraftCloud/notes.md",
            r"\\server\share\a.md",
            r"\\?\Volume{0b1c}\a.md",
            r"\\?\UNC\server",
            r"\\?\C:",
            r"\\?\C:\Docs\CON",
            r"\\?\C:\Docs\nul.txt",
            r"\\?\C:\Docs\COM1.md",
            r"\\?\C:\Docs\lpt9",
            r"\\?\C:\Docs\trailing.",
            r"\\?\C:\Docs\trailing ",
            r"\\?\C:\Docs\..\up",
            r"\\?\C:\a/b",
        ] {
            assert_eq!(plain(keep), None, "{keep}");
        }
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(plain(&long), None, "past MAX_PATH only the verbatim form works");
    }

    /// #6: the opened app gets none of our stdin, stdout or stderr, so a script reading
    /// `craftcloud open`'s output isn't kept waiting until that app quits.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_opened_app_holds_none_of_our_output() {
        let mut child = super::command("sleep", &["30".as_ref()]).spawn().unwrap();
        let fds: Vec<_> = (0..3).map(|fd| std::fs::read_link(format!("/proc/{}/fd/{fd}", child.id()))).collect();
        let _ = child.kill();
        let _ = child.wait();
        for (fd, target) in fds.into_iter().enumerate() {
            assert_eq!(target.unwrap(), std::path::Path::new("/dev/null"), "fd {fd}");
        }
    }

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
