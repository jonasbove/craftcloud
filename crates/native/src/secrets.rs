//! The client's sign-in: in the OS keychain (macOS Keychain, Windows Credential Manager, the
//! Secret Service on Linux). Where there is no keychain (a Linux box without a Secret Service),
//! a file readable only by the user, next to the settings.
//!
//! What is stored is a [`Credential`]: OxiCloud's tokens, or the user name and app password of
//! another WebDAV server (docs/WEBDAV-STORAGE.md § 10). A `kind` field tells them apart and
//! defaults to `bearer` when absent, so every entry written before it reads unchanged, and
//! OxiCloud's entries keep exactly the shape 0.2.0 wrote (an older client still reads them).

use std::path::{Path, PathBuf};

use craftcloud_core::api::auth::Tokens;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "CraftCloud";

/// How the client signs in to a server (docs/WEBDAV-STORAGE.md § 10). Treat either like the
/// account's password.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// OxiCloud's access and refresh tokens (the device flow).
    Bearer(Tokens),
    /// An app or WebDAV password the person made on the server, never the account password.
    AppPassword { user: String, secret: String },
}

impl std::fmt::Debug for Credential {
    // Never print a secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bearer(_) => f.write_str("Credential::Bearer { .. }"),
            Self::AppPassword { .. } => f.write_str("Credential::AppPassword { .. }"),
        }
    }
}

/// The entry as written: 0.2.0's two fields for tokens (no `kind`), and `kind: app_password`
/// with the user name and secret for an app password.
#[derive(Serialize, Deserialize)]
struct Stored {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    access_token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    refresh_token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    user: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    secret: String,
}

const APP_PASSWORD: &str = "app_password";

impl Credential {
    fn to_json(&self) -> Result<String, String> {
        let stored = match self {
            Self::Bearer(t) => Stored {
                kind: None,
                access_token: t.access_token.clone(),
                refresh_token: t.refresh_token.clone(),
                user: String::new(),
                secret: String::new(),
            },
            Self::AppPassword { user, secret } => Stored {
                kind: Some(APP_PASSWORD.into()),
                access_token: String::new(),
                refresh_token: String::new(),
                user: user.clone(),
                secret: secret.clone(),
            },
        };
        serde_json::to_string(&stored).map_err(|e| e.to_string())
    }

    fn from_json(json: &str) -> Option<Self> {
        let s: Stored = serde_json::from_str(json).ok()?;
        match s.kind.as_deref() {
            None | Some("bearer") => Some(Self::Bearer(Tokens { access_token: s.access_token, refresh_token: s.refresh_token })),
            Some(APP_PASSWORD) => Some(Self::AppPassword { user: s.user, secret: s.secret }),
            // A kind this build doesn't know (a later one): not a sign-in it can use.
            Some(_) => None,
        }
    }
}

/// Where the sign-in for one server lives.
pub struct TokenStore {
    server: String,
    fallback: Option<PathBuf>,
    /// `false` only in tests: the private file, never the keychain of the computer running them
    /// (`CRAFTCLOUD_KEYCHAIN` can't be set from a test: the environment is process-wide).
    keychain: bool,
}

impl TokenStore {
    /// `fallback_dir` is the settings folder, used only when the keychain is missing.
    pub fn new(server: &str, fallback_dir: Option<&Path>) -> Self {
        Self { server: server.to_owned(), fallback: fallback_dir.map(|d| d.join("tokens.json")), keychain: true }
    }

    /// A store in `dir/tokens.json` only, for tests.
    #[cfg(test)]
    pub(crate) fn file_only(server: &str, dir: &Path) -> Self {
        Self { keychain: false, ..Self::new(server, Some(dir)) }
    }

