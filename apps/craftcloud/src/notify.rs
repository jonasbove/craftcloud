//! Desktop notifications for what needs the person while the window may be closed: a conflict,
//! the sign-in running out, and a sync that can't go on. Each is told once, not every cycle.
//!
//! On Linux and the BSDs they go to the desktop's notification service
//! (`org.freedesktop.Notifications`, over D-Bus) from a thread of their own, so a service that is
//! slow or missing never holds up the window or the tray; clicking one shows the window. On macOS
//! and Windows the tray's dot and the window say it.

use std::collections::BTreeSet;

use craftcloud_core::sync::{SyncProgress, SyncState};

/// One notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub kind: Kind,
    pub summary: String,
    pub body: String,
}

/// What a notification is about; a newer one of the same kind replaces the older one on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Conflict,
    SignedOut,
    CantSync,
}

/// A state that stops the sync and waits for the person, as far as telling them goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Problem {
    SignedOut,
    FolderMissing,
    MassDelete,
    Failed,
}

/// What was last told, so each problem is told once rather than every second.
#[derive(Debug, Default)]
pub struct Watch {
    /// The problem last told; it is told again only after the sync has been up to date.
    problem: Option<Problem>,
    /// The conflict copies (by their path) already told.
    conflicts: BTreeSet<String>,
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Watch {
    /// The notifications this progress calls for that haven't been given yet.
    pub fn see(&mut self, p: &SyncProgress) -> Vec<Note> {
        let mut notes = Vec::new();
        let problem = match &p.state {
            SyncState::SignedOut => Some(Problem::SignedOut),
            SyncState::FolderMissing => Some(Problem::FolderMissing),
            SyncState::MassDelete { .. } => Some(Problem::MassDelete),
            SyncState::Failed(_) => Some(Problem::Failed),
            // Only a good sync ends a problem: a failing sync goes back through "Checking…" every
            // cycle, and being offline comes and goes with the network (the tray's dot shows it).
            SyncState::UpToDate => {
                self.problem = None;
                None
            }
            SyncState::Idle | SyncState::Scanning | SyncState::Downloading | SyncState::Paused | SyncState::Offline(_) => None,
        };
        if let Some(problem) = problem.filter(|now| self.problem != Some(*now)) {
            self.problem = Some(problem);
            notes.push(match (problem, &p.state) {
                (Problem::SignedOut, _) => Note {
                    kind: Kind::SignedOut,
                    summary: "Signed out of CraftCloud".into(),
                    body: "The session ended (the password changed, or this device was removed). Nothing was deleted. Sign in again to keep syncing."
                        .into(),
                },
                (Problem::MassDelete, SyncState::MassDelete { deletes }) => Note {
                    kind: Kind::CantSync,
                    summary: format!("CraftCloud would delete {deletes} files"),
                    body: "They vanished at once, here or in the cloud. Their deletion waits until you decide; everything else keeps syncing.".into(),
                },
                (Problem::FolderMissing, _) => Note {
                    kind: Kind::CantSync,
                    summary: "Your CraftCloud folder is missing".into(),
                    body: "It was moved or deleted, or it's on a drive that isn't plugged in. Nothing was deleted in the cloud.".into(),
                },
                (_, SyncState::Failed(why)) => Note {
                    kind: Kind::CantSync,
                    summary: "CraftCloud can't sync".into(),
                    body: format!("{why}. It's tried again at the next cycle."),
                },
                (_, _) => Note { kind: Kind::CantSync, summary: "CraftCloud can't sync".into(), body: String::new() },
            });
        }
        // Each conflict copy once; one that is settled and comes back is told again.
        let fresh: Vec<_> = p.conflicts.iter().filter(|c| !self.conflicts.contains(&c.copy)).collect();
        if let Some(first) = fresh.first() {
            let more = match fresh.len() {
                1 => String::new(),
                n => format!(" (and {} more)", n - 1),
            };
            notes.push(Note {
                kind: Kind::Conflict,
                summary: format!("{} changed in two places{more}", name_of(&first.original)),
                body: "Both versions are kept. Open CraftCloud to decide which one stays, or keep both as separate files.".into(),
            });
        }
        self.conflicts = p.conflicts.iter().map(|c| c.copy.clone()).collect();
        notes
    }
}

/// Sends notifications from a thread of its own.
pub struct Notifier {
    watch: Watch,
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    notes: Option<std::sync::mpsc::Sender<Note>>,
}

impl Notifier {
    /// `show` is called when a notification is clicked (Linux and the BSDs).
    pub fn spawn(show: impl Fn() + Send + Sync + 'static) -> Self {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new().name("craftcloud-notify".into()).spawn(move || dbus::run(&rx, std::sync::Arc::new(show)));
            if let Err(e) = &spawned {
                log::warn!("no notifications (couldn't start their thread: {e})");
            }
            Self { watch: Watch::default(), notes: spawned.ok().map(|_| tx) }
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let _ = show;
            Self { watch: Watch::default() }
        }
    }

