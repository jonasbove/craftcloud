//! The signed-in home page and the panels it shares with the last setup step and the device
//! pages (design frames `v2-web-home`, `v2-web-setup-5`, `v2-web-device-connected`/`-denied`).

use craftcloud_core::api::auth;
use egui::{RichText, Ui};

use super::admin::when;
use super::{INSTALL_GUIDE_URL, Page, WebApp, bytes, host};
use crate::backend::Backend;
use crate::chrome::{self, Icon, Kind};
use crate::theme::Tokens;
use crate::widgets::error_line;

impl<B: Backend> WebApp<B> {
    fn public_url(&self) -> String {
        self.config.as_ref().map_or_else(|| self.server.to_string(), |c| c.public_url.clone())
    }

    fn version(&self) -> String {
        self.config.as_ref().map_or_else(|| "0.1.0".to_owned(), |c| c.version.clone())
    }

    pub(super) fn home_ui(&mut self, ui: &mut Ui, area: egui::Rect) {
        let t = Tokens::get(ui.ctx());
        let Some(me) = self.me.clone() else { return };
        let devices = self.devices.clone();
        let public = self.public_url();
        let version = self.version();
        let clock = self.clock;
        let phone = area.width() < 600.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let side = if phone { 16 } else { 24 };
            egui::Frame::new().inner_margin(egui::Margin { left: side, right: side, top: 24, bottom: 32 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                chrome::hud(ui, "01 / Home", t.muted);
                ui.add_space(14.0);
                let size = if area.width() < 700.0 { 40.0 } else { 56.0 };
                chrome::headline(ui, size, &[&[("Your ", false), ("cloud", true), (",", false)], &[("ready.", false)]]);
                ui.add_space(16.0);
                ui.scope(|ui| {
                    // Never wider than the page: set_max_width would widen a phone's column to 640.
                    ui.set_max_width(ui.available_width().min(640.0));
                    chrome::body(
                        ui,
                        "Files live on this server and in a CraftCloud folder on each computer. Open and save them with the Craft apps; changes sync on their own.",
                        if phone { 16.0 } else { 18.0 },
                    );
                });
                ui.add_space(14.0);
                // OxiCloud 0.10.0 lists no devices for some signed-in clients (T-045), so an empty
                // list says nothing rather than "0 devices".
                let count = match &devices {
                    Some(d) if d.is_empty() => String::new(),
                    Some(d) if d.len() == 1 => "1 device".to_owned(),
                    Some(d) => format!("{} devices", d.len()),
                    None => String::new(),
                };
                let mut parts = vec![host(&self.server)];
                if !count.is_empty() {
                    parts.push(count);
                }
                parts.push("Versions kept: 5".into());
                let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
                chrome::meta(ui, &refs);
                ui.add_space(if phone { 28.0 } else { 40.0 });

                let wide = ui.available_width() >= 900.0;
                let gap = 14.0;
                let w = if wide { ((ui.available_width() - 2.0 * gap) / 3.0).floor() } else { ui.available_width() };
                let storage = |ui: &mut Ui| {
                    chrome::panel(ui, "Storage", |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(bytes(me.storage_used_bytes)).size(24.0).family(crate::theme::display()).color(t.ink_strong));
                            let of = if me.storage_quota_bytes > 0 { format!("of {}", bytes(me.storage_quota_bytes)) } else { "no limit".into() };
                            ui.label(RichText::new(of).size(14.0).color(t.muted));
                        });
                        ui.add_space(6.0);
                        let fraction = (me.storage_quota_bytes > 0).then(|| me.storage_used_bytes as f32 / me.storage_quota_bytes as f32);
                        crate::kit::storage_meter(ui, ui.available_width(), fraction);
                        ui.add_space(8.0);
                        let pct = fraction.map_or_else(|| "No quota".to_owned(), |f| format!("{:.0} % used", f * 100.0));
                        chrome::meta(ui, &[&pct, "Old versions count too"]);
                    });
                };
                let devices_panel = |ui: &mut Ui| devices_panel(ui, devices.as_deref(), &public, clock);
                let client = |ui: &mut Ui| desktop_client_panel(ui, &version);
                if wide {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for part in [&storage as &dyn Fn(&mut Ui), &devices_panel, &client] {
                            ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| part(ui));
                        }
                    });
                } else {
                    storage(ui);
                    ui.add_space(gap);
                    devices_panel(ui);
                    ui.add_space(gap);
                    client(ui);
                }
                if let Some(e) = &self.error {
                    ui.add_space(12.0);
                    error_line(ui, e);
                }
            });
        });
    }

    /// Setup step 5: the desktop client and the address to give it.
    pub(super) fn setup_done(&mut self, ui: &mut Ui) {
        let public = self.public_url();
        chrome::body(
            ui,
            "Put the desktop client on your computers. It keeps a CraftCloud folder in sync; the Craft apps open and save files there.",
            14.0,
        );
        ui.add_space(14.0);
        desktop_client_panel(ui, &self.version());
        ui.add_space(14.0);
        chrome::panel(ui, "Connect a device", |ui| {
            chrome::copy_row(ui, &public);
            ui.add_space(4.0);
            ui.label(
                RichText::new("Enter this address in the desktop client, then approve it in the browser tab it opens.")
                    .size(12.5)
                    .color(Tokens::get(ui.ctx()).muted),
            );
        });
    }

    /// The answer to a device request: connected or not.
    pub(super) fn device_done_ui(&mut self, ui: &mut Ui, area: egui::Rect, approved: bool) {
        let t = Tokens::get(ui.ctx());
        let name = self.device.as_ref().map_or_else(|| "The device".to_owned(), |d| d.client_name.clone());
        let (title, caption) =
            if approved { (format!("{name} is connected"), "Device approved") } else { (format!("{name} was not connected"), "Device denied") };
        let mut home = false;
        let now = self.clock.map(|c| c.now_unix);
        chrome::sheet(ui, area, egui::vec2(560.0, 380.0), &chrome::Hero { title: &title, caption, keep_case: false }, |ui, part| match part {
            chrome::Part::Body => {
                ui.horizontal(|ui| {
                    if approved {
                        chrome::badge(ui, "Connected", t.good);
                    } else {
                        chrome::badge(ui, "Denied", t.bad);
                    }
                    if let Some(now) = now {
                        ui.add_space(4.0);
                        let stamp = unix_to_rfc3339(now);
                        chrome::hud(ui, &when(&stamp, self.clock, false), t.muted);
                    }
                });
                ui.add_space(12.0);
                if approved {
                    chrome::headline(ui, 40.0, &[&[(&format!("{name} is"), false)], &[("connected", true), (".", false)]]);
                    ui.add_space(12.0);
                    ui.label(RichText::new("You can close this tab: the client takes it from here and starts syncing.").size(14.0).color(t.muted));
                } else {
                    ui.label(
                        RichText::new("Denied. It got no access. If that was you after all, start the sign-in again in the client.")
                            .size(14.0)
                            .color(t.muted),
                    );
                }
            }
            chrome::Part::ActionsRight => home = chrome::button(ui, "Go to home", Kind::Secondary, Some(Icon::Arrow), true).clicked(),
            _ => {}
        });
        if home {
            self.device_result = None;
            self.device = None;
            self.form.code.clear();
            self.enter(Page::Home);
        }
    }
}

