//! Admin › Users and Admin › Server, for owners and admins (design frames `v2-web-admin-*`).
//!
//! - **Users:** the launcher table (avatar, role, storage meter, status, last sign-in, actions),
//!   search and sort, the new-user and reset-password sheets, (de)activate, role and quota
//!   changes, and a details overlay with the person's sign-ins and desktop clients.
//! - **Server:** the facts strip, the web apps switch and app grid (shared with setup step 4),
//!   saved to `settings.env` through craftcloud-admin, and the single sign-on settings.
//!
//! This module keeps its own request bookkeeping ([`Admin::net`]), so `web.rs` only routes to it.

use craftcloud_core::api::admin::{self, Person, Role, Session};
use craftcloud_core::api::{expect_ok, oidc};
use craftcloud_core::catalog;
use craftcloud_core::http::{HttpRequest, Method};
use craftcloud_core::server_settings::ServerSettings;
use egui::{Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};

use super::{Page, WebApp, WebAppChoice};
use crate::backend::{Backend, Net, NetResult};
use crate::chrome::{self, Icon, Kind};
use crate::kit;
use crate::theme::Tokens;
use crate::widgets::error_line;

/// The two admin tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Users,
    Server,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Key {
    List,
    Create(String),
    Reset(String),
    SetActive(String, bool),
    SetRole(String, Role),
    Quota,
    Person,
    Sessions,
    Revoke,
    Catalog,
    LoadSettings,
    SaveSettings,
    Oidc,
    SaveOidc,
    TestOidc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Name,
    LastSignIn,
    Storage,
    Role,
}

const SORTS: [(&str, Sort); 4] = [("Name", Sort::Name), ("Last sign-in", Sort::LastSignIn), ("Storage", Sort::Storage), ("Role", Sort::Role)];

/// Quota choices in the details overlay (GiB; 0 is unlimited).
const QUOTAS: [u64; 7] = [1, 5, 10, 50, 100, 500, 0];
const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Default)]
struct NewUser {
    username: String,
    email: String,
    password: String,
    quota_gb: String,
    admin: bool,
}

enum Sheet {
    NewUser(NewUser),
    Reset { id: String, name: String, email: String, password: String },
}

struct Details {
    id: String,
    sessions: Option<Vec<Session>>,
}

/// The single sign-on form, as typed.
#[derive(Clone, Default, PartialEq, Eq)]
struct SsoForm {
    enabled: bool,
    provider_name: String,
    issuer_url: String,
    client_id: String,
    client_secret: String,
    scopes: String,
    admin_groups: String,
    auto_provision: bool,
    /// Password sign-in stays on next to the provider (OxiCloud's `disable_password_login`, inverted).
    password_login: bool,
}

impl SsoForm {
    fn from_settings(s: &oidc::Settings) -> Self {
        Self {
            enabled: s.enabled,
            provider_name: s.provider_name.clone(),
            issuer_url: s.issuer_url.clone(),
            client_id: s.client_id.clone(),
            client_secret: String::new(),
            scopes: s.scopes.clone(),
            admin_groups: s.admin_groups.clone(),
            auto_provision: s.auto_provision,
            password_login: !s.disable_password_login,
        }
    }

    fn to_save(&self) -> oidc::SaveSettings {
        oidc::SaveSettings {
            enabled: self.enabled,
            issuer_url: self.issuer_url.trim().to_owned(),
            client_id: self.client_id.trim().to_owned(),
            client_secret: Some(self.client_secret.clone()).filter(|s| !s.is_empty()),
            scopes: self.scopes.trim().to_owned(),
            auto_provision: self.auto_provision,
            admin_groups: self.admin_groups.trim().to_owned(),
            // Never without a way in: password sign-in goes off only while the provider is on.
            disable_password_login: self.enabled && !self.password_login,
            provider_name: self.provider_name.trim().to_owned(),
        }
    }
}

/// The admin pages' state.
#[derive(Default)]
pub(super) struct Admin {
    pub(super) net: Net<Key>,
    people: Option<Vec<Person>>,
    error: Option<String>,
    search: String,
    sort: Option<(Sort, bool)>,
    menu: Option<(String, egui::Pos2)>,
    sheet: Option<Sheet>,
    sheet_error: Option<String>,
    details: Option<Details>,
    toast: Option<(String, String, f64)>,
    // Server tab.
    server_loaded: bool,
    saved_choice: Option<(bool, Vec<String>)>,
    server_error: Option<String>,
    app_search: String,
    oidc: Option<oidc::Settings>,
    sso: SsoForm,
    sso_error: Option<String>,
    sso_note: Option<String>,
    /// The last "Test connection" answer: whether the server reached the provider, and its words.
    sso_test: Option<(bool, String)>,
}

impl Admin {
    fn sort(&self) -> (Sort, bool) {
        self.sort.unwrap_or((Sort::Name, true))
    }

    fn person(&self, id: &str) -> Option<&Person> {
        self.people.as_ref()?.iter().find(|p| p.id == id)
    }
}

impl<B: Backend> WebApp<B> {
    fn admin_send(&mut self, key: Key, req: HttpRequest) {
        self.admin.net.send(&mut self.backend, key, req);
    }

    /// Starts the loads a tab needs (called when the page is entered or refreshed).
    pub(super) fn admin_load(&mut self, tab: Tab) {
        match tab {
            Tab::Users => {
                self.admin.error = None;
                let req = admin::list(&self.server, 500, 0);
                self.admin_send(Key::List, req);
            }
            Tab::Server => {
                self.admin.server_error = None;
                self.admin.sso_error = None;
                let catalog = HttpRequest::new(Method::Get, self.server.join("craftcloud/apps.json"));
                self.admin_send(Key::Catalog, catalog);
                let settings = HttpRequest::new(Method::Get, self.server.join("craftcloud/api/settings"));
                self.admin_send(Key::LoadSettings, settings);
                let req = oidc::settings(&self.server);
                self.admin_send(Key::Oidc, req);
            }
        }
    }

