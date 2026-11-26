//! "Start CraftCloud when I log in", the way craft-launcher's `crates/native/src/login.rs` does it
//! (MIT OR Apache-2.0, Copyright (c) 2026 ArtCraft Team):
//!
//! - Linux and the BSDs: an XDG autostart entry, `~/.config/autostart/io.github.jonasbove.craftcloud.desktop`.
//!   From the Flatpak the sandbox's own config folder is not one the session reads, so there the
//!   desktop is asked through the XDG Background portal (`RequestBackground` with `autostart`),
//!   which writes an entry of that name in the host's autostart folder, with `flatpak run` in its
//!   `Exec` ([`flatpak`]).
//! - macOS: a LaunchAgent, `~/Library/LaunchAgents/io.github.jonasbove.craftcloud.plist`.
//! - Windows: a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! A login start passes [`HIDDEN_FLAG`]: the client syncs from the tray without opening its window.
//!
//! Versions before the app id was `io.github.jonasbove.craftcloud` wrote `craftcloud.desktop` and
//! `com.craftcloud.client.plist`. Such an entry still counts as on, and turning the setting on or
//! off removes it, so a start at login moves to the new name by itself and never starts twice.

#[cfg(not(any(target_os = "macos", windows)))]
mod flatpak;

#[cfg(not(windows))]
use std::path::Path;
use std::path::PathBuf;

/// The command-line flag a login start passes.
pub const HIDDEN_FLAG: &str = "--hidden";

#[cfg_attr(windows, allow(dead_code))]
const NAME: &str = "craftcloud";

/// The program to start: the AppImage when running from one (its mounted path changes every
/// run), `craftcloud` from PATH when installed with Nix (a store path changes on every update and
/// is garbage-collected later), else this executable.
fn program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(program_from(std::env::var_os("APPIMAGE"), exe))
}

/// [`program`], given `$APPIMAGE` and this executable. The AppImage runtime sets `$APPIMAGE` to the
/// `.AppImage` file itself; `exe` is then inside its mount (`/tmp/.mount_…`), gone after the run.
fn program_from(appimage: Option<std::ffi::OsString>, exe: PathBuf) -> PathBuf {
    match appimage.filter(|v| !v.is_empty()) {
        Some(appimage) => PathBuf::from(appimage),
        None => stable(exe),
    }
}

fn stable(exe: PathBuf) -> PathBuf {
    if cfg!(not(any(target_os = "macos", windows))) && exe.starts_with("/nix/store") { PathBuf::from(NAME) } else { exe }
}

/// The entry's file name, named for the app id.
#[cfg(not(any(target_os = "macos", windows)))]
const ENTRY: &str = "io.github.jonasbove.craftcloud.desktop";
#[cfg(target_os = "macos")]
const ENTRY: &str = "io.github.jonasbove.craftcloud.plist";

/// The file name versions before the app id used (see the module docs).
#[cfg(not(any(target_os = "macos", windows)))]
const OLD_ENTRY: &str = "craftcloud.desktop";
#[cfg(target_os = "macos")]
const OLD_ENTRY: &str = "com.craftcloud.client.plist";

/// The folder the entry goes in: `$XDG_CONFIG_HOME/autostart`, else `~/.config/autostart`.
#[cfg(not(any(target_os = "macos", windows)))]
fn entry_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart"))
}

/// The folder the entry goes in: `~/Library/LaunchAgents`.
#[cfg(target_os = "macos")]
fn entry_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/LaunchAgents"))
}

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn is_enabled() -> bool {
    #[cfg(not(any(target_os = "macos", windows)))]
    if flatpak::inside() {
        return flatpak::is_enabled();
    }
    #[cfg(windows)]
    {
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(RUN_KEY).and_then(|k| k.get_value::<String, _>("CraftCloud")).is_ok()
    }
    #[cfg(not(windows))]
    {
        entry_dir().is_some_and(|dir| is_enabled_in(&dir))
    }
}

/// Turns the start at login on or off. From the Flatpak this only asks the desktop, on a thread of
/// its own (it may show the person a dialog first): [`is_enabled`] gives the wanted state until the
/// answer is in, and [`take_problem`] what went wrong, if anything did.
pub fn set_enabled(on: bool) -> Result<(), String> {
    #[cfg(not(any(target_os = "macos", windows)))]
    if flatpak::inside() {
        return flatpak::set_enabled(on);
    }
    #[cfg(windows)]
    {
        let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(RUN_KEY).map_err(|e| e.to_string())?;
        if on {
            let command = format!("\"{}\" {HIDDEN_FLAG}", program()?.display());
            key.set_value("CraftCloud", &command).map_err(|e| e.to_string())
        } else {
            match key.delete_value("CraftCloud") {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        }
    }
    #[cfg(not(windows))]
    {
        let dir = entry_dir().ok_or("no home folder")?;
        set_enabled_in(&dir, on, || program().map(|p| contents(&p)))
    }
}

/// What went wrong with a change of the setting that finished in the background (the Flatpak's
/// portal request), told once; `None` when nothing did, and always outside the Flatpak.
pub fn take_problem() -> Option<String> {
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        flatpak::take_problem()
    }
    #[cfg(any(target_os = "macos", windows))]
    {
        None
    }
}

