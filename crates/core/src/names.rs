//! File names the sync has to be careful with: names it never syncs ([`ignored`]), names the
//! server can't take (never uploaded, said once), and names Windows can't have
//! ([`windows_trouble`]): left alone by a Windows client ([`Platform::Windows`]), flagged but
//! synced by the others. Two cloud names one Windows folder can't hold apart ([`clashes`]) are
//! held there too.

use std::collections::{BTreeMap, BTreeSet};

/// The kind of computer the sync runs on, for the rules that differ by file system. A parameter
/// rather than a `cfg`, so both sets of rules are tested everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// Windows: no `< > : " | ? *`, no name ending in a dot or a space, no reserved device
    /// names, and names that differ only in case are one file.
    Windows,
    /// Linux, macOS and the rest: any name the cloud takes.
    Other,
}

impl Platform {
    /// The computer this program is built for.
    pub const fn current() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Other }
    }
}

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
///
/// On Windows it also leaves alone every path with a name Windows can't have
/// ([`local_refuses`]), file or folder: such a name can't be made here, so it is never
/// downloaded, and must never read as "deleted here" either. And on every computer it leaves
/// alone a path with a name the server can't take (`server`, that server's [`NameRules`]): such
/// a name never reaches the server, and one another client put there is never taken as deleted.
pub fn skipped(path: &str, is_file: bool, platform: Platform, server: &NameRules) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let Some((last, parents)) = parts.split_last() else { return false };
    parents.iter().any(|s| ignored_always(s))
        || ignored_always(last)
        || (is_file && temporary(last))
        || parts.iter().any(|s| local_refuses(s, platform).is_some() || server.refuses(s).is_some())
}

/// Why this computer can't have this name (one path segment), if it can't: on Windows, whatever
/// [`windows_trouble`] says (the same rule the other computers warn with); elsewhere nothing.
pub fn local_refuses(name: &str, platform: Platform) -> Option<&'static str> {
    match platform {
        Platform::Windows => windows_trouble(name),
        Platform::Other => None,
    }
}

/// The first place in a cloud path this computer can't have (`Weird/trailing./a.txt` →
/// `Weird/trailing.`), with why.
pub fn local_refused_in(path: &str, platform: Platform) -> Option<(&str, &'static str)> {
    let mut end = 0;
    for segment in path.split('/') {
        end += segment.len();
        if let Some(why) = local_refuses(segment, platform) {
            return Some((&path[..end], why));
        }
        end += 1;
    }
    None
}

/// What a path is called on this computer's file system, for telling whether two cloud names
/// would be one file here: on Windows, case doesn't count (`Report.txt` is `report.txt`).
pub fn same_file_key(path: &str, platform: Platform) -> String {
    match platform {
        Platform::Windows => path.chars().flat_map(char::to_lowercase).collect(),
        Platform::Other => path.to_owned(),
    }
}

/// Cloud places this computer can't keep apart (on Windows: names that differ only in case,
/// `Docs/Report.txt` and `Docs/report.txt`), each with the name it clashes with.
///
/// Of each such group only the one this computer already has (`here`, by its exact name) keeps
/// syncing; the others are held, and when none is here, all of them are. Only the top-most place
/// is returned (a held folder holds everything in it). `cloud` is every cloud file and folder
/// path; their parent folders count too.
pub fn clashes<'a>(cloud: impl IntoIterator<Item = &'a str>, here: impl Fn(&str) -> bool, platform: Platform) -> BTreeMap<String, String> {
    let mut held: BTreeMap<String, String> = BTreeMap::new();
    if platform == Platform::Other {
        return held;
    }
    // Every place, by depth, so a folder is settled before what is in it.
    let mut by_depth: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for path in cloud {
        let mut end = 0;
        for (depth, segment) in path.split('/').enumerate() {
            end += segment.len();
            by_depth.entry(depth).or_default().insert(path[..end].to_owned());
            end += 1;
        }
    }
    for places in by_depth.values() {
        let mut groups: BTreeMap<String, Vec<&String>> = BTreeMap::new();
        for place in places {
            let inside_held = held.keys().any(|h| place.strip_prefix(h.as_str()).is_some_and(|rest| rest.starts_with('/')));
            if !inside_held {
                groups.entry(same_file_key(place, platform)).or_default().push(place);
            }
        }
        for group in groups.into_values().filter(|g| g.len() > 1) {
            let keeper = group.iter().find(|p| here(p.as_str()));
            for place in &group {
                if Some(place) == keeper {
                    continue;
                }
                let other = keeper.or_else(|| group.iter().find(|p| *p != place)).map_or_else(String::new, |p| (*p).clone());
                held.insert((*place).clone(), other);
            }
        }
    }
    held
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

/// The names a server can't take, as data (docs/WEBDAV-STORAGE.md § 7.3): part of what a server
/// guarantees the sync (`storage::Capabilities::names`), so that what the sync leaves alone for a
/// server goes through the one rule, [`skipped`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameRules {
    /// A backslash in a name is refused (or mishandled, which is worse).
    pub no_backslash: bool,
    /// A control character in a name is refused.
    pub no_control: bool,
    /// Names (one path segment) longer than 255 bytes of UTF-8 are refused.
    pub max_255_bytes: bool,
}

