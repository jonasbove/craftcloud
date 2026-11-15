//! The public page a share link opens (`/s/<token>`): no sign-in. It asks for the link's password
//! when it has one, then shows the file (or folder) with a download button. A shared folder's
//! subfolders open in place, with breadcrumbs back up to the shared folder, never above it (T-015).
//!
//! OxiCloud unlocks a link by setting its `oxi_shares` cookie on the answer of `GET /api/s/{token}`
//! (or of the password check), after which the item downloads by its id like any file. That
//! cookie is `Secure`, so links work over HTTPS (and on `localhost`), not over plain HTTP. The
//! same cookie lets a visitor list and download the shared folder's subfolders: OxiCloud 0.10.0's
//! `GET /api/folders/{id}/resources` and `GET /api/folders/{id}/download` accept it, and a link's
//! read right reaches everything below its folder.
//!
//! Everything the server sends is shown as data, never trusted: names are cleaned before they
//! are shown or used for a saved file, a folder lists at most [`MAX_ITEMS`] items, and folders
//! open at most [`MAX_DEPTH`] levels below the shared one.

use craftcloud_core::ServerUrl;
use craftcloud_core::api::resources::{File, Resource};
use craftcloud_core::api::shares::{self, Access, Share};
use craftcloud_core::api::{files, folders};
use egui::{Rect, RichText, Ui};

use crate::backend::{Backend, Net, NetResult};
use crate::chrome::{self, Icon, Kind};
use crate::theme::Tokens;
use crate::widgets::error_line;

/// How many items of a folder are listed: OxiCloud answers at most 200 a page, so five pages.
const MAX_ITEMS: usize = 1000;
/// Items asked for per page (OxiCloud's most).
const PAGE: u32 = 200;
/// How many levels below the shared folder a visitor can open.
const MAX_DEPTH: usize = 32;
/// A row of the folder list, as in the frames.
const ROW_H: f32 = 40.0;
/// The longest name shown, in characters.
const MAX_NAME: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Access,
    Verify,
    File,
    /// A page of the open folder's listing (`n` tells stale answers apart).
    Folder(u64),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum State {
    Loading,
    Password,
    Expired,
    Gone,
    Open(Box<Share>),
}

pub struct SharePage {
    server: ServerUrl,
    token: String,
    net: Net<Key>,
    pub(crate) state: State,
    pub(crate) password: String,
    error: Option<String>,
    file: Option<File>,
    /// Why the shared file's details couldn't be read.
    file_error: Option<String>,
    folder: Vec<Resource>,
    /// The open folder's listing has come in (all of it, or up to [`MAX_ITEMS`]).
    listed: bool,
    /// More items than [`MAX_ITEMS`]: only the first are shown.
    capped: bool,
    /// Pages asked for of the open folder.
    pages: usize,
    /// Why the open folder couldn't be listed; it is never shown as empty then.
    list_error: Option<String>,
    /// The folders open, the shared one first: `(id, name)`.
    trail: Vec<(String, String)>,
    /// Bumped on every folder change.
    listing: u64,
}

impl SharePage {
    /// Starts loading the link `token`. A token no OxiCloud link could have is gone at once,
    /// without asking the server.
    pub fn new(backend: &mut dyn Backend, server: ServerUrl, token: &str) -> Self {
        let mut net = Net::default();
        let state = if plausible_token(token) {
            net.send(backend, Key::Access, shares::access(&server, token));
            State::Loading
        } else {
            State::Gone
        };
        Self {
            server,
            token: token.chars().take(128).collect(),
            net,
            state,
            password: String::new(),
            error: None,
            file: None,
            file_error: None,
            folder: Vec::new(),
            listed: false,
            capped: false,
            pages: 0,
            list_error: None,
            trail: Vec::new(),
            listing: 0,
        }
    }

    pub fn receive(&mut self, backend: &mut dyn Backend, answers: &mut Vec<(u64, NetResult)>) {
        for (key, result) in self.net.receive(answers) {
            match key {
                Key::Access | Key::Verify => match result {
                    Ok(resp) => self.on_access(backend, &key, shares::parse_access(&resp)),
                    Err(why) => self.error = Some(why),
                },
                Key::File => match result.and_then(|r| files::parse_file(&r).map_err(|e| e.to_string())) {
                    Ok(f) => self.file = Some(f),
                    Err(why) => self.file_error = Some(why),
                },
                Key::Folder(n) if n == self.listing => self.on_page(backend, result),
                Key::Folder(_) => {}
            }
        }
    }

