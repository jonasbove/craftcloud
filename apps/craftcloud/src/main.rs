//! CraftCloud's desktop client: signs in through the browser, then keeps a local folder in sync
//! with the cloud, from the tray. `--hidden` (a start at login) starts without the window.
//! `craftcloud <command>` is the command line instead (`cli`, docs/CLI.md).

// A GUI program on Windows: no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod cli;
mod geometry;
mod notify;
mod tray;

use std::process::ExitCode;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::Tokens;
use craftcloud_core::http::HttpRequest;
use craftcloud_core::sync::{ClientSettings, ConflictCopy, Keep, SyncPlan, SyncProgress, SyncState};
use craftcloud_native::control;
use craftcloud_native::http::{ThreadBackend, Waker};
use craftcloud_native::secrets::TokenStore;
use craftcloud_native::session::Session;
use craftcloud_native::settings::{self, Prefs, Store};
use craftcloud_native::sync::{self as transfers, Command, Handle};
use craftcloud_ui_egui::backend::{Backend, NetResult};
use craftcloud_ui_egui::desktop::{DesktopApp, DesktopHost, FileFacts, WallClock, status_text};

use crate::tray::{RECENT, TrayAction, TrayHandle, TrayView};

struct NativeHost {
    net: ThreadBackend,
    store: Store,
    waker: Waker,
    started: Instant,
    sync: Option<Handle>,
    /// The newest tokens: the sync refreshes them and the UI's own requests need them too.
    fresh: Arc<Mutex<Option<Tokens>>>,
    /// Quit was picked in the window.
    quit: bool,
    /// What the command line's `status|sync|pause|resume` reach (craftcloud_native::control).
    control: control::Slot,
    /// This client's claim on the sync folder while it syncs it (one engine per folder).
    folder: Option<control::FolderLock>,
    /// Why syncing couldn't start (another engine has the folder), shown as the sync state.
    blocked: Option<String>,
}

impl NativeHost {
    fn tokens(&self, server: &str) -> TokenStore {
        TokenStore::new(server, self.store.dir())
    }

    fn command(&self, command: Command) {
        if let Some(h) = &self.sync {
            h.send(command);
        }
    }

    /// Claims the sync folder again when it was missing at the start and is back now.
    fn claim_again(&mut self) {
        if self.sync.is_none() || !self.folder.as_ref().is_some_and(control::FolderLock::is_empty) {
            return;
        }
        match control::claim_folder(std::path::Path::new(&self.store.get().0.local_folder)) {
            Ok(lock) => self.folder = Some(lock),
            Err(e) => {
                log::error!("{e}");
                self.blocked = Some(e);
                self.sync = None;
            }
        }
    }
}

impl Backend for NativeHost {
    fn send(&mut self, id: u64, request: HttpRequest) {
        self.net.send(id, request);
    }

    fn poll(&mut self) -> Vec<(u64, NetResult)> {
        self.net.poll()
    }

