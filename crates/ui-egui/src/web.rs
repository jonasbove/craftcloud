//! The web frontend's pages: first-run setup, sign-in, the `/device` approval page the desktop
//! client sends people to, a signed-in home, and the admin pages ([`admin`]). Design frames:
//! `v2-web-*`.
//!
//! The browser keeps the session in OxiCloud's HttpOnly cookies; this code never sees a token.

use craftcloud_core::api::{ApiError, auth};
use craftcloud_core::catalog;

use crate::app_grid::{Card, app_grid};
use craftcloud_core::http::{HttpRequest, Method};
use craftcloud_core::server_settings::ServerSettings;
use craftcloud_core::{RuntimeConfig, ServerUrl};
use egui::{RichText, Ui};

use crate::backend::{Backend, Net, NetResult};
use crate::chrome::{self, Icon, Kind};
use crate::theme::Tokens;
use crate::widgets::{error_line, logo, muted};

mod admin;
mod apps_menu;
mod files;
mod links;
mod oidc;
mod pages;
pub use admin::{Clock, Tab as AdminTab};
mod share;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Status,
    Config,
    Me,
    Setup,
    Login,
    Logout,
    VerifyInfo,
    Verify(bool),
    Catalog,
    LoadSettings,
    SaveSettings,
    Devices,
}

/// Where to go after signing in.
#[derive(Clone, Debug, PartialEq, Eq)]
enum After {
    Home,
    Device,
}

#[derive(Debug, PartialEq, Eq)]
enum Page {
    Loading,
    Setup(u8),
    Login(After),
    Device,
    Home,
    Files,
    Admin(AdminTab),
    /// Your share links (`/links`), from the account menu.
    Links,
    /// The public page of a share link (`/s/<token>`).
    Share,
    Failed(String),
}

#[derive(Default)]
struct Form {
    name: String,
    email: String,
    password: String,
    password2: String,
    code: String,
}

pub struct WebApp<B: Backend> {
    pub backend: B,
    server: ServerUrl,
    /// The page the browser asked for, e.g. `/device`.
    path: String,
    page: Page,
    net: Net<Key>,
    /// The theme is installed on the context that draws (set on the first frame).
    themed: Option<egui::Context>,
    status: Option<auth::InitStatus>,
    config: Option<RuntimeConfig>,
    config_done: bool,
    me: Option<auth::Me>,
    me_done: bool,
    form: Form,
    error: Option<String>,
    device: Option<auth::DeviceRequest>,
    device_result: Option<bool>,
    /// The Craft web apps this image bundles: `None` while loading, empty on the slim image.
    catalog: Option<Vec<catalog::WebApp>>,
    web_apps: WebAppChoice,
    files: files::Files,
    admin: admin::Admin,
    sso: oidc::Sso,
    /// The signed-in person's desktop clients (home page), `None` until loaded.
    devices: Option<Vec<auth::Device>>,
    /// A sign-in just succeeded; if `/api/auth/me` still says nobody, the browser dropped the
    /// session cookie.
    signing_in: bool,
    /// The browser's clock and time zone, for "Today 18:02" (unset in tests: UTC dates).
    clock: Option<Clock>,
    share: Option<share::SharePage>,
    links: links::Links,
    /// The account menu under the person's name in the header is open.
    account_open: bool,
    /// The Apps menu (the Craft web apps), from the header's APPS entry.
    apps_menu: apps_menu::AppsMenu,
    /// The colour theme's id, as this browser keeps it ([`THEME_PREF`]); empty for CraftCloud's
    /// own look.
    theme: String,
    /// The colour theme last given to the context.
    applied_theme: Option<String>,
}

/// The key this browser keeps the colour theme under (localStorage on the web).
pub const THEME_PREF: &str = "craftcloud.theme";

/// The web-apps choice being edited (setup step 4, Admin › Server).
#[derive(Default)]
struct WebAppChoice {
    on: bool,
    disabled: Vec<String>,
    loaded: bool,
}

impl<B: Backend> WebApp<B> {
    /// `server` is the page's own origin; `path` and `code` come from its URL.
    pub fn new(ctx: &egui::Context, mut backend: B, server: ServerUrl, path: &str, code: Option<String>) -> Self {
        crate::theme::install(ctx);
        let mut net = Net::default();
        net.send(&mut backend, Key::Status, auth::status(&server));
        net.send(
            &mut backend,
            Key::Config,
            craftcloud_core::http::HttpRequest::new(craftcloud_core::http::Method::Get, server.join("craftcloud/config.json")),
        );
        net.send(&mut backend, Key::Me, auth::me(&server));
        let theme = backend.load_pref(THEME_PREF).unwrap_or_default();
        let form = Form { code: code.unwrap_or_default(), ..Form::default() };
        let files = files::Files::new(server.clone());
        let links = links::Links::new(server.clone());
        let share = path
            .starts_with("/s/")
            .then(|| craftcloud_core::api::shares::token_of(path))
            .flatten()
            .map(|token| share::SharePage::new(&mut backend, server.clone(), token));
        Self {
            backend,
            server,
            path: path.to_owned(),
            page: Page::Loading,
            net,
            themed: None,
            status: None,
            config: None,
            config_done: false,
            me: None,
            me_done: false,
            form,
            error: None,
            device: None,
            device_result: None,
            catalog: None,
            web_apps: WebAppChoice::default(),
            files,
            admin: admin::Admin::default(),
            sso: oidc::Sso::default(),
            devices: None,
            signing_in: false,
            clock: None,
            share,
            links,
            account_open: false,
            apps_menu: apps_menu::AppsMenu::default(),
            theme,
            applied_theme: None,
        }
    }

