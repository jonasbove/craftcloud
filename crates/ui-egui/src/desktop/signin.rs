//! The sign-in sheets: Connect (step 1), Continue in your browser (step 2) and Choose what to
//! sync (step 3), over the empty window.

use craftcloud_core::ServerUrl;
use egui::{Rect, Ui, vec2};

use super::{DesktopApp, DesktopHost, Key, Screen, folders, host_of};
use crate::chrome::{self, Hero, Icon, Kind, Part, desk};
use crate::connect::{ConnectAction, ConnectScreen};
use crate::theme::Tokens as Look;

const STEPS: [&str; 3] = ["Server", "Browser", "Folders"];

/// The step cells' names; a narrow window shows only the numbers, so the title keeps its room.
fn steps_for(area: Rect) -> [&'static str; 3] {
    if area.width() < 760.0 { ["", "", ""] } else { STEPS }
}

/// Waiting for the keychain: a quiet line in the middle of the window.
pub(super) fn keychain_ui(ui: &mut Ui, area: Rect) {
    let t = Look::get(ui.ctx());
    ui.scope_builder(egui::UiBuilder::new().max_rect(area.shrink(chrome::PAD)), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((area.height() / 2.0 - 60.0).max(0.0));
            chrome::hud(ui, "Opening the system keychain…", t.muted);
            ui.add_space(8.0);
            ui.label(egui::RichText::new("If it asks to be unlocked, unlock it to carry on.").size(13.0).color(t.muted));
        });
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
}

fn error_text(ui: &mut Ui, error: &str) {
    let t = Look::get(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(error).size(13.0).color(t.bad)).wrap());
}

