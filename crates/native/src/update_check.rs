//! Says when a newer CraftCloud release is out: at most once a day the client reads the
//! project's GitHub release list (craft-libs' `craft-update`) and keeps the newest release that
//! is newer than itself, for Settings › About. Check and notify only: nothing is downloaded or
//! installed. The request carries only `CraftCloud/<version>` as its user agent.
//!
//! The look runs on a thread of its own, started once the window has drawn
//! ([`UpdateChecks::start`]), and only while the setting is on. Failures (no network, GitHub's
//! request limit) are logged and never shown.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use craft_update::http::Fetched;
use craft_update::{App, CheckError, Http};
use craftcloud_core::update::{NewVersion, due};
use serde::{Deserialize, Serialize};

use crate::http::Waker;

/// CraftCloud as craft-update knows it. Its own developer and app id, not the suite's defaults
/// (`ArtCraft`, `ai.storyteller.<id>`); only `repo` is read by the check.
pub const APP: App = App {
    developer: "CraftCloud contributors",
    bundle_id_override: Some("io.github.jonasbove.craftcloud"),
    ..App::new("craftcloud", "CraftCloud", "Keep a folder in sync with your own CraftCloud server", "jonasbove/craftcloud")
};

/// The file next to the settings that remembers the last look and what it found.
pub const STATE_FILE: &str = "update-check.json";

/// How often the thread wakes to see whether a look is due (the look itself is daily).
const WAKE_EVERY: Duration = Duration::from_secs(60 * 60);

/// The largest state file read: it holds a time, a version and a link.
const MAX_STATE: u64 = 16 * 1024;

/// What [`STATE_FILE`] holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// When the last look was made (Unix seconds), whatever came of it.
    pub checked: Option<i64>,
    /// The newest release the last answered look found newer than the client then running.
    pub latest: Option<Latest>,
}

/// A release found by a look.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latest {
    pub version: String,
    pub url: String,
}

/// What GitHub's answer means for a client running `running`: the newest published, stable
/// release when it is newer (drafts and pre-releases are never offered), `None` when there is
/// none, and an error when the answer can't be read or GitHub's request limit was reached.
pub fn decide(fetched: Fetched, running: &str) -> Result<Option<Latest>, CheckError> {
    let text = match fetched {
        Fetched::Body { text, .. } => text,
        Fetched::NotModified | Fetched::NotFound => return Ok(None),
        Fetched::RateLimited { .. } => return Err(CheckError::RateLimited),
    };
    let release = craft_update::parse_github_release_list(&text, APP.repo, false)?;
    let Some(running) = craft_update::parse_version(running) else { return Ok(None) };
    Ok(release.and_then(|r| {
        let newer = r.version.as_ref().is_some_and(|v| craft_update::version::offers_update(v, &running));
        newer.then(|| Latest { version: r.version_label(), url: r.html_url })
    }))
}

/// One wake of the thread: looks when a look is [`due`] (asking `fetch`), and records the time
/// of every look, answered or not, so a failing network never makes it ask more than daily.
/// Returns whether it looked.
pub fn step(saved: &mut Saved, now: i64, running: &str, fetch: impl FnOnce() -> Result<Fetched, String>) -> bool {
    if !due(saved.checked, now) {
        return false;
    }
    saved.checked = Some(now);
    match fetch().map_err(CheckError::Failed).and_then(|answer| decide(answer, running)) {
        Ok(latest) => {
            if let Some(l) = &latest {
                log::info!("CraftCloud {} is out ({})", l.version, l.url);
            }
            saved.latest = latest;
        }
        Err(e) => log::info!("couldn't look for a new CraftCloud version: {e}"),
    }
    true
}

/// What Settings › About shows from `saved` for a client running `running`: the release found,
/// while it is still newer (after an update it no longer is).
pub fn offered(saved: &Saved, running: &str, package_managed: bool) -> Option<NewVersion> {
    let latest = saved.latest.as_ref()?;
    let (found, running) = (craft_update::parse_version(&latest.version)?, craft_update::parse_version(running)?);
    craft_update::version::offers_update(&found, &running).then(|| NewVersion {
        version: latest.version.clone(),
        url: release_page(&latest.url),
        package_managed,
    })
}

