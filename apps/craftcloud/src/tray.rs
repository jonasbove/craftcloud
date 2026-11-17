//! The tray (menu bar) icon, as craft-launcher's `apps/artcraft-launcher/src/tray.rs` does it
//! (MIT OR Apache-2.0, Copyright (c) 2026 ArtCraft Team): `tray-icon`, with the StatusNotifierItem
//! backend on Linux (D-Bus directly, no GTK). The icon is the CraftCloud logo with a dot for the
//! sync state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craftcloud_core::sync::SyncState;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayAction {
    OpenFolder,
    OpenWeb,
    SyncNow,
    TogglePause,
    Show,
    /// Show the window with the Settings sheet open.
    Settings,
    /// Open a recent file (cloud path) with its app.
    OpenRecent(String),
    Quit,
}

/// How many recent files the menu lists.
pub const RECENT: usize = 5;

/// What the tray shows: the state, its line, and the recent files (cloud paths, newest first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayView {
    pub state: SyncState,
    pub line: String,
    pub recent: Vec<String>,
    /// Files are going up or down (not only being checked).
    pub moving: bool,
}

/// How long a sync's check may run before the tray says "Checking…". The check every cycle that
/// finds nothing to do is over long before, and showing it would change the icon and the menu
/// twice a cycle; a tray host such as Wayle rebuilds an open menu at every change.
const QUIET_CHECK: Duration = Duration::from_secs(10);

/// While files move, the count and the recent files change at most this often.
const PROGRESS_EVERY: Duration = Duration::from_secs(2);

/// Whether the tray should change from `shown` to `next` now; `false` holds it, and a later look
/// shows it. `busy_for` is how long the sync has been busy, `since_shown` how long ago the tray
/// last changed.
fn show_now(shown: Option<&TrayView>, next: &TrayView, busy_for: Duration, since_shown: Duration) -> bool {
    let Some(shown) = shown else { return true };
    if shown == next {
        return false;
    }
    // A state each cycle comes back to: a check that starts from it is routine until it runs long.
    let settled = matches!(
        shown.state,
        SyncState::UpToDate | SyncState::Offline(_) | SyncState::Failed(_) | SyncState::FolderMissing | SyncState::MassDelete { .. }
    );
    if next.state.is_busy() && !next.moving {
        return !settled || busy_for >= QUIET_CHECK;
    }
    if next.moving && shown.moving {
        return since_shown >= PROGRESS_EVERY;
    }
    true
}

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    pause: MenuItem,
    /// "Open CraftCloud folder" and "Sync now": off while the folder is missing, as in the window.
    folder: MenuItem,
    sync: MenuItem,
    /// What the menu items last showed.
    items: Option<MenuState>,
    recent: Vec<MenuItem>,
    /// The paths the recent items stand for (read by the menu's event handler).
    recent_paths: Arc<Mutex<Vec<String>>>,
    shown_recent: Vec<String>,
    shown: Option<Look>,
    /// The status line last shown: menu and tooltip change only when it does, as every change makes
    /// the tray host reload the menu (and some, such as Wayle, rebuild it while it's opening).
    line: String,
}

/// How long Quit waits for the window to take it up before the client exits anyway (a window
/// that gets no frames, e.g. minimised on Wayland, never would).
const QUIT_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    UpToDate,
    Syncing,
    Paused,
    Error,
}

impl Look {
    fn of(state: &SyncState) -> Self {
        match state {
            SyncState::Idle | SyncState::UpToDate => Self::UpToDate,
            SyncState::Scanning | SyncState::Downloading => Self::Syncing,
            SyncState::Paused | SyncState::MassDelete { .. } | SyncState::SignedOut => Self::Paused,
            SyncState::Offline(_) | SyncState::FolderMissing | SyncState::Failed(_) => Self::Error,
        }
    }

    fn dot(self) -> [u8; 3] {
        match self {
            Self::UpToDate => [0x3f, 0xd6, 0xb8],
            Self::Syncing => [0xa9, 0xc4, 0x8e],
            Self::Paused => [0xff, 0xc1, 0x4d],
            Self::Error => [0xff, 0x62, 0x59],
        }
    }
}