    /// Tells the person about anything new in this progress that needs them.
    pub fn see(&mut self, p: &SyncProgress) {
        let notes = self.watch.see(p);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        if let Some(tx) = &self.notes {
            for note in notes {
                let _ = tx.send(note);
            }
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        drop(notes);
    }
}

/// The freedesktop notification service, with zbus's blocking API on the notifier's own thread.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod dbus {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use zbus::blocking::{Connection, MessageIterator, connection};
    use zbus::zvariant::Value;

    use super::{Kind, Note};

    const NAME: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";
    /// The app id: the desktop entry and the icon's name.
    const APP_ID: &str = "io.github.jonasbove.craftcloud";

    type Show = Arc<dyn Fn() + Send + Sync>;

    pub(super) fn run(notes: &Receiver<Note>, show: Show) {
        let conn = match connection::Builder::session().map(|b| b.method_timeout(Duration::from_secs(10))).and_then(connection::Builder::build) {
            Ok(conn) => conn,
            Err(e) => {
                log::warn!("no notifications (no session bus: {e})");
                // Drain, so the senders never notice.
                for _ in notes {}
                return;
            }
        };
        // The ids of the notifications on screen, by kind: a newer one replaces the older one, and
        // a click on any of them shows the window.
        let shown: Arc<Mutex<BTreeMap<Kind, u32>>> = Arc::default();
        listen_for_clicks(&conn, Arc::clone(&shown), show);
        let mut warned = false;
        for note in notes {
            let replaces = shown.lock().unwrap_or_else(PoisonError::into_inner).get(&note.kind).copied().unwrap_or(0);
            match notify(&conn, &note, replaces) {
                Ok(id) => {
                    shown.lock().unwrap_or_else(PoisonError::into_inner).insert(note.kind, id);
                }
                Err(e) if !warned => {
                    warned = true;
                    log::warn!("couldn't show a notification (is a notification service running?): {e}");
                }
                Err(e) => log::debug!("couldn't show a notification: {e}"),
            }
        }
    }

    fn notify(conn: &Connection, note: &Note, replaces: u32) -> zbus::Result<u32> {
        let actions = ["default", "Open CraftCloud"];
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("desktop-entry", Value::from(APP_ID));
        // Notify(app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout).
        let body = ("CraftCloud", replaces, APP_ID, note.summary.as_str(), note.body.as_str(), &actions[..], hints, -1_i32);
        let reply = conn.call_method(Some(NAME), PATH, Some(NAME), "Notify", &body)?;
        reply.body().deserialize::<u32>()
    }