/// On when the entry, or the one an earlier version wrote, is in `dir`.
#[cfg(not(windows))]
fn is_enabled_in(dir: &Path) -> bool {
    dir.join(ENTRY).is_file() || dir.join(OLD_ENTRY).is_file()
}

/// Writes or removes the entry in `dir`, and removes an earlier version's entry either way. When
/// turning it on, the new entry is written before the old one goes, so a failed write leaves the
/// old one working.
#[cfg(not(windows))]
fn set_enabled_in(dir: &Path, on: bool, contents: impl FnOnce() -> Result<String, String>) -> Result<(), String> {
    if on {
        let text = contents()?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(ENTRY), text).map_err(|e| e.to_string())?;
    } else {
        remove(&dir.join(ENTRY))?;
    }
    remove(&dir.join(OLD_ENTRY))
}

#[cfg(not(windows))]
fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn contents(program: &Path) -> String {
    // Exec quoting per the Desktop Entry spec: the path in double quotes, with \ " ` $ escaped.
    let escaped: String =
        program.display().to_string().chars().flat_map(|c| if matches!(c, '\\' | '"' | '`' | '$') { vec!['\\', c] } else { vec![c] }).collect();
    format!(
        "[Desktop Entry]\nType=Application\nName=CraftCloud\nComment=Keeps your CraftCloud folder in sync\nExec=\"{escaped}\" {HIDDEN_FLAG}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(target_os = "macos")]
fn contents(program: &Path) -> String {
    let xml = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>io.github.jonasbove.craftcloud</string>\n<key>ProgramArguments</key><array><string>{}</string><string>{HIDDEN_FLAG}</string></array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n",
        xml(&program.display().to_string())
    )
}

#[cfg(all(test, not(any(target_os = "macos", windows))))]
mod tests {
    use super::{ENTRY, OLD_ENTRY, contents, is_enabled_in, program_from, set_enabled_in, stable};
    use std::path::{Path, PathBuf};

    #[test]
    fn an_appimage_starts_from_its_file_not_its_mount() {
        let mounted = PathBuf::from("/tmp/.mount_CraftCabc123/usr/bin/craftcloud");
        assert_eq!(
            program_from(Some("/home/john/Apps/CraftCloud.AppImage".into()), mounted.clone()),
            PathBuf::from("/home/john/Apps/CraftCloud.AppImage")
        );
        // An empty $APPIMAGE is no AppImage.
        assert_eq!(program_from(Some("".into()), mounted.clone()), mounted);
        assert_eq!(program_from(None, PathBuf::from("/usr/bin/craftcloud")), PathBuf::from("/usr/bin/craftcloud"));
        assert_eq!(program_from(None, PathBuf::from("/nix/store/abc-craftcloud/bin/craftcloud")), PathBuf::from("craftcloud"));
    }

    #[test]
    fn nix_installs_start_from_path() {
        assert_eq!(stable(PathBuf::from("/nix/store/abc-craftcloud-0.1.0/bin/craftcloud")), PathBuf::from("craftcloud"));
        assert_eq!(stable(PathBuf::from("/opt/craftcloud/craftcloud")), PathBuf::from("/opt/craftcloud/craftcloud"));
    }

    #[test]
    fn the_autostart_entry_quotes_the_program() {
        let text = contents(Path::new("/home/john/Apps/Craft $Cloud.AppImage"));
        assert!(text.contains("Exec=\"/home/john/Apps/Craft \\$Cloud.AppImage\" --hidden"), "{text}");
    }

    fn entry() -> Result<String, String> {
        Ok(contents(Path::new("/usr/bin/craftcloud")))
    }

    fn write_old(dir: &Path) -> Result<(), String> {
        std::fs::write(dir.join(OLD_ENTRY), "[Desktop Entry]\n").map_err(|e| e.to_string())
    }

    #[test]
    fn the_entry_is_named_for_the_app_id() {
        assert_eq!(ENTRY, "io.github.jonasbove.craftcloud.desktop");
    }

    #[test]
    fn an_entry_from_an_earlier_version_counts_as_on() -> Result<(), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        assert!(!is_enabled_in(dir.path()));
        write_old(dir.path())?;
        assert!(is_enabled_in(dir.path()));
        Ok(())
    }

    #[test]
    fn turning_it_on_moves_an_old_entry_to_the_new_name() -> Result<(), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        write_old(dir.path())?;
        set_enabled_in(dir.path(), true, entry)?;
        assert!(dir.path().join(ENTRY).is_file());
        // Left behind, it would start a second copy at every login.
        assert!(!dir.path().join(OLD_ENTRY).exists());
        assert!(is_enabled_in(dir.path()));
        Ok(())
    }

    #[test]
    fn turning_it_off_removes_the_old_entry_too() -> Result<(), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        write_old(dir.path())?;
        set_enabled_in(dir.path(), false, entry)?;
        assert!(!dir.path().join(OLD_ENTRY).exists());
        assert!(!is_enabled_in(dir.path()));
        // Off again, with nothing there, is fine.
        set_enabled_in(dir.path(), false, entry)
    }

    #[test]
    fn a_failed_write_keeps_the_old_entry() -> Result<(), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        write_old(dir.path())?;
        assert!(set_enabled_in(dir.path(), true, || Err("no program".into())).is_err());
        assert!(dir.path().join(OLD_ENTRY).is_file());
        Ok(())
    }
}