    fn on_access(&mut self, backend: &mut dyn Backend, key: &Key, access: Result<Access, craftcloud_core::api::ApiError>) {
        match access {
            Ok(Access::Open(share)) => {
                self.error = None;
                self.password.clear();
                if share.item_type == "folder" {
                    let name = share.item_name.as_deref().map(shown).filter(|n| !n.is_empty()).unwrap_or_else(|| "Shared folder".into());
                    self.trail = vec![(share.item_id.clone(), name)];
                    self.list(backend);
                } else {
                    self.net.send(backend, Key::File, files::info(&self.server, &share.item_id));
                }
                self.state = State::Open(share);
            }
            Ok(Access::NeedsPassword) => {
                if *key == Key::Verify {
                    self.error = Some("That password isn't right.".into());
                }
                self.state = State::Password;
            }
            Ok(Access::Expired) => self.state = State::Expired,
            Ok(Access::Gone) => self.state = State::Gone,
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Lists the open folder (the last of the trail) from its first page. The link's cookie lets
    /// a visitor list the shared folder's subfolders too.
    fn list(&mut self, backend: &mut dyn Backend) {
        self.listing += 1;
        self.folder.clear();
        self.listed = false;
        self.capped = false;
        self.pages = 0;
        self.list_error = None;
        self.next_page(backend, None);
    }

    fn next_page(&mut self, backend: &mut dyn Backend, cursor: Option<&str>) {
        let Some((id, _)) = self.trail.last() else { return };
        let req = folders::contents(&self.server, id, Some(PAGE), cursor);
        self.pages += 1;
        self.net.send(backend, Key::Folder(self.listing), req);
    }

    /// Takes a page of the open folder and asks for the next while there is room.
    fn on_page(&mut self, backend: &mut dyn Backend, result: NetResult) {
        let page = match result.and_then(|r| folders::parse_contents(&r).map_err(|e| e.to_string())) {
            Ok(page) => page,
            Err(why) => {
                self.list_error = Some(why);
                self.listed = true;
                return;
            }
        };
        let room = MAX_ITEMS.saturating_sub(self.folder.len()).min(PAGE as usize);
        let more = page.items.len() > room;
        self.folder.extend(page.items.into_iter().take(room).filter(|r| !r.id().is_empty()));
        match page.next_cursor.filter(|c| !c.is_empty()) {
            Some(cursor) if !more && self.folder.len() < MAX_ITEMS && self.pages < MAX_ITEMS / PAGE as usize => {
                self.next_page(backend, Some(&cursor));
            }
            Some(_) => {
                self.capped = true;
                self.listed = true;
            }
            None => {
                self.capped = more;
                self.listed = true;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        if self.net.is_busy() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let busy = self.net.is_busy();
        match &self.state {
            State::Loading => {
                ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, "LOADING…", egui::FontId::monospace(11.0), t.muted);
                if let Some(e) = &self.error {
                    let e = e.clone();
                    chrome::sheet(
                        ui,
                        area,
                        egui::vec2(480.0, 260.0),
                        &chrome::Hero { title: "Can't open this link", caption: "Shared link", keep_case: false },
                        |ui, part| {
                            if part == chrome::Part::Body {
                                error_line(ui, &e);
                            }
                        },
                    );
                }
            }
            State::Expired | State::Gone => {
                let (title, text) = if self.state == State::Expired {
                    ("This link has expired", "Ask the person who shared it for a new link.")
                } else {
                    ("This link doesn't work", "It was removed, or the address is incomplete. Ask the person who shared it.")
                };
                chrome::sheet(ui, area, egui::vec2(480.0, 260.0), &chrome::Hero { title, caption: "Shared link", keep_case: false }, |ui, part| {
                    if part == chrome::Part::Body {
                        ui.label(RichText::new(text).size(14.0).color(t.muted));
                    }
                });
            }
            State::Password => {
                let mut submit = false;
                chrome::sheet(
                    ui,
                    area,
                    egui::vec2(480.0, 330.0),
                    &chrome::Hero { title: "Enter the password", caption: "Shared link", keep_case: false },
                    |ui, part| match part {
                        chrome::Part::Body => {
                            ui.label(RichText::new("The person who shared this link protected it with a password.").size(14.0).color(t.muted));
                            ui.add_space(12.0);
                            let field = chrome::field(ui, "Password", &mut self.password, true, None);
                            submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if let Some(e) = &self.error {
                                ui.add_space(8.0);
                                error_line(ui, e);
                            }
                        }
                        chrome::Part::ActionsRight => {
                            let ready = !self.password.is_empty() && !busy;
                            submit |= chrome::button(ui, "Open", Kind::Primary, Some(Icon::Arrow), ready).clicked();
                        }
                        _ => {}
                    },
                );
                if submit && !self.password.is_empty() && !busy {
                    let req = shares::verify(&self.server, &self.token, &self.password);
                    self.net.send(backend, Key::Verify, req);
                }
            }
            State::Open(share) => {
                let share = share.clone();
                self.open_ui(ui, area, backend, &share);
            }
        }
    }

    fn open_ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend, share: &Share) {
        let t = Tokens::get(ui.ctx());
        let folder = share.item_type == "folder";
        let name = self
            .file
            .as_ref()
            .map(|f| shown(&f.name))
            .or_else(|| share.item_name.as_deref().map(shown))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| if folder { "Shared folder".into() } else { "Shared file".into() });
        let expires = share.expires_at.as_deref().and_then(|e| e.get(..10)).map(|d| format!("Link works until {d}"));
        let caption = expires.unwrap_or_else(|| "Shared with you".into());
        let mut download: Option<(String, String)> = None;
        // A subfolder to open, or how many levels of the trail to keep.
        let mut open: Option<(String, String)> = None;
        let mut back_to: Option<usize> = None;
        let height = if folder { 520.0 } else { 330.0 };
        chrome::sheet(
            ui,
            area,
            egui::vec2(560.0, height),
            &chrome::Hero { title: &name, caption: &caption, keep_case: false },
            |ui, part| match part {
                chrome::Part::Body => {
                    if let Some(f) = &self.file {
                        let mut rows = vec![("Size", crate::web::bytes(f.size))];
                        let kind = super::files::model::Item::from_resource(Resource::File(f.clone()), |_| String::new()).kind_label();
                        rows.push(("Type", shown(&kind)));
                        chrome::facts(ui, "File", &rows);
                    } else if folder {
                        (open, back_to) = self.folder_ui(ui, &mut download);
                    } else if let Some(why) = &self.file_error {
                        error_line(ui, &format!("Can't show this file's details: {}", shown(why)));
                    } else {
                        ui.label(RichText::new("Loading…").size(14.0).color(t.muted));
                    }
                }
                chrome::Part::ActionsLeft => {
                    ui.label(RichText::new("Shared from CraftCloud").size(12.5).color(t.muted));
                }
                chrome::Part::ActionsRight => {
                    let label = if folder { "Download all" } else { "Download" };
                    let ready = folder || self.file.is_some() || self.file_error.is_some();
                    if chrome::button(ui, label, Kind::Primary, Some(Icon::Download), ready).clicked() {
                        download = Some(if folder {
                            let root = self.trail.first().map_or(name.as_str(), |(_, n)| n.as_str());
                            (folders::download_zip(&self.server, &share.item_id).url, format!("{}.zip", saved_name(root)))
                        } else {
                            (files::download(&self.server, &share.item_id).url, saved_name(&name))
                        });
                    }
                }
                chrome::Part::HeroRight => {}
            },
        );
        if let Some((url, name)) = download {
            backend.download(&url, &name);
        }
        if let Some(level) = open {
            self.trail.push(level);
            self.list(backend);
        } else if let Some(keep) = back_to {
            self.trail.truncate(keep.max(1));
            self.list(backend);
        }
    }

    /// The open folder: breadcrumbs below the shared folder, the count, then its rows. Returns a
    /// subfolder to open, or how many levels of the trail to keep.
    fn folder_ui(&self, ui: &mut Ui, download: &mut Option<(String, String)>) -> (Option<(String, String)>, Option<usize>) {
        let t = Tokens::get(ui.ctx());
        let mut open = None;
        let mut back_to = None;
        if self.trail.len() > 1 {
            back_to = crumbs(ui, &self.trail);
            ui.add_space(2.0);
        }
        let n = self.folder.len();
        let count = if let Some(why) = &self.list_error {
            chrome::hud(ui, "Can't list this folder", t.bad);
            ui.add_space(6.0);
            error_line(ui, &shown(why));
            return (None, back_to);
        } else if !self.listed {
            "Loading…".to_owned()
        } else if self.capped {
            format!("The first {n} items in this folder")
        } else {
            match n {
                0 => "This folder is empty.".to_owned(),
                1 => "1 item in this folder".to_owned(),
                n => format!("{n} items in this folder"),
            }
        };
        chrome::hud(ui, &count, t.muted);
        ui.add_space(1.0);
        let deep_enough = self.trail.len() > MAX_DEPTH;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, r) in self.folder.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
                if i > 0 {
                    ui.painter().hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0, t.line));
                }
                let name = shown(r.name());
                match r {
                    Resource::Folder(f) => {
                        // Never a folder already open above (a loop), nor deeper than MAX_DEPTH.
                        let can_open = !deep_enough && !self.trail.iter().any(|(id, _)| *id == f.id);
                        if folder_row(ui, rect, (i, &f.id), &name, can_open).clicked() && can_open {
                            open = Some((f.id.clone(), name));
                        }
                    }
                    Resource::File(f) => {
                        if file_row(ui, rect, &name, f.size).clicked() {
                            *download = Some((files::download(&self.server, &f.id).url, saved_name(&name)));
                        }
                    }
                }
            }
        });
        if self.capped {
            ui.add_space(10.0);
            ui.label(RichText::new(format!("Only the first {MAX_ITEMS} are shown. DOWNLOAD ALL has everything.")).size(12.5).color(t.muted));
        }
        (open, back_to)
    }
}

