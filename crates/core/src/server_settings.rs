//! The server's own settings, changed from the admin pages: whether the Craft apps run in the
//! browser, and which of them. One file on the data volume (`settings.env`, `KEY=value` lines) is
//! the only source; `CRAFTCLOUD_WEB_APPS` in `.env` only seeds it on the first start.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettings {
    /// The Craft apps run in the browser.
    pub web_apps: bool,
    /// Bundled apps the admin switched off (app ids, e.g. `effectcraft`).
    #[serde(default)]
    pub disabled_apps: Vec<String>,
}

impl ServerSettings {
    /// Reads `settings.env`. Unknown keys and comments are ignored; a missing key keeps its default.
    pub fn from_env_file(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            match key.trim() {
                "CRAFTCLOUD_WEB_APPS" => s.web_apps = is_on(value.trim()),
                "CRAFTCLOUD_DISABLED_APPS" => s.disabled_apps = parse_ids(value),
                _ => {}
            }
        }
        s.normalise();
        s
    }

    pub fn to_env_file(&self) -> String {
        format!(
            "# CraftCloud server settings, written by Admin › Server. Edit only while the server is stopped.\n\
             CRAFTCLOUD_WEB_APPS={}\nCRAFTCLOUD_DISABLED_APPS={}\n",
            if self.web_apps { "on" } else { "off" },
            self.disabled_apps.join(",")
        )
    }

    /// Sorted, unique, valid ids only.
    pub fn normalise(&mut self) {
        self.disabled_apps.retain(|id| is_app_id(id));
        self.disabled_apps.sort();
        self.disabled_apps.dedup();
    }

    /// Whether `app` may be served, given the apps this image bundles.
    pub fn serves(&self, app: &str, bundled: &[String]) -> bool {
        self.web_apps && bundled.iter().any(|b| b == app) && !self.disabled_apps.iter().any(|d| d == app)
    }
}

/// `on`, `true`, `1`, `yes` (any case) mean on.
pub fn is_on(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "on" | "true" | "1" | "yes")
}

/// App ids are lowercase letters, digits and dashes (as in `apps.toml`).
pub fn is_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn parse_ids(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|id| is_app_id(id)).map(str::to_owned).collect()
}

/// The app id in an `/apps/<id>/…` path, if it is one.
pub fn app_of_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/apps/")?;
    let id = rest.split(['/', '?']).next()?;
    is_app_id(id).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::{ServerSettings, app_of_path};

    #[test]
    fn round_trips_through_the_env_file() {
        let s = ServerSettings { web_apps: true, disabled_apps: vec!["materialcraft".into(), "effectcraft".into()] };
        let text = s.to_env_file();
        assert!(text.contains("CRAFTCLOUD_WEB_APPS=on"));
        let back = ServerSettings::from_env_file(&text);
        assert_eq!(back.disabled_apps, ["effectcraft", "materialcraft"], "sorted on read");
        assert!(back.web_apps);
    }

    #[test]
    fn reading_is_forgiving() {
        let s = ServerSettings::from_env_file("# hi\nCRAFTCLOUD_WEB_APPS = YES\nCRAFTCLOUD_DISABLED_APPS= a , ../x, B ,a\nOTHER=1\n");
        assert!(s.web_apps);
        assert_eq!(s.disabled_apps, ["a"], "invalid ids dropped, duplicates removed");
        assert_eq!(ServerSettings::from_env_file(""), ServerSettings::default());
    }

    #[test]
    fn serves_only_bundled_enabled_apps_when_on() {
        let bundled = vec!["wordcraft".to_owned(), "effectcraft".to_owned()];
        let mut s = ServerSettings { web_apps: true, disabled_apps: vec!["effectcraft".into()] };
        assert!(s.serves("wordcraft", &bundled));
        assert!(!s.serves("effectcraft", &bundled), "switched off");
        assert!(!s.serves("photocraft", &bundled), "not in this image");
        s.web_apps = false;
        assert!(!s.serves("wordcraft", &bundled));
    }

    #[test]
    fn finds_the_app_in_a_path() {
        assert_eq!(app_of_path("/apps/wordcraft/index.html"), Some("wordcraft"));
        assert_eq!(app_of_path("/apps/wordcraft/"), Some("wordcraft"));
        assert_eq!(app_of_path("/apps/wordcraft?x=1"), Some("wordcraft"));
        assert_eq!(app_of_path("/apps/../etc"), None);
        assert_eq!(app_of_path("/craftcloud/apps.json"), None);
    }
}
