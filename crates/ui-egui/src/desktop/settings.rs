//! The Settings sheet over the main window (design frame `v2-desktop-settings`): Account, On this
//! computer, About.

use craftcloud_core::sync::{Appearance, SyncProgress, SyncState};
use egui::{Rect, Ui, vec2};

use super::{DesktopApp, DesktopHost, host_of};
use crate::chrome::{self, Hero, Icon, Kind, Part, desk};
use crate::theme::Tokens as Look;

// The project's home: the `repository` field of the workspace Cargo.toml, so moving it is one line.
const SOURCE_URL: &str = env!("CARGO_PKG_REPOSITORY");
const RELEASES_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases");
const ISSUES_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/issues");

/// What was picked on the sheet (acted on after it is drawn).
#[derive(Default)]
struct Picks {
    close: bool,
    sign_out: bool,
    login: Option<bool>,
    pause: Option<bool>,
    appearance: Option<Appearance>,
    device_done: bool,
    open: Option<&'static str>,
}

impl<H: DesktopHost> DesktopApp<H> {
    pub(super) fn settings_ui(&mut self, ui: &mut Ui, area: Rect, p: &SyncProgress) {
        let version = env!("CARGO_PKG_VERSION");
        let caption = format!("CraftCloud {version} · {}", self.device);
        let mut picks = Picks::default();
        let wide = area.width() >= 760.0;
        chrome::sheet(
            ui,
            area,
            vec2(area.width() - 64.0, area.height() - 48.0),
            &Hero { title: "Settings", caption: &caption, keep_case: false },
            |ui, part| match part {
                Part::HeroRight => {}
                Part::Body => {
                    if wide {
                        ui.columns(2, |cols| {
                            self.account_panel(&mut cols[0], &mut picks);
                            self.computer_panel(&mut cols[1], p, &mut picks);
                            cols[1].add_space(14.0);
                            about_panel(&mut cols[1], &mut picks);
                        });
                    } else {
                        self.account_panel(ui, &mut picks);
                        ui.add_space(14.0);
                        self.computer_panel(ui, p, &mut picks);
                        ui.add_space(14.0);
                        about_panel(ui, &mut picks);
                    }
                    if let Some(e) = &self.error {
                        let t = Look::get(ui.ctx());
                        ui.label(egui::RichText::new(e).size(13.0).color(t.bad));
                    }
                }
                Part::ActionsLeft => {}
                Part::ActionsRight => picks.close = chrome::button(ui, "Done", Kind::Primary, Some(Icon::Check), true).clicked(),
            },
        );
        self.apply_picks(picks);
    }

    fn apply_picks(&mut self, picks: Picks) {
        if let Some(on) = picks.login
            && let Err(e) = self.host.set_start_at_login(on)
        {
            self.error = Some(format!("Couldn't change that: {e}"));
        }
        if let Some(paused) = picks.pause {
            self.host.pause(paused);
        }
        if let Some(a) = picks.appearance {
            self.settings.appearance = a;
            self.save_prefs();
        }
        if picks.device_done || (picks.close && self.device_changed()) {
            self.settings.device_name = self.device.trim().to_owned();
            self.save_prefs();
        }
        if let Some(url) = picks.open {
            self.host.open_url(url);
        }
        if picks.close {
            self.overlay = None;
        }
        if picks.sign_out {
            self.sign_out();
        }
    }

    fn device_changed(&self) -> bool {
        self.device.trim() != self.settings.device_name && !(self.settings.device_name.is_empty() && self.device.trim() == self.host.computer_name())
    }

    fn save_prefs(&mut self) {
        if let Err(e) = self.host.save_prefs(&self.settings) {
            self.error = Some(format!("Couldn't save: {e}"));
        }
    }

    fn account_panel(&mut self, ui: &mut Ui, picks: &mut Picks) {
        let t = Look::get(ui.ctx());
        let host = self.server.as_ref().map(host_of).unwrap_or_default();
        let user = self.me.as_ref().map_or_else(|| "…".to_owned(), |m| m.user.username.clone());
        chrome::panel(ui, "Account", |ui| {
            for (key, value) in [("Server", host), ("User", user)] {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(80.0, 22.0), egui::Sense::hover());
                    let g = ui.painter().layout_no_wrap(key.to_uppercase(), egui::FontId::monospace(11.0), t.muted);
                    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.muted);
                    ui.add(egui::Label::new(egui::RichText::new(value).size(14.0).color(t.ink_strong)).truncate());
                });
            }
            ui.add_space(6.0);
            let field = chrome::field(ui, "This device", &mut self.device, false, Some("Names the conflict copies made on this computer."));
            if field.lost_focus() && self.device_changed() {
                picks.device_done = true;
            }
            ui.add_space(10.0);
            picks.sign_out = chrome::button(ui, "Sign out", Kind::Secondary, Some(Icon::SignOut), true).clicked();
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Signing out keeps your files in the local folder.").size(12.0).color(t.muted));
        });
    }

    fn computer_panel(&mut self, ui: &mut Ui, p: &SyncProgress, picks: &mut Picks) {
        let login = self.host.starts_at_login();
        let paused = p.state == SyncState::Paused;
        let appearance = self.settings.appearance;
        chrome::panel(ui, "On this computer", |ui| {
            desk::setting_row(ui, "Start at login", |ui| picks.login = desk::off_on(ui, "Start at login", login, true));
            desk::setting_row(ui, "Pause syncing", |ui| picks.pause = desk::off_on(ui, "Pause syncing", paused, true));
            desk::setting_row(ui, "Appearance", |ui| {
                let all = [Appearance::System, Appearance::Dark, Appearance::Light];
                let current = all.iter().position(|a| *a == appearance).unwrap_or(0);
                picks.appearance = desk::cells(ui, "Appearance", &["System", "Dark", "Light"], current, true, None).map(|i| all[i]);
            });
        });
    }
}

fn about_panel(ui: &mut Ui, picks: &mut Picks) {
    let t = Look::get(ui.ctx());
    chrome::panel(ui, "About", |ui| {
        let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
        chrome::meta(ui, &[&format!("CraftCloud {}", env!("CARGO_PKG_VERSION")), &platform, "MIT or Apache-2.0"]);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (i, (label, url)) in
                [("Source", SOURCE_URL), ("Release notes", RELEASES_URL), ("Report a problem", ISSUES_URL)].into_iter().enumerate()
            {
                if i > 0 {
                    chrome::hud(ui, "·", t.muted);
                }
                if desk::hud_link(ui, label).clicked() {
                    picks.open = Some(url);
                }
            }
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Look based on ArtCraft Launcher by the ArtCraft team.").size(12.0).color(t.muted));
    });
}