/// Whether `token` could be an OxiCloud link's (a UUID in 0.10.0; any short URL-safe word here).
fn plausible_token(token: &str) -> bool {
    (1..=128).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A name from the server as it is shown: control and text-direction characters become `�`, and
/// it is cut at [`MAX_NAME`] characters.
fn shown(name: &str) -> String {
    let mut out: String =
        name.chars()
            .take(MAX_NAME)
            .map(|c| {
                if c.is_control() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                    '\u{fffd}'
                } else {
                    c
                }
            })
            .collect();
    if name.chars().nth(MAX_NAME).is_some() {
        out.push('…');
    }
    out
}

/// The name a download is saved under: the last part of `name` (no folders, `/` or `\`), never
/// `.` or `..`.
fn saved_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean = shown(last);
    let clean = clean.trim();
    if clean.is_empty() || clean == "." || clean == ".." { "download".into() } else { clean.to_owned() }
}

/// The open folders as breadcrumbs, the shared folder first; returns how many levels to keep
/// when one above the open folder is clicked. Names keep their case.
fn crumbs(ui: &mut Ui, trail: &[(String, String)]) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut back = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, (_, name)) in trail.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("/").size(13.0).color(t.muted));
            }
            if i + 1 == trail.len() {
                ui.add(egui::Label::new(RichText::new(name).size(13.0).color(t.ink_strong)).truncate());
            } else {
                let link =
                    ui.add(egui::Label::new(RichText::new(name).size(13.0).color(t.accent_ink).underline()).truncate().sense(egui::Sense::click()));
                link.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, format!("Back to {name}")));
                if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    back = Some(i + 1);
                }
            }
        }
    });
    back
}