    /// The browser's clock: seconds since 1970 (UTC) and the minutes its time zone adds to UTC.
    pub fn set_clock(&mut self, now_unix: i64, utc_offset_minutes: i32) {
        self.clock = Some(Clock { now_unix, utc_offset_minutes });
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if self.themed.as_ref() != Some(ui.ctx()) {
            crate::theme::install(ui.ctx());
            self.themed = Some(ui.ctx().clone());
            self.applied_theme = None;
        }
        // On the first frame and when it changes; CraftCloud's own look reads no theme file.
        if self.applied_theme.as_deref() != Some(self.theme.as_str()) {
            crate::theme::set_palette(ui.ctx(), crate::themes::Palette::for_id(&self.theme));
            self.applied_theme = Some(self.theme.clone());
        }
        let mut answers = self.backend.poll();
        for (key, result) in self.net.receive(&mut answers) {
            self.on_answer(key, result);
        }
        self.files.receive(&mut self.backend, &mut answers);
        self.links.receive(&mut self.backend, &mut answers);
        if let Some(share) = &mut self.share {
            share.receive(&mut self.backend, &mut answers);
        }
        for (key, result) in self.admin.net.receive(&mut answers) {
            self.on_admin_answer(key, result);
        }
        for (key, result) in self.sso.net.receive(&mut answers) {
            self.on_sso_answer(key, result);
        }
        if self.net.is_busy() || self.admin.net.is_busy() || self.sso.net.is_busy() || self.links.is_busy() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let t = Tokens::get(ui.ctx());
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, t.bg);
        let (header_rect, middle, footer_rect) = chrome::page_rects(full);

        let user = self.me.as_ref().map(|m| m.user.username.clone());
        let signed_in = user.is_some() && matches!(self.page, Page::Home | Page::Device | Page::Files | Page::Admin(_) | Page::Links);
        let is_admin = self.me.as_ref().is_some_and(|m| m.user.is_admin());
        // (label, selected, enabled, tag) and where each goes.
        let mut nav: Vec<(NavCell<'_>, Option<Page>)> = Vec::new();
        if signed_in {
            nav.push((("Home", self.page == Page::Home, true, None), Some(Page::Home)));
            nav.push((("Files", self.page == Page::Files, true, None), Some(Page::Files)));
            if is_admin {
                let tab = if let Page::Admin(tab) = self.page { tab } else { AdminTab::Users };
                nav.push((("Admin", matches!(self.page, Page::Admin(_)), true, None), Some(Page::Admin(tab))));
            }
        }
        // APPS, after Files, when the server hosts Craft web apps and some are switched on.
        let apps = apps_menu::offered(self.config.as_ref(), signed_in);
        let apps_entry = (!apps.is_empty()).then(|| {
            let at = 2.min(nav.len());
            nav.insert(at, (("Apps", false, true, None), None));
            at
        });
        let cells: Vec<(&str, bool, bool, Option<&str>)> = nav.iter().map(|(cell, _)| *cell).collect();
        let info = chrome::HeaderInfo {
            nav: &cells,
            user: if signed_in { user.as_deref() } else { None },
            user_selected: self.account_open || self.page == Page::Links,
            menu: apps_entry,
            menu_open: self.apps_menu.open,
        };
        let clicks = chrome::header(ui, header_rect, &info);
        let apps_clicked = apps_entry.is_some() && clicks.nav == apps_entry;
        if apps_clicked {
            self.apps_menu.toggle();
            self.account_open = false;
        }
        if clicks.user {
            self.account_open = !self.account_open;
            self.apps_menu.close();
            if self.account_open {
                self.links.count_for_menu(&mut self.backend);
            }
        }
        if !signed_in {
            self.account_open = false;
        }
        if apps_entry.is_none() {
            self.apps_menu.close();
        }
        if let Some(page) = clicks.nav.and_then(|i| nav.get_mut(i)).and_then(|(_, page)| page.take()) {
            self.enter(page);
        }
        if clicks.sign_out && !self.net.is_busy() {
            let req = auth::logout(&self.server);
            self.send(Key::Logout, req);
        }

        let version = self.config.as_ref().map_or_else(String::new, |c| format!(" {}", c.version));
        let status = if matches!(self.page, Page::Setup(_)) { "first run".to_owned() } else { host(&self.server) };
        chrome::footer(
            ui,
            footer_rect,
            &format!("CraftCloud{version} · {status}"),
            &[("Docs \u{2197}", DOCS_URL), ("Source \u{2197}", SOURCE_URL), ("Look based on ArtCraft Launcher by the ArtCraft team", "")],
        );
        // Sheet pages dim the whole window, header and footer too (the design's scrim).
        if matches!(self.page, Page::Setup(_) | Page::Login(_) | Page::Device | Page::Failed(_)) {
            let dim = egui::Color32::from_black_alpha(if t.dark { 90 } else { 30 });
            ui.painter().rect_filled(header_rect, 0.0, dim);
            ui.painter().rect_filled(footer_rect, 0.0, dim);
        }

        ui.scope_builder(egui::UiBuilder::new().max_rect(middle), |ui| {
            ui.set_clip_rect(middle);
            match self.page {
                Page::Loading => {
                    ui.painter().text(middle.center(), egui::Align2::CENTER_CENTER, "LOADING…", egui::FontId::monospace(11.0), t.muted);
                }
                Page::Failed(ref why) => {
                    let why = why.clone();
                    chrome::sheet(
                        ui,
                        middle,
                        egui::vec2(520.0, 300.0),
                        &chrome::Hero { title: "Can't reach the server", caption: &host(&self.server), keep_case: false },
                        |ui, part| {
                            if part == chrome::Part::Body {
                                error_line(ui, &why);
                            }
                        },
                    );
                }
                Page::Setup(step) => self.setup_ui(ui, middle, step),
                Page::Login(_) => self.login_ui(ui, middle),
                Page::Device => self.device_ui(ui, middle),
                Page::Home => self.home_ui(ui, middle),
                Page::Files => self.files.ui(ui, middle, &mut self.backend),
                Page::Admin(tab) => self.admin_ui(ui, middle, tab),
                Page::Links => self.links.ui(ui, middle, &mut self.backend),
                Page::Share => {
                    if let Some(share) = &mut self.share {
                        share.ui(ui, middle, &mut self.backend);
                    }
                }
            }
        });
        if self.account_open
            && let Some(anchor) = clicks.user_rect
        {
            self.account_menu_ui(ui, anchor, clicks.user);
        }
        if self.apps_menu.open
            && let Some((anchor, trigger)) = clicks.menu
            && let Some(id) = self.apps_menu.ui(ui, anchor, header_rect.bottom() + 4.0, trigger, &apps, apps_clicked)
        {
            self.backend.open_tab(&apps_menu::app_url(&self.server, id));
        }
        if std::mem::take(&mut self.files.go_to_links) {
            self.enter(Page::Links);
        }
        if std::mem::take(&mut self.links.go_to_files) {
            self.enter(Page::Files);
        }
    }