/// The menu items that follow the sync's state (design frame `v2-desktop-tray`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MenuState {
    /// "Pause syncing", or "Resume syncing" while paused. Only a pause is resumed: signed out and
    /// waiting for you share the paused icon, but a pick pauses them (`main.rs`, `TogglePause`).
    pause: PauseItem,
    /// "Open CraftCloud folder" and "Sync now": off while the folder is missing, as in the window.
    folder: bool,
    sync: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PauseItem {
    Pause,
    Resume,
}

impl PauseItem {
    fn label(self) -> &'static str {
        match self {
            Self::Pause => "Pause syncing",
            Self::Resume => "Resume syncing",
        }
    }
}

impl MenuState {
    fn of(state: &SyncState) -> Self {
        let missing = *state == SyncState::FolderMissing;
        let pause = if *state == SyncState::Paused { PauseItem::Resume } else { PauseItem::Pause };
        Self { pause, folder: !missing, sync: !missing }
    }
}

const LOGO_PNG: &[u8] = include_bytes!("../assets/craftcloud-logo-64.png");

/// The logo (64×64) with a status dot bottom right, cut out of the logo with a transparent ring.
fn draw(look: Look) -> Result<tray_icon::Icon, String> {
    let mut img = image::load_from_memory_with_format(LOGO_PNG, image::ImageFormat::Png).map_err(|e| e.to_string())?.into_rgba8();
    let (w, h) = img.dimensions();
    let inside = |x: f32, y: f32, cx: f32, cy: f32, r: f32| (x - cx).powi(2) + (y - cy).powi(2) <= r * r;
    let [r, g, b] = look.dot();
    let (cx, cy) = (w as f32 - 13.0, h as f32 - 13.0);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if inside(fx, fy, cx, cy, 10.0) {
                img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
            } else if inside(fx, fy, cx, cy, 13.0) {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
    }
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).map_err(|e| e.to_string())
}

impl Tray {
    /// `wake` is called after a menu pick so the window's next frame handles it, even when hidden.
    pub fn new(wake: impl Fn() + Send + Sync + 'static, tx: Sender<TrayAction>) -> Result<Self, String> {
        let status = MenuItem::new("CraftCloud", false, None);
        let folder = MenuItem::new("Open CraftCloud folder", true, None);
        let web = MenuItem::new("Open in browser", true, None);
        let sync = MenuItem::new("Sync now", true, None);
        let pause = MenuItem::new("Pause syncing", true, None);
        let show = MenuItem::new("Show CraftCloud", true, None);
        let settings = MenuItem::new("Settings…", true, None);
        let quit = MenuItem::new("Quit CraftCloud", true, None);
        let recent: Vec<MenuItem> = (0..RECENT).map(|_| MenuItem::new("—", false, None)).collect();
        let recent_menu = Submenu::new("Recent files", true);
        for item in &recent {
            recent_menu.append(item).map_err(|e| e.to_string())?;
        }
        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &folder,
            &web,
            &sync,
            &pause,
            &recent_menu,
            &PredefinedMenuItem::separator(),
            &show,
            &settings,
            &quit,
        ])
        .map_err(|e| e.to_string())?;
        // A left click opens the window; the menu is on the right (on macOS the click opens the menu).
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .with_tooltip("CraftCloud")
            .with_icon(draw(Look::UpToDate)?);
        if !cfg!(any(target_os = "macos", target_os = "windows")) {
            // A stable id for the StatusNotifierItem, as the launcher (not the process id).
            builder = builder.with_id("craftcloud").with_title("CraftCloud");
        }
        let icon = builder.build().map_err(|e| e.to_string())?;
        let wake = Arc::new(wake);
        let mut ids = vec![
            (folder.id().clone(), 0),
            (web.id().clone(), 1),
            (sync.id().clone(), 2),
            (pause.id().clone(), 3),
            (show.id().clone(), 4),
            (quit.id().clone(), 5),
            (settings.id().clone(), 6),
        ];
        ids.extend(recent.iter().enumerate().map(|(i, item)| (item.id().clone(), 10 + i)));
        let recent_paths: Arc<Mutex<Vec<String>>> = Arc::default();
        let paths = Arc::clone(&recent_paths);
        {
            let (tx, wake) = (tx.clone(), Arc::clone(&wake));
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    let _ = tx.send(TrayAction::Show);
                    wake();
                }
            }));
        }
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let Some((_, n)) = ids.iter().find(|(id, _)| *id == e.id) else { return };
            let action = match *n {
                0 => TrayAction::OpenFolder,
                1 => TrayAction::OpenWeb,
                2 => TrayAction::SyncNow,
                3 => TrayAction::TogglePause,
                4 => TrayAction::Show,
                6 => TrayAction::Settings,
                n @ 10.. => {
                    let Some(path) = paths.lock().unwrap_or_else(PoisonError::into_inner).get(n - 10).cloned() else { return };
                    TrayAction::OpenRecent(path)
                }
                _ => {
                    std::thread::spawn(|| {
                        std::thread::sleep(QUIT_GRACE);
                        log::warn!("Quit wasn't taken up in time: exiting");
                        std::process::exit(0);
                    });
                    TrayAction::Quit
                }
            };
            let _ = tx.send(action);
            wake();
        }));
        Ok(Self { icon, status, pause, folder, sync, items: None, recent, recent_paths, shown_recent: Vec::new(), shown: None, line: String::new() })
    }

    /// Shows the sync state in the icon, tooltip and first menu line, and the recent files.
    pub fn show(&mut self, view: &TrayView) {
        if self.shown_recent != view.recent {
            for (i, item) in self.recent.iter().enumerate() {
                match view.recent.get(i) {
                    Some(path) => {
                        item.set_text(path.rsplit('/').next().unwrap_or(path));
                        item.set_enabled(true);
                    }
                    None => {
                        item.set_text("—");
                        item.set_enabled(false);
                    }
                }
            }
            view.recent.clone_into(&mut self.recent_paths.lock().unwrap_or_else(PoisonError::into_inner));
            self.shown_recent = view.recent.clone();
        }
        self.show_state(&view.state, &view.line);
    }

    fn show_state(&mut self, state: &SyncState, line: &str) {
        let look = Look::of(state);
        if self.shown != Some(look) {
            if let Ok(icon) = draw(look) {
                let _ = self.icon.set_icon(Some(icon));
            }
            self.shown = Some(look);
        }
        // Apart from the icon: signed out shares the paused look, but not the pause item.
        let items = MenuState::of(state);
        if self.items != Some(items) {
            self.pause.set_text(items.pause.label());
            self.folder.set_enabled(items.folder);
            self.sync.set_enabled(items.sync);
            self.items = Some(items);
        }
        if self.line != line {
            self.line = line.to_owned();
            self.status.set_text(format!("CraftCloud · {line}"));
            let _ = self.icon.set_tooltip(Some(format!("CraftCloud · {line}")));
        }
    }
}

