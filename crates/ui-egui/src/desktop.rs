//! The desktop client's screens (design frames `v2-desktop-*`): connect to a server, sign in
//! through the browser and choose folders (sheets over the empty window), then the main window
//! with its ACTIVITY and FOLDERS cells, and the Settings and Conflict sheets over it.

mod activity;
mod folders;
mod settings;
mod signin;

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::{self, DevicePoll, DeviceStart, Me, Tokens};
use craftcloud_core::api::dav::{self, Entry};
use craftcloud_core::http::{HttpRequest, Method};
use craftcloud_core::sync::{Appearance, ClientSettings, ConflictCopy, Keep, SyncPlan, SyncProgress, SyncState};
use egui::{Color32, Ui};

use crate::backend::{Backend, Net, NetResult};
use crate::chrome::{self, desk};
use crate::connect::ConnectScreen;
use crate::theme::Tokens as Look;

/// What the platform knows about a local file (for the Conflict sheet).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFacts {
    pub size: u64,
    /// When it was last written, local time: "today 13:58" or "2026-11-12 09:10".
    pub modified: String,
}

/// The wall clock, for "synced 2 min ago" and "today".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallClock {
    /// Seconds since 1970 (UTC).
    pub unix: i64,
    /// The local date, "2026-11-13".
    pub date: String,
}

/// What the desktop screens need from the platform, beyond HTTP.
pub trait DesktopHost: Backend {
    /// "CraftCloud on Laptop": shown on the approval page and in the device list.
    fn client_name(&self) -> String;
    /// This computer's own name ("Laptop"), the default device name in conflict copies.
    fn computer_name(&self) -> String;
    /// The suggested local folder, `~/CraftCloud`.
    fn default_folder(&self) -> String;
    /// Saves the settings and puts the tokens in the keychain.
    fn save_session(&mut self, settings: &ClientSettings, tokens: &Tokens) -> Result<(), String>;
    /// Starts (or restarts) syncing with these settings.
    fn start_sync(&mut self, plan: SyncPlan, tokens: Tokens);
    fn progress(&self) -> SyncProgress;
    /// The newest tokens: the sync refreshes them, so the ones from the sign-in run out.
    fn fresh_tokens(&self) -> Option<Tokens>;
    /// Runs a sync cycle now instead of at the next interval.
    fn sync_now(&mut self);
    /// Carries out deletions that were paused because there were so many.
    fn confirm_deletes(&mut self);
    /// Brings back the files whose deletion was paused, instead of deleting them.
    fn discard_deletes(&mut self);
    /// The sync folder is gone and the person chose to start over: makes it again, empty, and
    /// syncs (everything downloads). The sync never does this on its own.
    fn recreate_folder(&mut self);
    /// Stops (`true`) or resumes (`false`) syncing.
    fn pause(&mut self, paused: bool);
    /// Saves new folder choices and syncs those from now on.
    fn change_folders(&mut self, settings: &ClientSettings) -> Result<(), String>;
    /// Saves choices that don't change what syncs (device name, appearance).
    fn save_prefs(&mut self, settings: &ClientSettings) -> Result<(), String>;
    /// Settles a conflict copy.
    fn resolve(&mut self, conflict: &ConflictCopy, keep: Keep);
    /// Size and time of a file in the local folder, by cloud path.
    fn local_file(&self, path: &str) -> Option<FileFacts>;
    fn starts_at_login(&self) -> bool;
    fn set_start_at_login(&mut self, on: bool) -> Result<(), String>;
    fn open_folder(&mut self, path: &str);
    /// Opens `url` in the system browser, as [`Backend::open_url`], and says whether that
    /// worked, so the sign-in can tell the person to open the address themselves.
    fn open_browser(&mut self, url: &str) -> bool {
        self.open_url(url);
        true
    }
    /// Opens the local copy of a synced file (cloud path) with its app.
    fn open_file(&mut self, path: &str);
    fn clock(&self) -> WallClock;
    /// Forgets the tokens and the server (the files stay on disk).
    fn sign_out(&mut self);
    /// Ends the client (the window and the tray).
    fn quit(&mut self);
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Config,
    Authorize,
    Poll,
    ListRoot,
    Me,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Activity,
    Folders,
}

/// A sheet over the main window.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Overlay {
    Settings,
    /// The conflict copy being decided.
    Conflict(String),
}