    /// The account menu, hung under the person's cell: who is signed in, then "Your share links"
    /// with how many there are. `opened_now`: the click that opened it is not a click outside it.
    fn account_menu_ui(&mut self, ui: &mut Ui, anchor: egui::Rect, opened_now: bool) {
        let t = Tokens::get(ui.ctx());
        let Some(me) = self.me.as_ref().map(|m| m.user.clone()) else { return };
        let width = 260.0;
        let screen = ui.ctx().content_rect();
        let x = anchor.left().min(screen.right() - width - 8.0).max(screen.left() + 8.0);
        let count = self.links.count();
        let mut go = false;
        let mut picked = None;
        let area = egui::Area::new(ui.scope_id().with("account-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(x, anchor.bottom() + 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::new().fill(t.bg).stroke(egui::Stroke::new(1.0, t.line_strong)).shadow(crate::kit::shadow(&t)).show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 12, bottom: 12 }).show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.add(egui::Label::new(RichText::new(&me.username).size(15.0).color(t.ink_strong)).truncate());
                        chrome::hud(ui, &format!("{} · {}", me.username, me.email), t.muted);
                    });
                    let (rule, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
                    ui.painter().hline(rule.x_range(), rule.center().y, t.hairline());
                    let (row, resp) = ui.allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(row, 0.0, t.sunken);
                    }
                    let left = egui::pos2(row.left() + 14.0, row.center().y);
                    ui.painter().text(left, egui::Align2::LEFT_CENTER, "Your share links", egui::FontId::proportional(13.5), t.ink);
                    if let Some(n) = count {
                        let right = egui::pos2(row.right() - 14.0, row.center().y);
                        ui.painter().text(right, egui::Align2::RIGHT_CENTER, n.to_string(), egui::FontId::monospace(12.0), t.muted);
                    }
                    let label = match count {
                        Some(n) => format!("Your share links, {n}"),
                        None => "Your share links".to_owned(),
                    };
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, true, &label));
                    go = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                    let (rule, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
                    ui.painter().hline(rule.x_range(), rule.center().y, t.hairline());
                    picked = theme_rows(ui, width, &self.theme);
                });
            });
        let outside = !opened_now
            && ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p));
        if let Some(id) = picked {
            // The menu stays open, so another theme is one click away.
            self.backend.save_pref(THEME_PREF, &id);
            self.theme = id;
            ui.ctx().request_repaint();
        }
        if go {
            self.account_open = false;
            self.enter(Page::Links);
        } else if outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.account_open = false;
        }
    }

    /// Asks for `/craftcloud/config.json` again (after the web apps were switched on or off).
    fn reload_config(&mut self) {
        let req = HttpRequest::new(Method::Get, self.server.join("craftcloud/config.json"));
        self.net.send(&mut self.backend, Key::Config, req);
    }

    fn send(&mut self, key: Key, request: craftcloud_core::http::HttpRequest) {
        self.error = None;
        self.net.send(&mut self.backend, key, request);
    }

    fn on_answer(&mut self, key: Key, result: NetResult) {
        let resp = match result {
            Ok(resp) => resp,
            Err(why) => {
                if key == Key::Config {
                    self.config_done = true;
                } else if self.page == Page::Loading {
                    self.page = Page::Failed(why);
                } else {
                    self.error = Some(why);
                }
                self.route();
                return;
            }
        };
        match key {
            Key::Status => match auth::parse_status(&resp) {
                Ok(s) => self.status = Some(s),
                Err(e) => self.page = Page::Failed(e.to_string()),
            },
            Key::Config => {
                self.config = RuntimeConfig::from_json(&resp.body).ok().filter(|_| resp.is_success());
                self.config_done = true;
            }
            Key::Me => {
                self.me = auth::parse_me(&resp).ok().flatten();
                self.me_done = true;
                if self.me.is_some() {
                    self.after_sign_in();
                } else if self.signing_in {
                    self.error = Some(
                        "Signed in, but this browser didn't keep the session. Open CraftCloud at its public address (https), then sign in again."
                            .into(),
                    );
                }
                self.signing_in = false;
            }
            Key::Setup => match auth::parse_setup(&resp) {
                // Setup doesn't sign in: do it with the same details.
                Ok(_) => {
                    let (name, password) = (self.form.name.clone(), self.form.password.clone());
                    self.send(Key::Login, auth::login(&self.server, &name, &password));
                }
                Err(e) => self.error = Some(e.to_string()),
            },
            Key::Login => match auth::parse_login(&resp) {
                Ok(_) => {
                    self.form.password.clear();
                    self.form.password2.clear();
                    self.send(Key::Me, auth::me(&self.server));
                    self.signing_in = true;
                }
                Err(e) => self.error = Some(sign_in_error(&e, &resp)),
            },
            Key::Logout => {
                self.me = None;
                self.account_open = false;
                self.apps_menu.close();
                self.links = links::Links::new(self.server.clone());
                self.admin = admin::Admin::default();
                self.sso = oidc::Sso::default();
                self.page = Page::Login(After::Home);
            }
            Key::VerifyInfo => match auth::parse_device_verify_info(&resp) {
                Ok(info) if info.valid => self.device = Some(info),
                Ok(_) => self.error = Some("That code isn't valid any more. Start the sign-in again on the device.".into()),
                Err(e) => self.error = Some(e.to_string()),
            },
            Key::Catalog => {
                self.catalog = Some(if resp.status == 404 { Vec::new() } else { catalog::parse(&resp.body).unwrap_or_default() });
            }
            Key::LoadSettings => match serde_json::from_slice::<ServerSettings>(&resp.body) {
                Ok(s) if resp.is_success() => {
                    self.web_apps = WebAppChoice { on: s.web_apps, disabled: s.disabled_apps, loaded: true };
                }
                _ => {
                    self.error =
                        Some(craftcloud_core::api::expect_ok(&resp).err().map_or("Couldn't read the server settings.".into(), |e| e.to_string()))
                }
            },
            Key::SaveSettings => match craftcloud_core::api::expect_ok(&resp) {
                Ok(()) => {
                    self.page = Page::Setup(5);
                    self.reload_config();
                }
                Err(e) => self.error = Some(e.to_string()),
            },
            Key::Devices => self.devices = Some(auth::parse_devices(&resp).unwrap_or_default()),
            Key::Verify(approve) => match craftcloud_core::api::expect_ok(&resp) {
                Ok(()) => self.device_result = Some(approve),
                Err(e) => self.error = Some(e.to_string()),
            },
        }
        self.route();
    }

    /// Picks the first page once the start-up answers are in.
    fn route(&mut self) {
        if self.page != Page::Loading || !self.config_done || !self.me_done {
            return;
        }
        let Some(status) = &self.status else { return };
        self.page = if self.share.is_some() {
            Page::Share
        } else if !status.initialized {
            Page::Setup(1)
        } else if self.me.is_some() {
            let page = if self.wants_device() {
                Page::Device
            } else if self.path.starts_with("/files") {
                Page::Files
            } else {
                self.home_or_admin()
            };
            self.enter(page);
            return;
        } else if self.wants_device() {
            Page::Login(After::Device)
        } else {
            Page::Login(After::Home)
        };
    }

    fn wants_device(&self) -> bool {
        self.path.trim_end_matches('/') == "/device"
    }

    /// Home, your share links (`/links`), or the admin page the address asks for (`/admin/users`,
    /// `/admin/server`) when the person may see it.
    fn home_or_admin(&self) -> Page {
        let path = self.path.trim_end_matches('/');
        if path == "/links" {
            return Page::Links;
        }
        if !path.starts_with("/admin") || !self.me.as_ref().is_some_and(|m| m.user.is_admin()) {
            return Page::Home;
        }
        Page::Admin(if path == "/admin/server" { AdminTab::Server } else { AdminTab::Users })
    }

    fn after_sign_in(&mut self) {
        match &self.page {
            Page::Setup(2) => self.page = Page::Setup(3),
            Page::Login(After::Device) => self.enter(Page::Device),
            Page::Login(After::Home) => {
                let page = self.home_or_admin();
                self.enter(page);
            }
            _ => {}
        }
    }

    fn enter(&mut self, page: Page) {
        if page == Page::Device && !self.form.code.trim().is_empty() {
            let req = auth::device_verify_info(&self.server, self.form.code.trim());
            self.send(Key::VerifyInfo, req);
        }
        if page == Page::Files {
            self.files.open(&mut self.backend, self.me.as_ref());
        }
        if let Page::Admin(tab) = page {
            self.admin_load(tab);
        }
        if page == Page::Links {
            self.links.open(&mut self.backend);
        }
        if page == Page::Home && !self.net.is_waiting(&Key::Devices) {
            let req = auth::devices(&self.server);
            self.send(Key::Devices, req);
        }
        self.page = page;
    }

    fn setup_ui(&mut self, ui: &mut Ui, area: egui::Rect, step: u8) {
        const STEPS: [&str; 5] = ["Welcome", "Owner", "Address", "Web apps", "Done"];
        let index = usize::from(step.clamp(1, 5)) - 1;
        let (title, caption) = match step {
            1 => ("Welcome to CraftCloud", "Step 1 of 5 · first run"),
            2 => ("Create the owner", "Step 2 of 5 · owner account"),
            3 => ("Where people find it", "Step 3 of 5 · server address"),
            4 => ("Apps in the browser?", "Step 4 of 5 · web apps · optional"),
            _ => ("Your cloud, ready", "Step 5 of 5 · done"),
        };
        let size = if step == 4 {
            egui::vec2(if area.width() < 600.0 { area.width() } else { area.width() - 64.0 }, area.height() - 48.0)
        } else {
            egui::vec2(880.0, 640.0)
        };
        let mut go: Option<u8> = None;
        let mut action = SetupAction::None;
        let hero = chrome::Hero { title, caption, keep_case: false };
        chrome::sheet(ui, area, size, &hero, |ui, part| match part {
            chrome::Part::HeroRight => chrome::step_cells(ui, &STEPS, index),
            chrome::Part::Body => match step {
                1 => self.setup_welcome(ui),
                2 => self.setup_account_ui(ui),
                3 => self.setup_address(ui),
                4 => self.web_apps_ui(ui),
                _ => self.setup_done(ui),
            },
            chrome::Part::ActionsLeft => {
                let back = match step {
                    2 => Some(1),
                    4 => Some(3),
                    5 => Some(4),
                    _ => None,
                };
                if let Some(back) = back
                    && chrome::button(ui, "Back", Kind::Secondary, None, !self.net.is_busy()).clicked()
                {
                    go = Some(back);
                }
            }
            chrome::Part::ActionsRight => {
                let busy = self.net.is_busy();
                match step {
                    1 => {
                        if chrome::button(ui, "Start setup", Kind::Primary, Some(Icon::Arrow), true).clicked() {
                            go = Some(2);
                        }
                    }
                    2 => {
                        let filled = !self.form.name.trim().is_empty() && !self.form.email.trim().is_empty() && !self.form.password.is_empty();
                        if chrome::button(ui, if busy { "Creating…" } else { "Create account" }, Kind::Primary, Some(Icon::Arrow), filled && !busy)
                            .clicked()
                        {
                            action = SetupAction::CreateOwner;
                        }
                    }
                    3 => {
                        if chrome::button(ui, "Continue", Kind::Primary, Some(Icon::Arrow), true).clicked() {
                            action = SetupAction::OpenWebApps;
                        }
                    }
                    4 => {
                        let ready = self.catalog.is_some() && self.web_apps.loaded;
                        let available = self.catalog.as_ref().is_some_and(|c| !c.is_empty());
                        if available && ready {
                            if chrome::button(ui, if busy { "Saving…" } else { "Save and continue" }, Kind::Primary, Some(Icon::Arrow), !busy)
                                .clicked()
                            {
                                action = SetupAction::SaveWebApps;
                            }
                        } else if chrome::button(ui, "Continue", Kind::Primary, Some(Icon::Arrow), ready).clicked() {
                            go = Some(5);
                        }
                    }
                    _ => {
                        if chrome::button(ui, "Go to home", Kind::Primary, Some(Icon::Arrow), true).clicked() {
                            action = SetupAction::Home;
                        }
                    }
                }
            }
        });
        if let Some(step) = go {
            self.error = None;
            self.page = Page::Setup(step);
        }
        match action {
            SetupAction::None => {}
            SetupAction::CreateOwner => {
                if self.form.password != self.form.password2 {
                    self.error = Some("The passwords don't match.".into());
                } else {
                    let req = auth::setup(&self.server, self.form.name.trim(), self.form.email.trim(), &self.form.password);
                    self.send(Key::Setup, req);
                }
            }
            SetupAction::OpenWebApps => self.open_web_apps_step(),
            SetupAction::SaveWebApps => {
                let mut choice = ServerSettings { web_apps: self.web_apps.on, disabled_apps: self.web_apps.disabled.clone() };
                choice.normalise();
                let body = serde_json::to_value(&choice).unwrap_or_default();
                let req = HttpRequest::new(Method::Put, self.server.join("craftcloud/api/settings")).json(&body);
                self.send(Key::SaveSettings, req);
            }
            SetupAction::Home => self.enter(Page::Home),
        }
    }

    fn setup_welcome(&mut self, ui: &mut Ui) {
        let wide = ui.available_width() >= 640.0;
        let text = |ui: &mut Ui| {
            chrome::headline(ui, 48.0, &[&[("Your files.", false)], &[("One ", false), ("cloud", true), (".", false)]]);
            ui.add_space(12.0);
            chrome::body(
                ui,
                "CraftCloud keeps your files on this server and in sync on every computer, so the Craft apps open and save them like any other folder.",
                18.0,
            );
            ui.add_space(10.0);
            chrome::meta(ui, &["Takes about a minute", "Nothing leaves this server"]);
        };
        if wide {
            ui.horizontal_top(|ui| {
                logo(ui, 128.0);
                ui.add_space(28.0);
                ui.vertical(text);
            });
        } else {
            logo(ui, 96.0);
            ui.add_space(16.0);
            text(ui);
        }
        ui.add_space(24.0);
        let mut rows = vec![("Address", self.server.to_string())];
        if let Some(c) = &self.config {
            rows.push(("Public URL", c.public_url.clone()));
            rows.push(("Version", format!("CraftCloud {}", c.version)));
        }
        rows.push(("Backend", "OxiCloud 0.10.0".into()));
        chrome::facts(ui, "This server", &rows);
    }

    fn setup_account_ui(&mut self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        chrome::body(ui, "The owner manages people and the server, and can't be deactivated. You can add admins later.", 14.0);
        ui.add_space(14.0);
        let wide = ui.available_width() >= 600.0;
        let gap = 16.0;
        let col = if wide { ((ui.available_width() - gap) / 2.0).floor() } else { ui.available_width() };
        let left = |ui: &mut Ui, form: &mut Form| {
            chrome::field(ui, "Username", &mut form.name, false, Some("Lower case, no spaces. Used to sign in."));
            ui.add_space(14.0);
            chrome::field(ui, "Email", &mut form.email, false, Some("For you and the other admins; never shown to others."));
        };
        let right = |ui: &mut Ui, form: &mut Form| {
            chrome::field(ui, "Password", &mut form.password, true, None);
            ui.add_space(14.0);
            chrome::field(ui, "Repeat password", &mut form.password2, true, None);
            ui.add_space(8.0);
            let n = form.password.chars().count();
            if n > 0 {
                ui.horizontal(|ui| {
                    let (word, tone) = match n {
                        0..=7 => ("Too short", t.bad),
                        8..=11 => ("OK", t.warn),
                        _ => ("Strong", t.good),
                    };
                    chrome::badge(ui, word, tone);
                    let matching = if form.password2.is_empty() {
                        ""
                    } else if form.password2 == form.password {
                        " · passwords match"
                    } else {
                        " · passwords differ"
                    };
                    ui.label(egui::RichText::new(format!("{n} characters{matching}")).size(12.0).color(t.muted));
                });
            }
        };
        if wide {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(egui::vec2(col, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| left(ui, &mut self.form));
                ui.allocate_ui_with_layout(egui::vec2(col, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| right(ui, &mut self.form));
            });
        } else {
            left(ui, &mut self.form);
            ui.add_space(14.0);
            right(ui, &mut self.form);
        }
        if let Some(e) = &self.error {
            ui.add_space(12.0);
            error_line(ui, e);
        }
    }

    fn setup_address(&mut self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        chrome::body(ui, "The address people type to reach CraftCloud. Share links and the desktop sign-in use it.", 14.0);
        ui.add_space(16.0);
        let origin = self.server.to_string();
        let public = self.config.as_ref().map(|c| c.public_url.clone());
        chrome::facts(ui, "Addresses", &[("Public URL", public.clone().unwrap_or_else(|| "unknown".into())), ("You're using", origin.clone())]);
        ui.add_space(12.0);
        match public.as_deref() {
            Some(p) if p.trim_end_matches('/') == origin => {
                ui.label(RichText::new("Matches the address you're using now.").color(t.good));
            }
            Some(_) => {
                ui.label(RichText::new("These differ. Share links and the desktop sign-in use the public URL: set CRAFTCLOUD_PUBLIC_URL in .env to the address people type, then restart.").color(t.warn));
            }
            None => muted(ui, "This server didn't say its public URL."),
        }
    }

    fn open_web_apps_step(&mut self) {
        self.page = Page::Setup(4);
        let catalog = HttpRequest::new(Method::Get, self.server.join("craftcloud/apps.json"));
        self.send(Key::Catalog, catalog);
        let settings = HttpRequest::new(Method::Get, self.server.join("craftcloud/api/settings"));
        self.send(Key::LoadSettings, settings);
    }

    /// Setup step 4: the web-apps switch and, when on, the apps to offer.
    fn web_apps_ui(&mut self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        let bundled = self.catalog.as_ref().map(Vec::len);
        let ready = self.catalog.is_some() && self.web_apps.loaded;
        let available = bundled.is_some_and(|n| n > 0);
        let apps = self.catalog.clone().unwrap_or_default();
        let offered = apps.len() - apps.iter().filter(|a| self.web_apps.disabled.contains(&a.id)).count();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            chrome::hud(ui, "Web apps", t.ink);
            if let Some(i) = crate::kit::cells(ui, &["Off", "On"], Some(usize::from(self.web_apps.on)), ready && available) {
                self.web_apps.on = i == 1;
            }
            let lists = self.web_apps.on && available;
            match crate::kit::cells(ui, &["All", "None"], None, lists) {
                Some(0) => self.web_apps.disabled.clear(),
                Some(_) => self.web_apps.disabled = apps.iter().map(|a| a.id.clone()).collect(),
                None => {}
            }
            if !ready {
                chrome::hud(ui, "Loading…", t.muted);
            } else if !available {
                chrome::badge(ui, "Needs the -apps image", t.warn);
            } else if lists {
                ui.label(RichText::new(format!("{offered} of {} on · click a card to switch it", apps.len())).size(13.0).color(t.muted));
            }
        });
        ui.add_space(10.0);
        if available {
            chrome::meta(ui, &["-apps image", "about 770 MB download", "2.1 GB on disk"]);
            ui.add_space(4.0);
        }
        muted(
            ui,
            if available {
                "Run the Craft apps in the browser too, so files open without installing anything. The desktop apps work either way."
            } else {
                "This server runs the slim image. To run the Craft apps in the browser, use the -apps image (about 770 MB download, 2.1 GB on disk) and set CRAFTCLOUD_WEB_APPS=on."
            },
        );
        ui.add_space(14.0);
        if self.web_apps.on && available {
            self.web_app_grid(ui, "");
        }
        if let Some(e) = &self.error {
            ui.add_space(12.0);
            error_line(ui, e);
        }
    }

    /// The app cards (setup step 4, Admin › Server): click one to switch it. `filter` (lower case)
    /// keeps the apps whose name or id contains it.
    fn web_app_grid(&mut self, ui: &mut Ui, filter: &str) {
        let t = Tokens::get(ui.ctx());
        let apps: Vec<catalog::WebApp> = self
            .catalog
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| filter.is_empty() || a.name.to_lowercase().contains(filter) || a.id.contains(filter))
            .collect();
        if apps.is_empty() {
            muted(ui, "No app matches.");
            return;
        }
        {
            let cards: Vec<Card<'_>> = apps
                .iter()
                .map(|app| {
                    let color = catalog::rgb(&app.color).map_or(t.accent, |(r, g, b)| egui::Color32::from_rgb(r, g, b));
                    let ink_hex = app.ink.get(usize::from(!t.dark)).map(String::as_str).unwrap_or(&app.color);
                    let ink = catalog::rgb(ink_hex).map_or(t.ink_strong, |(r, g, b)| egui::Color32::from_rgb(r, g, b));
                    Card {
                        id: &app.id,
                        name: &app.name,
                        tagline: &app.tagline,
                        version: &app.version,
                        color,
                        ink,
                        opens: &app.default,
                        experimental: app.is_experimental(),
                    }
                })
                .collect();
            let disabled = self.web_apps.disabled.clone();
            for id in app_grid(ui, &cards, |id| !disabled.iter().any(|d| d == id)) {
                if let Some(pos) = self.web_apps.disabled.iter().position(|d| d == &id) {
                    self.web_apps.disabled.remove(pos);
                } else {
                    self.web_apps.disabled.push(id);
                }
            }
        }
    }

    fn login_ui(&mut self, ui: &mut Ui, area: egui::Rect) {
        self.sso_check();
        if self.sso_busy_ui(ui, area) {
            return;
        }
        let password_login = self.sso.password_login();
        let device = self.wants_device();
        let caption = host(&self.server);
        let title = if device { "Sign in to connect" } else { "Sign in" };
        let mut submit = false;
        let mut height: f32 = if device { 440.0 } else { 370.0 };
        if self.sso.offered().is_some() {
            height += if password_login { 84.0 } else { -60.0 };
        }
        if self.sso.error.is_some() {
            height += 30.0;
        }
        let code = self.form.code.trim().to_uppercase();
        chrome::sheet(ui, area, egui::vec2(480.0, height), &chrome::Hero { title, caption: &caption, keep_case: false }, |ui, part| match part {
            chrome::Part::Body => {
                if device {
                    let t = Tokens::get(ui.ctx());
                    egui::Frame::new().stroke(t.hairline()).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("A computer wants to connect").size(14.0).color(t.ink_strong));
                        let line = if code.is_empty() { "You confirm it next".to_owned() } else { format!("Code {code} · you confirm it next") };
                        chrome::hud(ui, &line, t.muted);
                    });
                    ui.add_space(12.0);
                }
                self.sso_sign_in_ui(ui);
                if password_login {
                    chrome::field(ui, "Username", &mut self.form.name, false, None);
                    ui.add_space(14.0);
                    let pw = chrome::field(ui, "Password", &mut self.form.password, true, None);
                    submit = pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
                if let Some(e) = &self.error {
                    ui.add_space(10.0);
                    error_line(ui, e);
                }
            }
            chrome::Part::ActionsLeft => {
                ui.label(RichText::new("Forgot it? Ask an admin to reset it.").size(12.5).color(Tokens::get(ui.ctx()).muted));
            }
            chrome::Part::ActionsRight if password_login => {
                let busy = self.net.is_busy();
                let ready = !self.form.name.trim().is_empty() && !self.form.password.is_empty() && !busy;
                if chrome::button(ui, if busy { "Signing in…" } else { "Sign in" }, Kind::Primary, Some(Icon::Arrow), ready).clicked() {
                    submit = true;
                }
            }
            _ => {}
        });
        let ready = !self.form.name.trim().is_empty() && !self.form.password.is_empty() && !self.net.is_busy();
        if submit && ready {
            let req = auth::login(&self.server, self.form.name.trim(), &self.form.password);
            self.send(Key::Login, req);
        }
    }

    fn device_ui(&mut self, ui: &mut Ui, area: egui::Rect) {
        let t = Tokens::get(ui.ctx());
        if let Some(approved) = self.device_result {
            self.device_done_ui(ui, area, approved);
            return;
        }
        let mut send: Option<craftcloud_core::http::HttpRequest> = None;
        let mut key = Key::VerifyInfo;
        match self.device.clone() {
            None => {
                let mut submit = false;
                let mut cancel = false;
                chrome::sheet(
                    ui,
                    area,
                    egui::vec2(560.0, 340.0),
                    &chrome::Hero { title: "Connect a device", caption: "Enter the code your client shows", keep_case: false },
                    |ui, part| match part {
                        chrome::Part::Body => {
                            let code = chrome::field(
                                ui,
                                "Code",
                                &mut self.form.code,
                                false,
                                Some("Eight characters, shown in the CraftCloud window on the computer you're connecting."),
                            );
                            submit = code.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if let Some(e) = &self.error {
                                ui.add_space(10.0);
                                error_line(ui, e);
                            }
                        }
                        chrome::Part::ActionsLeft => {
                            if chrome::button(ui, "Cancel", Kind::Secondary, None, true).clicked() {
                                cancel = true;
                            }
                        }
                        chrome::Part::ActionsRight => {
                            let ready = !self.form.code.trim().is_empty() && !self.net.is_busy();
                            if chrome::button(ui, "Continue", Kind::Primary, Some(Icon::Arrow), ready).clicked() {
                                submit = true;
                            }
                        }
                        chrome::Part::HeroRight => {}
                    },
                );
                if cancel {
                    self.error = None;
                    self.enter(Page::Home);
                    return;
                }
                if submit && !self.form.code.trim().is_empty() && !self.net.is_busy() {
                    send = Some(auth::device_verify_info(&self.server, self.form.code.trim()));
                }
            }
            Some(info) => {
                let who = self.me.as_ref().map_or_else(|| "your".to_owned(), |m| format!("{}'s", m.user.username));
                let code = self.form.code.trim().to_uppercase();
                let busy = self.net.is_busy();
                let mut answer: Option<bool> = None;
                chrome::sheet(
                    ui,
                    area,
                    egui::vec2(560.0, 470.0),
                    &chrome::Hero { title: &format!("Approve {}?", info.client_name), caption: "Device request", keep_case: false },
                    |ui, part| match part {
                        chrome::Part::Body => {
                            chrome::facts(
                                ui,
                                "Device",
                                &[("Name", info.client_name.clone()), ("Access", "All your files, until you remove it".into())],
                            );
                            ui.add_space(12.0);
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 72.0), egui::Sense::hover());
                            ui.painter().rect(rect, 0.0, t.sunken, t.hairline(), egui::StrokeKind::Inside);
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &code, egui::FontId::monospace(28.0), t.ink_strong);
                            ui.add_space(12.0);
                            ui.label(
                            RichText::new(format!(
                                "Approve only if this code is on the screen of the computer you're connecting. It gets access to all of {who} files."
                            ))
                            .size(12.5)
                            .color(t.muted),
                        );
                            if let Some(e) = &self.error {
                                ui.add_space(10.0);
                                error_line(ui, e);
                            }
                        }
                        chrome::Part::ActionsLeft => {
                            let deny = chrome::button(ui, "Deny", Kind::Secondary, None, !busy);
                            answer = answer.or(deny.clicked().then_some(false));
                        }
                        chrome::Part::ActionsRight => {
                            let approve = chrome::button(ui, "Approve", Kind::Primary, Some(Icon::Check), !busy);
                            answer = answer.or(approve.clicked().then_some(true));
                        }
                        chrome::Part::HeroRight => {}
                    },
                );
                if let Some(approve) = answer {
                    key = Key::Verify(approve);
                    send = Some(auth::device_verify(&self.server, self.form.code.trim(), approve));
                }
            }
        }
        if let Some(req) = send {
            self.send(key, req);
        }
    }
}

