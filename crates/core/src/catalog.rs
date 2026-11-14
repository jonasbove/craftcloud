//! The Craft apps and the files they open.
//!
//! - [`APPS`]: `apps.toml`'s table (names, colours, `opens`, `default`), built in so it works
//!   offline, on the slim image and in the desktop client. A test keeps it in step with `apps.toml`.
//! - [`parse`]: `/craftcloud/apps.json`, the Craft web apps the `-apps` image bundles (written by
//!   `scripts/fetch-apps` from `apps.toml`). Absent on the slim image.

use serde::Deserialize;

/// One Craft app, as `apps.toml` lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CraftApp {
    pub id: &'static str,
    pub name: &'static str,
    /// `#rrggbb` surface colour.
    pub color: &'static str,
    /// Text colours on dark and light backgrounds.
    pub ink: [&'static str; 2],
    /// Opened through CraftCloud's shim in the browser.
    pub experimental: bool,
    /// Extensions (lower case) the app can open.
    pub opens: &'static [&'static str],
    /// Extensions the app claims when a file is opened; each extension has one default app.
    pub default: &'static [&'static str],
}

impl CraftApp {
    /// Whether this app opens a file of this name (by its extension, any case).
    pub fn opens_name(&self, name: &str) -> bool {
        extension(name).is_some_and(|e| self.opens.contains(&e.as_str()))
    }
}

pub const APPS: &[CraftApp] = &[
    CraftApp {
        id: "wordcraft",
        name: "WordCraft",
        color: "#3f5fe0",
        ink: ["#8ea2ff", "#2a44b8"],
        experimental: false,
        opens: &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "ott", "fodt", "rtf", "txt", "md", "html", "htm", "tex"],
        default: &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "ott", "fodt", "rtf", "txt", "md"],
    },
    CraftApp {
        id: "gridcraft",
        name: "GridCraft",
        color: "#1f9d55",
        ink: ["#4fd68a", "#12703a"],
        experimental: false,
        opens: &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"],
        default: &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"],
    },
    CraftApp {
        id: "deckcraft",
        name: "DeckCraft",
        color: "#f07a1e",
        ink: ["#ffa060", "#a84f0a"],
        experimental: false,
        opens: &["deckcraft", "pptx", "potx", "ppsx"],
        default: &["deckcraft", "pptx", "potx", "ppsx"],
    },
    CraftApp {
        id: "pdfcraft",
        name: "PdfCraft",
        color: "#e8204f",
        ink: ["#f23a62", "#b70228"],
        experimental: false,
        opens: &["pdf"],
        default: &["pdf"],
    },
    CraftApp {
        id: "figcraft",
        name: "FigCraft",
        color: "#02da96",
        ink: ["#4fe8b6", "#047a56"],
        experimental: false,
        opens: &["figcraft", "fig", "jam", "penpot", "sketch", "svg", "png", "jpg", "jpeg", "webp"],
        default: &["figcraft", "fig", "jam", "penpot", "sketch"],
    },
    CraftApp {
        id: "cadcraft",
        name: "CADCraft",
        color: "#3ea6e8",
        ink: ["#7cc8ff", "#1a6fa8"],
        experimental: false,
        opens: &["dxf", "dwg"],
        default: &["dxf", "dwg"],
    },
    CraftApp {
        id: "filmcraft",
        name: "FilmCraft",
        color: "#8b5cf6",
        ink: ["#b69cff", "#6a3fd6"],
        experimental: false,
        opens: &["fcproj", "mp4", "mov", "m4v", "webm", "mkv"],
        default: &["fcproj", "mp4", "mov", "m4v", "webm", "mkv"],
    },
    CraftApp {
        id: "lightcraft",
        name: "LightCraft",
        color: "#f2a516",
        ink: ["#ffc14d", "#8a5800"],
        experimental: false,
        opens: &["jpg", "jpeg", "png", "tif", "tiff", "webp", "heic", "dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2"],
        default: &["dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2"],
    },
    CraftApp {
        id: "vectorcraft",
        name: "VectorCraft",
        color: "#e8573f",
        ink: ["#ff8a70", "#b83a24"],
        experimental: true,
        opens: &["vectorcraft", "drawcraft", "svg", "svgz", "ai", "eps", "emf", "wmf", "pdf", "dxf"],
        default: &["vectorcraft", "drawcraft", "svg", "svgz", "ai", "eps", "emf", "wmf"],
    },
    CraftApp {
        id: "effectcraft",
        name: "EffectCraft",
        color: "#e0368f",
        ink: ["#ff7ab8", "#b0206c"],
        experimental: true,
        opens: &["ecproj", "aep", "aepx"],
        default: &["ecproj", "aep", "aepx"],
    },
    CraftApp {
        id: "photocraft",
        name: "PhotoCraft",
        color: "#2f7bf5",
        ink: ["#7fb0ff", "#1a5bc4"],
        experimental: true,
        opens: &["pcraft", "psd", "psb", "ora", "kra", "xcf", "exr", "png", "jpg", "jpeg", "webp", "tif", "tiff", "heic", "gif", "bmp"],
        default: &["pcraft", "psd", "psb", "ora", "kra", "xcf", "exr", "png", "jpg", "jpeg", "webp", "tif", "tiff", "heic", "gif", "bmp"],
    },
    CraftApp {
        id: "designcraft",
        name: "DesignCraft",
        color: "#7bb51c",
        ink: ["#a6e04a", "#457008"],
        experimental: true,
        opens: &["designcraft", "idml", "indd", "indt"],
        default: &["designcraft", "idml", "indd", "indt"],
    },
    CraftApp {
        id: "soundcraft",
        name: "SoundCraft",
        color: "#1fa5c9",
        ink: ["#5fd0ee", "#0e7290"],
        experimental: true,
        opens: &["scraft", "scraftz", "wav", "flac", "mp3", "ogg", "m4a", "mid", "midi"],
        default: &["scraft", "scraftz", "wav", "flac", "mp3", "ogg", "m4a", "mid", "midi"],
    },
    CraftApp {
        id: "texturecraft",
        name: "TextureCraft",
        color: "#c2672b",
        ink: ["#e58a4e", "#9c4a15"],
        experimental: true,
        opens: &["tcraft", "glb", "gltf", "fbx", "dae", "usd", "usda", "usdc", "usdz", "abc", "obj", "ply", "stl"],
        default: &["tcraft", "glb", "gltf", "fbx", "dae", "usd", "usda", "usdc", "usdz", "abc", "obj", "ply", "stl"],
    },
    CraftApp {
        id: "materialcraft",
        name: "MaterialCraft",
        color: "#178a9c",
        ink: ["#63c5d3", "#106573"],
        experimental: true,
        opens: &["mcraft", "sbs"],
        default: &["mcraft", "sbs"],
    },
];

