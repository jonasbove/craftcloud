//! File names the sync has to be careful with: names it never syncs ([`ignored`]), names the
//! server can't take (never uploaded, said once), and names another computer can't have
//! (Windows), which still sync but are flagged.

/// Whether a file or folder name is never synced (in either direction), wherever it is:
///
/// | Name | What it is |
/// |---|---|
/// | `.craftcloud` | CraftCloud's own folder (state, versions) |
/// | `*.craftcloud-part` | a download in progress (this client) |
/// | `*.craft_tmp` | a Craft app's save in progress |
/// | `*.part`, `*.tmp` | partial files that browsers and editors write before renaming |
/// | `~$*` | Microsoft Office's owner/lock files |
/// | `.~lock.*#`, `~lock.*#` | LibreOffice's lock files |
/// | `*.swp`, `*.swo` | Vim's swap files |
/// | `.#*`, `#*#`, `*~` | Emacs's lock, auto-save and backup files (and other editors' `~` backups) |
/// | `.DS_Store` | macOS Finder's folder settings |
/// | `Thumbs.db`, `desktop.ini` | Windows Explorer's thumbnails and folder settings |
///
/// The editor rules are about files (`temporary`): a FOLDER named `projekt.tmp` or `Backup~` syncs
/// like any other (T-062). Only `ignored_always` applies to folders.
pub fn ignored(name: &str) -> bool {
    ignored_always(name) || temporary(name)
}

/// Never synced, file or folder: CraftCloud's own folder and part files, and the system's
/// folder-settings files.
pub fn ignored_always(name: &str) -> bool {
    name == crate::api::dav::CRAFTCLOUD_DIR
        || name.ends_with(".craftcloud-part")
        || name.ends_with(".craft_tmp")
        || matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini")
}

/// Whether the sync leaves this cloud-style path alone on BOTH sides (T-063): under an
/// always-ignored folder, an always-ignored name, or a FILE with a temporary-looking name. The
/// local scan, the cloud listing and the saved state all go through this one rule, so an ignored
/// name is never downloaded, never uploaded, and never counts as "deleted here".
pub fn skipped(path: &str, is_file: bool) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let Some((last, parents)) = parts.split_last() else { return false };
    parents.iter().any(|s| ignored_always(s)) || ignored_always(last) || (is_file && temporary(last))
}

/// A FILE name that looks like an app's temporary, lock or backup file (not synced; shown as
/// "looks like a temporary file" in `status`). Never applied to folders.
pub fn temporary(name: &str) -> bool {
    const SUFFIXES: [&str; 4] = [".part", ".tmp", ".swp", ".swo"];
    SUFFIXES.iter().any(|s| name.ends_with(s))
        || name.starts_with("~$")
        || ((name.starts_with(".~lock.") || name.starts_with("~lock.")) && name.ends_with('#'))
        || name.starts_with(".~lock")
        || name.starts_with(".#")
        || (name.len() > 1 && name.starts_with('#') && name.ends_with('#'))
        || name.ends_with('~')
}

/// Why the server can't take this name (one path segment), if it can't. OxiCloud 0.10.0 answers
/// a backslash in a name badly (docs/UPSTREAM.md C12): such a name must never reach it.
pub fn server_refuses(name: &str) -> Option<&'static str> {
    if name.contains('\\') {
        Some("the cloud can't take a backslash (\\) in a name; rename it to sync it")
    } else if name.chars().any(char::is_control) {
        Some("the cloud can't take a control character in a name; rename it to sync it")
    } else if name.len() > 255 {
        Some("the name is longer than the cloud allows (255 bytes); shorten it to sync it")
    } else {
        None
    }
}

/// The first segment of a cloud path the server can't take, with why.
pub fn refused_in(path: &str) -> Option<(&str, &'static str)> {
    path.split('/').find_map(|segment| server_refuses(segment).map(|why| (segment, why)))
}

