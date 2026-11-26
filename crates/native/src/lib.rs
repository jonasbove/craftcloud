//! CraftCloud's desktop platform layer: HTTP on worker threads, tokens in the keychain, settings
//! through craft-settings, and the sync transfers. Only the desktop client depends on it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod control;
pub mod experimental;
#[cfg(test)]
mod fake_server;
pub mod http;
pub mod local_files;
pub mod login;
pub mod open;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod storage;
pub mod sync;
pub mod update_check;
pub mod url_scheme;

use std::path::PathBuf;

/// The folder name the client suggests for the synced files, in the user's home folder.
pub const DEFAULT_FOLDER_NAME: &str = "CraftCloud";

/// The suggested sync folder, `~/CraftCloud` (`%USERPROFILE%\CraftCloud` on Windows), or `None`
/// when the home folder is unknown.
pub fn default_sync_folder() -> Option<PathBuf> {
    home_from(|key| std::env::var_os(key)).map(|home| home.join(DEFAULT_FOLDER_NAME))
}

fn home_from(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let keys: &[&str] = if cfg!(windows) { &["USERPROFILE", "HOME"] } else { &["HOME"] };
    keys.iter().filter_map(|key| var(key)).find(|value| !value.is_empty()).map(PathBuf::from)
}

/// This computer's short name ("Laptop"), used in conflict copy names.
pub fn device_name() -> String {
    let host = gethostname::gethostname().to_string_lossy().trim().to_owned();
    let short = host.split('.').next().unwrap_or(&host).to_owned();
    if short.is_empty() { "this computer".to_owned() } else { short }
}

/// The name this device shows on the approval page and in the device list: "CraftCloud on Laptop".
pub fn client_name() -> String {
    let host = gethostname::gethostname().to_string_lossy().trim().to_owned();
    if host.is_empty() { "CraftCloud desktop".to_owned() } else { format!("CraftCloud on {host}") }
}

/// Now: seconds since 1970 and the local date ("2026-11-13").
pub fn wall_clock() -> (i64, String) {
    let now = jiff::Zoned::now();
    (now.timestamp().as_second(), now.strftime("%Y-%m-%d").to_string())
}

/// Size and last write ("today 13:58", or "2026-11-12 09:10") of the file at `cloud_path` in the
/// local folder `root`. `None` when the path would leave the folder or there is no such file.
pub fn file_facts(root: &std::path::Path, cloud_path: &str) -> Option<(u64, String)> {
    let path = root.join(craftcloud_core::sync::local_relative(cloud_path)?);
    let meta = std::fs::symlink_metadata(path).ok().filter(std::fs::Metadata::is_file)?;
    let modified = jiff::Timestamp::try_from(meta.modified().ok()?).ok()?.to_zoned(jiff::tz::TimeZone::system());
    let today = jiff::Zoned::now().date();
    let when = if modified.date() == today { modified.strftime("today %H:%M").to_string() } else { modified.strftime("%Y-%m-%d %H:%M").to_string() };
    Some((meta.len(), when))
}

/// A system font with CJK glyphs, as its bytes and the face's index in a collection (`.ttc`), for
/// the desktop's fallback. Looks in the usual places only; `None` when there is none.
pub fn cjk_font() -> Option<(Vec<u8>, u32)> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
    } else if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        dirs.push(windir.join("Fonts"));
    } else {
        // fontconfig knows which installed faces cover CJK, wherever they live (NixOS, Home Manager, Flatpak).
        if let Some(found) = first_usable(fontconfig_cjk()) {
            return Some(found);
        }
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts", "/run/current-system/sw/share/X11/fonts"].map(PathBuf::from));
        let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("fonts")));
        if let Ok(user) = std::env::var("USER") {
            dirs.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/share/fonts")));
        }
        if let Some(home) = home_from(|k| std::env::var_os(k)) {
            dirs.push(home.join(".local/share/fonts"));
            dirs.push(home.join(".fonts"));
            dirs.push(home.join(".nix-profile/share/fonts"));
        }
    }
    first_usable(find_font(&dirs).into_iter().map(|path| (path, 0)).collect())
}