/// `text` laid out on one line, cut with `…` to `width`.
fn one_line(ui: &Ui, text: &str, size: f32, color: egui::Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), egui::FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

/// A subfolder: its name and OPEN, the whole row a button. Without OPEN when it can't be opened.
fn folder_row(ui: &mut Ui, rect: Rect, key: (usize, &str), name: &str, can_open: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let id = ui.make_persistent_id(("share-folder", key));
    let resp = ui.interact(rect, id, if can_open { egui::Sense::click() } else { egui::Sense::hover() });
    if resp.hovered() && can_open {
        ui.painter().rect_filled(rect.shrink2(egui::vec2(0.0, 0.5)), 0.0, t.sunken);
    }
    let open = chrome::hud_galley(ui, "Open", t.accent_ink);
    let room = rect.width() - if can_open { open.size().x + 24.0 } else { 0.0 };
    let label = one_line(ui, &format!("{name}/"), 14.0, t.ink_strong, room);
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - label.size().y / 2.0), label, t.ink_strong);
    if can_open {
        ui.painter().galley(egui::pos2(rect.right() - 8.0 - open.size().x, rect.center().y - open.size().y / 2.0), open, t.accent_ink);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can_open, format!("Open {name}")));
    if can_open { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A file: its name (cut to fit), its size and DOWNLOAD (Secondary); returns the button's answer.
fn file_row(ui: &mut Ui, rect: Rect, name: &str, size: u64) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    let button = chrome::button(&mut row, "Download", Kind::Secondary, Some(Icon::Download), true);
    row.label(RichText::new(crate::web::bytes(size)).size(12.0).color(t.muted));
    row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(RichText::new(name).size(14.0).color(t.ink_strong)).truncate());
    });
    button
}