    fn open_url(&mut self, url: &str) {
        if let Err(e) = craftcloud_native::open_url(url) {
            log::warn!("couldn't open the browser: {e}");
        }
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

impl DesktopHost for NativeHost {
    fn client_name(&self) -> String {
        craftcloud_native::client_name()
    }

    fn default_folder(&self) -> String {
        craftcloud_native::default_sync_folder().map(|p| p.display().to_string()).unwrap_or_default()
    }

    fn save_session(&mut self, settings: &ClientSettings, tokens: &Tokens) -> Result<(), String> {
        // Chosen just now (setup, or signing in again): the one time the folder is made. The sync
        // never makes it on its own (a folder that vanished may be on an unplugged drive).
        if !settings.local_folder.is_empty() {
            std::fs::create_dir_all(&settings.local_folder).map_err(|e| format!("can't make {}: {e}", settings.local_folder))?;
        }
        self.store.set(Prefs(settings.clone()));
        self.store.flush().map_err(|e| e.to_string())?;
        self.tokens(&settings.server).save(tokens)
    }

    fn start_sync(&mut self, plan: SyncPlan, tokens: Tokens) {
        let Ok(server) = ServerUrl::parse(&plan.server) else {
            log::error!("not a server address: {}", plan.server);
            return;
        };
        if let Some(why) = desktop_refusal(&self.store.get().0) {
            // Never a sync with another kind of server through OxiCloud's driver.
            log::error!("{why}");
            self.sync = None;
            self.blocked = Some(why);
            return;
        }
        // One engine per folder: let go of the old claim before making the new one.
        self.sync = None;
        self.folder = None;
        match control::claim_folder(std::path::Path::new(&plan.local_folder)) {
            Ok(lock) => {
                self.folder = Some(lock);
                self.blocked = None;
            }
            Err(e) => {
                log::error!("{e}");
                self.blocked = Some(e);
                return;
            }
        }
        *self.fresh.lock().unwrap_or_else(PoisonError::into_inner) = Some(tokens.clone());
        let fresh = Arc::clone(&self.fresh);
        let notify = Box::new(move |t: &Tokens| *fresh.lock().unwrap_or_else(PoisonError::into_inner) = Some(t.clone()));
        // Shared with the command line: refreshes are locked and taken over (session.rs).
        let session = Session::shared(server, tokens, TokenStore::new(&plan.server, self.store.dir()), self.store.dir()).on_new_tokens(notify);
        let device = Some(self.store.get().0.device_name.trim().to_owned()).filter(|d| !d.is_empty()).unwrap_or_else(craftcloud_native::device_name);
        let (server, local_folder, links) = (plan.server.clone(), plan.local_folder.clone(), plan.clone());
        let handle = transfers::start_with(plan, session, device, Arc::clone(&self.waker));
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(control::Attached { progress: Arc::clone(&handle.progress), commands: handle.sender(), server, local_folder, plan: links });
        self.sync = Some(handle);
    }

    fn progress(&self) -> SyncProgress {
        if let Some(why) = &self.blocked {
            return SyncProgress { state: SyncState::Failed(why.clone()), ..SyncProgress::default() };
        }
        self.sync.as_ref().map(|h| h.progress.lock().unwrap_or_else(PoisonError::into_inner).clone()).unwrap_or_default()
    }

    fn fresh_tokens(&self) -> Option<Tokens> {
        self.fresh.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn sync_now(&mut self) {
        self.claim_again();
        self.command(Command::SyncNow);
    }

    fn confirm_deletes(&mut self) {
        self.command(Command::ConfirmDeletes);
    }

    fn discard_deletes(&mut self) {
        self.command(Command::DiscardDeletes);
    }

    fn recreate_folder(&mut self) {
        let folder = self.store.get().0.local_folder.clone();
        if let Err(e) = std::fs::create_dir_all(&folder) {
            // The sync stays held as "folder missing"; the log says why.
            log::error!("can't make {folder}: {e}");
            return;
        }
        self.claim_again();
        self.command(Command::SyncNow);
    }

    fn pause(&mut self, paused: bool) {
        self.command(Command::Pause(paused));
        self.command(Command::SyncNow);
    }

    fn change_folders(&mut self, settings: &ClientSettings) -> Result<(), String> {
        self.store.set(Prefs(settings.clone()));
        self.store.flush().map_err(|e| e.to_string())?;
        self.command(Command::Replan(SyncPlan::from(settings)));
        if let Some(a) = self.control.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            a.local_folder.clone_from(&settings.local_folder);
        }
        Ok(())
    }

    fn save_prefs(&mut self, settings: &ClientSettings) -> Result<(), String> {
        let before = self.store.get().0.device_name.clone();
        self.store.set(Prefs(settings.clone()));
        self.store.flush().map_err(|e| e.to_string())?;
        if before != settings.device_name {
            let device = Some(settings.device_name.trim().to_owned()).filter(|d| !d.is_empty()).unwrap_or_else(craftcloud_native::device_name);
            self.command(Command::Rename(device));
        }
        Ok(())
    }

    fn local_file(&self, path: &str) -> Option<FileFacts> {
        let root = std::path::PathBuf::from(&self.store.get().0.local_folder);
        craftcloud_native::file_facts(&root, path).map(|(size, modified)| FileFacts { size, modified })
    }

    fn computer_name(&self) -> String {
        craftcloud_native::device_name()
    }

    fn clock(&self) -> WallClock {
        let (unix, date) = craftcloud_native::wall_clock();
        WallClock { unix, date }
    }

    fn quit(&mut self) {
        self.quit = true;
    }

    fn resolve(&mut self, conflict: &ConflictCopy, keep: Keep) {
        self.command(Command::Resolve(conflict.clone(), keep));
    }

    fn starts_at_login(&self) -> bool {
        craftcloud_native::login::is_enabled()
    }

    fn set_start_at_login(&mut self, on: bool) -> Result<(), String> {
        craftcloud_native::login::set_enabled(on)
    }

    fn open_browser(&mut self, url: &str) -> bool {
        craftcloud_native::open_url(url).map_err(|e| log::warn!("couldn't open the browser: {e}")).is_ok()
    }

    fn open_folder(&mut self, path: &str) {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(opener).arg(path).spawn() {
            log::warn!("couldn't open {path}: {e}");
        }
    }

    fn open_file(&mut self, path: &str) {
        let root = std::path::PathBuf::from(&self.store.get().0.local_folder);
        if let Err(e) = craftcloud_native::open::resolve(&root, path).map_err(|e| e.to_string()).and_then(|p| craftcloud_native::open::open(&p)) {
            log::warn!("couldn't open {path}: {e}");
        }
    }

    fn sign_out(&mut self) {
        let server = self.store.get().0.server.clone();
        self.tokens(&server).delete();
        let old = self.store.get().0.clone();
        self.store.set(Prefs(ClientSettings { server, device_name: old.device_name, appearance: old.appearance, ..ClientSettings::default() }));
        if let Err(e) = self.store.flush() {
            log::warn!("couldn't save the settings: {e}");
        }
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.sync = None;
        *self.fresh.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.folder = None;
        self.blocked = None;
    }
}

/// What this desktop client says to settings naming another kind of server with the opt-in on:
/// it syncs CraftCloud's own server only until it learns the others (docs/WEBDAV-STORAGE.md § 14,
/// pull request 10); the command line's `craftcloud daemon` and `sync` do sync them.
const DESKTOP_OXICLOUD_ONLY: &str = "This desktop client syncs with a CraftCloud server only for now; `craftcloud daemon` or `craftcloud sync` syncs this server from the command line.";

/// Why the desktop client won't sync with the saved settings' kind of server, if it won't:
/// refused by this build without the opt-in (`craftcloud_native::experimental`), or not yet
/// known to the desktop client. `None` for CraftCloud's own server.
fn desktop_refusal(settings: &ClientSettings) -> Option<String> {
    match craftcloud_native::experimental::allowed_here(settings) {
        Err(why) => Some(why.to_owned()),
        Ok(()) if settings.storage != craftcloud_core::sync::Storage::Oxicloud => Some(DESKTOP_OXICLOUD_ONLY.to_owned()),
        Ok(()) => None,
    }
}

/// Wakes whichever is running: the window's frames, or the loop that waits in the tray without one.
struct Wake {
    window: Mutex<Option<egui::Context>>,
    nudge: SyncSender<()>,
}

impl Wake {
    fn wake(&self) {
        if let Some(ctx) = self.window.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            ctx.request_repaint();
        }
        let _ = self.nudge.try_send(());
    }

    fn set_window(&self, ctx: Option<egui::Context>) {
        *self.window.lock().unwrap_or_else(PoisonError::into_inner) = ctx;
    }
}

/// The files that most recently went up, came down or moved, newest first (for the tray).
fn recent_files(p: &SyncProgress) -> Vec<String> {
    use craftcloud_core::sync::ActivityKind;
    let mut out: Vec<String> = Vec::new();
    for e in p.activity.iter().rev().filter(|e| matches!(e.kind, ActivityKind::Up | ActivityKind::Down | ActivityKind::Moved)) {
        if !out.contains(&e.path) {
            out.push(e.path.clone());
        }
        if out.len() == RECENT {
            break;
        }
    }
    out
}

/// What the tray asked of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Show,
    Quit,
}

