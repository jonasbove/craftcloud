//! The tray (menu bar) icon, as craft-launcher's `apps/artcraft-launcher/src/tray.rs` does it
//! (MIT OR Apache-2.0, Copyright (c) 2026 ArtCraft Team): `tray-icon`, with the StatusNotifierItem
//! backend on Linux (D-Bus directly, no GTK). The icon is the CraftCloud logo with a dot for the
//! sync state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

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
}

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    pause: MenuItem,
    /// "Open CraftCloud folder" and "Sync now": off while the folder is missing, as in the window.
    folder: MenuItem,
    sync: MenuItem,
    /// Whether they were last shown off.
    missing: bool,
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
        Ok(Self {
            icon,
            status,
            pause,
            folder,
            sync,
            missing: false,
            recent,
            recent_paths,
            shown_recent: Vec::new(),
            shown: None,
            line: String::new(),
        })
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
            self.pause.set_text(if look == Look::Paused { "Resume syncing" } else { "Pause syncing" });
            self.shown = Some(look);
        }
        let missing = *state == SyncState::FolderMissing;
        if self.missing != missing {
            self.folder.set_enabled(!missing);
            self.sync.set_enabled(!missing);
            self.missing = missing;
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

/// The tray as the client sees it. Where the tray works without a window event loop (the
/// StatusNotifierItem backend on Linux and the BSDs), it lives on a thread of its own, so a tray
/// host or D-Bus that is slow to answer never holds up the window; elsewhere it's made on the main
/// thread once the window's event loop runs.
pub struct TrayHandle {
    actions: Receiver<TrayAction>,
    ready: Arc<AtomicBool>,
    inner: Inner,
    last: Option<TrayView>,
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
            let tray = Tray::new(move || wake(), tx);
            done_t.store(true, Ordering::Release);
            match tray {
                Ok(mut tray) => {
                    ready_t.store(true, Ordering::Release);
                    log::info!("tray icon ready");
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
        Self { actions, ready, inner: Inner::Thread(updates), last: None }
    }

    /// Makes the tray on this thread (macOS and Windows, inside the window's event loop).
    pub fn local(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (tx, actions) = channel();
        match Tray::new(wake, tx) {
            Ok(tray) => Some(Self { actions, ready: Arc::new(AtomicBool::new(true)), inner: Inner::Local(tray), last: None }),
            Err(e) => {
                log::warn!("no tray icon ({e}); closing the window quits");
                None
            }
        }
    }

    /// Whether the tray icon is up, so closing the window can leave the client in it.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn actions(&self) -> Vec<TrayAction> {
        self.actions.try_iter().collect()
    }

    pub fn show(&mut self, view: TrayView) {
        if self.last.as_ref() == Some(&view) {
            return;
        }
        match &mut self.inner {
            Inner::Local(tray) => tray.show(&view),
            Inner::Thread(updates) => {
                let _ = updates.send(view.clone());
            }
        }
        self.last = Some(view);
    }
}