/// How long the tray may take to come up before the client says so in its log.
const TRAY_TIMEOUT: Duration = Duration::from_secs(5);

/// The tray host's well-known name on the session bus (the StatusNotifierItem specification).
/// The icon shows only while something owns it: Plasma, Xfce's panel, a bar's tray module, or
/// GNOME's AppIndicator extension; stock GNOME has none.
pub const HOST_NAME: &str = "org.kde.StatusNotifierWatcher";

/// Follows the tray host on the session bus. The StatusNotifierItem backend registers the icon
/// whether or not a host is there (one may come later: a bar that starts after the client), so
/// "the tray is up" must be read off the bus, not off the registration: with no host, closing
/// the window would leave the client running with no icon and no window.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod host {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use zbus::blocking::fdo::DBusProxy;
    use zbus::blocking::{Connection, MessageIterator};

    use super::HOST_NAME;

    /// Whether a host owns the name on `conn` now.
    pub(super) fn present(conn: &Connection) -> zbus::Result<bool> {
        Ok(DBusProxy::new(conn)?.name_has_owner(HOST_NAME.try_into()?)?)
    }

    /// Keeps `ready` true while a host owns the name on `conn`: one look, then every change of
    /// the name's owner; `changed` is called after each. Returns only when the connection ends.
    pub(super) fn follow(conn: &Connection, ready: &AtomicBool, changed: &dyn Fn(bool)) -> zbus::Result<()> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg(0, HOST_NAME)?
            .build();
        // Subscribed before the look, so a host that comes up in between isn't missed.
        let signals = MessageIterator::for_match_rule(rule, conn, Some(16))?;
        let now = present(conn)?;
        ready.store(now, Ordering::Release);
        changed(now);
        for message in signals.flatten() {
            // NameOwnerChanged(name, old_owner, new_owner): an empty new owner is the host going.
            let Ok((_, _, new_owner)) = message.body().deserialize::<(String, String, String)>() else { continue };
            let now = !new_owner.is_empty();
            ready.store(now, Ordering::Release);
            changed(now);
        }
        Ok(())
    }

    /// [`follow`] on the session bus, on a thread of its own; `wake` after every change. Without
    /// a session bus the icon can't have registered either, so `ready` stays false.
    pub(super) fn follow_on_thread(ready: Arc<AtomicBool>, wake: impl Fn() + Send + 'static) {
        let spawned = std::thread::Builder::new().name("craftcloud-tray-host".into()).spawn(move || {
            let conn = match Connection::session() {
                Ok(conn) => conn,
                Err(e) => {
                    log::warn!("no tray (no session bus: {e}); closing the window quits");
                    return;
                }
            };
            let result = follow(&conn, &ready, &|present| {
                if present {
                    log::info!("tray icon ready (a tray host owns {HOST_NAME})");
                } else {
                    log::warn!(
                        "no tray host on the session bus ({HOST_NAME}): the icon shows once one starts (on GNOME, the AppIndicator extension); until then closing the window quits"
                    );
                }
                wake();
            });
            if let Err(e) = result {
                log::warn!("lost track of the tray host: {e}");
            }
        });
        if let Err(e) = spawned {
            log::warn!("no tray (couldn't follow the tray host: {e}); closing the window quits");
        }
    }
}