    pub(super) fn on_admin_answer(&mut self, key: Key, result: NetResult) {
        let resp = match result {
            Ok(resp) => resp,
            Err(why) => {
                match key {
                    Key::Catalog | Key::LoadSettings | Key::SaveSettings => self.admin.server_error = Some(why),
                    Key::Oidc | Key::SaveOidc | Key::TestOidc => self.admin.sso_error = Some(why),
                    Key::Create(_) | Key::Reset(_) => self.admin.sheet_error = Some(why),
                    _ => self.admin.error = Some(why),
                }
                return;
            }
        };
        let a = &mut self.admin;
        match key {
            Key::List => match admin::parse_list(&resp) {
                Ok((people, _)) => a.people = Some(people),
                Err(e) => a.error = Some(e.to_string()),
            },
            Key::Create(name) => match admin::parse_create(&resp) {
                Ok(_) => {
                    a.sheet = None;
                    a.toast = Some((format!("{name} can sign in now"), "Give them the password in person, with this address.".into(), 0.0));
                    let req = admin::list(&self.server, 500, 0);
                    self.admin_send(Key::List, req);
                }
                Err(e) => a.sheet_error = Some(e.to_string()),
            },
            Key::Reset(name) => match expect_ok(&resp) {
                Ok(()) => {
                    a.sheet = None;
                    a.toast =
                        Some((format!("Password changed for {name}"), format!("Tell {name} in person. Their desktop clients stay connected."), 0.0));
                }
                Err(e) => a.sheet_error = Some(e.to_string()),
            },
            Key::SetActive(name, active) => match expect_ok(&resp) {
                Ok(()) => {
                    let (title, body) = if active {
                        (format!("{name} is active again"), "They can sign in and sync.")
                    } else {
                        (format!("{name} is deactivated"), "They can't sign in or sync until you activate them. Their files stay.")
                    };
                    a.toast = Some((title, body.into(), 0.0));
                    self.admin_refresh_people();
                }
                Err(e) => a.error = Some(e.to_string()),
            },
            Key::SetRole(name, role) => match expect_ok(&resp) {
                Ok(()) => {
                    let title = match role {
                        Role::Admin => format!("{name} is an admin now"),
                        Role::User => format!("{name} is a user now"),
                    };
                    a.toast = Some((title, "The change applies at their next page load.".into(), 0.0));
                    self.admin_refresh_people();
                }
                Err(e) => a.error = Some(e.to_string()),
            },
            Key::Quota => match expect_ok(&resp) {
                Ok(()) => self.admin_refresh_people(),
                Err(e) => a.error = Some(e.to_string()),
            },
            Key::Person => match admin::parse_get(&resp) {
                Ok(p) => {
                    if let Some(list) = a.people.as_mut()
                        && let Some(slot) = list.iter_mut().find(|q| q.id == p.id)
                    {
                        *slot = p;
                    }
                }
                Err(e) => a.error = Some(e.to_string()),
            },
            Key::Sessions => match admin::parse_sessions(&resp) {
                Ok(s) => {
                    if let Some(d) = a.details.as_mut() {
                        d.sessions = Some(s);
                    }
                }
                Err(e) => a.error = Some(e.to_string()),
            },
            Key::Revoke => match expect_ok(&resp) {
                Ok(()) => {
                    if let Some(id) = a.details.as_ref().map(|d| d.id.clone()) {
                        let req = admin::sessions(&self.server, &id);
                        self.admin_send(Key::Sessions, req);
                    }
                }
                Err(e) => a.error = Some(e.to_string()),
            },
            Key::Catalog => {
                self.catalog = Some(if resp.status == 404 { Vec::new() } else { catalog::parse(&resp.body).unwrap_or_default() });
            }
            Key::LoadSettings => match serde_json::from_slice::<ServerSettings>(&resp.body) {
                Ok(s) if resp.is_success() => {
                    a.saved_choice = Some((s.web_apps, s.disabled_apps.clone()));
                    self.web_apps = WebAppChoice { on: s.web_apps, disabled: s.disabled_apps, loaded: true };
                    a.server_loaded = true;
                }
                _ => a.server_error = Some(expect_ok(&resp).err().map_or("Couldn't read the server settings.".into(), |e| e.to_string())),
            },
            Key::SaveSettings => match expect_ok(&resp) {
                Ok(()) => {
                    a.saved_choice = Some((self.web_apps.on, self.web_apps.disabled.clone()));
                    a.toast = Some(("Web apps saved".into(), "The change applies to the next page that opens an app.".into(), 0.0));
                }
                Err(e) => a.server_error = Some(e.to_string()),
            },
            Key::Oidc => match oidc::parse_settings(&resp) {
                Ok(s) => {
                    a.sso = SsoForm::from_settings(&s);
                    a.oidc = Some(s);
                }
                Err(e) => a.sso_error = Some(e.to_string()),
            },
            Key::SaveOidc => match expect_ok(&resp) {
                Ok(()) => {
                    a.sso_note = Some("Saved. Single sign-on uses the new settings now.".into());
                    let req = oidc::settings(&self.server);
                    self.admin_send(Key::Oidc, req);
                }
                Err(e) => a.sso_error = Some(e.to_string()),
            },
            Key::TestOidc => match oidc::parse_test(&resp) {
                Ok(t) if t.success => {
                    let issuer = t.issuer.unwrap_or_default();
                    a.sso_test = Some((true, format!("The server reached the provider ({issuer}).")));
                }
                Ok(t) => a.sso_test = Some((false, format!("The server can't use this issuer URL: {}", t.message))),
                Err(e) => a.sso_error = Some(e.to_string()),
            },
        }
    }

    fn admin_refresh_people(&mut self) {
        let req = admin::list(&self.server, 500, 0);
        self.admin_send(Key::List, req);
    }