/// The client, which outlives its windows: the sync, its settings and the tray.
struct Client {
    desktop: DesktopApp<NativeHost>,
    /// `None` where the desktop has no tray: then closing the window quits.
    tray: Option<TrayHandle>,
    /// The sign-in, read from the keychain on a thread of its own (it can wait on an unlock).
    keychain: Option<Receiver<Option<Tokens>>>,
    /// Whether the window system can hide a window and show it again (Wayland can't).
    can_hide: bool,
    /// The window closed into the tray (where it can't hide): carry on without one.
    closed_to_tray: bool,
    /// Asks from elsewhere: show the window (the app started again, or a notification clicked),
    /// or take a sign-in `craftcloud login` saved.
    shows: Receiver<control::ToWindow>,
    notes: notify::Notifier,
    wake: Arc<Wake>,
}

impl Client {
    /// Shows the sync state in the tray and carries out its picks; returns what needs a window.
    fn tray_tick(&mut self) -> Vec<Request> {
        if let Some(answer) = self.keychain.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.keychain = None;
            self.desktop.keychain_answered(answer);
        }
        let progress = self.desktop.host.progress();
        self.notes.see(&progress);
        let mut requests = Vec::new();
        for ask in self.shows.try_iter().collect::<Vec<_>>() {
            match ask {
                control::ToWindow::Show => requests.push(Request::Show),
                control::ToWindow::SignedIn => {
                    if let Some((server, dir)) = self.take_sign_in() {
                        self.keychain = Some(read_keychain(server, Some(dir), &self.wake));
                    }
                }
            }
        }
        let Some(tray) = &mut self.tray else { return requests };
        let moving = progress.state == SyncState::Downloading && progress.files_total > 0;
        tray.show(TrayView {
            state: progress.state.clone(),
            line: status_text(&progress),
            recent: recent_files(&progress),
            moving,
            stuck_move: progress.stuck_move().is_some(),
        });
        for action in tray.actions() {
            let settings = self.desktop.settings().clone();
            match action {
                TrayAction::OpenFolder => self.desktop.host.open_folder(&settings.local_folder),
                TrayAction::OpenWeb => self.desktop.host.open_url(&settings.server),
                TrayAction::SyncNow => self.desktop.host.sync_now(),
                TrayAction::TogglePause => {
                    let paused = matches!(progress.state, SyncState::Paused);
                    self.desktop.host.pause(!paused);
                }
                TrayAction::Show => requests.push(Request::Show),
                TrayAction::Settings => {
                    self.desktop.show_settings();
                    requests.push(Request::Show);
                }
                TrayAction::OpenRecent(path) => self.desktop.host.open_file(&path),
                TrayAction::Quit => requests.push(Request::Quit),
            }
        }
        requests
    }

    /// `craftcloud login` saved a sign-in: a client still waiting for one reads the settings again
    /// (the command line set the server) and returns the server and the settings folder, whose
    /// sign-in is then read from the keychain on a thread of its own, as at the start.
    fn take_sign_in(&mut self) -> Option<(String, std::path::PathBuf)> {
        let dir = self.desktop.host.store.dir()?.to_path_buf();
        let store = settings::open_in(&dir);
        let saved = store.get().0.clone();
        if !self.desktop.signed_in_elsewhere(saved.clone()) {
            return None;
        }
        self.desktop.host.store = store;
        if let Some(why) = desktop_refusal(&saved) {
            // Signed in to another kind of server on the command line: this desktop client
            // doesn't sync it (yet), and says so instead of reading the keychain.
            log::warn!("{why}");
            self.desktop.refuse(why);
            return None;
        }
        log::info!("a sign-in was saved on the command line: taking it");
        Some((saved.server, dir))
    }

    /// Whether closing the window can leave the client running in the tray.
    fn has_tray(&self) -> bool {
        self.tray.as_ref().is_some_and(TrayHandle::is_ready)
    }

    /// Carries on with no window until the tray asks for one (`true`) or for Quit (`false`).
    fn wait_in_tray(&mut self, woken: &Receiver<()>) -> bool {
        log::debug!("no window: carrying on in the tray");
        loop {
            let requests = self.tray_tick();
            if requests.contains(&Request::Quit) {
                return false;
            }
            if requests.contains(&Request::Show) {
                return true;
            }
            // The tray's status line follows the sync, so look again now and then.
            if woken.recv_timeout(Duration::from_secs(1)) == Err(RecvTimeoutError::Disconnected) {
                return false;
            }
        }
    }
}

/// One window onto the client.
struct Window<'a> {
    client: &'a mut Client,
    quitting: bool,
    /// A system CJK font, read on a thread of its own (the bundled faces have no CJK glyphs).
    cjk: Option<Receiver<Option<FontFile>>>,
    /// Shown, not hidden in the tray.
    shown: bool,
    /// The place and size eframe restored were looked at (and put right when bad).
    placed: bool,
    /// The last good place and size, saved instead of a minimised or hidden window's.
    geometry: geometry::Keeper,
}