/// Why Windows can't have this name (one path segment), if it can't: it still syncs here and in
/// the cloud, but a Windows computer syncing the same folder can't make it.
pub fn windows_trouble(name: &str) -> Option<&'static str> {
    if name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        return Some("Windows can't have < > : \" | ? * in a name");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("Windows can't have a name that ends in a dot or a space");
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    reserved.then_some("Windows reserves this name (CON, PRN, AUX, NUL, COM1–9, LPT1–9)")
}

/// Where to restore a trashed item when its old place is taken (T-047): `Docs/a.txt` →
/// `Docs/a (restored).txt`, then `Docs/a (restored 2).txt` for `n` 2 and on. A folder's name is
/// kept whole (`Photos (restored)`), as is a name starting with a dot (`.env (restored)`).
pub fn restored_name(path: &str, is_folder: bool, n: u32) -> String {
    let (dir, name) = path.rsplit_once('/').map_or(("", path), |(d, n)| (d, n));
    let (stem, ext) = match name.rfind('.') {
        Some(i) if !is_folder && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let tag = if n <= 1 { "restored".to_owned() } else { format!("restored {n}") };
    let renamed = format!("{stem} ({tag}){ext}");
    if dir.is_empty() { renamed } else { format!("{dir}/{renamed}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn restored_names_say_so() {
        use super::restored_name;
        assert_eq!(restored_name("Docs/a.txt", false, 1), "Docs/a (restored).txt");
        assert_eq!(restored_name("Docs/a.txt", false, 2), "Docs/a (restored 2).txt");
        assert_eq!(restored_name("report.final.pdf", false, 1), "report.final (restored).pdf");
        assert_eq!(restored_name("Docs/.env", false, 1), "Docs/.env (restored)");
        assert_eq!(restored_name("Photos/2026.11", true, 1), "Photos/2026.11 (restored)");
    }

    use super::{ignored, refused_in, server_refuses, windows_trouble};

    #[test]
    fn lock_swap_and_temporary_files_never_sync() {
        for name in [
            ".craftcloud",
            ".report.1234-1.craft_tmp",
            ".a.docx.craftcloud-part",
            "x.part",
            "~WRL0001.tmp",
            "~$report.docx",
            ".~lock.report.odt#",
            "~lock.report.doc#",
            ".notes.md.swp",
            ".notes.md.swo",
            ".#doc.txt",
            "doc.txt~",
            "#notes.org#",
            ".DS_Store",
            "Thumbs.db",
            "desktop.ini",
        ] {
            assert!(ignored(name), "{name}");
        }
        for name in [
            "report.docx",
            ".env",
            "notes.md",
            "part.txt",
            "tmp",
            "lock.doc",
            "swp.txt",
            "Desktop.ini.bak",
            "a~b.txt",
            "#doc.txt#x",
            "~notes.md",
            "#",
            "#tag.md",
        ] {
            assert!(!ignored(name), "{name}");
        }
    }

    #[test]
    fn names_the_server_cant_take() {
        assert!(server_refuses("back\\slash.txt").is_some());
        assert!(server_refuses("bell\u{7}.txt").is_some());
        assert!(server_refuses(&"x".repeat(256)).is_some());
        for fine in ["notes.md", "Été.txt", "a:b.txt", "CON", "q?.txt"] {
            assert_eq!(server_refuses(fine), None, "{fine}");
        }
        assert_eq!(refused_in("Docs/a\\b/c.txt").map(|(s, _)| s), Some("a\\b"));
        assert_eq!(refused_in("Docs/ok.txt"), None);
    }

    #[test]
    fn names_windows_cant_have() {
        for bad in
            ["a:b.txt", "what?.md", "star*.png", "pipe|", "quote\".txt", "<x>", "trailing.", "trailing ", "CON", "con.txt", "Nul", "COM1.log", "lpt9"]
        {
            assert!(windows_trouble(bad).is_some(), "{bad}");
        }
        for fine in ["notes.md", "COM0", "COM10", "CONFIG.sys", "console.txt", ".env", "a.b.c"] {
            assert_eq!(windows_trouble(fine), None, "{fine}");
        }
    }
}
