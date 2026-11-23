//! Start at login from the Flatpak, through the XDG Background portal.
//!
//! An autostart entry written inside the sandbox lands in `~/.var/app/<app id>/config/autostart`,
//! which the session never reads, and its `Exec` would name `/app/bin/craftcloud`, which only exists
//! in the sandbox. So the desktop is asked instead: `org.freedesktop.portal.Background`'s
//! `RequestBackground` with `autostart` and the command line `craftcloud --hidden`. The portal writes
//! `<app id>.desktop` into the host's autostart folder with `flatpak run --command=craftcloud
//! <app id> --hidden` in its `Exec` (and removes it when asked with `autostart` off). It may ask the
//! person first, and answers later, through a `Response` signal, so the request runs on a thread
//! of its own and never holds up the window.
//!
//! Whether it is on is read from that host entry: the manifest's `--filesystem=home` lets the
//! sandbox see `~/.config/autostart`. While a request is out, the wanted state is shown.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use super::{ENTRY, HIDDEN_FLAG, NAME, OLD_ENTRY};

/// True inside a Flatpak sandbox: flatpak puts `/.flatpak-info` in every sandbox and sets
/// `FLATPAK_ID` for the app.
pub(super) fn inside() -> bool {
    detect(Path::new("/.flatpak-info").is_file(), std::env::var_os("FLATPAK_ID"))
}

fn detect(info_file: bool, flatpak_id: Option<OsString>) -> bool {
    info_file || flatpak_id.is_some_and(|id| !id.is_empty())
}

/// The host's autostart folder as the sandbox sees it: flatpak points `XDG_CONFIG_HOME` into
/// `~/.var/app`, and passes the host's own value as `HOST_XDG_CONFIG_HOME`; without one, the
/// default `~/.config`.
fn host_entry_dir(host_config: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let config = host_config
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart"))
}

/// The command line the portal puts in the entry, after `flatpak run --command=`.
fn commandline() -> Vec<String> {
    vec![NAME.to_owned(), HIDDEN_FLAG.to_owned()]
}

/// A request that is out, and what went wrong with the last one that finished.
struct State {
    /// The state asked for while a request is out, with the request's number.
    pending: Option<(u64, bool)>,
    requests: u64,
    problem: Option<String>,
}

impl State {
    const NEW: Self = Self { pending: None, requests: 0, problem: None };

    /// A request for `on` goes out; its number.
    fn ask(&mut self, on: bool) -> u64 {
        self.requests += 1;
        self.pending = Some((self.requests, on));
        self.problem = None;
        self.requests
    }

    /// Request `number` finished. Only the newest request decides what is shown: an older one
    /// finishing late changes nothing.
    fn finished(&mut self, number: u64, result: Result<(), String>) {
        if self.pending.is_some_and(|(n, _)| n == number) {
            self.pending = None;
            self.problem = result.err();
        }
    }
}

static STATE: Mutex<State> = Mutex::new(State::NEW);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn is_enabled() -> bool {
    if let Some((_, wanted)) = state().pending {
        return wanted;
    }
    host_entry_dir(std::env::var_os("HOST_XDG_CONFIG_HOME"), std::env::var_os("HOME")).is_some_and(|dir| dir.join(ENTRY).is_file())
}

pub(super) fn set_enabled(on: bool) -> Result<(), String> {
    // An entry an earlier version wrote inside the sandbox never started anything; it goes, so it
    // can't confuse anyone looking in ~/.var/app.
    if let Some(dir) = super::entry_dir() {
        for name in [ENTRY, OLD_ENTRY] {
            if let Err(e) = super::remove(&dir.join(name)) {
                log::info!("couldn't remove the sandbox's old autostart entry: {e}");
            }
        }
    }
    let number = state().ask(on);
    let spawned = std::thread::Builder::new().name("craftcloud-background-portal".into()).spawn(move || {
        let result = portal::request(on, commandline());
        if let Err(e) = &result {
            log::warn!("start at login: {e}");
        }
        state().finished(number, result);
    });
    if let Err(e) = spawned {
        state().finished(number, Ok(()));
        return Err(format!("couldn't ask the desktop: {e}"));
    }
    Ok(())
}

pub(super) fn take_problem() -> Option<String> {
    state().problem.take()
}

