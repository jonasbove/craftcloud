//! The Settings sheet over the main window (design frame `v2-desktop-settings`): Account, On this
//! computer, About.

use craftcloud_core::sync::{Appearance, SyncProgress, SyncState};
use craftcloud_core::update::NewVersion;
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
    /// "Check for updates" switched.
    updates: Option<bool>,
    /// A colour theme's id, or empty for CraftCloud's own look.
    theme: Option<String>,
    device_done: bool,
    open: Option<String>,
}

impl<H: DesktopHost> DesktopApp<H> {
    pub(super) fn settings_ui(&mut self, ui: &mut Ui, area: Rect, p: &SyncProgress) {
        let version = env!("CARGO_PKG_VERSION");
        let caption = format!("CraftCloud {version} · {}", self.device);
        let mut picks = Picks::default();
        // Only while the check is on: switched off, an earlier find isn't shown either.
        let new_version = self.settings.check_for_updates.then(|| self.host.new_version()).flatten();
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
                            about_panel(&mut cols[1], new_version.as_ref(), &mut picks);
                        });
                    } else {
                        self.account_panel(ui, &mut picks);
                        ui.add_space(14.0);
                        self.computer_panel(ui, p, &mut picks);
                        ui.add_space(14.0);
                        about_panel(ui, new_version.as_ref(), &mut picks);
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
        if let Some(on) = picks.updates {
            self.settings.check_for_updates = on;
            self.save_prefs();
        }
        if let Some(theme) = picks.theme {
            self.settings.theme = theme;
            self.save_prefs();
        }
        if picks.device_done || (picks.close && self.device_changed()) {
            self.settings.device_name = self.device.trim().to_owned();
            self.save_prefs();
        }
        if let Some(url) = picks.open {
            self.host.open_url(&url);
        }
        if picks.close {
            self.overlay = None;
        }
        if picks.sign_out {
            self.sign_out_everywhere();
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
        let updates = self.settings.check_for_updates;
        let theme = self.settings.theme.clone();
        chrome::panel(ui, "On this computer", |ui| {
            desk::setting_row(ui, "Start at login", |ui| picks.login = desk::off_on(ui, "Start at login", login, true));
            // Named for the state it is in, not as an on/off "Pause syncing" (whose "Off" read as a double negative).
            desk::setting_row(ui, "Syncing", |ui| {
                picks.pause = desk::cells(ui, "Syncing", &["Paused", "Active"], usize::from(!paused), true, None).map(|i| i == 0);
            });
            desk::setting_row(ui, "Appearance", |ui| {
                let all = [Appearance::System, Appearance::Dark, Appearance::Light];
                let current = all.iter().position(|a| *a == appearance).unwrap_or(0);
                picks.appearance = desk::cells(ui, "Appearance", &["System", "Dark", "Light"], current, true, None).map(|i| all[i]);
            });
            // CraftCloud's own look, then the colour themes; one with a single variant says so
            // ("Nord (dark only)") and shows it whatever Appearance says.
            desk::setting_row(ui, "Theme", |ui| picks.theme = theme_picker(ui, &theme));
            desk::setting_row(ui, "Check for updates", |ui| picks.updates = desk::off_on(ui, "Check for updates", updates, true));
        });
    }
}

/// The colour theme picker: returns the id chosen (empty for CraftCloud's own look).
fn theme_picker(ui: &mut Ui, current: &str) -> Option<String> {
    let themes = crate::themes::all();
    let labels: Vec<String> = std::iter::once(crate::themes::CRAFTCLOUD_LOOK.to_owned()).chain(themes.iter().map(|t| t.label())).collect();
    let options: Vec<&str> = labels.iter().map(String::as_str).collect();
    let at = themes.iter().position(|t| t.id == current).map_or(0, |i| i + 1);
    crate::kit::picker(ui, "theme", at, &options, 230.0)
        .map(|i| i.checked_sub(1).and_then(|i| themes.get(i)).map_or_else(String::new, |t| t.id.to_owned()))
}

/// The About panel; `new_version` is a newer release the update check found.
fn about_panel(ui: &mut Ui, new_version: Option<&NewVersion>, picks: &mut Picks) {
    let t = Look::get(ui.ctx());
    chrome::panel(ui, "About", |ui| {
        let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
        chrome::meta(ui, &[&format!("CraftCloud {}", env!("CARGO_PKG_VERSION")), &platform, "MIT or Apache-2.0"]);
        if let Some(new) = new_version {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(format!("CraftCloud {} is out.", new.version)).size(14.0).color(t.ink_strong));
            // A package manager's copy is updated there; replacing it by hand would leave its record stale.
            let how = if new.package_managed { "Update it with your package manager." } else { "Download it from the release page." };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(egui::RichText::new(how).size(12.0).color(t.muted));
                if desk::hud_link(ui, "What's new").clicked() {
                    picks.open = Some(new.url.clone());
                }
            });
        }
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
                    picks.open = Some(url.to_owned());
                }
            }
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Look based on ArtCraft Launcher by the ArtCraft team.").size(12.0).color(t.muted));
    });
}
