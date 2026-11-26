//! What a storage server guarantees the sync, as plain data (docs/WEBDAV-STORAGE.md § 7.3).
//!
//! The sync engine relies on a server for a handful of things: a create that refuses to overwrite,
//! an overwrite that refuses a version it didn't start from, a move that never lands on an
//! existing name, ETags that change with every write, a trash and earlier versions. Which of them
//! a server has is a [`Capabilities`]. For OxiCloud they are filled in by hand
//! ([`Capabilities::oxicloud`]) from what the engine relies on today; the check on connect that
//! measures them on other servers comes later (§ 14). What the sync may do with a server follows
//! from them: [`mode_for`] gives its [`Mode`], and `reconcile::restrict` cuts every plan to it.
//!
//! The check on connect that measures them on another server is [`probe`]: the steps and the
//! verdict as data and pure functions, run over HTTP by `craftcloud-native`.
//!
//! No I/O here: the drivers that talk to a server are in `craftcloud-native` (`storage`).

pub mod probe;

use serde::{Deserialize, Serialize};

use crate::names::NameRules;

/// Whether a server keeps a write guard it was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Guard {
    /// A write it doesn't allow is refused (412) and nothing changes.
    Honoured,
    /// It writes anyway.
    Ignored,
}

/// Whether a delete with `If-Match` is refused when the file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteGuard {
    Honoured,
    Ignored,
    /// Deletes don't go over WebDAV (OxiCloud: its REST trash, which takes no guard; the engine
    /// lists the file just before, the window docs/WEBDAV-STORAGE.md § 8 describes).
    NotUsed,
}

/// Whether a folder's ETag changes when something in it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderEtags {
    Follow,
    DontFollow,
    /// Seen to miss changes: never used to skip a folder (every folder is listed every cycle).
    NotTrusted,
}

/// What a `PUT` into a folder that isn't there answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingParent {
    /// 404.
    NotFound,
    /// 409.
    Conflict,
    Other,
}

/// A checksum a server lists for its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Checksum {
    Sha1,
    Md5,
    Adler32,
}

/// Where a delete goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trash {
    /// The server's own trash (restored the way its profile says).
    Server,
    /// CraftCloud's own, `.craftcloud/trash/` on the server.
    Client,
}

/// Where the bytes a save replaces are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Versions {
    /// The server keeps them itself (by file id).
    Server,
    /// CraftCloud copies them to `.craftcloud/versions/<path>/<time>` (5 kept).
    Client,
    None,
}

/// Where the free space comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quota {
    /// OxiCloud's `GET /api/auth/me`.
    OxiCloudRest,
    /// Nextcloud's OCS user endpoint.
    Ocs,
    /// WebDAV's quota properties.
    WebDavProps,
    Unknown,
}

/// A ceiling a known server's profile puts on the mode, from measurements the check on connect
/// can't make itself (docs/WEBDAV-STORAGE.md § 7.5, "Atomicity"; § 16, Q1): a guard the server
/// honours in a single request but loses under a race (OpenCloud's conditional uploads, reva#959;
/// its `MOVE` with `Overwrite: F`, both of two creates won in 1 of 3 runs). A profile can only
/// lower the mode this way, never raise it ([`mode_for`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Lowered {
    /// The most the sync may do with this kind of server.
    pub to: Mode,
    /// Why, in plain words (shown as a row of "What was checked").
    pub why: String,
}