impl<'a> Window<'a> {
    /// `saved` is the window's place and size from the last run, as eframe keeps it. A window
    /// that can't hide (Wayland) is shown whatever was asked: it opens after all. So is one with
    /// no tray to come back from (no tray host, or no icon): the window is then the client.
    fn new(client: &'a mut Client, cjk: Option<Receiver<Option<FontFile>>>, shown: bool, saved: Option<String>) -> Self {
        if !shown && client.can_hide && !client.has_tray() {
            log::info!("started hidden, but there is no tray to come back from: the window opens");
        }
        let shown = shown || !client.can_hide || !client.has_tray();
        Self { client, quitting: false, cjk, shown, placed: false, geometry: geometry::Keeper::new(saved) }
    }

    /// Puts right a window that eframe restored from a bad save (once, when its size is known).
    fn place(&mut self, ctx: &egui::Context) {
        let (inner, outer, monitor) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().outer_rect, i.viewport().monitor_size));
        if self.placed || inner.is_none() {
            return;
        }
        self.placed = true;
        if let Some(commands) = geometry::restore(inner, outer, monitor) {
            log::info!("the saved window place was off (minimised or hidden when saved): back to the default");
            for command in commands {
                ctx.send_viewport_cmd(command);
            }
        }
    }
}

/// A font file's bytes and the face's index in it.
type FontFile = (Vec<u8>, u32);

/// Reads the system's CJK font for the window's fallback, off the UI thread.
fn read_cjk_font(ctx: &egui::Context) -> Option<Receiver<Option<FontFile>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("craftcloud-fonts".into()).spawn(move || {
        let _ = tx.send(craftcloud_native::cjk_font());
        ctx.request_repaint();
    });
    spawned.ok().map(|_| rx)
}

/// What brings the window back from the taskbar, the Dock or the tray, in front: out of the
/// taskbar first (a minimised window that is only made visible stays an icon; Windows won't focus
/// one), then shown, then focused.
const SHOW: [egui::ViewportCommand; 3] =
    [egui::ViewportCommand::Minimized(false), egui::ViewportCommand::Visible(true), egui::ViewportCommand::Focus];

impl eframe::App for Window<'_> {
    /// The tray, the control channel and Quit are handled here, not in `ui`: eframe calls only
    /// `logic` while the window is minimised or hidden (and `ui` draws nothing while it is hidden),
    /// so a Show handled in `ui` never came.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.place(ctx);
        for request in self.client.tray_tick() {
            match request {
                Request::Show => {
                    self.shown = true;
                    for command in SHOW {
                        ctx.send_viewport_cmd(command);
                    }
                    // A full frame next, to paint the window as it comes back.
                    ctx.request_repaint();
                }
                Request::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        // With a tray, closing the window keeps syncing in the background: the window hides where
        // it can, and otherwise (Wayland) really closes while the client carries on in the tray.
        let mut closing = false;
        if self.client.has_tray() && !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            match on_close(self.client.can_hide, cfg!(target_os = "macos")) {
                OnClose::Minimize => {
                    // A proper hide on macOS needs AppKit calls (unsafe); minimising keeps it in the Dock.
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                OnClose::Hide => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.shown = false;
                    closing = true;
                }
                OnClose::ToTray => self.client.closed_to_tray = true,
            }
        }
        if std::mem::take(&mut self.client.desktop.host.quit) {
            self.quitting = true;
            self.client.closed_to_tray = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let shown = self.shown;
        self.geometry.look(ctx.input(|i| geometry::worth_keeping(shown, i.viewport())));
        // Whether eframe takes the window as one to draw: `None` where the window system says
        // nothing about occlusion (Windows), which eframe reads as drawn.
        let drawn = ctx.input(|i| i.viewport().visible()).unwrap_or(true);
        // A window that isn't shown is hidden now, whatever eframe did with it (#9): eframe makes
        // every window hidden and shows it itself after the first frame it paints, also one that
        // asked to start hidden (`post_rendering`). This command runs after that in the same pass,
        // so the window is hidden again before the event loop goes on. It also hides a window just
        // closed into the tray. Not on Wayland, where a hide is ignored (`can_hide`).
        if !self.shown && (drawn || closing) && self.client.can_hide && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        // The tray, the control channel and the sync wake this pass themselves; the tick is a
        // spare for a window that isn't drawn, where a pass runs only this. On a shown window it
        // would be a full frame a second, drawn on the CPU by a software renderer (#3).
        if let Some(tick) = logic_tick(self.shown, drawn) {
            ctx.request_repaint_after(tick);
        }
    }

    /// eframe has just written the window's place and size: never a minimised or hidden one's.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.geometry.save(storage);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // A hidden window draws nothing. On Windows the window system says nothing about
        // occlusion, so eframe takes a hidden window for a drawn one and runs `ui` and paints a
        // frame nobody sees at every tick; drawn in software, a full frame a second kept a core
        // busy. The frame that shows the window comes with Show, in the same pass as its commands.
        if self.shown {
            self.client.desktop.ui(ui);
        }
        if self.client.desktop.host.quit {
            // Quit was picked: `logic` closes the window on the next pass.
            ctx.request_repaint();
        }
        if let Some(found) = self.cjk.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.cjk = None;
            match found {
                Some((bytes, index)) => {
                    let mut data = egui::FontData::from_owned(bytes);
                    data.index = index;
                    craftcloud_ui_egui::theme::add_fallback_font(&ctx, "system-cjk", data);
                }
                None => log::info!("no CJK font found; Chinese, Japanese and Korean names show as boxes"),
            }
        }
    }
}