#[cfg(test)]
mod tests {
    use craftcloud_core::ServerUrl;
    use craftcloud_core::http::{HttpResponse, Method};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::State;
    use crate::backend::FakeBackend;
    use crate::web::{Page, WebApp};

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/oxicloud-0.10.0").join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn visit(token: &str) -> Harness<'static, WebApp<FakeBackend>> {
        let server = ServerUrl::parse("https://cloud.example.org").unwrap();
        let app = WebApp::new(&egui::Context::default(), FakeBackend::default(), server, &format!("/s/{token}"), None);
        let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app: &mut WebApp<FakeBackend>| app.ui(ui), app);
        let b = &mut h.state_mut().backend;
        b.answer("/api/auth/status", Ok(HttpResponse::new(200, br#"{"initialized":true,"admin_count":1,"registration_allowed":false}"#.to_vec())));
        b.answer(
            "/craftcloud/config.json",
            Ok(HttpResponse::new(200, br#"{"version":"0.1.0","web_apps":false,"public_url":"https://cloud.example.org"}"#.to_vec())),
        );
        b.answer("/api/auth/me", Ok(HttpResponse::new(401, Vec::new())));
        h.run();
        h
    }

    #[test]
    fn a_link_with_a_password_asks_for_it_then_downloads() {
        let mut h = visit("tok1");
        assert_eq!(h.state().page, Page::Share);
        let needs = br#"{"error":"Password required","requiresPassword":true}"#.to_vec();
        h.state_mut().backend.answer("/api/s/tok1", Ok(HttpResponse::new(401, needs.clone())));
        h.run();
        h.get_by_label("Enter the password");
        h.state_mut().share.as_mut().unwrap().password = "wrong".into();
        h.run();
        h.get_by_label("OPEN").click();
        h.run();
        h.state_mut().backend.answer("/api/s/tok1/verify", Ok(HttpResponse::new(401, needs)));
        h.run();
        h.get_by_label("That password isn't right.");

        h.state_mut().share.as_mut().unwrap().password = "open-sesame".into();
        h.run();
        h.get_by_label("OPEN").click();
        h.run();
        let body: serde_json::Value = serde_json::from_slice(&h.state().backend.last_to("/api/s/tok1/verify").body).unwrap();
        assert_eq!(body, serde_json::json!({"password": "open-sesame"}));
        h.state_mut().backend.answer("/api/s/tok1/verify", Ok(HttpResponse::new(200, fixture("share-create.json"))));
        h.run();
        assert!(matches!(h.state().share.as_ref().unwrap().state, State::Open(_)));
        let item = "6d728410%2D1791%2D454c%2Db27f%2Ddf91ebe60264";
        assert_eq!(h.state().backend.last_to(&format!("/api/files/{item}?metadata=true")).method, Method::Get);
        h.state_mut().backend.answer(&format!("/api/files/{item}?metadata=true"), Ok(HttpResponse::new(200, fixture("file-upload.json"))));
        h.run();
        h.get_by_label("plan.txt");
        h.get_by_label("DOWNLOAD").click();
        h.run();
        let (url, name) = h.state().backend.downloads.last().cloned().unwrap();
        assert_eq!((url.as_str(), name.as_str()), (format!("https://cloud.example.org/api/files/{item}").as_str(), "plan.txt"));
    }

    #[test]
    fn a_shared_folder_opens_its_subfolders_but_never_above_itself() {
        // T-015.
        let mut h = visit("tokF");
        let share = br#"{"id":"s1","item_id":"top","item_name":"Team","item_type":"folder","token":"tokF","url":"https://cloud.example.org/s/tokF","has_password":false,"created_at":"2026-11-13T09:00:00Z"}"#;
        h.state_mut().backend.answer("/api/s/tokF", Ok(HttpResponse::new(200, share.to_vec())));
        h.run();
        let listing =
            |items: serde_json::Value| Ok(HttpResponse::new(200, serde_json::json!({"items": items, "next_cursor": null}).to_string().into_bytes()));
        let sub = serde_json::json!([{"resource_type": "folder", "resource": {"id": "sub", "name": "Photos", "path": "", "parent_id": "top", "modified_at": "2026-11-12T09:10:00Z", "etag": "e"}}]);
        h.state_mut().backend.answer("/api/folders/top/resources?limit=200", listing(sub));
        h.run();
        assert!(h.query_by_label("Back to Team").is_none(), "no breadcrumbs at the shared folder");
        h.get_by_label("Open Photos").click();
        h.run();
        h.state_mut().backend.answer("/api/folders/sub/resources?limit=200", listing(serde_json::json!([])));
        h.run();
        h.get_by_label("THIS FOLDER IS EMPTY.");
        h.get_by_label("Back to Team").click();
        h.run();
        assert!(h.state().backend.waiting("/api/folders/top/resources?limit=200"), "back at the shared folder");
    }

    #[test]
    fn an_expired_link_says_so() {
        let mut h = visit("old");
        h.state_mut().backend.answer("/api/s/old", Ok(HttpResponse::new(410, Vec::new())));
        h.run();
        h.get_by_label("This link has expired");
    }

    fn shared_folder(token: &str) -> Harness<'static, WebApp<FakeBackend>> {
        let mut h = visit(token);
        let share = format!(
            r#"{{"id":"s1","item_id":"top","item_name":"Team","item_type":"folder","token":"{token}","url":"https://cloud.example.org/s/{token}","has_password":false,"created_at":"2026-11-13T09:00:00Z"}}"#
        );
        h.state_mut().backend.answer(&format!("/api/s/{token}"), Ok(HttpResponse::new(200, share.into_bytes())));
        h.run();
        h
    }

    fn page(items: serde_json::Value, next: Option<&str>) -> Result<HttpResponse, String> {
        Ok(HttpResponse::new(200, serde_json::json!({"items": items, "next_cursor": next}).to_string().into_bytes()))
    }

    fn folder(id: &str, name: &str) -> serde_json::Value {
        serde_json::json!({"resource_type": "folder", "resource": {"id": id, "name": name, "path": "", "parent_id": null, "modified_at": "2026-11-12T09:10:00Z", "etag": id}})
    }

    fn file(id: &str, name: &str) -> serde_json::Value {
        serde_json::json!({"resource_type": "file", "resource": {"id": id, "name": name, "path": "", "size": 248000, "mime_type": "application/octet-stream",
            "folder_id": null, "modified_at": "2026-11-12T09:10:00Z", "category": "Document", "etag": format!("e-{id}")}})
    }

    #[test]
    fn a_folder_says_loading_until_it_is_listed_and_never_empty_when_listing_fails() {
        let mut h = shared_folder("tokL");
        h.get_by_label("LOADING…");
        assert!(h.query_by_label("THIS FOLDER IS EMPTY.").is_none(), "not empty before the listing came");
        h.state_mut().backend.answer("/api/folders/top/resources?limit=200", Err("The server didn't answer".into()));
        h.run();
        h.get_by_label("CAN'T LIST THIS FOLDER");
        h.get_by_label("The server didn't answer");
        assert!(h.query_by_label("THIS FOLDER IS EMPTY.").is_none(), "a failed listing is never an empty folder");
    }

    #[test]
    fn a_big_folder_follows_its_pages_but_only_so_far() {
        let mut h = shared_folder("tokB");
        let items = |from: usize| serde_json::Value::Array((from..from + 200).map(|i| file(&format!("f{i}"), &format!("File {i}.txt"))).collect());
        h.state_mut().backend.answer("/api/folders/top/resources?limit=200", page(items(0), Some("c1")));
        h.run();
        h.get_by_label("LOADING…");
        for n in 1..5 {
            h.state_mut()
                .backend
                .answer(&format!("/api/folders/top/resources?limit=200&cursor=c{n}"), page(items(n * 200), Some(&format!("c{}", n + 1))));
            h.run();
        }
        assert_eq!(h.state().backend.sent_to("/api/folders/top/resources?limit=200&cursor=c5"), 0, "five pages at most");
        h.get_by_label("THE FIRST 1000 ITEMS IN THIS FOLDER");
        h.get_by_label("Only the first 1000 are shown. DOWNLOAD ALL has everything.");
        assert_eq!(h.state().share.as_ref().unwrap().folder.len(), 1000);
    }

    #[test]
    fn names_from_the_server_are_cleaned_and_a_download_is_a_bare_file_name() {
        assert_eq!(super::saved_name("../../etc/passwd"), "passwd");
        assert_eq!(super::saved_name("..\\..\\boot.ini"), "boot.ini");
        assert_eq!(super::saved_name(".."), "download");
        assert_eq!(super::saved_name("a/"), "download");
        assert_eq!(super::shown("bad\u{202e}txt.exe\n"), "bad\u{fffd}txt.exe\u{fffd}");
        assert_eq!(super::shown(&"x".repeat(5000)).chars().count(), 256, "cut, with an ellipsis");

        let mut h = shared_folder("tokN");
        h.state_mut().backend.answer("/api/folders/top/resources?limit=200", page(serde_json::json!([file("f1", "../../secret.txt")]), None));
        h.run();
        h.get_by_label("../../secret.txt");
        h.get_by_label("DOWNLOAD").click();
        h.run();
        let (url, name) = h.state().backend.downloads.last().cloned().unwrap();
        assert_eq!((url.as_str(), name.as_str()), ("https://cloud.example.org/api/files/f1", "secret.txt"));
    }

    #[test]
    fn a_folder_that_lists_itself_never_opens_again() {
        let mut h = shared_folder("tokS");
        h.state_mut()
            .backend
            .answer("/api/folders/top/resources?limit=200", page(serde_json::json!([folder("top", "Again"), folder("", "No id")]), None));
        h.run();
        h.get_by_label("1 ITEM IN THIS FOLDER");
        h.get_by_label("Open Again").click();
        h.run();
        assert_eq!(h.state().backend.sent_to("/api/folders/top/resources?limit=200"), 1, "nothing more was listed");
        assert!(h.query_by_label("Back to Team").is_none());
    }

    #[test]
    fn folders_open_only_so_deep() {
        let mut h = shared_folder("tokD");
        let mut at = "top".to_owned();
        for level in 0..=super::MAX_DEPTH {
            let next = format!("d{level}");
            h.state_mut()
                .backend
                .answer(&format!("/api/folders/{at}/resources?limit=200"), page(serde_json::json!([folder(&next, &format!("Level {level}"))]), None));
            h.run();
            h.get_by_label(&format!("Open Level {level}")).click();
            h.run();
            if level == super::MAX_DEPTH {
                assert!(!h.state().backend.waiting(&format!("/api/folders/{next}/resources?limit=200")), "not deeper than MAX_DEPTH");
            }
            at = next;
        }
        assert_eq!(h.state().share.as_ref().unwrap().trail.len(), super::MAX_DEPTH + 1);
    }

    #[test]
    fn a_removed_link_says_so() {
        let mut h = visit("gone");
        h.state_mut().backend.answer("/api/s/gone", Ok(HttpResponse::new(404, Vec::new())));
        h.run();
        h.get_by_label("This link doesn't work");
    }

    #[test]
    fn a_token_no_link_could_have_is_gone_without_asking() {
        let h = visit("a..b");
        h.get_by_label("This link doesn't work");
        assert_eq!(h.state().backend.sent_to("/verify"), 0);
        assert!(h.state().backend.sent.iter().all(|(_, r)| !r.url.contains("/api/s/")), "the server wasn't asked");
        assert!(!super::plausible_token(&"a".repeat(129)));
        assert!(super::plausible_token("aeddf06a-4200-4b05-ad1c-272cf305e350"));
    }

    #[test]
    fn a_shared_file_names_its_format() {
        let mut h = visit("tok1");
        h.state_mut().backend.answer("/api/s/tok1", Ok(HttpResponse::new(200, fixture("share-create.json"))));
        h.run();
        let item = "6d728410%2D1791%2D454c%2Db27f%2Ddf91ebe60264";
        let info = br#"{"id":"6d728410-1791-454c-b27f-df91ebe60264","name":"Q3 report.docx","path":"/Personal/Q3 report.docx","size":248000}"#;
        h.state_mut().backend.answer(&format!("/api/files/{item}?metadata=true"), Ok(HttpResponse::new(200, info.to_vec())));
        h.run();
        h.get_by_label("DOCX document");
    }

    #[test]
    fn a_file_whose_details_fail_still_downloads() {
        let mut h = visit("tok1");
        h.state_mut().backend.answer("/api/s/tok1", Ok(HttpResponse::new(200, fixture("share-create.json"))));
        h.run();
        let item = "6d728410%2D1791%2D454c%2Db27f%2Ddf91ebe60264";
        h.state_mut().backend.answer(&format!("/api/files/{item}?metadata=true"), Err("The server didn't answer".into()));
        h.run();
        h.get_by_label("Can't show this file's details: The server didn't answer");
        h.get_by_label("DOWNLOAD").click();
        h.run();
        let (_, name) = h.state().backend.downloads.last().cloned().unwrap();
        assert_eq!(name, "plan v2.txt");
    }

    #[test]
    fn on_a_phone_the_sheet_fits_the_window() {
        let server = ServerUrl::parse("https://cloud.example.org").unwrap();
        let app = WebApp::new(&egui::Context::default(), FakeBackend::default(), server, "/s/tokP", None);
        let mut h = Harness::builder().with_size(egui::vec2(390.0, 844.0)).build_ui_state(|ui, app: &mut WebApp<FakeBackend>| app.ui(ui), app);
        let b = &mut h.state_mut().backend;
        b.answer("/api/auth/status", Ok(HttpResponse::new(200, br#"{"initialized":true,"admin_count":1,"registration_allowed":false}"#.to_vec())));
        b.answer(
            "/craftcloud/config.json",
            Ok(HttpResponse::new(200, br#"{"version":"0.1.0","web_apps":false,"public_url":"https://cloud.example.org"}"#.to_vec())),
        );
        b.answer("/api/auth/me", Ok(HttpResponse::new(401, Vec::new())));
        h.run();
        let share = br#"{"id":"s1","item_id":"top","item_name":"Team","item_type":"folder","token":"tokP","url":"https://cloud.example.org/s/tokP","has_password":false,"created_at":"2026-11-13T09:00:00Z"}"#;
        h.state_mut().backend.answer("/api/s/tokP", Ok(HttpResponse::new(200, share.to_vec())));
        h.run();
        let long = "A very long file name that would never fit on a phone screen at all.docx";
        h.state_mut().backend.answer("/api/folders/top/resources?limit=200", page(serde_json::json!([file("f1", long)]), None));
        h.run();
        let all = h.get_by_label("DOWNLOAD ALL").rect();
        let one = h.get_by_label("DOWNLOAD").rect();
        for r in [all, one] {
            assert!(r.min.x >= 12.0 && r.max.x <= 390.0 - 12.0, "{r:?} inside the sheet");
        }
        let name = h.get_by_label(long).rect();
        assert!(name.max.x <= one.min.x, "the name is cut before the button: {name:?} {one:?}");
    }
}