/// A header nav entry: (label, selected, enabled, tag).
type NavCell<'a> = (&'a str, bool, bool, Option<&'a str>);

enum SetupAction {
    None,
    CreateOwner,
    OpenWebApps,
    SaveWebApps,
    Home,
}

// The project's home: the `repository` field of the workspace Cargo.toml, so moving it is one line.
const SOURCE_URL: &str = env!("CARGO_PKG_REPOSITORY");
const DOCS_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/tree/main/docs");
const INSTALL_GUIDE_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "#downloads");

/// `cloud.example.org:8086`
fn host(server: &ServerUrl) -> String {
    let s = server.to_string();
    s.split_once("://").map_or(s.clone(), |(_, rest)| rest.trim_end_matches('/').to_owned())
}

fn sign_in_error(e: &ApiError, resp: &craftcloud_core::http::HttpResponse) -> String {
    match e {
        ApiError::Status { status: 401, .. } => "Wrong username or password.".into(),
        ApiError::Status { status: 429, .. } => match retry_after(resp) {
            Some(secs) => format!("Too many sign-in attempts. Try again in {secs} s."),
            None => "Too many sign-in attempts. Try again in a minute.".into(),
        },
        other => other.to_string(),
    }
}

/// How long a 429 asks to wait: the `Retry-After` header (seconds), else OxiCloud's
/// `retry_after_secs` in the body.
fn retry_after(resp: &craftcloud_core::http::HttpResponse) -> Option<u64> {
    if let Some(secs) = resp.header("Retry-After").and_then(|v| v.trim().parse().ok()) {
        return Some(secs);
    }
    serde_json::from_slice::<serde_json::Value>(&resp.body).ok()?.get("retry_after_secs")?.as_u64()
}