/// The spare tick `logic` asks for: every second while the window is hidden or minimised, where
/// eframe calls only `logic`; none while it is drawn, where the window's own ticks and the wakes do.
fn logic_tick(shown: bool, drawn: bool) -> Option<Duration> {
    (!shown || !drawn).then_some(Duration::from_secs(1))
}

/// What closing the window does when there is a tray.
#[derive(Debug, PartialEq, Eq)]
enum OnClose {
    Minimize,
    Hide,
    /// Let the window close; the client carries on in the tray without one.
    ToTray,
}

fn on_close(can_hide: bool, macos: bool) -> OnClose {
    if macos {
        OnClose::Minimize
    } else if can_hide {
        OnClose::Hide
    } else {
        OnClose::ToTray
    }
}

/// Whether a window opened here can hide and come back: not under Wayland, where winit ignores
/// `set_visible(false)`. winit opens a Wayland window wherever `WAYLAND_DISPLAY` or
/// `WAYLAND_SOCKET` is set (Linux and the BSDs).
fn window_can_hide_here(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    let wayland = || ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"].iter().any(|name| env(name).is_some_and(|v| !v.is_empty()));
    !(cfg!(all(unix, not(target_os = "macos"))) && wayland())
}

/// The tray works without a window event loop only with the StatusNotifierItem backend.
const TRAY_WITHOUT_WINDOW: bool = cfg!(all(unix, not(target_os = "macos")));

fn options(visible: bool) -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default()
        .with_title("CraftCloud")
        // The Wayland app id and the X11 window class: the desktop entry's name and StartupWMClass
        // (packaging/linux/io.github.jonasbove.craftcloud.desktop), so the dock finds its icon.
        .with_app_id("io.github.jonasbove.craftcloud")
        .with_inner_size(geometry::DEFAULT_SIZE)
        .with_min_inner_size(geometry::MIN_SIZE)
        .with_visible(visible);
    let viewport = match eframe::icon_data::from_png_bytes(include_bytes!("../assets/craftcloud-logo-256.png")) {
        Ok(icon) => viewport.with_icon(Arc::new(icon)),
        Err(e) => {
            log::warn!("no window icon: {e}");
            viewport
        }
    };
    eframe::NativeOptions { viewport, ..Default::default() }
}

/// How long the keychain may take before the client says so in its log (the window is up meanwhile).
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Reads the saved sign-in on a thread of its own: the keychain answers over D-Bus and may first
/// ask to be unlocked, which must never keep the window from opening.
fn read_keychain(server: String, dir: Option<std::path::PathBuf>, wake: &Arc<Wake>) -> Receiver<Option<Tokens>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (done_t, wake) = (Arc::clone(&done), Arc::clone(wake));
    let reader = std::thread::Builder::new().name("craftcloud-keychain".into()).spawn(move || {
        let tokens = TokenStore::new(&server, dir.as_deref()).load();
        done_t.store(true, std::sync::atomic::Ordering::Release);
        let _ = tx.send(tokens);
        wake.wake();
    });
    if reader.is_ok() {
        let _ = std::thread::Builder::new().name("craftcloud-keychain-watch".into()).spawn(move || {
            std::thread::sleep(KEYCHAIN_TIMEOUT);
            if !done.load(std::sync::atomic::Ordering::Acquire) {
                log::warn!("the keychain hasn't answered after {} s (locked?); waiting for it", KEYCHAIN_TIMEOUT.as_secs());
            }
        });
    }
    rx
}