    /// Shows the window when one of ours is clicked (`ActionInvoked`), on a thread of its own.
    fn listen_for_clicks(conn: &Connection, shown: Arc<Mutex<BTreeMap<Kind, u32>>>, show: Show) {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .and_then(|b| b.interface(NAME))
            .and_then(|b| b.member("ActionInvoked"))
            .map(zbus::match_rule::Builder::build);
        let signals = match rule.and_then(|rule| MessageIterator::for_match_rule(rule, conn, Some(16))) {
            Ok(signals) => signals,
            Err(e) => {
                log::info!("notifications won't open the window when clicked: {e}");
                return;
            }
        };
        let spawned = std::thread::Builder::new().name("craftcloud-notify-clicks".into()).spawn(move || {
            for message in signals.flatten() {
                let Ok((id, _action)) = message.body().deserialize::<(u32, String)>() else { continue };
                if shown.lock().unwrap_or_else(PoisonError::into_inner).values().any(|ours| *ours == id) {
                    show();
                }
            }
        });
        if let Err(e) = spawned {
            log::info!("notifications won't open the window when clicked: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use craftcloud_core::sync::{ConflictCopy, SyncProgress, SyncState};

    use super::{Kind, Watch};

    fn at(state: SyncState) -> SyncProgress {
        SyncProgress { state, ..SyncProgress::default() }
    }

    fn kinds(watch: &mut Watch, p: &SyncProgress) -> Vec<Kind> {
        watch.see(p).into_iter().map(|n| n.kind).collect()
    }

    #[test]
    fn a_problem_is_told_once_until_the_sync_is_up_to_date_again() {
        let mut w = Watch::default();
        assert_eq!(kinds(&mut w, &at(SyncState::Scanning)), []);
        assert_eq!(kinds(&mut w, &at(SyncState::Failed("The server answered 500".into()))), [Kind::CantSync]);
        // Every cycle goes back through "Checking…" and fails again: nothing new to tell.
        for state in [SyncState::Scanning, SyncState::Failed("The server answered 502".into()), SyncState::Offline("down".into())] {
            assert_eq!(kinds(&mut w, &at(state)), []);
        }
        assert_eq!(kinds(&mut w, &at(SyncState::UpToDate)), []);
        assert_eq!(kinds(&mut w, &at(SyncState::Failed("again".into()))), [Kind::CantSync]);
        // Another problem is news.
        assert_eq!(kinds(&mut w, &at(SyncState::SignedOut)), [Kind::SignedOut]);
        assert_eq!(kinds(&mut w, &at(SyncState::SignedOut)), []);
        assert_eq!(kinds(&mut w, &at(SyncState::FolderMissing)), [Kind::CantSync]);
        assert_eq!(kinds(&mut w, &at(SyncState::MassDelete { deletes: 40 })), [Kind::CantSync]);
    }

    #[test]
    fn being_offline_or_paused_is_not_a_notification() {
        let mut w = Watch::default();
        for state in [SyncState::Idle, SyncState::Offline("no network".into()), SyncState::Paused, SyncState::Downloading, SyncState::UpToDate] {
            assert_eq!(kinds(&mut w, &at(state)), []);
        }
    }

    #[test]
    fn each_conflict_copy_is_told_once() {
        let conflict = |name: &str| ConflictCopy { original: format!("/Documents/{name}.docx"), copy: format!("/Documents/{name} (conflict).docx") };
        let mut w = Watch::default();
        let mut p = SyncProgress { state: SyncState::UpToDate, conflicts: vec![conflict("Q3 report")], ..SyncProgress::default() };
        let notes = w.see(&p);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].summary, "Q3 report.docx changed in two places");
        assert!(w.see(&p).is_empty());
        p.conflicts.push(conflict("Budget"));
        p.conflicts.push(conflict("Notes"));
        assert_eq!(w.see(&p)[0].summary, "Budget.docx changed in two places (and 1 more)");
        assert!(w.see(&p).is_empty());
        // Settled, then the same file conflicts again: news again.
        p.conflicts.clear();
        assert!(w.see(&p).is_empty());
        p.conflicts.push(conflict("Q3 report"));
        assert_eq!(kinds(&mut w, &p), [Kind::Conflict]);
    }
}