impl NameRules {
    /// OxiCloud 0.10.0: it answers a backslash in a name badly, so such a name must never reach
    /// it; control characters and names longer than 255 bytes are refused.
    pub const fn oxicloud() -> Self {
        Self { no_backslash: true, no_control: true, max_255_bytes: true }
    }

    /// Why this server can't take this name (one path segment), if it can't.
    pub fn refuses(&self, name: &str) -> Option<&'static str> {
        if self.no_backslash && name.contains('\\') {
            Some("the cloud can't take a backslash (\\) in a name; rename it to sync it")
        } else if self.no_control && name.chars().any(char::is_control) {
            Some("the cloud can't take a control character in a name; rename it to sync it")
        } else if self.max_255_bytes && name.len() > 255 {
            Some("the name is longer than the cloud allows (255 bytes); shorten it to sync it")
        } else {
            None
        }
    }

    /// The first segment of a cloud path this server can't take, with why.
    pub fn refused_in<'a>(&self, path: &'a str) -> Option<(&'a str, &'static str)> {
        path.split('/').find_map(|segment| self.refuses(segment).map(|why| (segment, why)))
    }
}

/// Why OxiCloud, CraftCloud's own server, can't take this name (one path segment), if it can't
/// ([`NameRules::oxicloud`]).
pub fn server_refuses(name: &str) -> Option<&'static str> {
    NameRules::oxicloud().refuses(name)
}

/// The first segment of a cloud path OxiCloud can't take, with why.
pub fn refused_in(path: &str) -> Option<(&str, &'static str)> {
    NameRules::oxicloud().refused_in(path)
}