fn main() -> ExitCode {
    // A command (`craftcloud ls`, …) runs and exits; it never opens a window.
    let quiet = std::env::args().len() > 1 && !std::env::args().skip(1).all(|a| a == craftcloud_native::login::HIDDEN_FLAG);
    // `RUST_LOG` as usual, except that the HTTP library's trace lines (request and answer bytes,
    // tokens included) never pass: `craftcloud_core::logging`.
    let logger = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(if quiet { "error" } else { "warn" })).build();
    let max = logger.filter();
    if log::set_boxed_logger(Box::new(craftcloud_core::logging::WireCapped(logger))).is_ok() {
        log::set_max_level(max);
    }
    if let Some(code) = cli::run_if_command() {
        return code;
    }
    // Windows: `craftcloud://` links start this executable (HKCU, refreshed on every start).
    if let Err(e) = craftcloud_native::url_scheme::register() {
        log::warn!("couldn't register craftcloud:// links: {e}");
    }
    let mut hidden = std::env::args().any(|a| a == craftcloud_native::login::HIDDEN_FLAG);
    let (nudge, woken) = sync_channel(1);
    let wake = Arc::new(Wake { window: Mutex::new(None), nudge });
    let waker: Waker = {
        let wake = Arc::clone(&wake);
        Arc::new(move || wake.wake())
    };
    let store = settings::open();
    // Started again while it runs (from the Start menu, the app menu, or a second entry at login):
    // the running client shows its window, and this one ends. A start at login shows nothing.
    if let Some(dir) = store.dir() {
        match second_start(hidden, || control::is_running(dir), || control::show_window(dir)) {
            Start::Run => {}
            Start::LeaveIt => {
                log::info!("CraftCloud is already running");
                return ExitCode::SUCCESS;
            }
            Start::Shown => {
                log::info!("CraftCloud is already running: its window is shown");
                return ExitCode::SUCCESS;
            }
        }
    }
    let (show_tx, shows) = std::sync::mpsc::channel();
    let show: control::ShowWindow = {
        let wake = Arc::clone(&wake);
        Arc::new(move |ask| {
            let _ = show_tx.send(ask);
            wake.wake();
        })
    };
    let saved = store.get().0.clone();
    // Settings naming another kind of server: refused by this build (docs/WEBDAV-STORAGE.md § 16)
    // or by this desktop client, which syncs CraftCloud's own server only so far; the window
    // opens and says so, and the keychain isn't read.
    let refusal = desktop_refusal(&saved);
    let keychain = (refusal.is_none() && !saved.server.is_empty()).then(|| read_keychain(saved.server.clone(), store.dir().map(Into::into), &wake));
    let control: control::Slot = Arc::default();
    // The command line's way in; a `craftcloud daemon` already answering keeps it.
    let _control_server = store
        .dir()
        .filter(|dir| !control::is_running(dir))
        .and_then(|dir| control::serve_window(dir, Arc::clone(&control), Arc::clone(&show)).map_err(|e| log::warn!("no control channel: {e}")).ok());
    let host = NativeHost {
        net: ThreadBackend::new(Arc::clone(&waker)),
        store,
        waker,
        started: Instant::now(),
        sync: None,
        fresh: Arc::default(),
        quit: false,
        control,
        folder: None,
        blocked: None,
    };
    // The desktop UI themes whichever window draws it; this context only stands in until then.
    let mut desktop = DesktopApp::waiting_for_keychain(&egui::Context::default(), host, saved);
    if let Some(why) = refusal {
        log::warn!("{why}");
        desktop.refuse(why);
    }
    let tray = TRAY_WITHOUT_WINDOW.then(|| {
        let wake = Arc::clone(&wake);
        TrayHandle::spawn(move || wake.wake())
    });
    let can_hide = window_can_hide_here(|name| std::env::var_os(name));
    let notes = notify::Notifier::spawn(move || show(control::ToWindow::Show));
    let mut client = Client { desktop, tray, keychain, can_hide, closed_to_tray: false, shows, notes, wake: Arc::clone(&wake) };

    // A hidden start gives the tray a moment to come up, so the window knows whether it can stay
    // hidden (X11) or must open (no tray host). Where a window can't hide (Wayland), a tray that
    // is up means no window is made at all, so none flashes up. Windows and macOS make their
    // tray with the window (`TrayHandle::local`), so there is nothing to wait for here.
    let tray_up = hidden && client.tray.as_ref().is_some_and(|t| wait_until(Duration::from_secs(3), || t.is_ready()));
    let mut window = !(tray_up && !can_hide);
    // One pass per window: where a window can't hide, closing it ends the pass and the client
    // carries on in the tray until the window is asked for again.
    loop {
        if window {
            let result = {
                let (wake, client) = (Arc::clone(&wake), &mut client);
                eframe::run_native(
                    "CraftCloud",
                    options(!hidden),
                    Box::new(move |cc| {
                        wake.set_window(Some(cc.egui_ctx.clone()));
                        if client.tray.is_none() && !TRAY_WITHOUT_WINDOW {
                            let wake = Arc::clone(&wake);
                            client.tray = TrayHandle::local(move || wake.wake());
                        }
                        let cjk = read_cjk_font(&cc.egui_ctx);
                        let saved = cc.storage.and_then(|s| s.get_string(geometry::KEY));
                        Ok(Box::new(Window::new(client, cjk, !hidden, saved)))
                    }),
                )
            };
            wake.set_window(None);
            if let Err(e) = result {
                eprintln!("craftcloud: couldn't open the window: {e}");
                return ExitCode::FAILURE;
            }
            if !std::mem::take(&mut client.closed_to_tray) {
                return ExitCode::SUCCESS;
            }
        }
        if !client.wait_in_tray(&woken) {
            return ExitCode::SUCCESS;
        }
        hidden = false;
        window = true;
    }
}

/// What a start does when a CraftCloud may already be running on this settings folder.
#[derive(Debug, PartialEq, Eq)]
enum Start {
    /// None runs (or it has no window, as `craftcloud daemon`): this one runs.
    Run,
    /// A start at login (`--hidden`) while one runs: end quietly.
    LeaveIt,
    /// The running one showed its window: end.
    Shown,
}

/// The same on every platform: one client per settings folder, so a second start never gets as
/// far as "another CraftCloud is already syncing". `running` and `show` ask over the control
/// channel (loopback TCP, `control.json`), which works alike on Linux, macOS and Windows.
fn second_start(hidden: bool, running: impl FnOnce() -> bool, show: impl FnOnce() -> bool) -> Start {
    if hidden {
        if running() { Start::LeaveIt } else { Start::Run }
    } else if show() {
        Start::Shown
    } else {
        Start::Run
    }
}