    /// The admin page: the toolbar row, then the tab.
    pub(super) fn admin_ui(&mut self, ui: &mut Ui, area: Rect, tab: Tab) {
        let t = Tokens::get(ui.ctx());
        let now = ui.input(|i| i.time);
        if let Some((_, _, shown)) = self.admin.toast.as_mut()
            && *shown == 0.0
        {
            *shown = now;
        }
        let busy = self.admin.net.is_busy();
        let mut switch_to = None;
        let mut refresh = false;
        let mut new_user = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let margin = if area.width() < 600.0 { 16 } else { 24 };
            egui::Frame::new().inner_margin(egui::Margin { left: margin, right: margin, top: 24, bottom: 32 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let wide = ui.available_width() >= 900.0;
                // The toolbar row.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if kit::refresh_button(ui, !busy).clicked() {
                        refresh = true;
                    }
                    let current = if tab == Tab::Users { 0 } else { 1 };
                    if let Some(i) = kit::cells(ui, &["Users", "Server"], Some(current), true) {
                        switch_to = Some(if i == 0 { Tab::Users } else { Tab::Server });
                    }
                    if tab == Tab::Users {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if chrome::button(ui, "New user", Kind::Primary, Some(Icon::Plus), true).clicked() {
                                new_user = true;
                            }
                            if wide {
                                self.sort_picker(ui, true);
                                kit::search_box(ui, &mut self.admin.search, "Search people", 200.0);
                            }
                        });
                    }
                });
                if tab == Tab::Users && !wide {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let w = (ui.available_width() - 158.0).max(100.0);
                        kit::search_box(ui, &mut self.admin.search, "Search people", w);
                        self.sort_picker(ui, false);
                    });
                }
                // The meta line (or the facts strip) sits as close under the toolbar as in Files.
                ui.add_space(16.0);
                match tab {
                    Tab::Users => self.users_ui(ui),
                    Tab::Server => self.server_ui(ui),
                }
            });
        });
        if new_user {
            self.admin.sheet_error = None;
            self.admin.sheet = Some(Sheet::NewUser(NewUser { quota_gb: "1".into(), ..NewUser::default() }));
        }
        if let Some(tab) = switch_to {
            self.page = Page::Admin(tab);
            self.admin_load(tab);
        } else if refresh {
            self.admin_load(tab);
        }
        self.menu_ui(ui);
        self.details_ui(ui, area);
        self.sheet_ui(ui, area);
        if let Some((title, body, shown)) = self.admin.toast.clone() {
            if kit::toast(ui, area, &title, &body) || (shown > 0.0 && now - shown > 8.0) {
                self.admin.toast = None;
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
        }
        let _ = t;
    }

    fn sort_picker(&mut self, ui: &mut Ui, label: bool) {
        let t = Tokens::get(ui.ctx());
        let (sort, _) = self.admin.sort();
        let current = SORTS.iter().position(|(_, s)| *s == sort).unwrap_or(0);
        let names: Vec<&str> = SORTS.iter().map(|(n, _)| *n).collect();
        if let Some(i) = kit::picker(ui, "sort", current, &names, 150.0) {
            let s = SORTS[i].1;
            self.admin.sort = Some((s, matches!(s, Sort::Name | Sort::Role)));
        }
        if label {
            chrome::hud(ui, "Sort", t.muted);
        }
    }

    /// The people, filtered and sorted.
    fn shown_people(&self) -> Vec<Person> {
        let Some(people) = &self.admin.people else { return Vec::new() };
        let q = self.admin.search.trim().to_lowercase();
        let mut list: Vec<Person> = people
            .iter()
            .filter(|p| q.is_empty() || p.username.to_lowercase().contains(&q) || p.email.to_lowercase().contains(&q))
            .cloned()
            .collect();
        let (sort, ascending) = self.admin.sort();
        let rank = |r: &str| match r {
            "owner" => 0,
            "admin" => 1,
            _ => 2,
        };
        list.sort_by(|a, b| {
            let o = match sort {
                Sort::Name => a.username.to_lowercase().cmp(&b.username.to_lowercase()),
                Sort::LastSignIn => a.last_login_at.cmp(&b.last_login_at),
                Sort::Storage => a.storage_used_bytes.cmp(&b.storage_used_bytes),
                Sort::Role => rank(&a.role).cmp(&rank(&b.role)).then_with(|| a.username.cmp(&b.username)),
            };
            if ascending { o } else { o.reverse() }
        });
        list
    }

    fn users_ui(&mut self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        if let Some(e) = self.admin.error.clone() {
            ui.horizontal_wrapped(|ui| {
                error_line(ui, &e);
                if chrome::button(ui, "Retry", Kind::Secondary, None, !self.admin.net.is_busy()).clicked() {
                    self.admin.error = None;
                    self.admin_refresh_people();
                }
            });
            ui.add_space(12.0);
        }
        let Some(all) = self.admin.people.clone() else {
            if self.admin.error.is_none() {
                chrome::hud(ui, "Loading people…", t.muted);
            }
            return;
        };
        let owners = all.iter().filter(|p| p.is_owner()).count();
        let used: u64 = all.iter().map(|p| p.storage_used_bytes).sum();
        let quota: u64 = all.iter().map(|p| p.storage_quota_bytes).sum();
        let people_word = if all.len() == 1 { "person".to_owned() } else { "people".to_owned() };
        let storage = if all.iter().any(|p| p.storage_quota_bytes == 0) {
            format!("{} GB used", gb(used))
        } else {
            format!("{} GB of {} GB", gb(used), gb(quota))
        };
        chrome::meta(
            ui,
            &[
                &format!("{} {people_word}", all.len()),
                &format!("{owners} owner"),
                &format!("{} deactivated", all.iter().filter(|p| !p.active).count()),
                &storage,
            ],
        );
        ui.add_space(8.0);
        let shown = self.shown_people();
        let me = self.me.as_ref().map(|m| m.user.id.clone()).unwrap_or_default();
        let width = ui.available_width();
        let inner = width - 32.0;
        let layout = if inner >= 1000.0 {
            Columns::Wide
        } else if inner >= 600.0 {
            Columns::Medium
        } else {
            Columns::Phone
        };
        let (sort, ascending) = self.admin.sort();
        let mut action: Option<RowAction> = None;
        egui::Frame::new().stroke(t.hairline()).inner_margin(egui::Margin::symmetric(16, 0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let w = ui.available_width();
            let cols = layout.widths(w);
            if layout != Columns::Phone {
                let (head, _) = ui.allocate_exact_size(vec2(w, 42.0), Sense::hover());
                ui.painter().hline(head.x_range(), head.bottom() - 0.5, t.hairline());
                let mut x = head.left();
                for (i, (label, s)) in layout.headers().iter().enumerate() {
                    let cell = Rect::from_min_size(pos2(x, head.top()), vec2(cols[i], 42.0));
                    x += cols[i];
                    if label.is_empty() {
                        continue;
                    }
                    let sorted = s.filter(|s| *s == sort).map(|_| ascending);
                    if kit::header_cell(ui, Rect::from_min_size(cell.min, vec2(cell.width().min(160.0), 42.0)), label, sorted).clicked()
                        && let Some(s) = s
                    {
                        let asc = if *s == sort { !ascending } else { matches!(s, Sort::Name | Sort::Role) };
                        self.admin.sort = Some((*s, asc));
                    }
                }
            }
            if shown.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(w, 80.0), Sense::hover());
                let text = if self.admin.search.trim().is_empty() {
                    "No people yet.".to_owned()
                } else {
                    format!("No one matches \u{201c}{}\u{201d}.", self.admin.search.trim())
                };
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text.clone(), egui::FontId::proportional(14.0), t.muted);
                ui.interact(r, ui.scope_id().with("empty"), Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
            }
            for (i, p) in shown.iter().enumerate() {
                if let Some(a) = self.person_row(ui, p, &cols, layout, p.id == me, i + 1 == shown.len()) {
                    action = Some(a);
                }
            }
        });
        if let Some(a) = action {
            self.row_action(a);
        }
    }

    /// One table row; returns what was clicked.
    fn person_row(&mut self, ui: &mut Ui, p: &Person, cols: &[f32], layout: Columns, is_me: bool, last: bool) -> Option<RowAction> {
        let t = Tokens::get(ui.ctx());
        let w = ui.available_width();
        let h = if layout == Columns::Phone { PHONE_ROW_H } else { 62.0 };
        let (row, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let menu_open = self.admin.menu.as_ref().is_some_and(|(id, _)| id == &p.id);
        if resp.hovered() || menu_open {
            ui.painter().rect_filled(row.expand2(vec2(16.0, 0.0)), 0.0, t.sunken);
        }
        if !last {
            ui.painter().hline(row.expand2(vec2(16.0, 0.0)).x_range(), row.bottom() - 0.5, t.hairline());
        }
        let mut action = None;
        let faded = !p.active;
        let name_cell = |ui: &mut Ui, rect: Rect| {
            ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                kit::avatar(ui, &p.username, 38.0, faded);
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, 38.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let name = if is_me { format!("{} (you)", p.username) } else { p.username.clone() };
                    ui.add(egui::Label::new(RichText::new(name).size(16.0).color(if faded { t.muted } else { t.ink_strong })).truncate());
                    ui.add(egui::Label::new(RichText::new(&p.email).size(12.0).color(t.muted)).truncate());
                });
            });
        };
        let storage_cell = |ui: &mut Ui, rect: Rect| {
            ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                ui.add_space((rect.height() - 26.0) / 2.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(RichText::new(storage_text(p)).size(13.0).color(t.ink));
                kit::storage_meter(ui, rect.width().min(120.0), fraction(p));
            });
        };
        let actions_cell = |ui: &mut Ui, rect: Rect, compact: bool, admin: &mut Admin| -> Option<RowAction> {
            let mut out = None;
            ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let more = kit::more_button(ui, &format!("More for {}", p.username), menu_open);
                if more.clicked() {
                    admin.menu = if menu_open { None } else { Some((p.id.clone(), more.rect.right_bottom())) };
                }
                if !compact {
                    if !p.active {
                        if chrome::button(ui, "Activate", Kind::Secondary, None, true).clicked() {
                            out = Some(RowAction::SetActive(p.id.clone(), true));
                        }
                    } else if chrome::button(ui, "Reset password", Kind::Secondary, None, true).clicked() {
                        out = Some(RowAction::Reset(p.id.clone()));
                    }
                }
            });
            out
        };
        let role_badge = |ui: &mut Ui| chrome::badge(ui, &p.role, role_tone(&t, &p.role));
        let status_badge = |ui: &mut Ui| {
            if p.active {
                chrome::badge(ui, "Active", t.good);
            } else {
                chrome::badge(ui, "Deactivated", t.bad);
            }
        };
        match layout {
            Columns::Phone => {
                // A card: name and email, then the badges and storage, then the actions under
                // them, each on a line of its own (nothing shares a line with the "…").
                let top = Rect::from_min_size(row.min + vec2(0.0, 14.0), vec2(w, 44.0));
                name_cell(ui, top);
                let indent = 48.0;
                let badges = Rect::from_min_size(pos2(row.left() + indent, top.bottom() + 8.0), vec2(w - indent, 20.0));
                ui.scope_builder(UiBuilder::new().max_rect(badges).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    role_badge(ui);
                    status_badge(ui);
                    ui.add(egui::Label::new(RichText::new(storage_text(p)).size(12.0).color(t.muted)).truncate());
                });
                let actions = Rect::from_min_size(pos2(row.left() + indent, badges.bottom() + 12.0), vec2(w - indent, chrome::CONTROL_H));
                ui.scope_builder(UiBuilder::new().max_rect(actions).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if !p.active {
                        if chrome::button(ui, "Activate", Kind::Secondary, None, true).clicked() {
                            action = Some(RowAction::SetActive(p.id.clone(), true));
                        }
                    } else if chrome::button(ui, "Reset password", Kind::Secondary, None, true).clicked() {
                        action = Some(RowAction::Reset(p.id.clone()));
                    }
                    let more = kit::more_button(ui, &format!("More for {}", p.username), menu_open);
                    if more.clicked() {
                        self.admin.menu = if menu_open { None } else { Some((p.id.clone(), more.rect.right_bottom())) };
                    }
                });
            }
            Columns::Medium | Columns::Wide => {
                let mut x = row.left();
                let mut cell = |i: usize| {
                    let r = Rect::from_min_size(pos2(x, row.top()), vec2(cols[i], h));
                    x += cols[i];
                    r
                };
                let c0 = cell(0);
                name_cell(ui, c0.shrink2(vec2(0.0, 0.0)).with_max_x(c0.right() - 12.0));
                let c1 = cell(1);
                ui.scope_builder(UiBuilder::new().max_rect(c1).layout(egui::Layout::left_to_right(egui::Align::Center)), role_badge);
                let c2 = cell(2);
                storage_cell(ui, c2.with_max_x(c2.right() - 16.0));
                let c3 = cell(3);
                ui.scope_builder(UiBuilder::new().max_rect(c3).layout(egui::Layout::left_to_right(egui::Align::Center)), status_badge);
                if layout == Columns::Wide {
                    let c4 = cell(4);
                    let text = p.last_login_at.as_deref().map_or_else(|| "Never".to_owned(), |s| when(s, self.clock, false));
                    ui.scope_builder(UiBuilder::new().max_rect(c4).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.label(RichText::new(text).size(13.0).color(t.muted));
                    });
                    let c5 = cell(5);
                    action = actions_cell(ui, c5, false, &mut self.admin).or(action);
                } else {
                    let c4 = cell(4);
                    action = actions_cell(ui, c4, true, &mut self.admin).or(action);
                }
            }
        }
        // Owners can't be changed by others here, and nobody changes their own role or status.
        let _ = is_me;
        action
    }

    fn row_action(&mut self, a: RowAction) {
        self.admin.menu = None;
        match a {
            RowAction::Details(id) => {
                self.admin.details = Some(Details { id: id.clone(), sessions: None });
                let req = admin::sessions(&self.server, &id);
                self.admin_send(Key::Sessions, req);
                let req = admin::get(&self.server, &id);
                self.admin_send(Key::Person, req);
            }
            RowAction::Reset(id) => {
                if let Some(p) = self.admin.person(&id).cloned() {
                    self.admin.sheet_error = None;
                    self.admin.sheet = Some(Sheet::Reset { id, name: p.username, email: p.email, password: String::new() });
                }
            }
            RowAction::SetActive(id, active) => {
                if let Some(name) = self.admin.person(&id).map(|p| p.username.clone()) {
                    let req = admin::set_active(&self.server, &id, active);
                    self.admin_send(Key::SetActive(name, active), req);
                }
            }
            RowAction::SetRole(id, role) => {
                if let Some(name) = self.admin.person(&id).map(|p| p.username.clone()) {
                    let req = admin::set_role(&self.server, &id, role);
                    self.admin_send(Key::SetRole(name, role), req);
                }
            }
            RowAction::Quota(id, bytes) => {
                let req = admin::set_quota(&self.server, &id, bytes);
                self.admin_send(Key::Quota, req);
            }
            RowAction::Revoke(id) => {
                let req = admin::revoke_session(&self.server, &id);
                self.admin_send(Key::Revoke, req);
            }
        }
    }

    /// The "…" menu of the row it was opened on.
    fn menu_ui(&mut self, ui: &mut Ui) {
        let Some((id, pos)) = self.admin.menu.clone() else { return };
        let Some(p) = self.admin.person(&id).cloned() else {
            self.admin.menu = None;
            return;
        };
        let me = self.me.as_ref().is_some_and(|m| m.user.id == p.id);
        let mut items: Vec<(&str, bool, RowAction)> =
            vec![("Details", false, RowAction::Details(id.clone())), ("Reset password…", false, RowAction::Reset(id.clone()))];
        if !p.is_owner() && !me {
            if p.role == "admin" {
                items.push(("Make user", false, RowAction::SetRole(id.clone(), Role::User)));
            } else {
                items.push(("Make admin", false, RowAction::SetRole(id.clone(), Role::Admin)));
            }
            if p.active {
                items.push(("Deactivate", true, RowAction::SetActive(id.clone(), false)));
            } else {
                items.push(("Activate", false, RowAction::SetActive(id.clone(), true)));
            }
        }
        let labels: Vec<(&str, bool)> = items.iter().map(|(l, d, _)| (*l, *d)).collect();
        let width = 220.0;
        let opened_now = ui.input(|i| i.pointer.any_click()) && ui.ctx().pointer_interact_pos().is_some_and(|q| q.distance(pos) < 40.0);
        match kit::menu(ui, egui::Id::unique(("admin-menu", &id)), pos2(pos.x - width, pos.y + 2.0), width, &labels, opened_now) {
            kit::MenuResult::Picked(i) => {
                let a = items.swap_remove(i).2;
                self.row_action(a);
            }
            kit::MenuResult::Dismissed => self.admin.menu = None,
            kit::MenuResult::Open => {}
        }
    }

    fn sheet_ui(&mut self, ui: &mut Ui, area: Rect) {
        let Some(mut sheet) = self.admin.sheet.take() else { return };
        kit::block(ui, area, "admin-sheet");
        let busy = self.admin.net.is_busy();
        let mut close = false;
        let mut send: Option<(Key, HttpRequest)> = None;
        let error = self.admin.sheet_error.clone();
        match &mut sheet {
            Sheet::NewUser(f) => {
                let mut submit = false;
                chrome::sheet(
                    ui,
                    area,
                    vec2(640.0, 470.0),
                    &chrome::Hero { title: "New user", caption: "Admin · people", keep_case: false },
                    |ui, part| match part {
                        chrome::Part::Body => {
                            let wide = ui.available_width() >= 520.0;
                            let gap = 16.0;
                            let col = if wide { ((ui.available_width() - gap) / 2.0).floor() } else { ui.available_width() };
                            let pairs: [[(&str, &mut String, bool, &str); 2]; 2] = [
                                [("Username", &mut f.username, false, "Lower case, no spaces. Used to sign in."), ("Email", &mut f.email, false, "")],
                                [
                                    ("Password", &mut f.password, true, "At least 8 characters. Share it in person."),
                                    ("Storage (GB)", &mut f.quota_gb, false, "Empty or 0: no limit."),
                                ],
                            ];
                            for pair in pairs {
                                if wide {
                                    ui.horizontal_top(|ui| {
                                        ui.spacing_mut().item_spacing.x = gap;
                                        for (label, value, pw, hint) in pair {
                                            ui.allocate_ui_with_layout(vec2(col, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                                chrome::field(ui, label, value, pw, Some(hint).filter(|h| !h.is_empty()));
                                            });
                                        }
                                    });
                                } else {
                                    for (label, value, pw, hint) in pair {
                                        chrome::field(ui, label, value, pw, Some(hint).filter(|h| !h.is_empty()));
                                        ui.add_space(10.0);
                                    }
                                }
                                ui.add_space(12.0);
                            }
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Role").size(14.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if let Some(i) = kit::cells(ui, &["User", "Admin"], Some(usize::from(f.admin)), true) {
                                        f.admin = i == 1;
                                    }
                                });
                            });
                            if f.admin {
                                ui.label(
                                    RichText::new("Admins manage people and the server, like you.").size(12.0).color(Tokens::get(ui.ctx()).muted),
                                );
                            }
                            if let Some(e) = &error {
                                ui.add_space(10.0);
                                error_line(ui, e);
                            }
                        }
                        chrome::Part::ActionsLeft => {
                            if chrome::button(ui, "Cancel", Kind::Secondary, None, true).clicked() {
                                close = true;
                            }
                        }
                        chrome::Part::ActionsRight => {
                            let ready = !f.username.trim().is_empty() && !f.email.trim().is_empty() && !f.password.is_empty() && !busy;
                            if chrome::button(ui, if busy { "Creating…" } else { "Create" }, Kind::Primary, Some(Icon::Plus), ready).clicked() {
                                submit = true;
                            }
                        }
                        chrome::Part::HeroRight => {}
                    },
                );
                if submit {
                    match new_user_request(f) {
                        Ok((quota, role)) => {
                            let req = admin::create(&self.server, f.username.trim(), f.email.trim(), &f.password, quota, role);
                            send = Some((Key::Create(f.username.trim().to_owned()), req));
                        }
                        Err(e) => self.admin.sheet_error = Some(e),
                    }
                }
            }
            Sheet::Reset { id, name, email, password } => {
                let mut submit = false;
                let caption = format!("{name} · {email}");
                chrome::sheet(
                    ui,
                    area,
                    vec2(520.0, 380.0),
                    &chrome::Hero { title: "Reset password", caption: &caption, keep_case: false },
                    |ui, part| match part {
                        chrome::Part::Body => {
                            let pw =
                                chrome::field(ui, "New password", password, true, Some(&format!("At least 8 characters. Tell {name} in person.")));
                            submit = pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("Their desktop clients stay connected; only the password changes.")
                                    .size(12.5)
                                    .color(Tokens::get(ui.ctx()).muted),
                            );
                            if let Some(e) = &error {
                                ui.add_space(10.0);
                                error_line(ui, e);
                            }
                        }
                        chrome::Part::ActionsLeft => {
                            if chrome::button(ui, "Cancel", Kind::Secondary, None, true).clicked() {
                                close = true;
                            }
                        }
                        chrome::Part::ActionsRight => {
                            if chrome::button(
                                ui,
                                if busy { "Saving…" } else { "Save" },
                                Kind::Primary,
                                Some(Icon::Check),
                                !password.is_empty() && !busy,
                            )
                            .clicked()
                            {
                                submit = true;
                            }
                        }
                        chrome::Part::HeroRight => {}
                    },
                );
                if submit && !password.is_empty() && !busy {
                    if password.chars().count() < 8 {
                        self.admin.sheet_error = Some("At least 8 characters.".into());
                    } else {
                        send = Some((Key::Reset(name.clone()), admin::reset_password(&self.server, id, password)));
                    }
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if !close {
            self.admin.sheet = Some(sheet);
        }
        if let Some((key, req)) = send {
            self.admin.sheet_error = None;
            self.admin_send(key, req);
        }
    }

    /// The details overlay of one person.
    fn details_ui(&mut self, ui: &mut Ui, area: Rect) {
        let Some(id) = self.admin.details.as_ref().map(|d| d.id.clone()) else { return };
        let Some(p) = self.admin.person(&id).cloned() else {
            self.admin.details = None;
            return;
        };
        let t = Tokens::get(ui.ctx());
        let sessions = self.admin.details.as_ref().and_then(|d| d.sessions.clone());
        let me = self.me.as_ref().is_some_and(|m| m.user.id == p.id);
        let rect = kit::overlay(ui, area, vec2(area.width() - 64.0, area.height() - 40.0), "details");
        let phone = rect.width() < 640.0;
        let mut action: Option<RowAction> = None;
        let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape)) && self.admin.sheet.is_none();
        ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.set_clip_rect(rect);
            ui.spacing_mut().item_spacing.y = 0.0;
            let hero =
                egui::Frame::new().fill(kit::hero_fill(&t)).inner_margin(egui::Margin { left: 24, right: 24, top: 26, bottom: 22 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 18.0;
                        if !phone {
                            kit::avatar(ui, &p.username, 64.0, !p.active);
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            chrome::meta(ui, &["Person", &p.username]);
                            ui.label(
                                RichText::new(&p.username).size(if phone { 26.0 } else { 32.0 }).family(crate::theme::display()).color(t.ink_strong),
                            );
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                chrome::badge(ui, &p.role, role_tone(&t, &p.role));
                                if p.active {
                                    chrome::badge(ui, "Active", t.good);
                                } else {
                                    chrome::badge(ui, "Deactivated", t.bad);
                                }
                                ui.label(RichText::new(&p.email).size(13.0).color(t.muted));
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            if kit::close_button(ui).clicked() {
                                close = true;
                            }
                            if !phone {
                                if !p.is_owner() && !me {
                                    if p.active {
                                        if chrome::button(ui, "Deactivate", Kind::Danger, None, true).clicked() {
                                            action = Some(RowAction::SetActive(p.id.clone(), false));
                                        }
                                    } else if chrome::button(ui, "Activate", Kind::Secondary, None, true).clicked() {
                                        action = Some(RowAction::SetActive(p.id.clone(), true));
                                    }
                                }
                                if chrome::button(ui, "Reset password", Kind::Secondary, None, true).clicked() {
                                    action = Some(RowAction::Reset(p.id.clone()));
                                }
                            }
                        });
                    });
                });
            let r = hero.response.rect;
            ui.painter().hline(r.x_range(), r.bottom() - 0.5, t.hairline());
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::same(24)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    if phone {
                        ui.horizontal_wrapped(|ui| {
                            if chrome::button(ui, "Reset password", Kind::Secondary, None, true).clicked() {
                                action = Some(RowAction::Reset(p.id.clone()));
                            }
                            if !p.is_owner() && !me {
                                if p.active {
                                    if chrome::button(ui, "Deactivate", Kind::Danger, None, true).clicked() {
                                        action = Some(RowAction::SetActive(p.id.clone(), false));
                                    }
                                } else if chrome::button(ui, "Activate", Kind::Secondary, None, true).clicked() {
                                    action = Some(RowAction::SetActive(p.id.clone(), true));
                                }
                            }
                        });
                        ui.add_space(14.0);
                    }
                    let wide = ui.available_width() >= 760.0;
                    let gap = 24.0;
                    let col = if wide { ((ui.available_width() - gap) / 2.0).floor() } else { ui.available_width() };
                    let clock = self.clock;
                    let left = |ui: &mut Ui, action: &mut Option<RowAction>| {
                        chrome::panel(ui, "Storage", |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{} GB", gb(p.storage_used_bytes)))
                                        .size(26.0)
                                        .family(crate::theme::display())
                                        .color(t.ink_strong),
                                );
                                let (of, tone) = match fraction(&p) {
                                    Some(f) => {
                                        (format!("of {} GB · {:.0} %", gb(p.storage_quota_bytes), f * 100.0), if f > 0.8 { t.warn } else { t.muted })
                                    }
                                    None => ("no limit".to_owned(), t.muted),
                                };
                                ui.label(RichText::new(of).size(14.0).color(tone));
                            });
                            ui.add_space(4.0);
                            kit::storage_meter(ui, ui.available_width(), fraction(&p));
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Quota").size(14.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let mut names: Vec<String> =
                                        QUOTAS.iter().map(|q| if *q == 0 { "No limit".into() } else { format!("{q} GB") }).collect();
                                    let current = QUOTAS.iter().position(|q| q * GIB == p.storage_quota_bytes);
                                    let current = current.unwrap_or_else(|| {
                                        names.push(format!("{} GB", gb(p.storage_quota_bytes)));
                                        names.len() - 1
                                    });
                                    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                                    if let Some(i) = kit::picker(ui, "quota", current, &refs, 150.0)
                                        && let Some(q) = QUOTAS.get(i)
                                    {
                                        *action = Some(RowAction::Quota(p.id.clone(), q * GIB));
                                    }
                                });
                            });
                        });
                        ui.add_space(14.0);
                        chrome::panel(ui, "Role", |ui| {
                            ui.horizontal(|ui| {
                                let note = if p.is_owner() {
                                    "The owner. Ownership can't be moved here."
                                } else if me {
                                    "Your own role can't be changed by you."
                                } else {
                                    "Admins manage people and the server."
                                };
                                ui.label(RichText::new(note).size(13.0).color(t.muted));
                                if !p.is_owner() {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let current = usize::from(p.role == "admin");
                                        if let Some(i) = kit::cells(ui, &["User", "Admin"], Some(current), !me)
                                            && i != current
                                        {
                                            *action = Some(RowAction::SetRole(p.id.clone(), if i == 1 { Role::Admin } else { Role::User }));
                                        }
                                    });
                                }
                            });
                        });
                        ui.add_space(14.0);
                        chrome::panel(ui, "Desktop clients", |ui| match &sessions {
                            None => {
                                chrome::hud(ui, "Loading…", t.muted);
                            }
                            Some(list) => {
                                let devices: Vec<&Session> = list.iter().filter(|s| s.is_device()).collect();
                                if devices.is_empty() {
                                    ui.label(RichText::new("None connected.").size(13.0).color(t.muted));
                                }
                                for s in devices {
                                    ui.horizontal(|ui| {
                                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                                        ui.painter().circle_filled(dot.center(), 4.0, if s.is_online { t.good } else { t.muted });
                                        ui.label(RichText::new("Desktop client").size(14.0));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if chrome::button(ui, "Revoke", Kind::Secondary, None, true).clicked() {
                                                *action = Some(RowAction::Revoke(s.id.clone()));
                                            }
                                            let seen = s.last_seen_at.as_deref().map_or_else(String::new, |w| when(w, clock, true));
                                            chrome::hud(ui, &seen, t.muted);
                                        });
                                    });
                                }
                            }
                        });
                    };
                    let right = |ui: &mut Ui, action: &mut Option<RowAction>| {
                        chrome::panel(ui, "Sign-ins", |ui| match &sessions {
                            None => {
                                chrome::hud(ui, "Loading…", t.muted);
                            }
                            Some(list) if list.is_empty() => {
                                ui.label(RichText::new("Not signed in anywhere right now.").size(13.0).color(t.muted));
                            }
                            Some(list) => {
                                for (i, s) in list.iter().enumerate() {
                                    if i > 0 {
                                        crate::widgets::hairline(ui);
                                    }
                                    ui.horizontal(|ui| {
                                        ui.set_min_height(32.0);
                                        ui.label(RichText::new(session_label(s)).size(14.0));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if !s.is_device() && chrome::button(ui, "Sign out", Kind::Secondary, None, true).clicked() {
                                                *action = Some(RowAction::Revoke(s.id.clone()));
                                            }
                                            let seen = s.last_seen_at.as_deref().map_or_else(String::new, |w| when(w, clock, true));
                                            ui.label(RichText::new(seen).font(egui::FontId::monospace(12.0)).color(t.muted));
                                        });
                                    });
                                }
                            }
                        });
                        ui.add_space(14.0);
                        let rows = [
                            ("Username", p.username.clone()),
                            ("Email", p.email.clone()),
                            ("Joined", p.created_at.as_deref().map_or_else(|| "—".to_owned(), |s| when(s, clock, true))),
                            ("Last sign-in", p.last_login_at.as_deref().map_or_else(|| "Never".to_owned(), |s| when(s, clock, true))),
                        ];
                        chrome::facts(ui, "Account", &rows);
                    };
                    if wide {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            ui.allocate_ui_with_layout(vec2(col, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| left(ui, &mut action));
                            ui.allocate_ui_with_layout(vec2(col, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| right(ui, &mut action));
                        });
                    } else {
                        left(ui, &mut action);
                        ui.add_space(14.0);
                        right(ui, &mut action);
                    }
                    if let Some(e) = &self.admin.error {
                        ui.add_space(12.0);
                        error_line(ui, e);
                    }
                });
            });
        });
        if close {
            self.admin.details = None;
        }
        if let Some(a) = action {
            self.row_action(a);
        }
    }

    /// Admin › Server: facts, the web apps and single sign-on.
    fn server_ui(&mut self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        let bundled = self.catalog.as_ref().map(Vec::len);
        let available = bundled.is_some_and(|n| n > 0);
        let ready = self.catalog.is_some() && self.admin.server_loaded;
        let version = self.config.as_ref().map_or_else(|| "unknown".to_owned(), |c| c.version.clone());
        let image = match bundled {
            None => "",
            Some(0) => " · slim image",
            Some(_) => " · -apps image",
        };
        let public = self.config.as_ref().map_or_else(|| self.server.to_string(), |c| c.public_url.clone());
        kit::facts_strip(
            ui,
            &[
                ("CraftCloud", format!("{version}{image}")),
                ("OxiCloud", "0.10.0 · unmodified".into()),
                ("Public URL", public),
                ("Versions kept", "5 per file".into()),
            ],
        );
        ui.add_space(28.0);

        // Web apps.
        let apps = self.catalog.clone().unwrap_or_default();
        let offered = apps.len() - apps.iter().filter(|a| self.web_apps.disabled.contains(&a.id)).count();
        let changed = self.admin.saved_choice.as_ref().is_some_and(|(on, off)| {
            let mut a = off.clone();
            a.sort();
            let mut b = self.web_apps.disabled.clone();
            b.sort();
            *on != self.web_apps.on || a != b
        });
        let busy = self.admin.net.is_busy();
        let mut save = false;
        // On a phone SAVE and the search get a row of their own, under the switches.
        let narrow = ui.available_width() < 600.0;
        let lists = self.web_apps.on && available;
        let save_and_search = |ui: &mut Ui, search: &mut String, save: &mut bool| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if chrome::button(ui, if busy && changed { "Saving…" } else { "Save" }, Kind::Primary, Some(Icon::Check), changed && !busy).clicked()
                {
                    *save = true;
                }
                if lists && (narrow || ui.available_width() > 260.0) {
                    kit::search_box(ui, search, "Search apps", if narrow { ui.available_width() } else { 200.0 });
                }
            });
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            chrome::hud(ui, "Web apps", t.ink);
            if let Some(i) = kit::cells(ui, &["Off", "On"], Some(usize::from(self.web_apps.on)), ready && available) {
                self.web_apps.on = i == 1;
            }
            match kit::cells(ui, &["All", "None"], None, lists) {
                Some(0) => self.web_apps.disabled.clear(),
                Some(_) => self.web_apps.disabled = apps.iter().map(|a| a.id.clone()).collect(),
                None => {}
            }
            if !ready {
                chrome::hud(ui, "Loading…", t.muted);
            } else if !available {
                chrome::badge(ui, "Needs the -apps image", t.warn);
            } else if self.web_apps.on {
                ui.label(RichText::new(format!("{offered} of {} on", apps.len())).size(13.0).color(t.muted));
            }
            if !narrow {
                save_and_search(ui, &mut self.admin.app_search, &mut save);
            }
        });
        if narrow {
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), chrome::CONTROL_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                save_and_search(ui, &mut self.admin.app_search, &mut save);
            });
        }
        ui.add_space(8.0);
        if ready && !available {
            ui.label(
                RichText::new(format!(
                    "This server runs the slim image: web apps need craftcloud:{version}-apps (about 770 MB download, 2.1 GB on disk), then CRAFTCLOUD_WEB_APPS=on or the switch above."
                ))
                .size(13.0)
                .color(t.warn),
            );
        } else {
            ui.label(
                RichText::new(if changed {
                    "Not saved yet. Stored on the server in /app/storage/.craftcloud/settings.env."
                } else {
                    "Changes apply at once after SAVE. Stored on the server in /app/storage/.craftcloud/settings.env."
                })
                .size(13.0)
                .color(if changed { t.warn } else { t.muted }),
            );
        }
        if let Some(e) = &self.admin.server_error {
            ui.add_space(6.0);
            error_line(ui, e);
        }
        ui.add_space(10.0);
        if available && ready {
            if self.web_apps.on {
                let q = self.admin.app_search.trim().to_lowercase();
                self.web_app_grid(ui, &q);
            } else {
                ui.label(RichText::new("Off: files open only in the desktop apps, on computers that have them.").size(13.0).color(t.muted));
            }
        }
        if save {
            let mut choice = ServerSettings { web_apps: self.web_apps.on, disabled_apps: self.web_apps.disabled.clone() };
            choice.normalise();
            let body = serde_json::to_value(&choice).unwrap_or_default();
            let req = HttpRequest::new(Method::Put, self.server.join("craftcloud/api/settings")).json(&body);
            self.admin.server_error = None;
            self.admin_send(Key::SaveSettings, req);
        }
        ui.add_space(36.0);
        self.sso_ui(ui);
    }

    /// The single sign-on settings (OxiCloud's OIDC admin settings).
    fn sso_ui(&mut self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        let Some(saved) = self.admin.oidc.clone() else {
            kit::section_title(ui, "Single sign-on", None);
            ui.add_space(8.0);
            match &self.admin.sso_error {
                Some(e) => error_line(ui, e),
                None => {
                    chrome::hud(ui, "Loading…", t.muted);
                }
            }
            return;
        };
        let state =
            if saved.enabled { format!("On · {}", if saved.provider_name.is_empty() { "SSO" } else { &saved.provider_name }) } else { "Off".into() };
        ui.horizontal(|ui| {
            chrome::hud(ui, "Single sign-on", t.ink);
            ui.add_space(8.0);
            chrome::badge(ui, &state, if saved.enabled { t.good } else { t.muted });
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new("Let people sign in with an OpenID Connect identity provider. docs/OIDC.md shows how to register CraftCloud there.")
                .size(13.0)
                .color(t.muted),
        );
        ui.add_space(12.0);
        let busy = self.admin.net.is_busy();
        let original = SsoForm::from_settings(&saved);
        let dirty = self.admin.sso != original;
        let mut save = false;
        let mut test = false;
        egui::Frame::new().stroke(t.hairline()).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let wide = ui.available_width() >= 720.0;
            let gap = 24.0;
            let col = if wide { ((ui.available_width() - gap) / 2.0).floor() } else { ui.available_width() };
            let env_hint = |field: &str, env: &str| saved.from_env(field).then(|| format!("Set by OXICLOUD_OIDC_{env}."));
            let f = &mut self.admin.sso;
            let secret_hint = if saved.client_secret_set { "Stored. Type a new one to replace it." } else { "Not set yet." };
            let left = |ui: &mut Ui, f: &mut SsoForm| {
                for (label, value, field, env, hint, pw) in [
                    ("Button name", &mut f.provider_name, "provider_name", "PROVIDER_NAME", "Shown as \u{201c}Sign in with …\u{201d}.", false),
                    ("Issuer URL", &mut f.issuer_url, "issuer_url", "ISSUER_URL", "For example https://id.example.org", false),
                    ("Client ID", &mut f.client_id, "client_id", "CLIENT_ID", "", false),
                    ("Client secret", &mut f.client_secret, "client_secret", "CLIENT_SECRET", secret_hint, true),
                ] {
                    let locked = env_hint(field, env);
                    ui.add_enabled_ui(locked.is_none(), |ui| {
                        chrome::field(ui, label, value, pw, locked.as_deref().or(Some(hint).filter(|h| !h.is_empty())));
                    });
                    ui.add_space(10.0);
                }
            };
            let right = |ui: &mut Ui, f: &mut SsoForm| {
                for (label, value, field, env, hint) in [
                    ("Scopes", &mut f.scopes, "scopes", "SCOPES", "Usually openid profile email (add groups for admin groups)."),
                    ("Admin groups", &mut f.admin_groups, "admin_groups", "ADMIN_GROUPS", "Members of these groups (comma-separated) become admins."),
                ] {
                    let locked = env_hint(field, env);
                    ui.add_enabled_ui(locked.is_none(), |ui| {
                        chrome::field(ui, label, value, false, locked.as_deref().or(Some(hint)));
                    });
                    ui.add_space(10.0);
                }
                for (label, value, field, env) in [
                    ("Sign-in with the provider", &mut f.enabled, "enabled", "ENABLED"),
                    ("Create accounts at first sign-in", &mut f.auto_provision, "auto_provision", "AUTO_PROVISION"),
                    ("Password sign-in", &mut f.password_login, "disable_password_login", "DISABLE_PASSWORD_LOGIN"),
                ] {
                    let locked = env_hint(field, env);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).size(14.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(i) = kit::cells(ui, &["Off", "On"], Some(usize::from(*value)), locked.is_none()) {
                                *value = i == 1;
                            }
                        });
                    });
                    if let Some(l) = locked {
                        ui.label(RichText::new(l).size(12.0).color(Tokens::get(ui.ctx()).muted));
                    }
                    ui.add_space(8.0);
                }
            };
            if wide {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(vec2(col, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| left(ui, f));
                    ui.allocate_ui_with_layout(vec2(col, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| right(ui, f));
                });
            } else {
                left(ui, f);
                right(ui, f);
            }
            ui.add_space(6.0);
            chrome::facts(ui, "Register with your provider", &[("Redirect URI", saved.callback_url.clone()), ("Sign-out", "not used".into())]);
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "The redirect URI follows CRAFTCLOUD_PUBLIC_URL. Settings from OXICLOUD_OIDC_* in .env win over these and can't be changed here.",
                )
                .size(12.0)
                .color(t.muted),
            );
            ui.add_space(12.0);
            // "Test connection" answers on a line of its own: the server's words can be long.
            match &self.admin.sso_test {
                Some((true, note)) => {
                    ui.label(RichText::new(note).size(13.0).color(t.good));
                    ui.add_space(8.0);
                }
                Some((false, why)) => {
                    error_line(ui, why);
                    ui.add_space(8.0);
                }
                None => {}
            }
            ui.horizontal(|ui| {
                if let Some(e) = &self.admin.sso_error {
                    error_line(ui, e);
                } else if let Some(n) = self.admin.sso_note.as_ref().filter(|_| !dirty) {
                    ui.label(RichText::new(n).size(13.0).color(t.good));
                } else if dirty {
                    ui.label(RichText::new("Not saved yet.").size(13.0).color(t.warn));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if chrome::button(ui, "Save single sign-on", Kind::Primary, Some(Icon::Check), dirty && !busy).clicked() {
                        save = true;
                    }
                    let issuer = !self.admin.sso.issuer_url.trim().is_empty();
                    if chrome::button(ui, "Test connection", Kind::Secondary, None, issuer && !busy).clicked() {
                        test = true;
                    }
                    if dirty && chrome::button(ui, "Undo", Kind::Secondary, None, !busy).clicked() {
                        self.admin.sso = original.clone();
                        self.admin.sso_error = None;
                    }
                });
            });
        });
        if test {
            self.admin.sso_error = None;
            self.admin.sso_test = None;
            let req = oidc::test_connection(&self.server, &self.admin.sso.issuer_url);
            self.admin_send(Key::TestOidc, req);
        }
        if save {
            self.admin.sso_test = None;
            let f = &self.admin.sso;
            let has_secret = saved.client_secret_set || !f.client_secret.is_empty();
            if f.enabled && (f.issuer_url.trim().is_empty() || f.client_id.trim().is_empty() || !has_secret) {
                self.admin.sso_error = Some("To switch it on, fill in the issuer URL, the client ID and the client secret.".into());
            } else if !f.issuer_url.trim().is_empty() && !f.issuer_url.trim().starts_with("https://") && !f.issuer_url.trim().starts_with("http://") {
                self.admin.sso_error = Some("The issuer URL starts with https://".into());
            } else if !f.password_login && !f.enabled {
                self.admin.sso_error = Some("Password sign-in can only be switched off while sign-in with the provider is on.".into());
            } else {
                self.admin.sso_error = None;
                self.admin.sso_note = None;
                let req = oidc::save_settings(&self.server, &f.to_save());
                self.admin.sso.client_secret.clear();
                self.admin_send(Key::SaveOidc, req);
            }
        }
    }
}