/// macOS and Windows have a tray of their own: an icon that registered is shown.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod host {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub(super) fn follow_on_thread(ready: Arc<AtomicBool>, wake: impl Fn() + Send + 'static) {
        ready.store(true, Ordering::Release);
        log::info!("tray icon ready");
        wake();
    }
}

/// The tray as the client sees it. Where the tray works without a window event loop (the
/// StatusNotifierItem backend on Linux and the BSDs), it lives on a thread of its own, so a tray
/// host or D-Bus that is slow to answer never holds up the window; elsewhere it's made on the main
/// thread once the window's event loop runs.
pub struct TrayHandle {
    actions: Receiver<TrayAction>,
    ready: Arc<AtomicBool>,
    inner: Inner,
    last: Option<TrayView>,
    /// When the tray last changed, and since when the sync has been busy.
    shown_at: Instant,
    busy_since: Option<Instant>,
}

enum Inner {
    Local(Tray),
    Thread(Sender<TrayView>),
}

impl TrayHandle {
    /// Starts the tray on its own thread; [`TrayHandle::is_ready`] says once it's up.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, actions) = channel();
        let (updates, rx) = channel::<TrayView>();
        let ready = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (ready_t, done_t, wake) = (Arc::clone(&ready), Arc::clone(&done), Arc::new(wake));
        let spawned = std::thread::Builder::new().name("craftcloud-tray".into()).spawn(move || {
            let wake_host = Arc::clone(&wake);
            let tray = Tray::new(move || wake(), tx);
            done_t.store(true, Ordering::Release);
            match tray {
                Ok(mut tray) => {
                    // The icon is registered (or will be, once a host comes up); whether anything
                    // shows it is the host's presence, followed on a thread of its own.
                    host::follow_on_thread(ready_t, move || wake_host());
                    for view in rx {
                        tray.show(&view);
                    }
                }
                Err(e) => log::warn!("no tray icon ({e}); closing the window quits"),
            }
        });
        if let Err(e) = spawned {
            log::warn!("no tray icon (couldn't start its thread: {e}); closing the window quits");
        } else {
            let _ = std::thread::Builder::new().name("craftcloud-tray-watch".into()).spawn(move || {
                std::thread::sleep(TRAY_TIMEOUT);
                if !done.load(Ordering::Acquire) {
                    log::warn!(
                        "the tray icon hasn't registered after {} s (is a tray host running?); the window works without it",
                        TRAY_TIMEOUT.as_secs()
                    );
                }
            });
        }
        Self { actions, ready, inner: Inner::Thread(updates), last: None, shown_at: Instant::now(), busy_since: None }
    }

    /// Makes the tray on this thread (macOS and Windows, inside the window's event loop).
    pub fn local(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (tx, actions) = channel();
        match Tray::new(wake, tx) {
            Ok(tray) => Some(Self {
                actions,
                ready: Arc::new(AtomicBool::new(true)),
                inner: Inner::Local(tray),
                last: None,
                shown_at: Instant::now(),
                busy_since: None,
            }),
            Err(e) => {
                log::warn!("no tray icon ({e}); closing the window quits");
                None
            }
        }
    }

    /// Whether the tray icon is up and something shows it (a tray host owns [`HOST_NAME`]), so
    /// closing the window can leave the client in it. It follows the host: a bar that starts
    /// later makes it true, a host that quits makes it false.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn actions(&self) -> Vec<TrayAction> {
        self.actions.try_iter().collect()
    }

    /// Shows the sync's state; called every second or so, it changes the tray only as
    /// [`show_now`] allows, so the menu doesn't change under the pointer at every cycle.
    pub fn show(&mut self, view: TrayView) {
        let now = Instant::now();
        let busy_for = if view.state.is_busy() {
            now - *self.busy_since.get_or_insert(now)
        } else {
            self.busy_since = None;
            Duration::ZERO
        };
        if !show_now(self.last.as_ref(), &view, busy_for, now - self.shown_at) {
            return;
        }
        self.shown_at = now;
        match &mut self.inner {
            Inner::Local(tray) => tray.show(&view),
            Inner::Thread(updates) => {
                let _ = updates.send(view.clone());
            }
        }
        self.last = Some(view);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use craftcloud_core::sync::SyncState;

    use super::{Look, MenuState, PROGRESS_EVERY, QUIET_CHECK, TrayView, show_now};

    fn view(state: SyncState, line: &str, moving: bool) -> TrayView {
        TrayView { state, line: line.into(), recent: Vec::new(), moving }
    }

    const NO_TIME: Duration = Duration::ZERO;

    #[test]
    fn a_routine_check_leaves_the_tray_alone() {
        let up = view(SyncState::UpToDate, "Up to date", false);
        let checking = view(SyncState::Scanning, "Checking…", false);
        // Every cycle checks; one that finds nothing changes neither the icon nor the menu.
        assert!(!show_now(Some(&up), &checking, NO_TIME, Duration::from_secs(60)));
        assert!(!show_now(Some(&up), &view(SyncState::Downloading, "Checking…", false), Duration::from_secs(3), NO_TIME));
        assert!(!show_now(Some(&up), &up, NO_TIME, NO_TIME));
        // Offline, a check every cycle until the server answers: the same.
        let offline = view(SyncState::Offline("down".into()), "Offline", false);
        assert!(!show_now(Some(&offline), &checking, NO_TIME, NO_TIME));
        // A check that runs long is shown.
        assert!(show_now(Some(&up), &checking, QUIET_CHECK, NO_TIME));
        // After a pause or at the start, checking is news.
        assert!(show_now(Some(&view(SyncState::Paused, "Paused", false)), &checking, NO_TIME, NO_TIME));
        assert!(show_now(None, &checking, NO_TIME, NO_TIME));
    }

    #[test]
    fn moving_files_are_shown_at_once_then_every_few_seconds() {
        let up = view(SyncState::UpToDate, "Up to date", false);
        let first = view(SyncState::Downloading, "Syncing 1 of 40", true);
        let next = view(SyncState::Downloading, "Syncing 2 of 40", true);
        assert!(show_now(Some(&up), &first, NO_TIME, NO_TIME));
        assert!(!show_now(Some(&first), &next, Duration::from_secs(1), Duration::from_millis(300)));
        assert!(show_now(Some(&first), &next, Duration::from_secs(3), PROGRESS_EVERY));
        // Done, or anything that needs the person: at once.
        assert!(show_now(Some(&next), &up, NO_TIME, NO_TIME));
        assert!(show_now(Some(&next), &view(SyncState::SignedOut, "Signed out", false), NO_TIME, NO_TIME));
        assert!(show_now(Some(&up), &view(SyncState::Failed("x".into()), "Error", false), NO_TIME, NO_TIME));
    }

    /// Every state the sync can be in, with the icon look the design frame gives it.
    fn states() -> Vec<(SyncState, Look)> {
        vec![
            (SyncState::Idle, Look::UpToDate),
            (SyncState::UpToDate, Look::UpToDate),
            (SyncState::Scanning, Look::Syncing),
            (SyncState::Downloading, Look::Syncing),
            (SyncState::Paused, Look::Paused),
            (SyncState::SignedOut, Look::Paused),
            (SyncState::MassDelete { deletes: 30 }, Look::Paused),
            (SyncState::Offline("down".into()), Look::Error),
            (SyncState::FolderMissing, Look::Error),
            (SyncState::Failed("x".into()), Look::Error),
        ]
    }

    #[test]
    fn the_icon_looks_follow_the_frame() {
        // v2-desktop-tray: "Paused also stands for signed out and waiting for you (mass delete); Error for
        // offline, folder missing and a failed sync."
        for (state, look) in states() {
            assert!(Look::of(&state) == look, "{state:?}");
        }
    }

    #[test]
    fn only_a_pause_offers_resume() {
        assert_eq!(MenuState::of(&SyncState::Paused).pause.label(), "Resume syncing");
        // Signed out and waiting for you share the paused icon, but the pick pauses them, so the item says so.
        for (state, _) in states().into_iter().filter(|(s, _)| *s != SyncState::Paused) {
            assert_eq!(MenuState::of(&state).pause.label(), "Pause syncing", "{state:?}");
        }
    }

    /// A session bus of its own (never the computer's): `dbus-daemon` on a socket in a temporary
    /// folder, ended with the test. `None` where there is no `dbus-daemon` to run.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    struct PrivateBus {
        daemon: std::process::Child,
        address: String,
        _dir: tempfile::TempDir,
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    impl PrivateBus {
        fn start() -> Option<Self> {
            use std::io::BufRead;
            let dir = tempfile::tempdir().ok()?;
            let socket = dir.path().join("bus");
            let mut daemon = std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address", &format!("--address=unix:path={}", socket.display())])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()?;
            let mut address = String::new();
            let read = daemon.stdout.take().map(|out| std::io::BufReader::new(out).read_line(&mut address));
            if !matches!(read, Some(Ok(n)) if n > 0) {
                let _ = daemon.kill();
                return None;
            }
            Some(Self { daemon, address: address.trim().to_owned(), _dir: dir })
        }

        fn connect(&self) -> zbus::blocking::Connection {
            zbus::blocking::connection::Builder::address(self.address.as_str()).unwrap().build().unwrap()
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    /// Stock GNOME has no tray host: the icon registers all the same (so it shows when one comes
    /// up later), but the client must know nobody shows it. Before this, `is_ready` was true
    /// whenever the registration went through, and closing the window on such a desktop left
    /// the client running with no icon and no window.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn the_tray_is_ready_only_while_a_host_owns_the_watcher_name() {
        use super::{HOST_NAME, host};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let Some(bus) = PrivateBus::start() else {
            eprintln!("skipped: no dbus-daemon to start a private bus with");
            return;
        };
        let ready = Arc::new(AtomicBool::new(false));
        let looks = Arc::new(AtomicUsize::new(0));
        let follower = bus.connect();
        let (ready_t, looks_t) = (Arc::clone(&ready), Arc::clone(&looks));
        std::thread::spawn(move || {
            let _ = host::follow(&follower, &ready_t, &|_| {
                looks_t.fetch_add(1, Ordering::AcqRel);
            });
        });
        let wait_for = |looked: usize, want: bool| {
            let end = std::time::Instant::now() + Duration::from_secs(10);
            while std::time::Instant::now() < end {
                if looks.load(Ordering::Acquire) >= looked && ready.load(Ordering::Acquire) == want {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        assert!(wait_for(1, false), "nothing owns {HOST_NAME} on a fresh bus");
        // A tray host starts (Plasma, a panel, GNOME's AppIndicator extension): it claims the name.
        let host = bus.connect();
        host.request_name(HOST_NAME).unwrap();
        assert!(wait_for(2, true), "a host came up");
        host.release_name(HOST_NAME).unwrap();
        assert!(wait_for(3, false), "the host went away");
    }

    #[test]
    fn a_missing_folder_turns_off_open_folder_and_sync_now() {
        let missing = MenuState::of(&SyncState::FolderMissing);
        assert!(!missing.folder && !missing.sync);
        assert_eq!(missing.pause.label(), "Pause syncing");
        for (state, _) in states().into_iter().filter(|(s, _)| *s != SyncState::FolderMissing) {
            let items = MenuState::of(&state);
            assert!(items.folder && items.sync, "{state:?}");
        }
    }
}