/// The project's releases page, opened when a found release's own link can't be trusted.
const RELEASES: &str = "https://github.com/jonasbove/craftcloud/releases";

/// The link About opens for a found release: its own page only when it is one of this project's
/// release pages over https, in plain URL characters; anything else (another site, `http:`, a
/// `file:` or script link, odd characters) becomes [`RELEASES`]. The link comes from the network
/// and from `update-check.json`, so it is never handed to the browser unchecked.
fn release_page(url: &str) -> String {
    let rest = url.strip_prefix(RELEASES).and_then(|r| r.strip_prefix('/'));
    let plain = |r: &str| !r.is_empty() && r.bytes().all(|b| b.is_ascii_alphanumeric() || b"._~/-".contains(&b));
    match rest {
        Some(r) if plain(r) && !r.contains("..") => url.to_owned(),
        _ => RELEASES.to_owned(),
    }
}

/// Reads [`STATE_FILE`]; a missing, oversized or unreadable one is an empty state.
pub fn read_state(path: &Path) -> Saved {
    let small = std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= MAX_STATE);
    if !small {
        return Saved::default();
    }
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Writes [`STATE_FILE`] through a temporary file, so a crash never leaves half of one.
pub fn write_state(path: &Path, saved: &Saved) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(saved).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Whether a copy at `exe` is one a package manager updates, for the note's wording only: a
/// Flatpak (`flatpak`), or a program under a package manager's folders (`/usr` but not
/// `/usr/local`, Nix, Guix, Snap, Homebrew). An AppImage (`appimage`) mounts its own `/usr`, and
/// a copy in the home folder, `/usr/local` or `/opt` (an unpacked tarball) updates by hand.
pub fn package_managed_at(exe: &Path, flatpak: bool, appimage: bool) -> bool {
    if flatpak {
        return true;
    }
    if appimage {
        return false;
    }
    let managed_roots = ["/usr", "/nix", "/gnu", "/snap", "/var/lib/snapd/snap", "/opt/homebrew", "/home/linuxbrew"];
    let by_hand = ["/usr/local"];
    let homebrew_on_intel = ["/usr/local/Cellar", "/usr/local/Homebrew"];
    homebrew_on_intel.iter().any(|root| exe.starts_with(root))
        || exe.components().any(|c| c.as_os_str() == ".linuxbrew")
        || (managed_roots.iter().any(|root| exe.starts_with(root)) && !by_hand.iter().any(|root| exe.starts_with(root)))
}

/// [`package_managed_at`] for this running copy.
pub fn package_managed() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.canonicalize().unwrap_or(exe);
    let flatpak = cfg!(target_os = "linux") && craft_update::process::in_sandbox();
    package_managed_at(&exe, flatpak, std::env::var_os("APPIMAGE").is_some())
}

/// The update check of a running client: off until [`UpdateChecks::start`], and then looking on
/// a thread of its own while the setting is on.
pub struct UpdateChecks {
    on: Arc<AtomicBool>,
    found: Arc<Mutex<Option<NewVersion>>>,
    dir: Option<PathBuf>,
    version: &'static str,
    waker: Waker,
    /// The window has drawn: the thread may start.
    allowed: bool,
    thread: Option<std::thread::Thread>,
}

impl UpdateChecks {
    /// Starts nothing yet. `dir` is the settings folder (the state is kept there), `version` the
    /// running client's, `on` the setting, and `waker` repaints the window when a look found
    /// something.
    pub fn new(dir: Option<PathBuf>, version: &'static str, on: bool, waker: Waker) -> Self {
        Self { on: Arc::new(AtomicBool::new(on)), found: Arc::default(), dir, version, waker, allowed: false, thread: None }
    }

    /// The window has drawn its first frame: start looking, if the setting is on. Called again,
    /// it does nothing.
    pub fn start(&mut self) {
        self.allowed = true;
        self.spawn_if_wanted();
    }