/// What a row, its menu or the details overlay asked for.
enum RowAction {
    Details(String),
    Reset(String),
    SetActive(String, bool),
    SetRole(String, Role),
    Quota(String, u64),
    Revoke(String),
}

/// A user card's height on a phone: 14 + name 44 + 8 + badges 20 + 12 + actions 34 + 14.
const PHONE_ROW_H: f32 = 146.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Columns {
    /// Name, role, storage, status, last sign-in, actions.
    Wide,
    /// Name, role, storage, status, "…".
    Medium,
    /// Two-line rows.
    Phone,
}

impl Columns {
    fn widths(self, w: f32) -> Vec<f32> {
        match self {
            Self::Wide => {
                let fixed = [150.0, 150.0, 132.0, 124.0, 206.0];
                let mut v = vec![(w - fixed.iter().sum::<f32>()).max(220.0)];
                v.extend(fixed);
                v
            }
            Self::Medium => {
                let fixed = [96.0, 140.0, 120.0, 50.0];
                let mut v = vec![(w - fixed.iter().sum::<f32>()).max(160.0)];
                v.extend(fixed);
                v
            }
            Self::Phone => vec![w],
        }
    }

    fn headers(self) -> Vec<(&'static str, Option<Sort>)> {
        match self {
            Self::Wide => vec![
                ("Name", Some(Sort::Name)),
                ("Role", Some(Sort::Role)),
                ("Storage", Some(Sort::Storage)),
                ("Status", None),
                ("Last sign-in", Some(Sort::LastSignIn)),
                ("", None),
            ],
            Self::Medium => {
                vec![("Name", Some(Sort::Name)), ("Role", Some(Sort::Role)), ("Storage", Some(Sort::Storage)), ("Status", None), ("", None)]
            }
            Self::Phone => vec![],
        }
    }
}