/// The first candidate whose face parses: egui panics on a font it can't read, so nothing reaches
/// it unchecked. A few tries at most (CJK fonts are large).
fn first_usable(candidates: Vec<(PathBuf, u32)>) -> Option<(Vec<u8>, u32)> {
    for (path, index) in candidates.into_iter().take(6) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if usable(&bytes, index) {
            return Some((bytes, index));
        }
        log::info!("skipping font {} (face {index}): it doesn't parse", path.display());
    }
    None
}

/// Whether `bytes` hold a readable face at `index` (a collection's face number; 0 for a single font).
fn usable(bytes: &[u8], index: u32) -> bool {
    skrifa::FontRef::from_index(bytes, index).is_ok()
}

/// The first face fontconfig lists as covering Japanese (which CJK faces all do), preferring Noto and
/// Source Han, as its file and index in it.
fn fontconfig_cjk() -> Vec<(PathBuf, u32)> {
    match std::process::Command::new("fc-list").args(["-f", "%{file}\t%{index}\n", ":lang=ja"]).output() {
        Ok(out) if out.status.success() => pick_fontconfig(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// Orders `fc-list -f '%{file}\t%{index}\n'` lines best first: a known CJK family, regular weights,
/// a variable font's default instance. fontconfig puts a variable font's named instance in the
/// upper 16 bits of the index (`7 << 16 | 3`); the face number is the lower 16 (T-025).
fn pick_fontconfig(listing: &str) -> Vec<(PathBuf, u32)> {
    let mut faces: Vec<(PathBuf, u32, bool)> = listing
        .lines()
        .filter_map(|line| {
            let (file, index) = line.split_once('\t')?;
            let raw: u32 = index.trim().parse().unwrap_or(0);
            Some((PathBuf::from(file), raw & 0xFFFF, raw >> 16 != 0))
        })
        .filter(|(file, _, _)| !file.as_os_str().is_empty())
        .collect();
    faces.sort_by_key(|(file, index, named)| (cjk_rank(file), *named, *index));
    let mut out: Vec<(PathBuf, u32)> = Vec::new();
    for (file, index, _) in faces {
        if !out.iter().any(|(f, i)| *f == file && *i == index) {
            out.push((file, index));
        }
    }
    out
}

/// Lower is better: Noto Sans CJK and Source Han Sans (by name, any version or packaging, e.g.
/// NixOS's `NotoSansCJK-VF.otf.ttc`) first, then the other known CJK faces, regular or variable
/// weights before bold.
fn cjk_rank(file: &std::path::Path) -> (u8, u8) {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let family = if name.contains("notosanscjk") || name.contains("sourcehansans") {
        0
    } else if CJK_FONTS.iter().any(|known| known.eq_ignore_ascii_case(&name)) {
        1
    } else if name.contains("cjk") || name.contains("wqy") || name.contains("droidsansfallback") {
        2
    } else {
        3
    };
    let weight = if name.contains("regular") || name.contains("-vf") || name.contains("variable") { 0 } else { 1 };
    (family, weight)
}

/// CJK faces by file name, best first (Regular weights; collections hold SC/TC/JP/KR together).
const CJK_FONTS: &[&str] = &[
    "NotoSansCJK-Regular.ttc",
    "NotoSansCJKsc-Regular.otf",
    "NotoSansCJKjp-Regular.otf",
    "SourceHanSans-Regular.ttc",
    "wqy-microhei.ttc",
    "wqy-zenhei.ttc",
    "DroidSansFallbackFull.ttf",
    "DroidSansFallback.ttf",
    "PingFang.ttc",
    "Hiragino Sans GB.ttc",
    "STHeiti Light.ttc",
    "msyh.ttc",
    "YuGothR.ttc",
    "msgothic.ttc",
    "malgun.ttf",
];

/// CJK font files (by [`cjk_rank`], best first) in `dirs` or up to four folders below them.
fn find_font(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = dirs.iter().map(|d| (d.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 4 {
                stack.push((path, depth + 1));
            } else if cjk_rank(&path).0 < 3 {
                files.push(path);
            }
        }
    }
    files.sort_by_key(|p| cjk_rank(p));
    files
}

/// Opens a page in the default browser.
pub fn open_url(url: &str) -> Result<(), String> {
    webbrowser::open(url).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_FOLDER_NAME, client_name, home_from};
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn home_comes_from_the_environment() {
        let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let found = home_from(|key| (key == home).then(|| OsString::from("/home/john")));
        assert_eq!(found, Some(PathBuf::from("/home/john")));
        assert_eq!(found.unwrap().join(DEFAULT_FOLDER_NAME), PathBuf::from("/home/john").join("CraftCloud"));
    }

    #[test]
    fn no_home_means_no_suggestion() {
        assert_eq!(home_from(|_| None), None);
        assert_eq!(home_from(|_| Some(OsString::new())), None);
    }

    #[test]
    fn fonts_are_found_by_name_best_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/wqy-microhei.ttc"), b"x").unwrap();
        std::fs::write(dir.path().join("a/NotoSansCJK-Regular.ttc"), b"x").unwrap();
        let found = super::find_font(&[dir.path().to_path_buf()]);
        assert!(found[0].ends_with("a/NotoSansCJK-Regular.ttc"));
        assert!(super::find_font(&[dir.path().join("missing")]).is_empty());
    }

    #[test]
    fn nixos_variable_cjk_fonts_are_found() {
        // nixpkgs ships the variable collection under a name no fixed list had (T-004 on NixOS).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("share/X11/fonts")).unwrap();
        std::fs::write(dir.path().join("share/X11/fonts/DejaVuSans.ttf"), b"x").unwrap();
        std::fs::write(dir.path().join("share/X11/fonts/NotoSansCJK-VF.otf.ttc"), b"x").unwrap();
        let found = super::find_font(&[dir.path().to_path_buf()]);
        assert!(found[0].ends_with("NotoSansCJK-VF.otf.ttc"));
    }

    #[test]
    fn fontconfig_answers_prefer_noto_cjk() {
        let listing = "/nix/store/x-wqy/share/fonts/wqy-zenhei.ttc\t1\n/run/current-system/sw/share/X11/fonts/NotoSansCJK-VF.otf.ttc\t2\n\n";
        let picked = super::pick_fontconfig(listing);
        assert!(picked[0].0.ends_with("NotoSansCJK-VF.otf.ttc"));
        assert_eq!(picked[0].1, 2);
        assert!(super::pick_fontconfig("").is_empty());
    }

    #[test]
    fn variable_font_instances_become_plain_face_numbers() {
        // NixOS: fontconfig reports 458755 = 7 << 16 | 3 (named instance 7 of face 3); egui panicked on it (T-025).
        let listing = "/f/NotoSansCJK-VF.otf.ttc\t458755\n/f/NotoSansCJK-VF.otf.ttc\t3\n/f/NotoSansCJK-VF.otf.ttc\t65536\n";
        let picked = super::pick_fontconfig(listing);
        // Face 3's default instance first (it was listed plainly), then face 0 (only seen as a named instance).
        let vf = std::path::PathBuf::from("/f/NotoSansCJK-VF.otf.ttc");
        assert_eq!(picked, vec![(vf.clone(), 3), (vf, 0)]);
    }

    /// The real search on this machine's fonts: run with `cargo test -p craftcloud-native -- --ignored`.
    #[test]
    #[ignore = "depends on the fonts installed here"]
    fn this_machines_cjk_font_parses() {
        if let Some((bytes, index)) = super::cjk_font() {
            assert!(super::usable(&bytes, index));
            eprintln!("CJK font: {} bytes, face {index}", bytes.len());
        } else {
            eprintln!("no CJK font installed");
        }
    }

    #[test]
    fn only_fonts_that_parse_reach_the_ui() {
        let inter: &[u8] = include_bytes!("../../ui-egui/assets/fonts/Inter-Variable.ttf");
        assert!(super::usable(inter, 0));
        assert!(!super::usable(inter, 3), "a single font has no face 3");
        assert!(!super::usable(inter, 458_755));
        assert!(!super::usable(b"not a font", 0));
    }

    #[test]
    fn the_device_is_named_after_the_computer() {
        assert!(client_name().starts_with("CraftCloud"));
    }
}
