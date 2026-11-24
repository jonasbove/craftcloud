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
    /// The name the app's releases give their web build, `<release_name>-web-<x.y.z>.zip`, when it
    /// differs from the id (which is stored, so it can't follow a rename). Empty: the id.
    #[serde(default)]
    pub release_name: String,
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
/// The web build's asset is `<release name>-web-<version>.zip`, so a pin can't name another app's
/// build or a version other than its own.
pub fn parse(text: &str) -> Result<Vec<(String, AppSpec)>, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| format!("apps.toml: {e}"))?;
    let apps = doc.get("apps").and_then(toml::Value::as_table).ok_or("apps.toml: no [apps] table")?;
    let mut out = Vec::with_capacity(apps.len());
    for (id, value) in apps {
        if !is_plain_name(id) {
            return Err(format!("apps.toml: app id {id:?} must be lower-case letters, digits, - and _"));
        }
        let mut spec: AppSpec = value.clone().try_into().map_err(|e| format!("apps.toml [apps.{id}]: {e}"))?;
        if spec.release_name.is_empty() {
            spec.release_name.clone_from(id);
        } else if !is_plain_name(&spec.release_name) {
            return Err(format!("apps.toml [apps.{id}]: release_name must be lower-case letters, digits, - and _"));
        }
        let pins = [("version", &spec.version), ("asset", &spec.asset), ("bridge", &spec.bridge)];
        if spec.web {
            if let Some((key, _)) = pins.iter().find(|(_, v)| v.is_empty()) {
                return Err(format!("apps.toml [apps.{id}]: a web build needs {key}"));
            }
            if !is_sha256_hex(&spec.sha256) {
                return Err(format!("apps.toml [apps.{id}]: sha256 must be 64 lower-case hex digits"));
            }
            let expected = web_asset_name(&spec.release_name, &spec.version);
            if spec.asset != expected {
                return Err(format!("apps.toml [apps.{id}]: asset must be {expected:?} (`<release_name>-web-<version>.zip`)"));
            }
        } else if let Some((key, _)) = pins.iter().chain(&[("sha256", &spec.sha256)]).find(|(_, v)| !v.is_empty()) {
            return Err(format!("apps.toml [apps.{id}]: {key} is for web builds; this app has web = false"));
        }
        out.push((id.clone(), spec));
    }
    Ok(out)
}

/// The file name of a web build in a release: `<release_name>-web-<version>.zip`.
pub fn web_asset_name(release_name: &str, version: &str) -> String {
    format!("{release_name}-web-{version}.zip")
}

/// Lower-case letters, digits, `-` and `_`: safe as a folder name and in a file name.
fn is_plain_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
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
                "[apps.{id}]\nname = \"A\"\nrepo = \"o/a\"\nversion = \"1.0.0\"\nasset = \"{id}-web-1.0.0.zip\"\nsha256 = \"{}\"\nbridge = \"native\"\nopens = []\ndefault = []\n{extra}",
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
            "[apps.prosecraft-rs]\nname = \"LoveCraft\"\nrepo = \"o/p\"\nweb = false\nopens = [\"prosecraft\"]\ndefault = [\"prosecraft\"]\n";
        let apps = parse(desktop).unwrap();
        assert!(!apps[0].1.web && apps[0].1.version.is_empty() && apps[0].1.bridge.is_empty());
        let err = parse(&format!("{desktop}bridge = \"native\"\n")).unwrap_err();
        assert!(err.contains("bridge is for web builds"), "{err}");
        let err = parse(&format!("{desktop}sha256 = \"{}\"\n", "0".repeat(64))).unwrap_err();
        assert!(err.contains("sha256 is for web builds"), "{err}");
        let err = parse(&desktop.replace("web = false\n", "")).unwrap_err();
        assert!(err.contains("a web build needs version"), "{err}");
    }

    #[test]
    fn the_release_name_defaults_to_the_id_and_names_the_asset() {
        let web = |extra: &str, asset: &str| {
            format!(
                "[apps.prosecraft-rs]\nname = \"LoveCraft\"\nrepo = \"o/p\"\n{extra}version = \"0.2.0\"\nasset = \"{asset}\"\nsha256 = \"{}\"\nbridge = \"native\"\nopens = []\ndefault = []\n",
                "0".repeat(64)
            )
        };
        let apps = parse(&web("", "prosecraft-rs-web-0.2.0.zip")).unwrap();
        assert_eq!(apps[0].1.release_name, "prosecraft-rs");
        let apps = parse(&web("release_name = \"lovecraft\"\n", "lovecraft-web-0.2.0.zip")).unwrap();
        assert_eq!((apps[0].0.as_str(), apps[0].1.release_name.as_str()), ("prosecraft-rs", "lovecraft"));
        // The asset follows the release name and the version.
        let err = parse(&web("release_name = \"lovecraft\"\n", "prosecraft-rs-web-0.2.0.zip")).unwrap_err();
        assert!(err.contains("asset must be \"lovecraft-web-0.2.0.zip\""), "{err}");
        let err = parse(&web("", "lovecraft-web-0.2.0.zip")).unwrap_err();
        assert!(err.contains("asset must be \"prosecraft-rs-web-0.2.0.zip\""), "{err}");
        let err = parse(&web("", "prosecraft-rs-web-0.1.0.zip")).unwrap_err();
        assert!(err.contains("asset must be"), "{err}");
        // It ends up in a file name.
        for bad in ["../x", "Love", "a b", "x/y"] {
            let err = parse(&web(&format!("release_name = \"{bad}\"\n"), &format!("{bad}-web-0.2.0.zip"))).unwrap_err();
            assert!(err.contains("release_name must be"), "{bad}: {err}");
        }
        // A desktop app may name it ahead of its first web build.
        let desktop =
            "[apps.prosecraft-rs]\nname = \"LoveCraft\"\nrepo = \"o/p\"\nweb = false\nrelease_name = \"lovecraft\"\nopens = []\ndefault = []\n";
        assert_eq!(parse(desktop).unwrap()[0].1.release_name, "lovecraft");
    }

    /// The real manifest parses, so a typo in it fails the gate, not the image build.
    #[test]
    fn repository_manifest_parses() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../apps.toml")).unwrap();
        assert!(!parse(&text).unwrap().is_empty());
    }
}
