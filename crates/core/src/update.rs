//! "A new version is out": what the desktop client found, and how often it may look. The look
//! itself (GitHub's release list, over the network) is desktop-only, in
//! `craftcloud_native::update_check`; this is plain data and arithmetic, so the UI can show it on
//! any target.

/// The least time between two looks for a new release: once a day.
pub const CHECK_EVERY_SECS: i64 = 24 * 60 * 60;

/// Whether it's time to look for a new release again: never looked (`last` is `None`), a day
/// or more since the last look, or a last look in the future (the clock was set back), which
/// would otherwise hold every look off until the clock caught up.
pub fn due(last: Option<i64>, now: i64) -> bool {
    last.is_none_or(|last| now < last || now.saturating_sub(last) >= CHECK_EVERY_SECS)
}

/// A release newer than the running client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewVersion {
    /// Its version, `0.3.0`.
    pub version: String,
    /// Its release page (release notes and downloads), on the project's GitHub.
    pub url: String,
    /// The running copy came from a package manager (deb, rpm, Arch, Flatpak, Homebrew, Nix),
    /// which updates it: the note says so instead of pointing at the downloads.
    pub package_managed: bool,
}

#[cfg(test)]
mod tests {
    use super::{CHECK_EVERY_SECS, due};

    const NOW: i64 = 1_795_000_000;

    #[test]
    fn a_client_that_never_looked_looks_now() {
        assert!(due(None, NOW));
    }

    #[test]
    fn it_looks_at_most_once_a_day() {
        assert!(!due(Some(NOW), NOW));
        assert!(!due(Some(NOW - 60), NOW));
        assert!(!due(Some(NOW - CHECK_EVERY_SECS + 1), NOW));
        assert!(due(Some(NOW - CHECK_EVERY_SECS), NOW));
        assert!(due(Some(NOW - 30 * CHECK_EVERY_SECS), NOW));
    }

    #[test]
    fn a_clock_set_back_does_not_stop_the_looks() {
        assert!(due(Some(NOW + 3600), NOW));
        assert!(due(Some(i64::MAX), NOW));
        assert!(due(Some(0), i64::MAX));
        assert!(due(Some(i64::MIN), NOW));
    }
}