/// The lower-case extension of a file name or path (`"Q3 report.DOCX"` → `"docx"`), if any.
pub fn extension(name: &str) -> Option<String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty() && !ext.contains(' ')).then(|| ext.to_ascii_lowercase())
}

/// The app that opens a file of this name by default (`.md` and `.txt` → WordCraft).
pub fn for_name(name: &str) -> Option<&'static CraftApp> {
    let ext = extension(name)?;
    APPS.iter().find(|a| a.default.contains(&ext.as_str()))
}

/// The app with this id (any case: `WordCraft` finds `wordcraft`).
pub fn app(id: &str) -> Option<&'static CraftApp> {
    APPS.iter().find(|a| a.id.eq_ignore_ascii_case(id.trim()))
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WebApp {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tagline: String,
    /// `#rrggbb`, the app's surface colour.
    #[serde(default)]
    pub color: String,
    /// Text colours on dark and light backgrounds.
    #[serde(default)]
    pub ink: Vec<String>,
    pub version: String,
    /// `native`, `lightcraft` or `shim` (experimental).
    #[serde(default)]
    pub bridge: String,
    /// Extensions this app claims when a file is opened.
    #[serde(default)]
    pub default: Vec<String>,
}

impl WebApp {
    /// Opened and saved through CraftCloud's shim rather than the app's own bridge.
    pub fn is_experimental(&self) -> bool {
        self.bridge == "shim"
    }
}

#[derive(Deserialize)]
struct Doc {
    apps: Vec<WebApp>,
}

pub fn parse(bytes: &[u8]) -> Result<Vec<WebApp>, String> {
    serde_json::from_slice::<Doc>(bytes).map(|d| d.apps).map_err(|e| e.to_string())
}