/// Why Windows can't have this name (one path segment), if it can't. A Windows client leaves it
/// alone ([`local_refuses`]); the others sync it and warn that a Windows computer can't.
///
/// Reserved device names count with an extension too (`CON.txt`, `nul.log`): Windows 11 lets
/// most programs make them, but Windows 10, older programs and some Explorer actions still take
/// them for the device, so a file of that name could be made on one Windows computer and be
/// unreadable or undeletable on the next. Leaving it in the cloud is the side that can be undone
/// (rename it, and it syncs).
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

    use super::{NameRules, Platform, ignored, refused_in, server_refuses, skipped, windows_trouble};

    const OXI: NameRules = NameRules::oxicloud();

    /// A name the server can't take is left alone by the one skip rule, file or folder, with
    /// everything in it; a server whose rules allow it syncs it.
    #[test]
    fn names_the_server_cant_take_are_skipped() {
        for platform in [Platform::Other, Platform::Windows] {
            for path in ["Docs/back\\slash.txt", "Docs/a\\b/c.txt", "bell\u{7}.txt"] {
                assert!(skipped(path, true, platform, &OXI), "{path}");
            }
            let long = format!("Docs/{}", "x".repeat(256));
            assert!(skipped(&long, true, platform, &OXI) && skipped(&long, false, platform, &OXI));
            assert!(!skipped("Docs/notes.md", true, platform, &OXI));
        }
        let anything = NameRules { no_backslash: false, no_control: false, max_255_bytes: false };
        assert!(!skipped("Docs/back\\slash.txt", true, Platform::Other, &anything));
        assert_eq!(anything.refused_in("Docs/a\\b/c.txt"), None);
        assert_eq!(OXI.refused_in("Docs/a\\b/c.txt"), refused_in("Docs/a\\b/c.txt"));
    }

    #[test]
    fn lock_swap_and_temporary_files_never_sync() {
        for name in [
            ".craftcloud",
            ".report.1234-1.craft_tmp",
            ".a.docx.craftcloud-part",
            ".trailing..44c77418e27569db-1794748821.craftcloud-part",
            ".ccpart-0123456789abcdef01234567.craftcloud-part",
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

    /// The Windows test names: what a Windows client leaves alone is exactly what the others warn
    /// about, and the others keep syncing it.
    #[test]
    fn windows_leaves_alone_what_the_others_warn_about() {
        use super::{Platform, local_refused_in, local_refuses, skipped};
        let weird = ["Weird/a:b.txt", "Weird/what?.md", "Weird/CON.txt", "Weird/trailing.", "Weird/trailing ", "Weird/con", "Weird/Nul.log"];
        for path in weird {
            let name = path.rsplit('/').next().unwrap_or(path);
            assert!(windows_trouble(name).is_some(), "{path}: warned about");
            assert_eq!(local_refuses(name, Platform::Windows), windows_trouble(name), "{path}: the same rule on Windows");
            assert!(skipped(path, true, Platform::Windows, &OXI), "{path}: left alone on Windows");
            assert!(!skipped(path, true, Platform::Other, &OXI), "{path}: synced elsewhere");
            assert_eq!(local_refuses(name, Platform::Other), None, "{path}");
        }
        // A folder Windows can't have holds everything in it, on Windows only.
        assert!(skipped("Weird/trailing./inside.txt", true, Platform::Windows, &OXI));
        assert!(skipped("Weird/what?", false, Platform::Windows, &OXI));
        assert!(!skipped("Weird/trailing./inside.txt", true, Platform::Other, &OXI));
        assert_eq!(local_refused_in("Weird/trailing./inside.txt", Platform::Windows).map(|(p, _)| p), Some("Weird/trailing."));
        assert_eq!(local_refused_in("Weird/trailing./inside.txt", Platform::Other), None);
        for fine in ["Weird/trailing", "Weird/CONFIG.sys", "Weird/notes.md"] {
            assert!(!skipped(fine, true, Platform::Windows, &OXI), "{fine}");
        }
    }

    #[test]
    fn names_windows_cant_keep_apart_are_held() {
        use super::{Platform, clashes};
        let cloud = ["Docs", "Docs/Report.txt", "Docs/report.txt", "Docs/other.txt", "Weird", "Weird/trailing"];
        // This computer has `Docs/Report.txt`: it keeps syncing, the other one is held.
        let here = |p: &str| ["Docs", "Docs/Report.txt"].contains(&p);
        let held = clashes(cloud, here, Platform::Windows);
        assert_eq!(held.into_iter().collect::<Vec<_>>(), [("Docs/report.txt".to_owned(), "Docs/Report.txt".to_owned())]);
        // Neither is here yet: both are held, so neither lands on the other.
        let held = clashes(cloud, |_| false, Platform::Windows);
        assert_eq!(held.keys().collect::<Vec<_>>(), ["Docs/Report.txt", "Docs/report.txt"]);
        // Folders: the held one holds what's in it; what's in the kept one still syncs.
        let cloud = ["Photos", "Photos/a.jpg", "photos", "photos/a.jpg", "photos/b.jpg"];
        let held = clashes(cloud, |p| p == "Photos", Platform::Windows);
        assert_eq!(held.keys().collect::<Vec<_>>(), ["photos"]);
        // Elsewhere case counts: nothing clashes.
        assert!(clashes(["Docs/Report.txt", "Docs/report.txt"], |_| false, Platform::Other).is_empty());
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