/// What a server guarantees the sync (docs/WEBDAV-STORAGE.md § 7.3).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default = "Capabilities::unknown")]
pub struct Capabilities {
    /// `PUT` with `If-None-Match: *` over an existing file.
    pub create_if_absent: Guard,
    /// `PUT` with a stale `If-Match`.
    pub replace_if_match: Guard,
    /// `DELETE` with a stale `If-Match`.
    pub delete_if_match: DeleteGuard,
    /// `MOVE` with `Overwrite: F` onto an existing name is refused.
    pub move_no_overwrite: bool,
    /// A file's ETag changes on every write.
    pub etag_changes_on_write: bool,
    /// A `MOVE` keeps a file's ETag (moves made elsewhere are paired by it).
    pub etag_kept_by_move: bool,
    /// The ETag a `PUT` answers is the one a listing shows.
    pub put_etag_is_listed: bool,
    pub folder_etags: FolderEtags,
    pub missing_parent: MissingParent,
    /// `PROPFIND` with `Depth: infinity` answers.
    pub depth_infinity: bool,
    /// `COPY` works (client versions need it).
    pub copy: bool,
    /// Files have ids that survive a `MOVE` (`oc:fileid`).
    pub file_ids: bool,
    pub checksums: Option<Checksum>,
    /// Names that differ only in case are one file.
    pub case_insensitive: bool,
    pub trash: Trash,
    pub versions: Versions,
    pub quota: Quota,
    /// The names it can't take (left alone by the one skip rule, `names::skipped`).
    pub names: NameRules,
    /// A ceiling the server's profile puts on the mode (none for a server nobody knows, and none
    /// for OxiCloud); absent in a saved check from before it existed.
    #[serde(default)]
    pub lowered: Option<Lowered>,
}

impl Capabilities {
    /// OxiCloud v0.10.0, CraftCloud's own server, as the sync engine relies on it today (checked
    /// against the server in docs/WEBDAV-STORAGE.md § 5 and `crates/native/tests/live.rs`).
    pub const fn oxicloud() -> Self {
        Self {
            // `dav::Write::Create`: a 412 is a conflict copy.
            create_if_absent: Guard::Honoured,
            // `dav::Write::Replace`: a 412 is a conflict copy.
            replace_if_match: Guard::Honoured,
            // Deletes go to the REST trash, after a fresh listing shows the version planned.
            delete_if_match: DeleteGuard::NotUsed,
            // `dav::move_to` sends `Overwrite: F`.
            move_no_overwrite: true,
            etag_changes_on_write: true,
            // `reconcile` pairs a cloud rename by its ETag.
            etag_kept_by_move: true,
            put_etag_is_listed: true,
            // UPSTREAM.md C8: every folder is listed every cycle.
            folder_etags: FolderEtags::NotTrusted,
            missing_parent: MissingParent::Conflict,
            // 403: listed one level at a time.
            depth_infinity: false,
            // Client versions are `COPY`s.
            copy: true,
            // `oc:fileid` answers 404: versions are keyed by path.
            file_ids: false,
            checksums: None,
            // The engine treats the cloud's names as case-sensitive (only a Windows or macOS
            // computer's own clash rule applies, `names::clashes`).
            case_insensitive: false,
            // `files::trash` and `folders::trash` over REST; restored by id.
            trash: Trash::Server,
            // `.craftcloud/versions/<path>/<time>`, newest 5.
            versions: Versions::Client,
            quota: Quota::OxiCloudRest,
            // A backslash, a control character, more than 255 bytes: never sent.
            names: NameRules::oxicloud(),
            lowered: None,
        }
    }

    /// A server nothing is known about: every guard taken as ignored, nothing trusted. What the
    /// check on connect starts from, and what a saved check is read as where it doesn't say
    /// (`mode_for` gives it download only at most, and the check itself refuses what it
    /// couldn't measure).
    pub const fn unknown() -> Self {
        Self {
            create_if_absent: Guard::Ignored,
            replace_if_match: Guard::Ignored,
            delete_if_match: DeleteGuard::Ignored,
            move_no_overwrite: false,
            etag_changes_on_write: false,
            etag_kept_by_move: false,
            put_etag_is_listed: false,
            folder_etags: FolderEtags::NotTrusted,
            missing_parent: MissingParent::Other,
            depth_infinity: false,
            copy: false,
            file_ids: false,
            checksums: None,
            case_insensitive: false,
            trash: Trash::Client,
            versions: Versions::None,
            quota: Quota::Unknown,
            names: NameRules::oxicloud(),
            lowered: None,
        }
    }
}