/// The connected desktop clients, or how to connect one when there are none.
fn devices_panel(ui: &mut Ui, devices: Option<&[auth::Device]>, public: &str, clock: Option<super::Clock>) {
    let t = Tokens::get(ui.ctx());
    match devices {
        Some(list) if !list.is_empty() => chrome::panel(ui, "Devices", |ui| {
            for (i, d) in list.iter().enumerate() {
                if i > 0 {
                    ui.add_space(4.0);
                    crate::widgets::hairline(ui);
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, t.good);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new(&d.client_name).size(15.0).color(t.ink_strong));
                        let since =
                            d.authorized_at.as_deref().map_or_else(String::new, |s| format!("Connected {}", lower_day(&when(s, clock, false))));
                        ui.label(RichText::new(since).size(12.5).color(t.muted));
                    });
                });
            }
        }),
        _ => chrome::panel(ui, "Connect a computer", |ui| {
            for (n, text) in [
                ("01", "Install the desktop client."),
                ("02", &format!("Start it and enter {public}") as &str),
                ("03", "Approve it in the browser tab it opens."),
            ] {
                ui.horizontal(|ui| {
                    chrome::hud(ui, n, t.accent_ink);
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(text).size(13.0)).wrap());
                });
            }
            if devices.is_none() {
                ui.add_space(4.0);
                chrome::hud(ui, "Loading devices…", t.muted);
            }
        }),
    }
}

/// Linux, macOS and Windows: version, architectures, how to open the unsigned build.
fn desktop_client_panel(ui: &mut Ui, version: &str) {
    let t = Tokens::get(ui.ctx());
    chrome::panel(ui, "Get the desktop client", |ui| {
        for (i, (os, arch, note)) in [
            ("Linux", "x86_64 · arm64", "AppImage, deb, rpm, Flatpak, Arch, tar.gz, or nix run."),
            ("macOS", "universal", "Unsigned: right-click the app, Open, then Open again."),
            ("Windows", "x64 · arm64", "Unsigned: SmartScreen › More info › Run anyway."),
        ]
        .into_iter()
        .enumerate()
        {
            if i > 0 {
                ui.add_space(4.0);
                crate::widgets::hairline(ui);
                ui.add_space(6.0);
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(os).size(16.0).color(t.ink_strong));
                        ui.add_space(4.0);
                        chrome::hud(ui, &format!("{version} · {arch}"), t.muted);
                    });
                    ui.add(egui::Label::new(RichText::new(note).size(12.5).color(t.muted)).wrap());
                });
            });
        }
        ui.add_space(10.0);
        if chrome::button(ui, "Downloads", Kind::Secondary, Some(Icon::Download), true).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(INSTALL_GUIDE_URL));
        }
    });
}

/// Seconds since 1970 → `2026-11-13T21:54:00Z`.
fn unix_to_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = super::admin::civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// "Today 09:10" → "today 09:10", for the middle of a sentence; a date ("13 Nov 2026") keeps its
/// month capitalised.
fn lower_day(when: &str) -> String {
    for day in ["Today", "Yesterday"] {
        if let Some(rest) = when.strip_prefix(day) {
            return format!("{}{rest}", day.to_lowercase());
        }
    }
    when.to_owned()
}