/// `12.4 GB`, `340 MB`, `3 KB` (decimal units, like the design).
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// The account menu's colour themes: a THEME heading, then CraftCloud's own look and each theme,
/// the one in use marked. A theme with one variant says so ("dark only"). Returns the id clicked
/// (empty for CraftCloud's own look).
fn theme_rows(ui: &mut Ui, width: f32, current: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let (head, _) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::hover());
    let g = chrome::hud_galley(ui, "Theme", t.muted);
    ui.painter().galley(egui::pos2(head.left() + 14.0, head.bottom() - g.size().y - 4.0), g, t.muted);
    let rows = std::iter::once(("", crate::themes::CRAFTCLOUD_LOOK.to_owned(), None))
        .chain(crate::themes::all().iter().map(|th| (th.id, th.name.clone(), th.only())));
    let mut picked = None;
    for (id, name, only) in rows {
        // An id this build doesn't know shows CraftCloud's own look, so that is the one marked.
        let on = if crate::themes::find(current).is_some() { id == current } else { id.is_empty() };
        let (row, resp) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(row, 0.0, t.sunken);
        }
        if on {
            let mark = egui::Rect::from_center_size(egui::pos2(row.left() + 18.0, row.center().y), egui::vec2(6.0, 6.0));
            ui.painter().rect_filled(mark, 0.0, t.accent_ink);
        }
        let color = if on { t.ink_strong } else { t.ink };
        ui.painter().text(egui::pos2(row.left() + 30.0, row.center().y), egui::Align2::LEFT_CENTER, &name, egui::FontId::proportional(13.5), color);
        if let Some(only) = only {
            ui.painter().text(
                egui::pos2(row.right() - 14.0, row.center().y),
                egui::Align2::RIGHT_CENTER,
                only,
                egui::FontId::proportional(12.0),
                t.muted,
            );
        }
        let label = match only {
            Some(only) => format!("Theme: {name} ({only})"),
            None => format!("Theme: {name}"),
        };
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, &label));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
            picked = Some(id.to_owned());
        }
    }
    picked
}

#[cfg(all(test, feature = "shots"))]
mod shots;
#[cfg(test)]
pub(crate) mod tests;
