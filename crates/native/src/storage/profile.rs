//! Server profiles (docs/WEBDAV-STORAGE.md § 7.4): what a known kind of server adds to the generic
//! WebDAV driver. **A profile knows where things are, never what they guarantee.** It supplies
//! addresses and the server's own features (its trash, its versions, its quota endpoint, how its
//! version string is read, the known-server checks of the probe's steps 16 and 17); only the check
//! on connect grants a capability, and the driver asks a profile for the server's trash or
//! versions only where that check confirmed them (`Trash::Server`, `Versions::Server`).
//!
//! [`ServerProfile`] is the hook the generic driver calls; [`Generic`] is the profile of a server
//! nobody knows (every method its default), [`nextcloud::Nextcloud`] the first known one,
//! [`opencloud::Opencloud`] the second (download only, § 16 Q1). The ownCloud profile plugs in
//! the same way.
//!
//! One thing a profile may say about guarantees: a **ceiling** on the mode
//! ([`ServerProfile::lowered`]), from measurements the check on connect can't make (a guard that
//! holds in one request but not under a race). It can only lower the mode, never raise it.

pub mod nextcloud;
pub mod opencloud;

use craftcloud_core::api::versions::Version;
use craftcloud_core::http::HttpRequest;
use craftcloud_core::storage::probe::{Base, ExtraCheck};
use craftcloud_core::storage::{Checksum, Lowered};
use craftcloud_core::sync::Storage;

use super::probe::Transport;
use super::{StorageError, TrashItem};

/// What a known server's profile adds to the generic driver. Every method has a default that
/// adds nothing, so a profile implements only what its server has.
pub trait ServerProfile: Send {
    /// The kind of server (what the settings and `--kind` call it).
    fn kind(&self) -> Storage;

    /// The server's version string now, for the check's re-run schedule (§ 7.5, "When"), where
    /// the server has a richer answer than its `Server` header (`status.php`). `None`: the driver
    /// falls back to the header.
    fn version(&self, _transport: &mut dyn Transport) -> Option<String> {
        None
    }

    /// The known-server checks of the probe's steps 16 (the trash round trip) and 17 (the server's
    /// versions). `None`: the probe records the client's trash and versions.
    fn probe_checks(&self) -> Option<(Box<dyn ExtraCheck>, Box<dyn ExtraCheck>)> {
        None
    }

    /// A ceiling on the mode from what is known about this kind of server and the check can't
    /// measure (docs/WEBDAV-STORAGE.md § 7.5, "Atomicity"; § 16, Q1: OpenCloud's guards don't
    /// hold under a race). Recorded in the check's capabilities, so `mode_for` applies it
    /// wherever the mode is read; it can only lower the mode measured, never raise it. `None`:
    /// the measurements decide alone.
    fn lowered(&self) -> Option<Lowered> {
        None
    }

    /// Verify every write by checksum (§ 16, Q13): the hash the driver sends in `OC-Checksum` on
    /// each `PUT` and compares with the server's `oc:checksums` of the result; a mismatch is a
    /// conflict, never a silent overwrite. `None`: no verification (the server has no checksums,
    /// or its version tags tell every write apart).
    fn verify_writes(&self) -> Option<Checksum> {
        None
    }

    /// The server's own trash, as items with their original cloud path (only asked where the
    /// check on connect confirmed `Trash::Server`).
    fn trash_list(&self, _transport: &mut dyn Transport) -> Result<Vec<TrashItem>, StorageError> {
        Err(StorageError::Failed("this kind of server has no trash of its own that CraftCloud knows".into()))
    }

    /// Puts an item of the server's own trash back where it was.
    fn restore(&self, _transport: &mut dyn Transport, item: &TrashItem) -> Result<(), StorageError> {
        Err(StorageError::Failed(format!("restoring {} from this server's trash isn't known", item.path)))
    }

    /// The server's own earlier versions of `path` (only asked where the check on connect
    /// confirmed `Versions::Server`), newest first.
    fn list_versions(&self, _transport: &mut dyn Transport, _base: &Base, path: &str) -> Result<Vec<Version>, StorageError> {
        Err(StorageError::Failed(format!("the server's own versions of {path} aren't known for this kind of server")))
    }

    /// The request that reads the bytes of the server's own version `stamp` of `path` (a `GET`).
    /// A profile never restores a version itself: the driver writes these bytes back with its
    /// guarded `replace` (`If-Match` on the file's version tag, and the checksum check where the
    /// profile asks for it), so a restore is a guarded write on every kind of server. A server's
    /// own "restore" call names no version of the file it replaces, and is never used.
    fn version_download(&self, _transport: &mut dyn Transport, _base: &Base, path: &str, _stamp: &str) -> Result<HttpRequest, StorageError> {
        Err(StorageError::Failed(format!("reading a version of {path} isn't known for this kind of server")))
    }

    /// The free space, where the profile knows an endpoint for it (Nextcloud's OCS user endpoint;
    /// WebDAV's `quota-available-bytes` says `-3` there). `None`: the driver reads the WebDAV
    /// quota properties.
    fn free_space(&self, _transport: &mut dyn Transport) -> Option<Result<Option<u64>, StorageError>> {
        None
    }
}

/// The profile of a server nobody knows: nothing beyond WebDAV.
pub struct Generic;

impl ServerProfile for Generic {
    fn kind(&self) -> Storage {
        Storage::Webdav
    }
}

/// The profile for a kind of server on `base` (the WebDAV base the settings hold): a known server
/// gets its profile, any other the generic one. `None` when the base isn't one that kind of server
/// has (a Nextcloud base must be `…/remote.php/dav/files/<user id>/`, an OpenCloud base
/// `…/remote.php/dav/spaces/<space id>/`).
pub fn for_kind(kind: Storage, base: &Base) -> Option<Box<dyn ServerProfile>> {
    match kind {
        Storage::Nextcloud => nextcloud::Nextcloud::from_base(base).map(|p| Box::new(p) as Box<dyn ServerProfile>),
        Storage::Opencloud => opencloud::Opencloud::from_base(base).map(|p| Box::new(p) as Box<dyn ServerProfile>),
        Storage::Oxicloud | Storage::Owncloud | Storage::Webdav => Some(Box::new(Generic)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_kind_gets_its_profile_and_the_rest_the_generic_one() {
        let nc = Base::parse("https://cloud.example.org/remote.php/dav/files/john/").unwrap();
        assert_eq!(for_kind(Storage::Nextcloud, &nc).map(|p| p.kind()), Some(Storage::Nextcloud));
        assert_eq!(for_kind(Storage::Webdav, &nc).map(|p| p.kind()), Some(Storage::Webdav));
        assert!(for_kind(Storage::Nextcloud, &Base::parse("https://nas.example.org/dav/").unwrap()).is_none(), "not a Nextcloud base");
        let opc =
            Base::parse("https://cloud.example.org/remote.php/dav/spaces/5d90215b-14f0-4b05-9f8d-8971b9810172$4ed81edc-3290-4003-a070-9f24bb826ff8/")
                .unwrap();
        assert_eq!(for_kind(Storage::Opencloud, &opc).map(|p| p.kind()), Some(Storage::Opencloud));
        assert!(for_kind(Storage::Opencloud, &nc).is_none(), "a files base isn't an OpenCloud space");
        let generic = Generic;
        assert!(generic.probe_checks().is_none() && generic.verify_writes().is_none() && generic.lowered().is_none());
    }
}