fn new_user_request(f: &NewUser) -> Result<(Option<u64>, Role), String> {
    let name = f.username.trim();
    if name.chars().any(char::is_whitespace) {
        return Err("The username can't have spaces.".into());
    }
    if !f.email.contains('@') {
        return Err("That email address is missing an @.".into());
    }
    if f.password.chars().count() < 8 {
        return Err("The password needs at least 8 characters.".into());
    }
    let q = f.quota_gb.trim().replace(',', ".");
    let quota = if q.is_empty() {
        Some(0)
    } else {
        match q.parse::<f64>() {
            Ok(v) if v >= 0.0 && v.is_finite() => Some((v * GIB as f64).round() as u64),
            _ => return Err("Storage is a number of GB, like 1 or 2.5.".into()),
        }
    };
    Ok((quota, if f.admin { Role::Admin } else { Role::User }))
}

fn role_tone(t: &Tokens, role: &str) -> egui::Color32 {
    match role {
        "owner" => t.accent_ink,
        "admin" => t.good,
        _ => t.muted,
    }
}

/// `12.4` (GiB, one decimal; whole numbers without).
pub(super) fn gb(bytes: u64) -> String {
    let v = bytes as f64 / GIB as f64;
    if (v - v.round()).abs() < 0.05 && v >= 1.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

fn fraction(p: &Person) -> Option<f32> {
    (p.storage_quota_bytes > 0).then(|| p.storage_used_bytes as f32 / p.storage_quota_bytes as f32)
}

fn storage_text(p: &Person) -> String {
    if p.storage_quota_bytes == 0 {
        format!("{} GB · no limit", gb(p.storage_used_bytes))
    } else {
        format!("{} of {} GB", gb(p.storage_used_bytes), gb(p.storage_quota_bytes))
    }
}

/// "Desktop client", "Web · Firefox", "Single sign-on · Safari".
fn session_label(s: &Session) -> String {
    if s.is_device() {
        return "Desktop client".into();
    }
    let ua = s.user_agent.as_deref().unwrap_or("");
    let browser = if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome/") || ua.contains("Chromium/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else if ua.is_empty() {
        "unknown"
    } else {
        "other client"
    };
    let how = if s.origin == "oidc" { "Single sign-on" } else { "Web" };
    format!("{how} · {browser}")
}

/// The browser's clock, for "Today 18:02" (set by the web app at start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub now_unix: i64,
    /// Minutes to add to UTC for local time.
    pub utc_offset_minutes: i32,
}