/// The folder choice, on the Choose sheet and the Folders tab: cloud folders with their ticks,
/// and the files at the top level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Choice {
    folders: Vec<(String, bool)>,
    root_files: bool,
}

impl Choice {
    /// The folders switched off (what [`ClientSettings::excluded`] keeps).
    fn unchosen(&self) -> Vec<String> {
        let mut off: Vec<String> = self.folders.iter().filter(|(_, on)| !*on).map(|(n, _)| n.clone()).collect();
        off.sort();
        off
    }

    /// Whether this choice differs from what syncs now.
    fn differs_from(&self, s: &ClientSettings) -> bool {
        let mut excluded = s.excluded.clone();
        excluded.sort();
        s.needs_migration() || self.unchosen() != excluded || self.root_files != s.sync_root_files
    }

    /// The settings with this choice.
    fn apply_to(&self, s: &ClientSettings) -> ClientSettings {
        ClientSettings { excluded: self.unchosen(), folders: Vec::new(), sync_root_files: self.root_files, ..s.clone() }
    }
}

enum Screen {
    /// Signed in before: waiting for the sign-in to come out of the system keychain, which can
    /// take a while (a locked keychain asks to be unlocked).
    Keychain,
    Connect(ConnectScreen),
    Browser {
        start: DeviceStart,
        interval: f64,
        next_poll: f64,
        until: f64,
        /// Whether the system browser opened (`false`: the sheet asks the person to open the address).
        opened: bool,
    },
    /// Step 3: what to sync, and where.
    Choose {
        choice: Choice,
        local: String,
        /// The local folder is being typed in.
        changing: bool,
    },
    Main(Tab),
}

pub struct DesktopApp<H: DesktopHost> {
    pub host: H,
    screen: Screen,
    overlay: Option<Overlay>,
    server: Option<ServerUrl>,
    tokens: Option<Tokens>,
    settings: ClientSettings,
    net: Net<Key>,
    /// The theme is installed on the context that draws (set on the first frame).
    themed: Option<egui::Context>,
    /// The appearance last given to the context.
    applied: Option<Appearance>,
    error: Option<String>,
    /// The Folders tab's edit, once the cloud folders are listed.
    editing: Option<Choice>,
    /// The Folders tab's order.
    sort: folders::Sort,
    /// Who is signed in (asked once the main window shows).
    me: Option<Me>,
    /// "This device" in Settings, as typed.
    device: String,
    /// The address typed without a scheme was tried with https://; this is the http:// one to
    /// try when that doesn't answer as CraftCloud.
    http_fallback: Option<ServerUrl>,
}

impl<H: DesktopHost> DesktopApp<H> {
    /// With saved settings and tokens the client goes straight to the main window and syncs.
    /// Starts with the sign-in still to come from the keychain ([`DesktopApp::keychain_answered`])
    /// when this computer was signed in before, and on the Connect screen otherwise.
    pub fn waiting_for_keychain(ctx: &egui::Context, host: H, settings: ClientSettings) -> Self {
        let signed_in_before = ServerUrl::parse(&settings.server).is_ok() && !settings.local_folder.is_empty();
        let mut app = Self::new(ctx, host, settings, None);
        if signed_in_before {
            app.screen = Screen::Keychain;
        }
        app
    }

    /// Whether the sign-in is still to come from the keychain.
    pub fn wants_keychain(&self) -> bool {
        matches!(self.screen, Screen::Keychain)
    }

    /// The keychain's answer: start syncing with the saved sign-in, or ask to connect again.
    pub fn keychain_answered(&mut self, tokens: Option<Tokens>) {
        if !self.wants_keychain() {
            return;
        }
        match tokens {
            Some(t) => {
                self.host.start_sync(SyncPlan::from(&self.settings), t.clone());
                self.tokens = Some(t);
                self.screen = Screen::Main(Tab::Activity);
            }
            None => {
                self.error = Some("The saved sign-in isn't in the keychain any more. Connect again.".into());
                self.screen = Screen::Connect(ConnectScreen::new(self.settings.server.clone()));
            }
        }
    }