impl<H: DesktopHost> DesktopApp<H> {
    pub(super) fn connect_ui(&mut self, ui: &mut Ui, area: Rect) {
        let t = Look::get(ui.ctx());
        let checking = self.net.is_waiting(&Key::Config);
        let found = self.net.is_waiting(&Key::Authorize);
        let host = self.server.as_ref().map(host_of).unwrap_or_default();
        let plain = self.server.as_ref().is_some_and(ServerUrl::is_plain_http);
        let mut action = None;
        let steps = steps_for(area);
        let hero = Hero { title: "Connect to CraftCloud", caption: "Step 1 of 3 · Server", keep_case: false };
        chrome::sheet(ui, area, vec2(640.0, 380.0), &hero, |ui, part| {
            let Screen::Connect(connect) = &mut self.screen else { return };
            match part {
                Part::HeroRight => chrome::step_cells(ui, &steps, 0),
                Part::Body => {
                    action = connect.body(ui).or(action.take());
                    ui.add_space(10.0);
                    if checking || found {
                        ui.horizontal(|ui| {
                            let (tag, tone, text) = if found {
                                let how = if plain { " over http:// (not encrypted)" } else { "" };
                                ("Found", t.good, format!("CraftCloud at {host}{how}"))
                            } else {
                                ("Checking", t.muted, format!("Looking for CraftCloud at {host}…"))
                            };
                            chrome::badge(ui, tag, tone);
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(text).size(13.0).color(t.ink));
                        });
                    }
                    if let Some(e) = &self.error {
                        error_text(ui, e);
                    }
                }
                Part::ActionsLeft => {
                    if chrome::button(ui, "Quit", Kind::Secondary, None, true).clicked() {
                        self.host.quit();
                    }
                }
                Part::ActionsRight => {
                    let busy = checking || found;
                    if chrome::button(ui, "Continue", Kind::Primary, Some(Icon::Arrow), connect.ready() && !busy).clicked() {
                        action = connect.submit();
                    }
                }
            }
        });
        if let Some(ConnectAction::Continue(url)) = action {
            self.check_server(url);
        }
    }

    pub(super) fn browser_ui(&mut self, ui: &mut Ui, area: Rect) {
        let t = Look::get(ui.ctx());
        let Screen::Browser { start, opened, .. } = &self.screen else { return };
        let (code, url, opened) = (start.user_code.clone(), start.browser_url().to_owned(), *opened);
        let host = self.server.as_ref().map(host_of).unwrap_or_default();
        let device = self.device.clone();
        let (mut cancel, mut again) = (false, false);
        let steps = steps_for(area);
        let hero = Hero { title: "Continue in your browser", caption: "Step 2 of 3 · Sign in", keep_case: false };
        // Whether the browser opened or not, the address is there to copy: the person may want it
        // on another computer, or the system may have no browser to open.
        let lead = if opened {
            format!("We opened {host} in your browser. Sign in there and check it shows this code:")
        } else {
            "Open this address in your browser, sign in, and check it shows this code:".to_owned()
        };
        chrome::sheet(ui, area, vec2(640.0, 500.0), &hero, |ui, part| match part {
            Part::HeroRight => chrome::step_cells(ui, &steps, 1),
            Part::Body => {
                chrome::body(ui, &lead, 14.0);
                ui.add_space(10.0);
                chrome::copy_row(ui, &url);
                ui.add_space(10.0);
                desk::code_box(ui, &code);
                ui.add_space(14.0);
                chrome::hud(ui, "Waiting for the browser…", t.accent_ink);
                ui.add_space(4.0);
                desk::busy_meter(ui);
                ui.add_space(10.0);
                chrome::meta(ui, &[&format!("This device: {device}"), std::env::consts::OS, "Rename later in Settings"]);
                if let Some(e) = &self.error {
                    error_text(ui, e);
                }
            }
            Part::ActionsLeft => cancel = chrome::button(ui, "Cancel", Kind::Secondary, None, true).clicked(),
            Part::ActionsRight => {
                let label = if opened { "Open browser again" } else { "Try the browser again" };
                again = chrome::button(ui, label, Kind::Secondary, Some(Icon::External), true).clicked();
            }
        });
        if again {
            let worked = self.host.open_browser(&url);
            if let Screen::Browser { opened, .. } = &mut self.screen {
                *opened = worked;
            }
        }
        if cancel {
            self.error = None;
            self.screen = Screen::Connect(ConnectScreen::new(self.settings.server.clone()));
        }
    }

    pub(super) fn choose_ui(&mut self, ui: &mut Ui, area: Rect) {
        let t = Look::get(ui.ctx());
        let mut start = false;
        let hero = Hero { title: "Choose what to sync", caption: "Step 3 of 3 · Folders", keep_case: false };
        let size = vec2(area.width() - 64.0, area.height() - 100.0).max(vec2(560.0, 400.0));
        let steps = steps_for(area);
        let home = std::path::Path::new(&self.host.default_folder()).parent().map(|p| p.display().to_string()).unwrap_or_default();
        chrome::sheet(ui, area, size, &hero, |ui, part| {
            let Screen::Choose { choice, local, changing } = &mut self.screen else { return };
            let short = match local.strip_prefix(home.as_str()) {
                Some(rest) if !home.is_empty() && (rest.starts_with('/') || rest.starts_with('\\')) => format!("~{rest}"),
                _ => local.clone(),
            };
            match part {
                Part::HeroRight => chrome::step_cells(ui, &steps, 2),
                Part::Body => {
                    if *changing {
                        let field =
                            chrome::field(ui, "Local folder", local, false, Some("A folder on this computer; it's made if it doesn't exist."));
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            *changing = false;
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.set_min_height(chrome::CONTROL_H);
                            ui.label(egui::RichText::new("Local folder").size(14.0).color(t.ink));
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(egui::RichText::new(short.as_str()).size(14.0).color(t.ink_strong)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if chrome::button(ui, "Change…", Kind::Secondary, None, true).clicked() {
                                    *changing = true;
                                }
                            });
                        });
                    }
                    ui.add_space(10.0);
                    folders::choose_table(ui, choice);
                    if choice.folders.is_empty() {
                        ui.add_space(6.0);
                        chrome::body(ui, "No folders in the cloud yet. Folders you make in the local folder sync too.", 13.0);
                    }
                    if let Some(e) = &self.error {
                        error_text(ui, e);
                    }
                }
                Part::ActionsLeft => {
                    let n = choice.folders.iter().filter(|(_, on)| *on).count();
                    let files = if choice.root_files { " · top-level files" } else { "" };
                    ui.label(egui::RichText::new(format!("{n} of {} folders{files}", choice.folders.len())).size(13.0).color(t.muted));
                }
                Part::ActionsRight => {
                    start = chrome::button(ui, "Start syncing", Kind::Primary, Some(Icon::Refresh), !local.trim().is_empty()).clicked();
                }
            }
        });
        if start {
            self.begin_sync();
        }
    }
}
