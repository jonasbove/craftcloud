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
    pub version: String,
    pub asset: String,
    pub sha256: String,
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

/// The apps in `apps.toml`, in file order. An id becomes a folder name, so it must be plain.
pub fn parse(text: &str) -> Result<Vec<(String, AppSpec)>, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| format!("apps.toml: {e}"))?;
    let apps = doc.get("apps").and_then(toml::Value::as_table).ok_or("apps.toml: no [apps] table")?;
    let mut out = Vec::with_capacity(apps.len());
    for (id, value) in apps {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
            return Err(format!("apps.toml: app id {id:?} must be lower-case letters, digits, - and _"));
        }
        let spec: AppSpec = value.clone().try_into().map_err(|e| format!("apps.toml [apps.{id}]: {e}"))?;
        if !is_sha256_hex(&spec.sha256) {
            return Err(format!("apps.toml [apps.{id}]: sha256 must be 64 lower-case hex digits"));
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

    /// The real manifest parses, so a typo in it fails the gate, not the image build.
    #[test]
    fn repository_manifest_parses() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../apps.toml")).unwrap();
        assert!(!parse(&text).unwrap().is_empty());
    }
}