/// What the portal's answer means for a request for `wanted`: `response` is 0 for done, 1 when the
/// person said no, 2 otherwise; `autostart` is the result's "will be started at login".
fn outcome(wanted: bool, response: u32, autostart: Option<bool>) -> Result<(), String> {
    match (response, autostart) {
        (0, None) => Ok(()),
        (0, Some(done)) if done == wanted => Ok(()),
        (0, Some(_)) if wanted => Err(NOT_ALLOWED.to_owned()),
        (0, Some(_)) => Err("The desktop kept CraftCloud's start at login on. Turn it off in its background apps settings.".to_owned()),
        (1, _) => Err(NOT_ALLOWED.to_owned()),
        _ => Err("The desktop couldn't change CraftCloud's start at login.".to_owned()),
    }
}

const NOT_ALLOWED: &str =
    "The desktop didn't allow CraftCloud to start at login. Allow it in the desktop's background apps settings, then try again.";

/// The message for a portal call that failed: a desktop without the Background portal (some
/// window managers' portals lack it) gets a way to do it by hand.
fn call_failed(error: &str) -> String {
    let missing = ["UnknownMethod", "UnknownInterface", "UnknownObject", "ServiceUnknown", "NameHasNoOwner"].iter().any(|name| error.contains(name));
    if missing {
        "This desktop doesn't offer start at login to Flatpak apps (it has no Background portal). Add `flatpak run io.github.jonasbove.craftcloud --hidden` to its startup programs instead.".to_owned()
    } else {
        format!("Couldn't ask the desktop to start CraftCloud at login: {error}")
    }
}

/// The D-Bus side, with zbus's blocking API on the request's own thread.
mod portal {
    use std::collections::HashMap;
    use std::time::Duration;

    use zbus::blocking::{MessageIterator, connection};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    use super::{call_failed, outcome};

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const BACKGROUND: &str = "org.freedesktop.portal.Background";
    const REQUEST: &str = "org.freedesktop.portal.Request";
    /// How long the person has to answer the desktop's question, if it asks one.
    const ANSWER: Duration = Duration::from_secs(300);