/// Polls `ready` until it holds or `limit` passes.
fn wait_until(limit: Duration, ready: impl Fn() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    ready()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::{Sender, channel};
    use std::time::{Duration, Instant};

    use craftcloud_native::control::ToWindow;
    use craftcloud_native::http::{ThreadBackend, Waker};
    use craftcloud_native::settings;
    use craftcloud_ui_egui::desktop::DesktopApp;
    use egui::{ViewportId, ViewportInfo};

    use super::{Client, NativeHost, OnClose, SHOW, Window, logic_tick, notify, on_close, window_can_hide_here};

    /// A client without a tray, keychain or notifications, its settings in `dir`.
    fn client(dir: &std::path::Path) -> (Client, Sender<ToWindow>) {
        let waker: Waker = Arc::new(|| {});
        let host = NativeHost {
            net: ThreadBackend::new(Arc::clone(&waker)),
            store: settings::open_in(dir),
            waker,
            started: Instant::now(),
            sync: None,
            fresh: Arc::default(),
            quit: false,
            control: Arc::default(),
            folder: None,
            blocked: None,
        };
        let saved = host.store.get().0.clone();
        let desktop = DesktopApp::waiting_for_keychain(&egui::Context::default(), host, saved);
        let (tx, shows) = channel();
        let wake = Arc::new(super::Wake { window: std::sync::Mutex::new(None), nudge: std::sync::mpsc::sync_channel(1).0 });
        (Client { desktop, tray: None, keychain: None, can_hide: true, closed_to_tray: false, shows, notes: notify::Notifier::off(), wake }, tx)
    }

    /// One pass of the window's logic, as eframe runs it while the window is minimised (no `ui`).
    fn minimised_pass(window: &mut Window<'_>) -> Vec<egui::ViewportCommand> {
        let mut input = egui::RawInput::default();
        input.viewports.insert(ViewportId::ROOT, ViewportInfo { minimized: Some(true), ..ViewportInfo::default() });
        assert_eq!(input.viewports[&ViewportId::ROOT].visible(), Some(false), "eframe skips `ui` for this window");
        let out = egui::Context::default().run_logic(&input, |ctx| eframe::App::logic(window, ctx, &mut eframe::Frame::_new_kittest()));
        out.viewport_commands.get(&ViewportId::ROOT).cloned().unwrap_or_default()
    }

    /// One pass as eframe runs it on Windows for a window that isn't minimised, hidden or not:
    /// the window system says nothing about occlusion there, so eframe takes the window as drawn
    /// and runs `logic` and `ui`, then paints. Returns the viewport commands and how many shapes
    /// `ui` drew.
    fn windows_pass(window: &mut Window<'_>) -> (Vec<egui::ViewportCommand>, usize) {
        let info = ViewportInfo { minimized: Some(false), ..ViewportInfo::default() };
        assert_eq!(info.visible(), None, "eframe reads this as drawn and runs `ui` too");
        drawn_pass(window, info)
    }

    /// The same on X11 once the window has been mapped: not minimised and not covered
    /// (`VisibilityNotify`). Unmapping it sends no new one, so a hidden window still reads so.
    fn x11_pass(window: &mut Window<'_>) -> (Vec<egui::ViewportCommand>, usize) {
        let info = ViewportInfo { minimized: Some(false), occluded: Some(false), ..ViewportInfo::default() };
        assert_eq!(info.visible(), Some(true), "eframe draws this window");
        drawn_pass(window, info)
    }

    /// A pass that runs `logic` and `ui`, as eframe does for a window it takes as drawn.
    fn drawn_pass(window: &mut Window<'_>, info: ViewportInfo) -> (Vec<egui::ViewportCommand>, usize) {
        let mut input = egui::RawInput::default();
        input.viewports.insert(ViewportId::ROOT, info);
        let mut frame = eframe::Frame::_new_kittest();
        let out = egui::Context::default().run_ui(input, |ui| {
            eframe::App::logic(window, ui.ctx(), &mut frame);
            eframe::App::ui(window, ui, &mut frame);
        });
        // egui's own `SetTheme` from the first pass is not the window's doing.
        let commands = out.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
        let commands = commands.into_iter().filter(|c| !matches!(c, egui::ViewportCommand::SetTheme(_))).collect();
        let shapes = out.shapes.len();
        out.drop_without_applying_deltas();
        (commands, shapes)
    }

    /// #9: `craftcloud --hidden` on Windows showed the window. eframe makes every window hidden
    /// and shows it itself after the first frame it paints, whatever the viewport asked for; the
    /// pass's commands run after that, so a hidden window hides itself again in every pass eframe
    /// draws, and draws nothing in it.
    #[test]
    fn a_hidden_start_keeps_the_window_hidden_and_draws_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, show) = client(dir.path());
        client.tray = Some(crate::tray::TrayHandle::up_for_tests());
        let mut window = Window::new(&mut client, None, false, None);
        let (commands, shapes) = windows_pass(&mut window);
        assert_eq!(commands, [egui::ViewportCommand::Visible(false)], "hidden again, after eframe's own show");
        assert_eq!(shapes, 0, "a hidden window draws nothing");
        assert_eq!(windows_pass(&mut window).0, [egui::ViewportCommand::Visible(false)], "and stays so");
        // The tray's Show (or a second start): shown and drawn in the same pass, not hidden again.
        show.send(ToWindow::Show).unwrap();
        let (commands, shapes) = windows_pass(&mut window);
        assert_eq!(commands, SHOW);
        assert!(shapes > 0, "the frame that shows the window is a real one");
        let (commands, shapes) = windows_pass(&mut window);
        assert!(commands.is_empty(), "a shown window is left alone: {commands:?}");
        assert!(shapes > 0);
        // Where a hide is ignored (Wayland), a window that couldn't start in the tray opens after all.
        client.can_hide = false;
        let mut window = Window::new(&mut client, None, false, None);
        let (commands, shapes) = windows_pass(&mut window);
        assert!(commands.is_empty(), "{commands:?}");
        assert!(shapes > 0);
    }

    /// A start at login on X11 (seen under Xfce) opened the window: eframe shows it after its
    /// first frame (as on Windows, #9), and X11 then says the window is shown. The same rule hides
    /// it again, but only where there is a tray to come back from: with no tray host (stock GNOME
    /// on Xorg) a hidden window would be a client nobody can reach, so the window opens.
    #[test]
    fn a_hidden_start_stays_hidden_where_there_is_a_tray() {
        let hide = egui::ViewportCommand::Visible(false);
        let dir = tempfile::tempdir().unwrap();
        let (mut client, show) = client(dir.path());
        client.tray = Some(crate::tray::TrayHandle::up_for_tests());
        let mut window = Window::new(&mut client, None, false, None);
        assert_eq!(x11_pass(&mut window), (vec![hide.clone()], 0), "hidden again on the first pass, and nothing drawn");
        assert_eq!(x11_pass(&mut window), (vec![hide.clone()], 0), "unmapped, X11 still says shown: hidden again");
        show.send(ToWindow::Show).unwrap();
        assert_eq!(x11_pass(&mut window).0, SHOW, "the tray's Show still brings it back");
        assert!(x11_pass(&mut window).0.is_empty());

        // Started normally: left alone.
        let mut window = Window::new(&mut client, None, true, None);
        assert!(x11_pass(&mut window).0.is_empty());

        // No tray host: the window is the client, and opens.
        client.tray = None;
        let mut window = Window::new(&mut client, None, false, None);
        let (commands, shapes) = x11_pass(&mut window);
        assert!(commands.is_empty(), "{commands:?}");
        assert!(shapes > 0, "drawn as a shown window");
        assert!(window.shown, "counted as shown, so it ticks and closes as a shown window");
    }

    #[test]
    fn a_second_start_shows_the_running_client_instead() {
        use super::{Start, second_start};
        use craftcloud_native::control;

        let dir = tempfile::tempdir().unwrap();
        let (mut client, show) = client(dir.path());
        let on_show = Arc::new(move |ask| {
            let _ = show.send(ask);
        });
        let server = control::serve_window(dir.path(), Arc::default(), on_show).unwrap();
        let start = |hidden| second_start(hidden, || control::is_running(dir.path()), || control::show_window(dir.path()));
        // From the Start menu or a shortcut: the running client's window comes up, this one ends.
        assert_eq!(start(false), Start::Shown);
        let mut window = Window::new(&mut client, None, true, None);
        assert_eq!(minimised_pass(&mut window), SHOW, "the running client brings its window back");
        // At login: nothing shows.
        assert_eq!(start(true), Start::LeaveIt);
        assert!(minimised_pass(&mut window).is_empty());
        drop(server);
        assert_eq!(start(false), Start::Run, "none running");
        assert_eq!(start(true), Start::Run);
        // The command line's daemon has no window: the desktop client starts (and says the folder is taken).
        let _headless = control::serve(dir.path(), Arc::default()).unwrap();
        assert_eq!(start(false), Start::Run);
        assert_eq!(start(true), Start::LeaveIt);
    }

    #[test]
    fn a_minimised_window_saves_its_last_good_place() {
        struct Storage(Option<String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                (key == super::geometry::KEY).then(|| self.0.clone()).flatten()
            }
            fn set_string(&mut self, key: &str, value: String) {
                if key == super::geometry::KEY {
                    self.0 = Some(value);
                }
            }
            fn remove_string(&mut self, _key: &str) {}
            fn flush(&mut self) {}
        }
        let dir = tempfile::tempdir().unwrap();
        let (mut client, _show) = client(dir.path());
        let mut window = Window::new(&mut client, None, true, Some("from the last run".into()));
        minimised_pass(&mut window);
        // eframe writes the minimised window's 0×0 at (-32000, -32000), then asks the app.
        let mut storage = Storage(Some("minimised".into()));
        eframe::App::save(&mut window, &mut storage);
        assert_eq!(storage.0.as_deref(), Some("from the last run"));
    }

    #[test]
    fn a_waiting_client_takes_a_sign_in_saved_on_the_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, _show) = client(dir.path());
        assert!(!client.desktop.wants_keychain(), "no server yet: the Connect sheet");
        assert_eq!(client.take_sign_in(), None, "nothing saved yet");
        // `craftcloud login http://127.0.0.1:8093`, in another process: the server goes in the settings.
        let mut cli = settings::open_in(dir.path());
        cli.set(settings::Prefs(craftcloud_core::sync::ClientSettings { server: "http://127.0.0.1:8093".into(), ..Default::default() }));
        cli.flush().unwrap();
        let (server, settings_dir) = client.take_sign_in().unwrap();
        assert_eq!((server.as_str(), settings_dir.as_path()), ("http://127.0.0.1:8093", dir.path()), "its keychain entry is read next");
        assert!(client.desktop.wants_keychain());
        assert_eq!(client.desktop.host.store.get().0.server, "http://127.0.0.1:8093");
        assert_eq!(client.take_sign_in(), None, "taken once");
    }

    #[test]
    fn show_brings_back_a_minimised_window() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, show) = client(dir.path());
        let mut window = Window::new(&mut client, None, true, None);
        assert!(!minimised_pass(&mut window).contains(&egui::ViewportCommand::Visible(true)), "nothing asked yet");
        // The tray's Show, a second start or a notification clicked.
        show.send(ToWindow::Show).unwrap();
        let commands = minimised_pass(&mut window);
        assert_eq!(commands, SHOW, "out of the taskbar, shown and focused, in that order");
        assert_eq!(SHOW[0], egui::ViewportCommand::Minimized(false));
    }

    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).into())
    }

    #[test]
    fn only_a_window_that_is_not_drawn_ticks_every_second() {
        // Drawn, a tick is a full frame (#3); hidden or minimised, it keeps the tray's Show working.
        assert_eq!(logic_tick(true, true), None);
        assert_eq!(logic_tick(false, true), Some(Duration::from_secs(1)));
        assert_eq!(logic_tick(true, false), Some(Duration::from_secs(1)));
    }

    #[test]
    fn a_wayland_window_closes_into_the_tray() {
        let linux = cfg!(all(unix, not(target_os = "macos")));
        assert_eq!(window_can_hide_here(env(&[("WAYLAND_DISPLAY", "wayland-1"), ("DISPLAY", ":0")])), !linux);
        assert_eq!(window_can_hide_here(env(&[("WAYLAND_SOCKET", "3")])), !linux);
        assert!(window_can_hide_here(env(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", ":0")])));
        assert_eq!(on_close(false, false), OnClose::ToTray);
        assert_eq!(on_close(true, false), OnClose::Hide);
        assert_eq!(on_close(true, true), OnClose::Minimize);
    }
}
