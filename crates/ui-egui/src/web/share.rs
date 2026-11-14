//! The public page a share link opens (`/s/<token>`): no sign-in. It asks for the link's password
//! when it has one, then shows the file (or folder) with a download button. A shared folder's
//! subfolders open in place, with breadcrumbs back up to the shared folder, never above it (T-015).
//!
//! OxiCloud unlocks a link by setting its `oxi_shares` cookie on the answer of `GET /api/s/{token}`
//! (or of the password check), after which the item downloads by its id like any file. That
//! cookie is `Secure`, so links work over HTTPS (and on `localhost`), not over plain HTTP.

use craftcloud_core::ServerUrl;
use craftcloud_core::api::resources::{File, Resource};
use craftcloud_core::api::shares::{self, Access, Share};
use craftcloud_core::api::{files, folders};
use egui::{Rect, RichText, Ui};

use crate::backend::{Backend, Net, NetResult};
use crate::chrome::{self, Icon, Kind};
use crate::theme::Tokens;
use crate::widgets::error_line;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Access,
    Verify,
    File,
    /// A listing of the open folder (`n` tells stale answers apart).
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
    folder: Vec<Resource>,
    /// The folders open, the shared one first: `(id, name)`.
    trail: Vec<(String, String)>,
    /// Bumped on every folder change.
    listing: u64,
}

impl SharePage {
    /// Starts loading the link `token`.
    pub fn new(backend: &mut dyn Backend, server: ServerUrl, token: &str) -> Self {
        let mut net = Net::default();
        net.send(backend, Key::Access, shares::access(&server, token));
        Self {
            server,
            token: token.to_owned(),
            net,
            state: State::Loading,
            password: String::new(),
            error: None,
            file: None,
            folder: Vec::new(),
            trail: Vec::new(),
            listing: 0,
        }
    }

    pub fn receive(&mut self, backend: &mut dyn Backend, answers: &mut Vec<(u64, NetResult)>) {
        for (key, result) in self.net.receive(answers) {
            let resp = match result {
                Ok(r) => r,
                Err(why) => {
                    self.error = Some(why);
                    continue;
                }
            };
            match key {
                Key::Access | Key::Verify => match shares::parse_access(&resp) {
                    Ok(Access::Open(share)) => {
                        self.error = None;
                        self.password.clear();
                        if share.item_type == "folder" {
                            let name = share.item_name.clone().unwrap_or_else(|| "Shared folder".into());
                            self.trail = vec![(share.item_id.clone(), name)];
                            self.list(backend);
                        } else {
                            self.net.send(backend, Key::File, files::info(&self.server, &share.item_id));
                        }
                        self.state = State::Open(share);
                    }
                    Ok(Access::NeedsPassword) => {
                        if key == Key::Verify {
                            self.error = Some("That password isn't right.".into());
                        }
                        self.state = State::Password;
                    }
                    Ok(Access::Expired) => self.state = State::Expired,
                    Ok(Access::Gone) => self.state = State::Gone,
                    Err(e) => self.error = Some(e.to_string()),
                },
                Key::File => self.file = files::parse_file(&resp).ok(),
                Key::Folder(n) if n == self.listing => {
                    self.folder = folders::parse_contents(&resp).map(|p| p.items).unwrap_or_default();
                }
                Key::Folder(_) => {}
            }
        }
    }

    /// Lists the open folder (the last of the trail). The link's cookie lets a visitor list the
    /// shared folder's subfolders too.
    fn list(&mut self, backend: &mut dyn Backend) {
        let Some((id, _)) = self.trail.last() else { return };
        self.listing += 1;
        self.folder.clear();
        let req = folders::contents(&self.server, id, Some(200), None);
        self.net.send(backend, Key::Folder(self.listing), req);
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
            .map(|f| f.name.clone())
            .or_else(|| share.item_name.clone())
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
                        if let Some((_, ext)) = f.name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) {
                            rows.push(("Type", format!("{} file", ext.to_uppercase())));
                        }
                        chrome::facts(ui, "File", &rows);
                    } else if folder {
                        if self.trail.len() > 1 {
                            back_to = crumbs(ui, &self.trail);
                            ui.add_space(8.0);
                        }
                        let n = self.folder.len();
                        let count = match n {
                            0 => "This folder is empty.".to_owned(),
                            1 => "1 item in this folder".to_owned(),
                            n => format!("{n} items in this folder"),
                        };
                        chrome::hud(ui, &count, t.muted);
                        ui.add_space(8.0);
                        for (i, r) in self.folder.iter().enumerate() {
                            if i > 0 {
                                crate::widgets::hairline(ui);
                            }
                            if let Resource::Folder(f) = r {
                                if folder_row(ui, &f.name).clicked() {
                                    open = Some((f.id.clone(), f.name.clone()));
                                }
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.set_min_height(chrome::CONTROL_H + 6.0);
                                ui.label(RichText::new(r.name()).size(14.0).color(t.ink_strong));
                                if let Resource::File(f) = r {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if chrome::button(ui, "Download", Kind::Secondary, Some(Icon::Download), true).clicked() {
                                            download = Some((files::download(&self.server, &f.id).url, f.name.clone()));
                                        }
                                        ui.label(RichText::new(crate::web::bytes(f.size)).size(12.0).color(t.muted));
                                    });
                                }
                            });
                        }
                    } else {
                        ui.label(RichText::new("Loading…").size(14.0).color(t.muted));
                    }
                }
                chrome::Part::ActionsLeft => {
                    ui.label(RichText::new("Shared from CraftCloud").size(12.5).color(t.muted));
                }
                chrome::Part::ActionsRight => {
                    let label = if folder { "Download all" } else { "Download" };
                    if chrome::button(ui, label, Kind::Primary, Some(Icon::Download), folder || self.file.is_some()).clicked() {
                        download = Some(if folder {
                            (folders::download_zip(&self.server, &share.item_id).url, format!("{name}.zip"))
                        } else {
                            (files::download(&self.server, &share.item_id).url, name.clone())
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
                ui.label(RichText::new(name).size(13.0).color(t.ink_strong));
            } else {
                let link = ui.add(egui::Label::new(RichText::new(name).size(13.0).color(t.accent_ink).underline()).sense(egui::Sense::click()));
                link.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, format!("Back to {name}")));
                if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    back = Some(i + 1);
                }
            }
        }
    });
    back
}

/// A subfolder: its name and OPEN, the whole row a button.
fn folder_row(ui: &mut Ui, name: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), chrome::CONTROL_H + 6.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, t.sunken);
    }
    ui.painter().text(
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{name}/"),
        egui::FontId::proportional(14.0),
        t.ink_strong,
    );
    let open = chrome::hud_galley(ui, "Open", t.accent_ink);
    ui.painter().galley(egui::pos2(rect.right() - 8.0 - open.size().x, rect.center().y - open.size().y / 2.0), open, t.accent_ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Open {name}")));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
}
