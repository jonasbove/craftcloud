//! The opt-in for other kinds of server until 0.3.0 (docs/WEBDAV-STORAGE.md § 16, "The opt-in
//! until 0.3.0"). The work lands on `main`, and a 0.2.x hotfix may be tagged from `main` at any
//! time, so every build keeps the new server kinds switched off unless the person running it
//! switches them on: the environment variable `CRAFTCLOUD_EXPERIMENTAL_WEBDAV`, read here and
//! nowhere else, and only its exact value `1` counts. Nothing in a released build, its packaging
//! or its settings sets it. The command line, the daemon and the desktop client all ask here.

use craftcloud_core::sync::{ClientSettings, Storage};

/// The environment variable that switches the new server kinds on.
pub const VARIABLE: &str = "CRAFTCLOUD_EXPERIMENTAL_WEBDAV";

/// What a build without the opt-in says to a settings file that names another kind of server.
/// The server is never treated as OxiCloud instead, and nothing syncs.
pub const REFUSED: &str = "This build syncs only with a CraftCloud server; set CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 to try other servers";

/// What a kind of server that has no driver yet says, with the opt-in on (Nextcloud, ownCloud and
/// OpenCloud get theirs in later pull requests; such a server is never taken for a plain WebDAV
/// server, whose client trash and versions don't belong in its drive).
pub const NOT_YET: &str = "This build can't sync with this kind of server yet (it knows CraftCloud's own server and other WebDAV servers)";

/// Whether the person running this process switched the other server kinds on.
pub fn experimental_webdav() -> bool {
    enabled_by(std::env::var_os(VARIABLE).as_deref())
}

/// [`experimental_webdav`] for a given value of the variable: only the exact text `1`.
pub fn enabled_by(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// Whether these settings may be synced with, by this build: CraftCloud's own server always;
/// another kind of server only with the opt-in, and only one that has a driver. `Err` carries
/// the message to show ([`REFUSED`], [`NOT_YET`]).
pub fn allowed(settings: &ClientSettings, experimental: bool) -> Result<(), &'static str> {
    match settings.storage {
        Storage::Oxicloud => Ok(()),
        _ if !experimental => Err(REFUSED),
        Storage::Webdav => Ok(()),
        Storage::Nextcloud | Storage::Owncloud | Storage::Opencloud => Err(NOT_YET),
    }
}

/// [`allowed`] with the process's own environment.
pub fn allowed_here(settings: &ClientSettings) -> Result<(), &'static str> {
    allowed(settings, experimental_webdav())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use craftcloud_core::sync::{ClientSettings, Storage};

    use super::{NOT_YET, REFUSED, allowed, enabled_by};

    /// Only the exact value `1` switches the new kinds on: not `true`, `yes`, `on`, ` 1` or `01`.
    #[test]
    fn only_the_exact_value_1_counts() {
        assert!(enabled_by(Some(OsStr::new("1"))));
        for off in ["0", "true", "yes", "on", "", " 1", "01", "1 ", "TRUE"] {
            assert!(!enabled_by(Some(OsStr::new(off))), "{off:?}");
        }
        assert!(!enabled_by(None));
    }

    /// Without the opt-in a settings file naming another kind is refused with the one message,
    /// never read as OxiCloud; with it, only a kind with a driver goes through.
    #[test]
    fn another_kind_of_server_is_refused_without_the_opt_in() {
        let with = |storage| ClientSettings { storage, server: "https://cloud.example.org".into(), ..ClientSettings::default() };
        for experimental in [false, true] {
            assert_eq!(allowed(&with(Storage::Oxicloud), experimental), Ok(()));
        }
        for other in [Storage::Webdav, Storage::Nextcloud, Storage::Owncloud, Storage::Opencloud] {
            assert_eq!(allowed(&with(other), false), Err(REFUSED), "{other:?}");
        }
        assert_eq!(allowed(&with(Storage::Webdav), true), Ok(()));
        for later in [Storage::Nextcloud, Storage::Owncloud, Storage::Opencloud] {
            assert_eq!(allowed(&with(later), true), Err(NOT_YET), "{later:?}");
        }
        assert_eq!(REFUSED, "This build syncs only with a CraftCloud server; set CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 to try other servers");
    }
}