    pub fn new(ctx: &egui::Context, mut host: H, settings: ClientSettings, tokens: Option<Tokens>) -> Self {
        crate::theme::install(ctx);
        let server = ServerUrl::parse(&settings.server).ok();
        let screen = match (&server, &tokens) {
            (Some(_), Some(t)) if !settings.local_folder.is_empty() => {
                host.start_sync(SyncPlan::from(&settings), t.clone());
                Screen::Main(Tab::Activity)
            }
            _ => Screen::Connect(ConnectScreen::new(settings.server.clone())),
        };
        let device = if settings.device_name.is_empty() { host.computer_name() } else { settings.device_name.clone() };
        Self {
            host,
            screen,
            overlay: None,
            server,
            tokens,
            settings,
            net: Net::default(),
            themed: None,
            applied: None,
            error: None,
            editing: None,
            sort: folders::Sort::Name,
            me: None,
            device,
            http_fallback: None,
        }
    }

    /// The saved choices (server, local folder, folders).
    pub fn settings(&self) -> &ClientSettings {
        &self.settings
    }

    /// Opens the Settings sheet (the tray's "Settings…").
    pub fn show_settings(&mut self) {
        if matches!(self.screen, Screen::Main(_)) {
            self.overlay = Some(Overlay::Settings);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if self.themed.as_ref() != Some(ui.ctx()) {
            crate::theme::install(ui.ctx());
            self.themed = Some(ui.ctx().clone());
            self.applied = None;
        }
        if self.applied != Some(self.settings.appearance) {
            ui.ctx().set_theme(match self.settings.appearance {
                Appearance::System => egui::ThemePreference::System,
                Appearance::Dark => egui::ThemePreference::Dark,
                Appearance::Light => egui::ThemePreference::Light,
            });
            self.applied = Some(self.settings.appearance);
        }
        let mut answers = self.host.poll();
        for (key, result) in self.net.receive(&mut answers) {
            self.on_answer(key, result);
        }
        self.tick(ui.ctx());
        let progress = self.host.progress();
        let t = Look::get(ui.ctx());
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, t.bg);
        let (header_rect, middle, footer_rect) = chrome::page_rects(full);

        let tab = match self.screen {
            Screen::Main(tab) => Some(tab),
            _ => None,
        };
        let nav: Vec<(&str, bool)> = match tab {
            Some(tab) => vec![
                ("Activity", tab == Tab::Activity && self.overlay.is_none()),
                ("Folders", tab == Tab::Folders && self.overlay.is_none()),
                ("Settings", self.overlay == Some(Overlay::Settings)),
            ],
            None => Vec::new(),
        };
        let (status, dot) = self.header_status(&progress, &t);
        match desk::header(ui, header_rect, &nav, &status, dot) {
            Some(0) => self.go(Tab::Activity),
            Some(1) => self.go(Tab::Folders),
            Some(2) => self.overlay = Some(Overlay::Settings),
            _ => {}
        }
        let footer = self.footer_text(&progress);
        let link = self.server.is_some().then_some("Open in browser");
        if desk::footer(ui, footer_rect, &footer, link) {
            let url = self.settings.server.clone();
            self.host.open_url(&url);
        }

        ui.scope_builder(egui::UiBuilder::new().max_rect(middle), |ui| {
            ui.set_clip_rect(middle);
            if let Some(tab) = tab {
                let content = middle.shrink(chrome::PAD);
                ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| match tab {
                    Tab::Activity => self.activity_ui(ui, &progress),
                    Tab::Folders => self.folders_ui(ui, &progress),
                });
            }
        });
        // Sheets cover the whole window, header and footer included.
        match &self.screen {
            Screen::Keychain => signin::keychain_ui(ui, middle),
            Screen::Connect(_) => self.connect_ui(ui, full),
            Screen::Browser { .. } => self.browser_ui(ui, full),
            Screen::Choose { .. } => self.choose_ui(ui, full),
            Screen::Main(_) => match self.overlay.clone() {
                Some(Overlay::Settings) => self.settings_ui(ui, full, &progress),
                Some(Overlay::Conflict(copy)) => self.conflict_ui(ui, full, &progress, &copy),
                None => {}
            },
        }
    }

    fn go(&mut self, tab: Tab) {
        self.overlay = None;
        if tab == Tab::Folders && self.screen != Screen::Main(Tab::Folders) {
            // Fresh from the cloud each time the tab opens.
            self.editing = None;
        }
        self.screen = Screen::Main(tab);
    }

    fn header_status(&self, p: &SyncProgress, t: &Look) -> (String, Color32) {
        match &self.screen {
            Screen::Main(_) => status_line(p, t),
            Screen::Choose { .. } => ("Signed in".into(), t.accent_ink),
            Screen::Keychain => ("Starting…".into(), t.muted),
            _ => ("Not connected".into(), t.faint),
        }
    }

    fn footer_text(&self, p: &SyncProgress) -> String {
        let host = self.server.as_ref().map(host_of).unwrap_or_default();
        let lead = match &self.screen {
            Screen::Main(_) => match &p.state {
                SyncState::UpToDate => match p.last_synced {
                    Some(at) => format!("Synced {}", ago(self.host.clock().unix - at)),
                    None => "Synced".into(),
                },
                s if s.is_busy() => "Syncing".into(),
                SyncState::Paused => "Paused".into(),
                SyncState::Offline(_) => "Offline".into(),
                SyncState::SignedOut => "Signed out".into(),
                _ => "Waiting".into(),
            },
            Screen::Choose { .. } => {
                let who = self.me.as_ref().map(|m| format!(" as {}", m.user.username)).unwrap_or_default();
                format!("Signed in{who}")
            }
            Screen::Keychain => "Starting".into(),
            _ => return "Not connected".into(),
        };
        if host.is_empty() { lead } else { format!("{lead} · {host}") }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = self.host.now();
        if let Screen::Browser { start, next_poll, until, .. } = &self.screen {
            if now >= *until {
                self.error = Some("The sign-in code ran out. Start again.".into());
                self.screen = Screen::Connect(ConnectScreen::new(self.settings.server.clone()));
                return;
            }
            if now >= *next_poll && !self.net.is_waiting(&Key::Poll) {
                let req = auth::device_token(self.server_ref(), &start.device_code);
                self.net.send(&mut self.host, Key::Poll, req);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        if matches!(self.screen, Screen::Main(_)) && self.settings.needs_migration() && self.editing.is_none() && !self.net.is_waiting(&Key::ListRoot)
        {
            self.list_root();
        }
        if matches!(self.screen, Screen::Main(_))
            && self.me.is_none()
            && !self.net.is_waiting(&Key::Me)
            && let Some(req) = self.authorized(auth::me)
        {
            self.net.send(&mut self.host, Key::Me, req);
        }
        if self.net.is_busy() || matches!(self.screen, Screen::Main(_)) {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    /// A request with the newest tokens, when signed in.
    fn authorized(&self, make: impl FnOnce(&ServerUrl) -> HttpRequest) -> Option<HttpRequest> {
        let tokens = self.host.fresh_tokens().or_else(|| self.tokens.clone())?;
        Some(tokens.authorize(make(self.server.as_ref()?)))
    }

    #[allow(clippy::expect_used)]
    fn server_ref(&self) -> &ServerUrl {
        // Every screen after Connect has a server; Connect never sends.
        self.server.as_ref().expect("a server is chosen before any request")
    }

    fn on_answer(&mut self, key: Key, result: NetResult) {
        let resp = match result {
            Ok(r) => r,
            Err(e) => {
                // A failed poll is retried on the next interval; who is signed in, on the next frame.
                if key == Key::Config
                    && let Some(http) = self.http_fallback.take()
                {
                    self.ask_config(http);
                } else if key == Key::Me {
                    self.me = None;
                } else if key != Key::Poll {
                    self.error = Some(e);
                }
                return;
            }
        };
        match key {
            Key::Config => {
                if craftcloud_core::RuntimeConfig::from_json(&resp.body).is_ok() && resp.is_success() {
                    self.http_fallback = None;
                    let req = auth::device_authorize(self.server_ref(), &self.host.client_name());
                    self.net.send(&mut self.host, Key::Authorize, req);
                } else if let Some(http) = self.http_fallback.take() {
                    self.ask_config(http);
                } else {
                    self.error = Some("That address answers, but it isn't a CraftCloud server.".into());
                }
            }
            Key::Authorize => match auth::parse_device_authorize(&resp) {
                Ok(start) => {
                    let now = self.host.now();
                    let opened = self.host.open_browser(start.browser_url());
                    let interval = start.interval.max(1) as f64;
                    let until = now + start.expires_in as f64;
                    self.screen = Screen::Browser { start, interval, next_poll: now + interval, until, opened };
                }
                Err(e) => self.error = Some(e.to_string()),
            },
            Key::Poll => self.on_poll(&resp),
            Key::ListRoot => match dav::parse_list(&resp) {
                Ok(entries) if matches!(self.screen, Screen::Main(_)) => {
                    let mut folders = top_folders(&entries);
                    // 0.1.0 kept a list of the folders that sync: turn it into the excluded ones.
                    let names: Vec<String> = folders.iter().map(|(n, _)| n.clone()).collect();
                    if let Some(next) = self.settings.migrate(&names) {
                        match self.host.change_folders(&next) {
                            Ok(()) => self.settings = next,
                            Err(e) => self.error = Some(format!("Couldn't save: {e}")),
                        }
                    }
                    // The Folders tab: tick what syncs now.
                    for (name, on) in &mut folders {
                        *on = craftcloud_core::sync::in_plan(&SyncPlan::from(&self.settings), name, true);
                    }
                    self.editing = Some(Choice { folders, root_files: self.settings.sync_root_files });
                }
                Ok(entries) => {
                    let local = self.host.default_folder();
                    self.screen = Screen::Choose { choice: Choice { folders: top_folders(&entries), root_files: true }, local, changing: false };
                }
                Err(e) => self.error = Some(e.to_string()),
            },
            // Kept even when it fails (`None` asks again): nothing else needs it.
            Key::Me => self.me = auth::parse_me(&resp).ok().flatten().or_else(|| self.me.take()),
        }
    }

    fn on_poll(&mut self, resp: &craftcloud_core::http::HttpResponse) {
        let now = self.host.now();
        let Screen::Browser { interval, next_poll, .. } = &mut self.screen else { return };
        match auth::parse_device_token(resp) {
            Ok(DevicePoll::Pending) => *next_poll = now + *interval,
            Ok(DevicePoll::SlowDown) => {
                *interval += 5.0;
                *next_poll = now + *interval;
            }
            Ok(DevicePoll::Approved(tokens)) => {
                let req = tokens.authorize(dav::list(self.server_ref(), ""));
                let me = tokens.authorize(auth::me(self.server_ref()));
                self.tokens = Some(tokens);
                self.net.send(&mut self.host, Key::ListRoot, req);
                self.net.send(&mut self.host, Key::Me, me);
            }
            Ok(DevicePoll::Denied) => {
                self.error = Some("Sign-in was denied in the browser.".into());
                self.screen = Screen::Connect(ConnectScreen::new(self.settings.server.clone()));
            }
            Ok(DevicePoll::Expired) => {
                self.error = Some("The sign-in code ran out. Start again.".into());
                self.screen = Screen::Connect(ConnectScreen::new(self.settings.server.clone()));
            }
            Err(e) => {
                self.error = Some(e.to_string());
                *next_poll = now + *interval;
            }
        }
    }

    /// Starts from the Connect sheet with this address.
    fn check_server(&mut self, url: ServerUrl) {
        // Typed without a scheme: https:// first, then http:// (a server on a LAN port).
        let typed = match &self.screen {
            Screen::Connect(c) => c.address.trim().to_owned(),
            _ => String::new(),
        };
        self.http_fallback = (!typed.contains("://") && !url.is_plain_http()).then(|| ServerUrl::parse(&format!("http://{typed}")).ok()).flatten();
        self.ask_config(url);
    }

    fn ask_config(&mut self, url: ServerUrl) {
        self.error = None;
        self.settings.server = url.to_string();
        let req = HttpRequest::new(Method::Get, url.join("craftcloud/config.json"));
        self.server = Some(url);
        self.net.send(&mut self.host, Key::Config, req);
    }

    fn begin_sync(&mut self) {
        let Screen::Choose { choice, local, .. } = &self.screen else { return };
        let Some(tokens) = self.tokens.clone() else { return };
        self.settings.local_folder = local.trim().to_owned();
        self.settings = choice.apply_to(&self.settings);
        if let Err(e) = self.host.save_session(&self.settings, &tokens) {
            self.error = Some(format!("Couldn't save the sign-in: {e}"));
            return;
        }
        self.error = None;
        self.host.start_sync(SyncPlan::from(&self.settings), tokens);
        self.screen = Screen::Main(Tab::Activity);
    }

    fn sign_out(&mut self) {
        self.host.sign_out();
        self.tokens = None;
        self.me = None;
        self.overlay = None;
        self.editing = None;
        self.settings = ClientSettings {
            server: self.settings.server.clone(),
            device_name: self.settings.device_name.clone(),
            appearance: self.settings.appearance,
            ..ClientSettings::default()
        };
        self.screen = Screen::Connect(ConnectScreen::new(self.settings.server.clone()));
    }

    /// Asks the cloud for its top-level folders (the Folders tab), once.
    fn list_root(&mut self) {
        if self.net.is_waiting(&Key::ListRoot) {
            return;
        }
        if let Some(req) = self.authorized(|s| dav::list(s, "")) {
            self.error = None;
            self.net.send(&mut self.host, Key::ListRoot, req);
        }
    }
}

impl PartialEq for Screen {
    fn eq(&self, other: &Self) -> bool {
        matches!((self, other), (Screen::Main(a), Screen::Main(b)) if a == b)
    }
}

/// The folders at the top of a root listing (not CraftCloud's own).
fn top_folders(entries: &[Entry]) -> Vec<(String, bool)> {
    entries
        .iter()
        .filter(|e| e.is_folder && !e.path.is_empty() && !e.path.contains('/') && !dav::is_craftcloud_internal(&e.path))
        .map(|e| (e.path.clone(), true))
        .collect()
}

/// `cloud.example.org` for `https://cloud.example.org/`.
fn host_of(url: &ServerUrl) -> String {
    let s = url.as_str();
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    s.trim_end_matches('/').to_owned()
}

/// "just now", "2 min ago", "3 h ago", "2 days ago".
fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        0..60 => "just now".into(),
        s @ 60..3_600 => format!("{} min ago", s / 60),
        s @ 3_600..86_400 => format!("{} h ago", s / 3_600),
        s => match s / 86_400 {
            1 => "a day ago".into(),
            d => format!("{d} days ago"),
        },
    }
}

/// The sync state in a few words, for the tray.
pub fn status_text(p: &SyncProgress) -> String {
    status_line(p, &Look::DARK).0
}

/// The header's status cell: a few words and the dot's colour (design frame `v2-desktop-states`).
fn status_line(p: &SyncProgress, t: &Look) -> (String, Color32) {
    let conflicts = p.conflicts.len();
    match &p.state {
        SyncState::Idle => ("Starting…".into(), t.muted),
        SyncState::Scanning => ("Checking…".into(), t.accent_ink),
        SyncState::Downloading if p.files_total == 0 => ("Checking…".into(), t.accent_ink),
        SyncState::Downloading => (format!("Syncing {} of {}", (p.files_done + 1).min(p.files_total), p.files_total), t.accent_ink),
        SyncState::UpToDate if !p.no_room.is_empty() => (format!("{} not synced", p.no_room.len() + p.not_synced.len()), t.warn),
        SyncState::UpToDate if !p.not_synced.is_empty() => (format!("Up to date · {} not synced", p.not_synced.len()), t.warn),
        SyncState::UpToDate if p.unsynced.values().any(|s| *s == craftcloud_core::sync::FileStatus::Error) => {
            let n = p.unsynced.values().filter(|s| **s == craftcloud_core::sync::FileStatus::Error).count();
            (format!("Up to date · {n} couldn't sync"), t.warn)
        }
        SyncState::UpToDate if conflicts == 1 => ("1 conflict".into(), t.warn),
        SyncState::UpToDate if conflicts > 1 => (format!("{conflicts} conflicts"), t.warn),
        SyncState::UpToDate => ("Up to date".into(), t.good),
        SyncState::Paused => ("Paused".into(), t.muted),
        SyncState::Offline(_) => ("Offline".into(), t.bad),
        SyncState::SignedOut => ("Signed out".into(), t.warn),
        SyncState::MassDelete { .. } => ("Waiting for you".into(), t.warn),
        SyncState::FolderMissing => ("Folder missing".into(), t.bad),
        SyncState::Failed(_) => ("Error".into(), t.bad),
    }
}

#[cfg(all(test, feature = "shots"))]
mod shots;
#[cfg(test)]
mod tests;