    /// The keychain entry, unless `CRAFTCLOUD_KEYCHAIN=off` asks for the private file instead (for
    /// servers without a desktop session, and tests that must not touch the real keychain).
    fn entry(&self) -> keyring::Result<keyring::Entry> {
        if !self.keychain || std::env::var("CRAFTCLOUD_KEYCHAIN").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
            return Err(keyring::Error::NoStorageAccess("switched off by CRAFTCLOUD_KEYCHAIN=off".into()));
        }
        keyring::Entry::new(SERVICE, &self.server)
    }

    /// Saves OxiCloud's tokens.
    pub fn save(&self, tokens: &Tokens) -> Result<(), String> {
        self.save_credential(&Credential::Bearer(tokens.clone()))
    }

    /// Saves any sign-in.
    pub fn save_credential(&self, credential: &Credential) -> Result<(), String> {
        let json = credential.to_json()?;
        match self.entry().and_then(|e| e.set_password(&json)) {
            Ok(()) => {
                // A keychain worked: don't leave an older fallback file behind.
                if let Some(path) = &self.fallback {
                    let _ = std::fs::remove_file(path);
                }
                Ok(())
            }
            Err(e) => {
                log::warn!("no keychain ({e}); keeping the sign-in in a private file");
                let path = self.fallback.as_ref().ok_or_else(|| format!("no keychain and no settings folder: {e}"))?;
                write_private(path, json.as_bytes())
            }
        }
    }

    /// OxiCloud's tokens, when the stored sign-in is one (an app password isn't).
    pub fn load(&self) -> Option<Tokens> {
        match self.load_credential()? {
            Credential::Bearer(t) => Some(t),
            Credential::AppPassword { .. } => None,
        }
    }

    /// The stored sign-in, whichever kind.
    pub fn load_credential(&self) -> Option<Credential> {
        let json =
            self.entry().and_then(|e| e.get_password()).ok().or_else(|| self.fallback.as_ref().and_then(|p| std::fs::read_to_string(p).ok()))?;
        Credential::from_json(&json)
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
    use craftcloud_core::api::auth::Tokens;

    use super::{Credential, TokenStore, write_private};

    #[cfg(unix)]
    #[test]
    fn the_fallback_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        write_private(&path, b"{}").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    /// OxiCloud's entry is written exactly as 0.2.0 wrote it (an older client still reads it),
    /// and an entry from before `kind` existed reads as tokens.
    #[test]
    fn oxicloud_tokens_keep_their_0_2_0_shape() {
        let tokens = Tokens { access_token: "a1".into(), refresh_token: "r1".into() };
        let json = Credential::Bearer(tokens.clone()).to_json().unwrap();
        assert_eq!(json, r#"{"access_token":"a1","refresh_token":"r1"}"#, "what 0.2.0 wrote, byte for byte");
        assert_eq!(Credential::from_json(r#"{"access_token":"a1","refresh_token":"r1"}"#), Some(Credential::Bearer(tokens.clone())));
        assert_eq!(Credential::from_json(r#"{"kind":"bearer","access_token":"a1","refresh_token":"r1"}"#), Some(Credential::Bearer(tokens)));
    }

    /// An app password is stored with its kind and user name; a kind this build doesn't know
    /// isn't a sign-in; no secret reaches `Debug`.
    #[test]
    fn app_passwords_are_stored_with_their_kind() {
        let app = Credential::AppPassword { user: "john".into(), secret: "SECRET-pw".into() };
        let json = app.to_json().unwrap();
        assert_eq!(json, r#"{"kind":"app_password","user":"john","secret":"SECRET-pw"}"#);
        assert_eq!(Credential::from_json(&json), Some(app.clone()));
        assert_eq!(Credential::from_json(r#"{"kind":"oidc","token":"x"}"#), None);
        assert_eq!(Credential::from_json("not json"), None);
        let shown = format!("{app:?} {:?}", Credential::Bearer(Tokens { access_token: "SECRET-a".into(), refresh_token: "SECRET-r".into() }));
        assert!(!shown.contains("SECRET"), "{shown}");
        assert_eq!(shown, "Credential::AppPassword { .. } Credential::Bearer { .. }");
    }

    /// The store keeps either kind in its file; `load` (tokens) answers nothing for an app
    /// password, so no OxiCloud code path ever takes one for tokens.
    #[test]
    fn the_store_keeps_either_kind() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::file_only("https://cloud.example.org", dir.path());
        let tokens = Tokens { access_token: "a1".into(), refresh_token: "r1".into() };
        store.save(&tokens).unwrap();
        assert_eq!(store.load(), Some(tokens.clone()));
        assert_eq!(store.load_credential(), Some(Credential::Bearer(tokens)));
        let written = std::fs::read_to_string(dir.path().join("tokens.json")).unwrap();
        assert_eq!(written, r#"{"access_token":"a1","refresh_token":"r1"}"#);

        let app = Credential::AppPassword { user: "john".into(), secret: "pw".into() };
        store.save_credential(&app).unwrap();
        assert_eq!(store.load(), None, "an app password is never tokens");
        assert_eq!(store.load_credential(), Some(app));
        store.delete();
        assert_eq!(store.load_credential(), None);
    }
}