/// What the sync may do with a server (docs/WEBDAV-STORAGE.md § 7.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Both ways: every write guarded, every delete into a trash, the old bytes kept before an
    /// overwrite.
    TwoWay,
    /// Cloud to this computer only: nothing is written, moved or deleted on the server; a change
    /// made here stays here, and a file deleted in the cloud goes into `.craftcloud/trash/` in the
    /// local folder, never away for good (§ 9).
    DownloadOnly,
    /// Nothing at all: the server can't show that a file changed. The default, so that a check
    /// read from a file that doesn't say is never taken as more.
    #[default]
    Refused,
}

impl Mode {
    /// The name in JSON (`two_way`, `download_only`, `refused`), the same the serde form uses.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TwoWay => "two_way",
            Self::DownloadOnly => "download_only",
            Self::Refused => "refused",
        }
    }

    /// In plain words: "two-way", "download only", "refused".
    pub const fn words(self) -> &'static str {
        match self {
            Self::TwoWay => "two-way",
            Self::DownloadOnly => "download only",
            Self::Refused => "refused",
        }
    }

    /// How much the mode allows: refused 0, download only 1, two-way 2.
    pub const fn allows(self) -> u8 {
        match self {
            Self::Refused => 0,
            Self::DownloadOnly => 1,
            Self::TwoWay => 2,
        }
    }

    /// The lesser of two modes (the one that allows less).
    pub const fn lesser(self, other: Self) -> Self {
        if other.allows() < self.allows() { other } else { self }
    }
}

/// The mode a server's capabilities allow (docs/WEBDAV-STORAGE.md § 7.3). Never two-way unless
/// every guard two-way sync relies on holds: a server that ignores a guard gets download only,
/// never a weaker two-way sync.
///
/// - Two-way needs a create that refuses to overwrite and an overwrite that refuses a stale ETag
///   (`If-None-Match: *`, `If-Match`), a move that never lands on an existing name, ETags that
///   change on every write, a trash (the server's, or the client's through `MOVE`), and somewhere
///   to keep the bytes an overwrite replaces (the server's versions, or the client's through
///   `COPY`; Q2: without either, download only).
/// - Download only needs listings with ETags and `GET`; refused when an ETag doesn't change with
///   the bytes, since then no change in the cloud could be seen.
/// - A guarded delete the server ignores doesn't lower the mode: deletes are checked by a fresh
///   listing, then moved into a trash (§ 8).
/// - A ceiling the server's profile put on the mode ([`Capabilities::lowered`], from measurements
///   the check can't make: a guard that holds once but not under a race) only ever lowers what
///   was measured, never raises it: a profile can take two-way away, not grant it.
///
/// Listings that fail, files without ETags or a check on connect that didn't finish are refused
/// too, but they leave no capabilities to decide from: the check on connect refuses them itself
/// (§ 7.5).
pub fn mode_for(c: &Capabilities) -> Mode {
    let measured = measured_mode(c);
    match &c.lowered {
        Some(l) => measured.lesser(l.to),
        None => measured,
    }
}

