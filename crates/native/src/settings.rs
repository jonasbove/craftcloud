//! The client's `config.toml`, through craft-settings like every Craft app (platform folder,
//! atomic saves, backups, hand edits kept).

use craftcloud_core::sync::ClientSettings;
use serde::{Deserialize, Serialize};

/// craft-settings' trait is implemented on this wrapper (the settings type lives in core).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Prefs(pub ClientSettings);

impl craft_settings::Settings for Prefs {
    const APP_ID: &'static str = "craftcloud";
    const SCHEMA_VERSION: u32 = 1;
}

pub type Store = craft_settings::SettingsStore<Prefs>;

/// Opens the settings in the platform's folder (`CRAFTCLOUD_CONFIG_DIR` overrides it).
pub fn open() -> Store {
    open_at(craft_settings::Location::Auto)
}

/// Opens the settings in this folder (a settings folder such as [`Store::dir`]), read afresh
/// from the disk: what `craftcloud login` saved while the client ran is in it.
pub fn open_in(dir: &std::path::Path) -> Store {
    open_at(craft_settings::Location::Dir(dir.to_path_buf()))
}

/// Opens the settings at `location`.
pub fn open_at(location: craft_settings::Location) -> Store {
    let (store, report) = Store::open(location);
    if let Some(moved) = &report.broken_moved_to {
        log::warn!("settings were damaged; moved aside to {}", moved.display());
    }
    store
}

#[cfg(test)]
mod tests {
    use super::{Prefs, open_at};
    use craft_settings::Location;
    use craftcloud_core::sync::ClientSettings;

    /// 0.1.0's folder list is migrated exactly once: after a save and a fresh start it is gone
    /// from config.toml (the store merges into the file, so it must be written as empty).
    #[test]
    fn the_folder_migration_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "schema_version = 1\nserver = \"https://cloud.example.org\"\nlocal_folder = \"/home/john/CraftCloud\"\nfolders = [\"Work\"]\nsync_root_files = true\n",
        )
        .unwrap();
        let mut store = open_at(Location::Dir(dir.path().to_path_buf()));
        let old = store.get().0.clone();
        assert!(old.needs_migration());
        let new = old.migrate(&["Work".to_owned(), "Photos".to_owned()]).unwrap();
        store.set(Prefs(new));
        store.flush().unwrap();
        drop(store);

        let store = open_at(Location::Dir(dir.path().to_path_buf()));
        let again: &ClientSettings = &store.get().0;
        assert!(!again.needs_migration(), "{again:?}");
        assert_eq!(again.excluded, ["Photos"]);
        assert!(again.sync_root_files);
        let file = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(file.contains("folders = []"), "{file}");
    }
}
