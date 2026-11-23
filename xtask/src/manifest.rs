//! `apps.toml`: the Craft web builds the `-apps` image bundles, in the order the file lists them.

use serde::Deserialize;

/// One `[apps.<id>]` table. Unknown keys are ignored (they are notes for people).
#[derive(Clone, Debug, Deserialize)]
pub struct AppSpec {
    pub name: String,
    #[serde(default)]
    pub tagline: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub ink: Vec<String>,
    pub repo: String,
    /// Whether the app has a web build in the `-apps` image (the default). `web = false`: a
    /// desktop app CraftCloud lists for `--app` and `apps`, with no pins and no bridge.
    #[serde(default = "yes")]
    pub web: bool,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub asset: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub bridge: String,
    #[serde(default)]
    pub query: String,
    pub opens: Vec<String>,
    #[serde(rename = "default")]
    pub defaults: Vec<String>,
    #[serde(default)]
    pub fixups: Vec<Fixup>,
}

/// A repair for a packaging bug in a pinned release (each one is also in docs/UPSTREAM.md).
/// The only kind: `{ copy = "<dir or file>", to = "<path>" }`, both relative to the app's root.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixup {
    pub copy: String,
    pub to: String,
}

const fn yes() -> bool {
    true
}

/// The apps in `apps.toml`, in file order. An id becomes a folder name, so it must be plain.
/// A web build has all of its pins and a bridge; a desktop app (`web = false`) has none of them,
/// so it can't be half pinned and fetched by mistake.
pub fn parse(text: &str) -> Result<Vec<(String, AppSpec)>, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| format!("apps.toml: {e}"))?;
    let apps = doc.get("apps").and_then(toml::Value::as_table).ok_or("apps.toml: no [apps] table")?;
    let mut out = Vec::with_capacity(apps.len());
    for (id, value) in apps {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
            return Err(format!("apps.toml: app id {id:?} must be lower-case letters, digits, - and _"));
        }
        let spec: AppSpec = value.clone().try_into().map_err(|e| format!("apps.toml [apps.{id}]: {e}"))?;
        let pins = [("version", &spec.version), ("asset", &spec.asset), ("bridge", &spec.bridge)];
        if spec.web {
            if let Some((key, _)) = pins.iter().find(|(_, v)| v.is_empty()) {
                return Err(format!("apps.toml [apps.{id}]: a web build needs {key}"));
            }
            if !is_sha256_hex(&spec.sha256) {
                return Err(format!("apps.toml [apps.{id}]: sha256 must be 64 lower-case hex digits"));
            }
        } else if let Some((key, _)) = pins.iter().chain(&[("sha256", &spec.sha256)]).find(|(_, v)| !v.is_empty()) {
            return Err(format!("apps.toml [apps.{id}]: {key} is for web builds; this app has web = false"));
        }
        out.push((id.clone(), spec));
    }
    Ok(out)
}

pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_file_order_and_defaults() {
        let text = r##"
[apps.zeta]
name = "Zeta"
repo = "o/zeta"
version = "1.0.0"
asset = "zeta-web-1.0.0.zip"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
bridge = "native"
opens = ["z"]
default = ["z"]

[apps.alpha]
name = "Alpha"
repo = "o/alpha"
version = "2.0.0"
asset = "alpha-web-2.0.0.zip"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
bridge = "shim"
query = "nosw"
opens = []
default = []
fixups = [{ copy = "a", to = "b" }]
"##;
        let apps = parse(text).unwrap();
        let ids: Vec<&str> = apps.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["zeta", "alpha"]);
        assert_eq!(apps[0].1.tagline, "");
        assert_eq!(apps[1].1.query, "nosw");
        assert_eq!(apps[1].1.fixups[0].to, "b");
    }

    #[test]
    fn refuses_unknown_fixups_and_bad_ids() {
        let base = |extra: &str, id: &str| {
            format!(
                "[apps.{id}]\nname = \"A\"\nrepo = \"o/a\"\nversion = \"1\"\nasset = \"a.zip\"\nsha256 = \"{}\"\nbridge = \"native\"\nopens = []\ndefault = []\n{extra}",
                "0".repeat(64)
            )
        };
        assert!(parse(&base("", "a")).is_ok());
        assert!(parse(&base("fixups = [{ move = \"a\", to = \"b\" }]", "a")).is_err());
        assert!(parse(&base("", "\"../a\"")).is_err());
        assert!(parse(&base("", "A")).is_err());
    }

    #[test]
    fn a_desktop_app_has_no_pins_and_a_web_build_has_all_of_them() {
        let desktop =
            "[apps.prosecraft-rs]\nname = \"ProseCraft-rs\"\nrepo = \"o/p\"\nweb = false\nopens = [\"prosecraft\"]\ndefault = [\"prosecraft\"]\n";
        let apps = parse(desktop).unwrap();
        assert!(!apps[0].1.web && apps[0].1.version.is_empty() && apps[0].1.bridge.is_empty());
        let err = parse(&format!("{desktop}bridge = \"native\"\n")).unwrap_err();
        assert!(err.contains("bridge is for web builds"), "{err}");
        let err = parse(&format!("{desktop}sha256 = \"{}\"\n", "0".repeat(64))).unwrap_err();
        assert!(err.contains("sha256 is for web builds"), "{err}");
        let err = parse(&desktop.replace("web = false\n", "")).unwrap_err();
        assert!(err.contains("a web build needs version"), "{err}");
    }

    /// The real manifest parses, so a typo in it fails the gate, not the image build.
    #[test]
    fn repository_manifest_parses() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../apps.toml")).unwrap();
        assert!(!parse(&text).unwrap().is_empty());
    }
}