    /// The setting changed: on starts looking (once the window has drawn), off stops.
    pub fn set_on(&mut self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
        if let Some(thread) = &self.thread {
            thread.unpark();
        }
        self.spawn_if_wanted();
    }

    /// Whether the window has drawn ([`UpdateChecks::start`] was called).
    pub fn started(&self) -> bool {
        self.allowed
    }

    /// Whether the looking thread runs.
    pub fn looking(&self) -> bool {
        self.thread.is_some()
    }

    /// The release to tell about, while the setting is on.
    pub fn found(&self) -> Option<NewVersion> {
        if !self.on.load(Ordering::Relaxed) {
            return None;
        }
        self.found.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn spawn_if_wanted(&mut self) {
        if !self.allowed || self.thread.is_some() || !self.on.load(Ordering::Relaxed) {
            return;
        }
        let (on, found, waker, version) = (Arc::clone(&self.on), Arc::clone(&self.found), Arc::clone(&self.waker), self.version);
        let path = self.dir.as_ref().map(|d| d.join(STATE_FILE));
        let spawned = std::thread::Builder::new().name("craftcloud-update-check".into()).spawn(move || {
            run(path.as_deref(), version, &on, &found, &waker);
        });
        match spawned {
            Ok(handle) => self.thread = Some(handle.thread().clone()),
            Err(e) => log::warn!("couldn't start the update check: {e}"),
        }
    }
}

/// The thread: shows what an earlier look found, then looks whenever a look is due and the
/// setting is on, waking hourly (or when the setting is switched on).
fn run(path: Option<&Path>, version: &str, on: &AtomicBool, found: &Mutex<Option<NewVersion>>, waker: &Waker) {
    let mut saved = path.map(read_state).unwrap_or_default();
    let managed = package_managed();
    let publish = |saved: &Saved| {
        let offer = offered(saved, version, managed);
        let mut slot = found.lock().unwrap_or_else(PoisonError::into_inner);
        if *slot != offer {
            *slot = offer;
            waker();
        }
    };
    publish(&saved);
    let mut http: Option<Http> = None;
    loop {
        if on.load(Ordering::Relaxed) {
            let looked = step(&mut saved, craft_update::now(), version, || {
                if http.is_none() {
                    http = Some(Http::new(&format!("CraftCloud/{version}"))?);
                }
                let url = format!("https://api.github.com/repos/{}/releases?per_page=30", APP.repo);
                http.as_ref().ok_or_else(|| "no HTTP client".to_owned())?.get_api(&url, None)
            });
            if looked {
                if let Some(p) = path
                    && let Err(e) = write_state(p, &saved)
                {
                    log::warn!("couldn't save the update check's state to {}: {e}", p.display());
                }
                publish(&saved);
            }
        }
        std::thread::park_timeout(WAKE_EVERY);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_this_projects_release_pages_are_opened() {
        let ok = "https://github.com/jonasbove/craftcloud/releases/tag/v0.3.0";
        assert_eq!(release_page(ok), ok);
        for bad in [
            "https://example.org/craftcloud/releases/tag/v0.3.0",
            "http://github.com/jonasbove/craftcloud/releases/tag/v0.3.0",
            "https://github.com/jonasbove/craftcloud/releasesX/tag/v0.3.0",
            "https://github.com/jonasbove/craftcloud/releases/../../other/repo",
            "https://github.com/jonasbove/craftcloud/releases/tag/v0.3.0?x=<script>",
            "https://github.com/jonasbove/craftcloud/releases/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(release_page(bad), RELEASES, "{bad}");
        }
    }

    #[test]
    fn a_foreign_link_in_githubs_answer_is_never_opened() {
        let Fetched::Body { text, .. } = body("newer.json") else { panic!("a body") };
        let tampered = text.replace("https://github.com/jonasbove/craftcloud/releases/tag/v0.3.0", "https://example.org/evil");
        assert_ne!(tampered, text, "the fixture holds the link this test replaces");
        // craft-update already swaps a foreign `html_url` for the releases page
        // (`safe_release_html_url`); this holds it to that, and `offered` checks again.
        let latest = decide(Fetched::Body { text: tampered, etag: None }, RUNNING).unwrap().expect("v0.3.0 is newer");
        assert!(!latest.url.contains("example.org"), "{}", latest.url);
        let found = offered(&Saved { checked: Some(NOW), latest: Some(latest) }, RUNNING, false).expect("offered");
        assert_eq!(found.url, RELEASES);
    }

    #[test]
    fn a_foreign_link_in_the_saved_state_is_never_offered() {
        let saved = Saved { checked: Some(1), latest: Some(Latest { version: "v99.0.0".into(), url: "https://example.org/x".into() }) };
        let found = offered(&saved, "0.2.0", false).expect("a newer version is still offered");
        assert_eq!(found.url, RELEASES);
    }

    use std::path::Path;

    use craft_update::CheckError;
    use craft_update::http::Fetched;
    use craftcloud_core::update::{CHECK_EVERY_SECS, NewVersion};

    use super::{APP, Latest, RELEASES, Saved, UpdateChecks, decide, offered, package_managed_at, read_state, release_page, step, write_state};

    const RUNNING: &str = "0.2.0";
    const NOW: i64 = 1_795_000_000;

    fn body(name: &str) -> Fetched {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/github-releases").join(name);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Fetched::Body { text, etag: None }
    }

    #[test]
    fn the_app_is_craftclouds_own() {
        assert_eq!(format!("https://github.com/{}", APP.repo), env!("CARGO_PKG_REPOSITORY"));
        assert_eq!(APP.bundle_id(), "io.github.jonasbove.craftcloud");
        assert_ne!(APP.developer, craft_update::app::DEVELOPER);
        assert_eq!(APP.id, "craftcloud");
    }

    #[test]
    fn a_newer_release_is_offered_with_its_page() {
        let latest = decide(body("newer.json"), RUNNING).unwrap();
        assert_eq!(latest, Some(Latest { version: "0.3.0".into(), url: "https://github.com/jonasbove/craftcloud/releases/tag/v0.3.0".into() }));
    }

    #[test]
    fn the_same_or_an_older_release_is_not_offered() {
        assert_eq!(decide(body("same.json"), RUNNING).unwrap(), None);
        assert_eq!(decide(body("older.json"), RUNNING).unwrap(), None);
    }

    #[test]
    fn drafts_and_pre_releases_are_never_offered() {
        assert_eq!(decide(body("prerelease.json"), RUNNING).unwrap(), None);
        assert_eq!(decide(body("draft.json"), RUNNING).unwrap(), None);
    }

    #[test]
    fn github_s_request_limit_is_an_error_not_an_offer() {
        assert_eq!(decide(Fetched::RateLimited { reset: Some(NOW + 600) }, RUNNING), Err(CheckError::RateLimited));
        // The limit's message as a body (a proxy that turned the 403 into a 200) offers nothing either.
        assert!(matches!(decide(body("rate-limited.json"), RUNNING), Err(CheckError::Failed(_))));
        assert_eq!(decide(Fetched::NotFound, RUNNING).unwrap(), None);
    }

    #[test]
    fn a_build_from_source_is_offered_only_a_higher_release() {
        assert_eq!(decide(body("same.json"), "0.2.0-dev").unwrap(), None);
        assert!(decide(body("newer.json"), "0.2.0-dev").unwrap().is_some());
    }

    #[test]
    fn it_looks_once_a_day_and_records_every_look() {
        let mut saved = Saved::default();
        assert!(step(&mut saved, NOW, RUNNING, || Ok(body("newer.json"))));
        assert_eq!(saved.checked, Some(NOW));
        assert_eq!(saved.latest.as_ref().map(|l| l.version.as_str()), Some("0.3.0"));

        let asked = std::cell::Cell::new(false);
        assert!(!step(&mut saved, NOW + 3600, RUNNING, || {
            asked.set(true);
            Ok(body("same.json"))
        }));
        assert!(!asked.get(), "not asked again within the day");

        // A failed look a day later counts as the day's look and keeps what was found.
        assert!(step(&mut saved, NOW + CHECK_EVERY_SECS, RUNNING, || Err("couldn't reach GitHub".into())));
        assert_eq!(saved.checked, Some(NOW + CHECK_EVERY_SECS));
        assert!(saved.latest.is_some());
        assert!(step(&mut saved, NOW + 2 * CHECK_EVERY_SECS, RUNNING, || Ok(Fetched::RateLimited { reset: None })));
        assert!(saved.latest.is_some());

        // An answered look with nothing newer forgets the old find.
        assert!(step(&mut saved, NOW + 3 * CHECK_EVERY_SECS, RUNNING, || Ok(body("same.json"))));
        assert_eq!(saved.latest, None);
    }

    #[test]
    fn a_find_is_shown_only_while_it_is_newer() {
        let saved = Saved {
            checked: Some(NOW),
            latest: Some(Latest { version: "0.3.0".into(), url: "https://github.com/jonasbove/craftcloud/releases/tag/v0.3.0".into() }),
        };
        assert_eq!(
            offered(&saved, RUNNING, true),
            Some(NewVersion {
                version: "0.3.0".into(),
                url: "https://github.com/jonasbove/craftcloud/releases/tag/v0.3.0".into(),
                package_managed: true
            })
        );
        assert_eq!(offered(&saved, "0.3.0", false), None, "updated since");
        assert_eq!(offered(&Saved::default(), RUNNING, false), None);
    }

    #[test]
    fn the_state_survives_a_restart_and_a_bad_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(super::STATE_FILE);
        assert_eq!(read_state(&path), Saved::default());
        let saved = Saved {
            checked: Some(NOW),
            latest: Some(Latest { version: "0.3.0".into(), url: "https://github.com/jonasbove/craftcloud/releases".into() }),
        };
        write_state(&path, &saved).unwrap();
        assert_eq!(read_state(&path), saved);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(read_state(&path), Saved::default());
        std::fs::write(&path, vec![b' '; 64 * 1024]).unwrap();
        assert_eq!(read_state(&path), Saved::default());
    }

    #[test]
    fn nothing_looks_before_the_window_draws_or_while_switched_off() {
        let waker: crate::http::Waker = std::sync::Arc::new(|| {});
        // On, but the window hasn't drawn: no thread (starting one here would ask GitHub).
        let checks = UpdateChecks::new(None, RUNNING, true, std::sync::Arc::clone(&waker));
        assert!(!checks.started() && !checks.looking());
        let mut off = UpdateChecks::new(None, RUNNING, false, waker);
        off.start();
        assert!(off.started());
        assert!(!off.looking(), "switched off: nothing runs");
        *off.found.lock().unwrap() = Some(NewVersion { version: "0.3.0".into(), url: String::new(), package_managed: false });
        assert_eq!(off.found(), None, "switched off: an earlier find isn't shown");
    }

    #[test]
    fn package_managers_copies_are_told_to_update_there() {
        let managed = |p: &str| package_managed_at(Path::new(p), false, false);
        assert!(managed("/usr/bin/craftcloud"), "deb, rpm and Arch");
        assert!(managed("/nix/store/abc-craftcloud-0.2.0/bin/craftcloud"));
        assert!(managed("/opt/homebrew/Cellar/craftcloud/0.2.0/bin/craftcloud"));
        assert!(managed("/usr/local/Cellar/craftcloud/0.2.0/bin/craftcloud"));
        assert!(managed("/home/linuxbrew/.linuxbrew/bin/craftcloud"));
        assert!(package_managed_at(Path::new("/app/bin/craftcloud"), true, false), "a Flatpak");
        assert!(!managed("/home/john/.local/bin/craftcloud"), "the tarball's install.sh");
        assert!(!managed("/usr/local/bin/craftcloud"));
        assert!(!managed("/opt/craftcloud/bin/craftcloud"));
        assert!(!managed("/Applications/CraftCloud.app/Contents/MacOS/craftcloud"));
        assert!(!managed("C:\\Program Files\\CraftCloud\\craftcloud.exe"));
        assert!(!package_managed_at(Path::new("/tmp/.mount_CraftCabc/usr/bin/craftcloud"), false, true), "an AppImage");
    }
}