/// [`mode_for`] from the measurements alone, before the profile's ceiling.
const fn measured_mode(c: &Capabilities) -> Mode {
    if !c.etag_changes_on_write {
        return Mode::Refused;
    }
    let guarded = matches!(c.create_if_absent, Guard::Honoured) && matches!(c.replace_if_match, Guard::Honoured) && c.move_no_overwrite;
    // Every server has a trash: its own, or the client's, a `MOVE` into `.craftcloud/trash/`
    // with `Overwrite: F`, which the guarded move above makes safe. So `c.trash` gates nothing.
    let versions = match c.versions {
        Versions::Server => true,
        Versions::Client => c.copy,
        Versions::None => false,
    };
    if guarded && versions { Mode::TwoWay } else { Mode::DownloadOnly }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::{ProptestConfig, any, prop_oneof, proptest};

    use super::{Capabilities, DeleteGuard, FolderEtags, Guard, Lowered, MissingParent, Mode, Quota, Trash, Versions, mode_for};
    use crate::names::NameRules;

    #[test]
    fn oxicloud_is_what_the_engine_relies_on() {
        let c = Capabilities::oxicloud();
        assert_eq!((c.create_if_absent, c.replace_if_match), (Guard::Honoured, Guard::Honoured), "both writes are guarded");
        assert_eq!(c.delete_if_match, DeleteGuard::NotUsed, "deletes go to the REST trash");
        assert!(c.move_no_overwrite && c.etag_changes_on_write && c.etag_kept_by_move && c.put_etag_is_listed && c.copy);
        assert_eq!(c.folder_etags, FolderEtags::NotTrusted, "UPSTREAM.md C8");
        assert_eq!(c.missing_parent, MissingParent::Conflict);
        assert!(!c.depth_infinity && !c.file_ids && !c.case_insensitive && c.checksums.is_none());
        assert_eq!((c.trash, c.versions, c.quota), (Trash::Server, Versions::Client, Quota::OxiCloudRest));
        assert_eq!(c.names, NameRules::oxicloud());
        assert_eq!(c.lowered, None, "no profile lowers CraftCloud's own server");
    }

    /// A profile's ceiling (docs/WEBDAV-STORAGE.md § 7.5, "Atomicity") takes two-way away from
    /// a server that measured two-way, leaves a lower measurement as it is, and never raises:
    /// a ceiling of two-way over a measured download only stays download only, over a refused
    /// server stays refused.
    #[test]
    fn a_profile_can_only_lower_the_mode() {
        let lowered = |to: Mode, c: Capabilities| Capabilities { lowered: Some(Lowered { to, why: "a race lost".into() }), ..c };
        let two_way = Capabilities::oxicloud();
        assert_eq!(mode_for(&lowered(Mode::DownloadOnly, two_way.clone())), Mode::DownloadOnly, "the OpenCloud case");
        assert_eq!(mode_for(&lowered(Mode::Refused, two_way.clone())), Mode::Refused);
        assert_eq!(mode_for(&lowered(Mode::TwoWay, two_way.clone())), Mode::TwoWay, "a ceiling at two-way changes nothing");
        let download_only = Capabilities { create_if_absent: Guard::Ignored, ..two_way.clone() };
        assert_eq!(mode_for(&lowered(Mode::TwoWay, download_only.clone())), Mode::DownloadOnly, "never raised");
        assert_eq!(mode_for(&lowered(Mode::DownloadOnly, download_only)), Mode::DownloadOnly);
        let refused = Capabilities { etag_changes_on_write: false, ..two_way };
        assert_eq!(mode_for(&lowered(Mode::TwoWay, refused.clone())), Mode::Refused);
        assert_eq!(mode_for(&lowered(Mode::DownloadOnly, refused)), Mode::Refused);
        assert_eq!((Mode::TwoWay.allows(), Mode::DownloadOnly.allows(), Mode::Refused.allows()), (2, 1, 0));
        assert_eq!(Mode::TwoWay.lesser(Mode::DownloadOnly), Mode::DownloadOnly);
        assert_eq!(Mode::Refused.lesser(Mode::TwoWay), Mode::Refused);
    }

    /// A saved check from before the ceiling existed reads as having none; one with it round-trips.
    #[test]
    fn the_ceiling_is_optional_in_a_saved_check() {
        let json = serde_json::to_string(&Capabilities::oxicloud()).unwrap();
        assert!(json.contains("\"lowered\":null"), "{json}");
        let without: Capabilities = serde_json::from_str(&json.replace(",\"lowered\":null", "")).unwrap();
        assert_eq!(without, Capabilities::oxicloud());
        let with = Capabilities { lowered: Some(Lowered { to: Mode::DownloadOnly, why: "reva#959".into() }), ..Capabilities::oxicloud() };
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.contains("\"lowered\":{\"to\":\"download_only\",\"why\":\"reva#959\"}"), "{json}");
        assert_eq!(serde_json::from_str::<Capabilities>(&json).unwrap(), with);
    }

    fn any_mode() -> impl proptest::strategy::Strategy<Value = Mode> {
        prop_oneof![proptest::strategy::Just(Mode::TwoWay), proptest::strategy::Just(Mode::DownloadOnly), proptest::strategy::Just(Mode::Refused)]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        /// For every set of capabilities and every ceiling, the mode with the ceiling allows no
        /// more than the mode without it, and exactly as much when the ceiling is at two-way.
        #[test]
        fn a_ceiling_never_raises_the_mode(
            create in any::<bool>(), replace in any::<bool>(), move_no_overwrite in any::<bool>(), etag_changes in any::<bool>(),
            copy in any::<bool>(), versions in 0u8..3, ceiling in any_mode(),
        ) {
            let without = Capabilities {
                create_if_absent: if create { Guard::Honoured } else { Guard::Ignored },
                replace_if_match: if replace { Guard::Honoured } else { Guard::Ignored },
                move_no_overwrite,
                etag_changes_on_write: etag_changes,
                copy,
                versions: match versions { 0 => Versions::Server, 1 => Versions::Client, _ => Versions::None },
                ..Capabilities::oxicloud()
            };
            let with = Capabilities { lowered: Some(Lowered { to: ceiling, why: String::new() }), ..without.clone() };
            proptest::prop_assert!(mode_for(&with).allows() <= mode_for(&without).allows(), "{:?} -> {:?}", mode_for(&without), mode_for(&with));
            if ceiling == Mode::TwoWay {
                proptest::prop_assert_eq!(mode_for(&with), mode_for(&without));
            }
        }
    }

    /// OxiCloud syncs both ways, exactly as before the modes.
    #[test]
    fn oxicloud_syncs_both_ways() {
        assert_eq!(mode_for(&Capabilities::oxicloud()), Mode::TwoWay);
    }

    /// Each thing two-way sync relies on, taken away alone, leaves download only; an ETag that
    /// doesn't change with the bytes leaves nothing.
    #[test]
    fn a_missing_guard_leaves_download_only() {
        let base = Capabilities::oxicloud;
        let lowered: [(&str, Capabilities); 6] = [
            ("create ignores If-None-Match", Capabilities { create_if_absent: Guard::Ignored, ..base() }),
            ("overwrite ignores If-Match", Capabilities { replace_if_match: Guard::Ignored, ..base() }),
            ("move overwrites", Capabilities { move_no_overwrite: false, ..base() }),
            ("no versions at all (Q2)", Capabilities { versions: Versions::None, ..base() }),
            ("client versions without COPY (Q2)", Capabilities { copy: false, ..base() }),
            (
                "everything ignored",
                Capabilities { create_if_absent: Guard::Ignored, replace_if_match: Guard::Ignored, move_no_overwrite: false, ..base() },
            ),
        ];
        for (why, c) in lowered {
            assert_eq!(mode_for(&c), Mode::DownloadOnly, "{why}");
        }
        assert_eq!(mode_for(&Capabilities { etag_changes_on_write: false, ..base() }), Mode::Refused);
        let none = Capabilities { etag_changes_on_write: false, create_if_absent: Guard::Ignored, versions: Versions::None, ..base() };
        assert_eq!(mode_for(&none), Mode::Refused, "refused wins over download only");
    }

    /// What doesn't gate the mode: a delete guard (deletes are checked, then trashed), the
    /// server's own versions without `COPY`, a client trash, ids, checksums, folder ETags.
    #[test]
    fn what_two_way_doesnt_need() {
        let base = Capabilities::oxicloud;
        let still: [(&str, Capabilities); 9] = [
            ("delete ignores If-Match", Capabilities { delete_if_match: DeleteGuard::Ignored, ..base() }),
            ("delete honours If-Match", Capabilities { delete_if_match: DeleteGuard::Honoured, ..base() }),
            ("server versions, no COPY", Capabilities { versions: Versions::Server, copy: false, ..base() }),
            ("client trash", Capabilities { trash: Trash::Client, ..base() }),
            ("file ids", Capabilities { file_ids: true, ..base() }),
            ("a move changes the ETag", Capabilities { etag_kept_by_move: false, ..base() }),
            ("a PUT's ETag isn't listed", Capabilities { put_etag_is_listed: false, ..base() }),
            ("folder ETags follow", Capabilities { folder_etags: FolderEtags::Follow, ..base() }),
            ("case-insensitive", Capabilities { case_insensitive: true, ..base() }),
        ];
        for (why, c) in still {
            assert_eq!(mode_for(&c), Mode::TwoWay, "{why}");
        }
    }
}
