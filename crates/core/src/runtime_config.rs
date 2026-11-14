//! `/craftcloud/config.json`: what a CraftCloud server says about itself. The container's
//! entrypoint writes it (docker/entrypoint.sh); the web frontend and the desktop client read it.

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RuntimeConfig {
    /// The CraftCloud image version, e.g. `0.1.0`.
    pub version: String,
    /// Whether this server hosts the Craft web apps (`CRAFTCLOUD_WEB_APPS=on` on an `-apps` image).
    pub web_apps: bool,
    /// The address people use to reach the server.
    pub public_url: String,
}

#[derive(Debug)]
pub struct RuntimeConfigError(serde_json::Error);

impl std::fmt::Display for RuntimeConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "this server's /craftcloud/config.json is not a CraftCloud config: {}", self.0)
    }
}

impl std::error::Error for RuntimeConfigError {}

impl RuntimeConfig {
    pub fn from_json(bytes: &[u8]) -> Result<Self, RuntimeConfigError> {
        serde_json::from_slice(bytes).map_err(RuntimeConfigError)
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeConfig;

    #[test]
    fn reads_what_the_entrypoint_writes() {
        let json = br#"{"version": "0.1.0-dev", "web_apps": false, "public_url": "http://localhost:8086"}"#;
        let config = RuntimeConfig::from_json(json).unwrap();
        assert_eq!(config.version, "0.1.0-dev");
        assert!(!config.web_apps);
        assert_eq!(config.public_url, "http://localhost:8086");
    }

    #[test]
    fn rejects_other_json() {
        assert!(RuntimeConfig::from_json(br#"{"initialized": true}"#).is_err());
        assert!(RuntimeConfig::from_json(b"<html>").is_err());
    }
}