/// `2026-11-13T21:54:22.12Z` → "Today 21:54", "Yesterday 21:54" or "13 Nov 2026" (with the time
/// too when `long`). Without a clock: the UTC date, and "21:54 UTC" when `long`.
pub(super) fn when(rfc3339: &str, clock: Option<Clock>, long: bool) -> String {
    let Some(unix) = parse_rfc3339(rfc3339) else { return rfc3339.to_owned() };
    let offset = clock.map_or(0, |c| i64::from(c.utc_offset_minutes) * 60);
    let local = unix + offset;
    let day = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(day);
    let hm = format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let date = format!("{d} {} {y}", MONTHS[(m as usize).clamp(1, 12) - 1]);
    match clock {
        Some(c) => {
            let today = (c.now_unix + offset).div_euclid(86_400);
            match today - day {
                0 => format!("Today {hm}"),
                1 => format!("Yesterday {hm}"),
                _ if long => format!("{date}, {hm}"),
                _ => date,
            }
        }
        None if long => format!("{date}, {hm} UTC"),
        None => date,
    }
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let rest = &s[19..];
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest {
        "" | "Z" | "z" => 0,
        tz => {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            let tz = tz.trim_start_matches(['+', '-']);
            let (th, tm) = tz.split_once(':').unwrap_or((tz.get(0..2)?, tz.get(2..4).unwrap_or("0")));
            sign * (th.parse::<i64>().ok()? * 3600 + tm.parse::<i64>().ok()? * 60)
        }
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(super) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

#[cfg(test)]
mod tests;