/// `#3f5fe0` → (63, 95, 224).
pub fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let [_, r, g, b] = v.to_be_bytes();
    Some((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::{APPS, app, extension, for_name, parse, rgb};

    #[test]
    fn reads_what_fetch_apps_writes() {
        let json = br##"{"apps":[{"id":"wordcraft","name":"WordCraft","tagline":"Writing and document design","color":"#3f5fe0","ink":["#8ea2ff","#2a44b8"],"version":"0.18.1","bridge":"native","query":"","opens":["docx"],"default":["docx","md"],"entry":"/apps/wordcraft/index.html"},{"id":"photocraft","name":"PhotoCraft","version":"0.23.1","bridge":"shim"}]}"##;
        let apps = parse(json).unwrap();
        assert_eq!(apps[0].tagline, "Writing and document design");
        assert!(!apps[0].is_experimental());
        assert!(apps[1].is_experimental());
        assert_eq!(rgb(&apps[0].color), Some((0x3f, 0x5f, 0xe0)));
        assert_eq!(rgb("blue"), None);
    }

    #[test]
    fn files_open_in_their_app() {
        assert_eq!(for_name("Q3 report.docx").map(|a| a.name), Some("WordCraft"));
        assert_eq!(for_name("notes.MD").map(|a| a.id), Some("wordcraft"));
        assert_eq!(for_name("todo.txt").map(|a| a.id), Some("wordcraft"));
        assert_eq!(for_name("photo.JPG").map(|a| a.name), Some("PhotoCraft"));
        assert_eq!(for_name("archive.zip"), None);
        assert_eq!(for_name(".bashrc"), None);
        assert_eq!(extension("Makefile"), None);
        assert_eq!(extension("Documents/v1.2/notes"), None, "a dot in a folder name isn't an extension");
    }

    #[test]
    fn apps_open_their_extensions_in_any_case() {
        let word = app("WordCraft").unwrap();
        assert!(word.opens_name("Documents/Notes.MD") && word.opens_name("a.Txt") && word.opens_name("page.html"));
        assert!(!word.opens_name("sheet.xlsx") && !word.opens_name("README"));
        // `.md` and `.txt` are WordCraft's alone.
        let others: Vec<&str> =
            APPS.iter().filter(|a| a.id != "wordcraft" && (a.opens_name("a.md") || a.opens_name("a.txt"))).map(|a| a.id).collect();
        assert!(others.is_empty(), "{others:?}");
        // A file can be opened by several apps (PNG: FigCraft, LightCraft, PhotoCraft) but has one default.
        assert_eq!(APPS.iter().filter(|a| a.opens_name("x.png")).count(), 3);
        assert_eq!(app("nope"), None);
        for a in APPS {
            assert!(a.default.iter().all(|e| a.opens.contains(e)), "{}: every default is opened", a.id);
        }
    }

    /// The built-in table says what `apps.toml` says.
    #[test]
    fn the_table_matches_apps_toml() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut found: Vec<[String; 7]> = Vec::new();
        let value = |line: &str| line.split_once('=').map(|(_, v)| v.split(" #").next().unwrap_or("").trim().to_owned()).unwrap_or_default();
        for line in text.lines() {
            if let Some(id) = line.strip_prefix("[apps.").and_then(|l| l.strip_suffix(']')) {
                found.push([id.to_owned(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new()]);
            }
            let Some(last) = found.last_mut() else { continue };
            let slot = match line.split('=').next().unwrap_or("").trim() {
                "name" => 1,
                "color" => 2,
                "bridge" => 3,
                "default" => 4,
                "opens" => 5,
                "ink" => 6,
                _ => continue,
            };
            last[slot] = value(line);
        }
        let list = |items: &[&str]| format!("[{}]", items.iter().map(|e| format!("\"{e}\"")).collect::<Vec<_>>().join(", "));
        assert_eq!(found.len(), APPS.len());
        for (app, [id, name, color, bridge, default, opens, ink]) in APPS.iter().zip(&found) {
            assert_eq!(app.id, id);
            assert_eq!(format!("\"{}\"", app.name), *name);
            assert_eq!(format!("\"{}\"", app.color), *color);
            assert_eq!(list(&app.ink), *ink, "{id}");
            assert_eq!(app.experimental, bridge == "\"shim\"", "{id}");
            assert_eq!(list(app.default), *default, "{id}");
            assert_eq!(list(app.opens), *opens, "{id}");
        }
    }
}