    pub(super) fn request(on: bool, commandline: Vec<String>) -> Result<(), String> {
        let conn = connection::Builder::session()
            .map(|b| b.method_timeout(Duration::from_secs(30)))
            .and_then(connection::Builder::build)
            .map_err(|e| format!("Couldn't reach the desktop (no session bus: {e})."))?;
        // Listen before asking, so the answer can't come first.
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(DEST)
            .and_then(|b| b.interface(REQUEST))
            .and_then(|b| b.member("Response"))
            .map(zbus::match_rule::Builder::build)
            .map_err(|e| call_failed(&e.to_string()))?;
        let answers = MessageIterator::for_match_rule(rule, &conn, Some(8)).map_err(|e| call_failed(&e.to_string()))?;

        let token = format!(
            "craftcloud_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos())
        );
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("reason", Value::from("Keeps your CraftCloud folder in sync from the tray after you log in."));
        options.insert("autostart", Value::from(on));
        options.insert("commandline", Value::from(commandline));
        options.insert("dbus-activatable", Value::from(false));
        let reply =
            conn.call_method(Some(DEST), PATH, Some(BACKGROUND), "RequestBackground", &("", options)).map_err(|e| call_failed(&e.to_string()))?;
        let handle: OwnedObjectPath = reply.body().deserialize().map_err(|e| call_failed(&e.to_string()))?;

        // Waited for on a thread of its own, so a desktop that never answers can't keep this one.
        // That thread ends with the answer, or when the connection is closed below or fails.
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("craftcloud-background-answer".into()).spawn(move || {
            for message in answers {
                let Ok(message) = message else { return };
                if message.header().path().is_some_and(|p| p.as_str() == handle.as_str()) {
                    let _ = tx.send(message.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().map_err(|e| e.to_string()));
                    return;
                }
            }
        });
        spawned.map_err(|e| format!("Couldn't wait for the desktop's answer: {e}"))?;
        let answer = rx.recv_timeout(ANSWER);
        // Closing the connection ends the answer thread's wait too, so nothing is left behind.
        if let Err(e) = conn.close() {
            log::debug!("closing the portal connection: {e}");
        }
        match answer {
            Ok(Ok((response, results))) => outcome(on, response, results.get("autostart").and_then(|v| bool::try_from(&**v).ok())),
            Ok(Err(e)) => Err(format!("The desktop's answer didn't make sense: {e}")),
            Err(_) => Err("The desktop didn't answer about start at login.".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{State, call_failed, commandline, detect, host_entry_dir, outcome};

    #[test]
    fn the_sandbox_is_found_by_its_info_file_or_app_id() {
        assert!(detect(true, None));
        assert!(detect(false, Some(OsString::from("io.github.jonasbove.craftcloud"))));
        assert!(!detect(false, None));
        assert!(!detect(false, Some(OsString::new())));
    }

    #[test]
    fn the_entry_is_read_from_the_hosts_autostart_folder_not_the_sandboxs() {
        // Inside the sandbox XDG_CONFIG_HOME is ~/.var/app/<id>/config; the host's is passed apart.
        assert_eq!(
            host_entry_dir(Some("/home/john/.myconfig".into()), Some("/home/john".into())),
            Some(PathBuf::from("/home/john/.myconfig/autostart"))
        );
        assert_eq!(host_entry_dir(None, Some("/home/john".into())), Some(PathBuf::from("/home/john/.config/autostart")));
        assert_eq!(host_entry_dir(Some(OsString::new()), Some("/home/john".into())), Some(PathBuf::from("/home/john/.config/autostart")));
        assert_eq!(host_entry_dir(None, None), None);
    }

    #[test]
    fn the_portal_starts_the_client_in_the_tray() {
        // The portal adds `flatpak run --command=<first> <app id>` in front of the rest.
        assert_eq!(commandline(), ["craftcloud", "--hidden"]);
    }

    #[test]
    fn the_portals_answer_decides() {
        assert_eq!(outcome(true, 0, Some(true)), Ok(()));
        assert_eq!(outcome(false, 0, Some(false)), Ok(()));
        // A portal that leaves the result out: the host entry tells the truth afterwards.
        assert_eq!(outcome(true, 0, None), Ok(()));
        // Allowed in the background, but not at login: off, and the person is told.
        assert!(outcome(true, 0, Some(false)).is_err());
        assert!(outcome(false, 0, Some(true)).is_err());
        assert!(outcome(true, 1, None).is_err());
        assert!(outcome(true, 2, None).is_err());
    }

    #[test]
    fn a_desktop_without_the_portal_gets_the_way_by_hand() {
        let e = call_failed("org.freedesktop.DBus.Error.UnknownMethod: No such interface “org.freedesktop.portal.Background”");
        assert!(e.contains("flatpak run io.github.jonasbove.craftcloud --hidden"), "{e}");
        assert!(call_failed("timed out").contains("timed out"));
    }

    #[test]
    fn the_newest_request_decides_what_is_shown() {
        let mut s = State::NEW;
        // On, then quickly off again: the switch shows off while both are out.
        let on = s.ask(true);
        let off = s.ask(false);
        assert_eq!(s.pending, Some((off, false)));
        // The first answer coming in late (refused, say) changes nothing.
        s.finished(on, Err("turned down".into()));
        assert_eq!(s.pending, Some((off, false)));
        assert_eq!(s.problem, None);
        // The newest one's answer is what counts.
        s.finished(off, Err("no portal".into()));
        assert_eq!(s.pending, None);
        assert_eq!(s.problem.as_deref(), Some("no portal"));
        // A new request clears the last problem.
        s.ask(true);
        assert_eq!(s.problem, None);
    }

    /// Asks the session's real portal to turn start at login *off*, so it changes nothing that
    /// isn't already off. Run it by hand on a desktop:
    /// `cargo test -p craftcloud-native --lib real_portal -- --ignored --nocapture`.
    /// Outside a Flatpak the portal may refuse the request (no app id); what this checks is that
    /// the call, the signal match and the wait come back with an answer or a message, never hang.
    #[test]
    #[ignore = "needs a desktop session bus"]
    fn the_real_portal_answers_or_says_why_not() {
        let result = super::portal::request(false, commandline());
        eprintln!("portal answered: {result:?}");
        if let Err(e) = result {
            assert!(!e.is_empty());
            assert!(!e.contains("didn't answer"), "the portal never answered: {e}");
        }
    }
}
