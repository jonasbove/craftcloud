//! The client's tokens: in the OS keychain (macOS Keychain, Windows Credential Manager, the
//! Secret Service on Linux). Where there is no keychain (a Linux box without a Secret Service),
//! a file readable only by the user, next to the settings.

use std::path::{Path, PathBuf};

use craftcloud_core::api::auth::Tokens;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "CraftCloud";

#[derive(Serialize, Deserialize)]
struct Stored {
    access_token: String,
    refresh_token: String,
}

/// Where the tokens for one server live.
pub struct TokenStore {
    server: String,
    fallback: Option<PathBuf>,
}

impl TokenStore {
    /// `fallback_dir` is the settings folder, used only when the keychain is missing.
    pub fn new(server: &str, fallback_dir: Option<&Path>) -> Self {
        Self { server: server.to_owned(), fallback: fallback_dir.map(|d| d.join("tokens.json")) }
    }

    /// The keychain entry, unless `CRAFTCLOUD_KEYCHAIN=off` asks for the private file instead (for
    /// servers without a desktop session, and tests that must not touch the real keychain).
    fn entry(&self) -> keyring::Result<keyring::Entry> {
        if std::env::var("CRAFTCLOUD_KEYCHAIN").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
            return Err(keyring::Error::NoStorageAccess("switched off by CRAFTCLOUD_KEYCHAIN=off".into()));
        }
        keyring::Entry::new(SERVICE, &self.server)
    }

    pub fn save(&self, tokens: &Tokens) -> Result<(), String> {
        let json = serde_json::to_string(&Stored { access_token: tokens.access_token.clone(), refresh_token: tokens.refresh_token.clone() })
            .map_err(|e| e.to_string())?;
        match self.entry().and_then(|e| e.set_password(&json)) {
            Ok(()) => {
                // A keychain worked: don't leave an older fallback file behind.
                if let Some(path) = &self.fallback {
                    let _ = std::fs::remove_file(path);
                }
                Ok(())
            }
            Err(e) => {
                log::warn!("no keychain ({e}); keeping the tokens in a private file");
                let path = self.fallback.as_ref().ok_or_else(|| format!("no keychain and no settings folder: {e}"))?;
                write_private(path, json.as_bytes())
            }
        }
    }

    pub fn load(&self) -> Option<Tokens> {
        let json =
            self.entry().and_then(|e| e.get_password()).ok().or_else(|| self.fallback.as_ref().and_then(|p| std::fs::read_to_string(p).ok()))?;
        let s: Stored = serde_json::from_str(&json).ok()?;
        Some(Tokens { access_token: s.access_token, refresh_token: s.refresh_token })
    }

    pub fn delete(&self) {
        if let Ok(entry) = self.entry() {
            let _ = entry.delete_credential();
        }
        if let Some(path) = &self.fallback {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Writes a file only the user can read (0600 on Unix), atomically.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::write_private;

    #[cfg(unix)]
    #[test]
    fn the_fallback_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        write_private(&path, b"{}").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
