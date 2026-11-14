//! The FOLDERS tab (design frame `v2-desktop-folders`): the toolbar row and the folder table with
//! Off/On cells. The same table, without its header, is on the Choose sheet.

use craftcloud_core::sync::{FolderStats, SyncProgress};
use egui::{Sense, Ui, vec2};

use super::{Choice, DesktopApp, DesktopHost, Key};
use crate::chrome::{self, Icon, Kind, desk};
use crate::theme::Tokens as Look;
use crate::web::bytes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sort {
    Name,
    Files,
    Size,
}

impl Sort {
    const ALL: [Self; 3] = [Self::Name, Self::Files, Self::Size];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Files => "Files",
            Self::Size => "Size",
        }
    }
}

/// The name shown for the files at the top level of the cloud.
const ROOT_FILES: &str = "Files at the top level";
const SYNC_W: f32 = 112.0;
const NUM_W: f32 = 90.0;

fn stats_of<'a>(stats: &'a [FolderStats], folder: &str) -> Option<&'a FolderStats> {
    stats.iter().find(|s| s.folder == folder)
}

/// A mono number cell, `width` wide.
fn number_cell(ui: &mut Ui, width: f32, text: &str, height: f32) {
    let t = Look::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let g = ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::monospace(12.0), t.muted);
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.muted);
}

/// "1,204"
pub(super) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One table row: icon, name (muted when off), files, size, and the Off/On cells. Returns the
/// new value when switched.
fn folder_row(ui: &mut Ui, name: &str, file: bool, on: bool, stats: Option<&FolderStats>, row: (f32, bool, bool)) -> Option<bool> {
    let (height, last, numbers) = row;
    let t = Look::get(ui.ctx());
    desk::row(ui, height, last, Sense::hover(), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let num_w = if numbers { NUM_W } else { 0.0 };
        let fill = (ui.available_width() - 2.0 * num_w - SYNC_W).max(60.0);
        ui.allocate_ui_with_layout(vec2(fill, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(fill);
            desk::item_icon(ui, file, if on { t.muted } else { t.line_strong });
            ui.add_space(10.0);
            let color = if on { t.ink_strong } else { t.muted };
            ui.add(egui::Label::new(egui::RichText::new(name).size(15.0).color(color)).truncate());
            if let Some(s) = stats.filter(|s| s.errors + s.conflicts + s.syncing > 0) {
                ui.add_space(8.0);
                let (tag, tone) = if s.errors > 0 {
                    (format!("{} error{}", s.errors, if s.errors == 1 { "" } else { "s" }), t.bad)
                } else if s.conflicts > 0 {
                    (format!("{} conflict{}", s.conflicts, if s.conflicts == 1 { "" } else { "s" }), t.warn)
                } else {
                    (format!("{} syncing", s.syncing), t.accent_ink)
                };
                chrome::badge(ui, &tag, tone);
            }
        });
        let (files, size) = match stats {
            Some(s) if on => (thousands(s.files), bytes(s.bytes)),
            _ => ("—".to_owned(), "—".to_owned()),
        };
        if numbers {
            number_cell(ui, NUM_W, &files, height);
            number_cell(ui, NUM_W, &size, height);
        }
        ui.add_space((SYNC_W - 112.0).max(0.0));
        desk::off_on(ui, name, on, true)
    })
    .1
}

/// The folder table without its header or numbers (the Choose sheet: nothing is counted yet).
/// Returns whether a choice changed.
pub(super) fn choose_table(ui: &mut Ui, choice: &mut Choice) -> bool {
    folder_table_sorted(ui, choice, &[], 34.0, false, Sort::Name)
}

fn folder_table_sorted(ui: &mut Ui, choice: &mut Choice, stats: &[FolderStats], row_h: f32, head: bool, sort: Sort) -> bool {
    let mut order: Vec<usize> = (0..choice.folders.len()).collect();
    let stat = |i: usize| stats_of(stats, &choice.folders[i].0).cloned().unwrap_or_default();
    match sort {
        Sort::Name => order.sort_by_key(|&i| choice.folders[i].0.to_lowercase()),
        Sort::Files => order.sort_by_key(|&i| std::cmp::Reverse(stat(i).files)),
        Sort::Size => order.sort_by_key(|&i| std::cmp::Reverse(stat(i).bytes)),
    }
    let mut changed = false;
    desk::list_frame(ui, |ui| {
        if head {
            desk::table_head(ui, &[("Cloud folder", 0.0), ("Files", NUM_W), ("Size", NUM_W), ("Sync", SYNC_W)]);
        }
        for i in order {
            let (name, on) = &mut choice.folders[i];
            if let Some(v) = folder_row(ui, name, false, *on, stats_of(stats, name), (row_h, false, head)) {
                *on = v;
                changed = true;
            }
        }
        if let Some(v) = folder_row(ui, ROOT_FILES, true, choice.root_files, stats_of(stats, ""), (row_h, true, head)) {
            choice.root_files = v;
            changed = true;
        }
    });
    changed
}

impl<H: DesktopHost> DesktopApp<H> {
    pub(super) fn folders_ui(&mut self, ui: &mut Ui, progress: &SyncProgress) {
        let t = Look::get(ui.ctx());
        if self.editing.is_none() && self.error.is_none() {
            self.list_root();
        }
        let loading = self.net.is_waiting(&Key::ListRoot);
        let dirty = self.editing.as_ref().is_some_and(|c| c.differs_from(&self.settings));
        let (mut apply, mut revert, mut refresh) = (false, false, false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            refresh = desk::square_button(ui, Icon::Refresh, "Refresh the folder list", !loading).clicked();
            ui.add_space(2.0);
            let files = progress.files_synced();
            let place = self.short_folder();
            let count = (files > 0).then(|| format!("{} files here", thousands(files)));
            let mut parts = vec![place.as_str()];
            if let Some(c) = &count {
                parts.push(c);
            }
            chrome::meta(ui, &parts);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                apply = chrome::button(ui, "Apply", Kind::Primary, Some(Icon::Check), dirty).clicked();
                revert = chrome::button(ui, "Revert", Kind::Secondary, None, dirty).clicked();
                if ui.available_width() > 260.0 {
                    let picker = egui::ComboBox::from_id_salt("craftcloud-folder-sort")
                        .width(150.0)
                        .height(200.0)
                        .selected_text(egui::RichText::new(self.sort.label()).size(14.0))
                        .show_ui(ui, |ui| {
                            for s in Sort::ALL {
                                ui.selectable_value(&mut self.sort, s, s.label());
                            }
                        });
                    let sort = self.sort.label();
                    picker.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, format!("Sort: {sort}")));
                    chrome::hud(ui, "Sort", t.muted);
                }
            });
        });
        ui.add_space(16.0);
        if refresh {
            self.editing = None;
            self.error = None;
            self.list_root();
        }
        if revert {
            self.editing = None;
        }
        if apply {
            self.apply_folders();
        }
        let Some(choice) = &mut self.editing else {
            if let Some(e) = self.error.clone() {
                let mut retry = false;
                desk::state_panel(ui, "Offline", t.bad, "Can't list the cloud folders", &e, |ui| {
                    retry = chrome::button(ui, "Retry now", Kind::Secondary, Some(Icon::Refresh), true).clicked();
                });
                if retry {
                    self.error = None;
                    self.list_root();
                }
            } else {
                chrome::hud(ui, "Loading the cloud folders…", t.muted);
                ui.add_space(6.0);
                desk::busy_meter(ui);
            }
            return;
        };
        let sort = self.sort;
        egui::ScrollArea::vertical().id_salt("craftcloud-folders").auto_shrink([false, true]).show(ui, |ui| {
            folder_table_sorted(ui, choice, &progress.folders, desk::ROW_H, true, sort);
            ui.add_space(10.0);
            chrome::body(
                ui,
                "Switching a folder off removes its local copy from this computer (files changed here and not yet synced stay); the cloud keeps it. Changes take effect when you apply them.",
                12.5,
            );
            if let Some(e) = &self.error {
                ui.label(egui::RichText::new(e).size(13.0).color(t.bad));
            }
        });
    }

    fn apply_folders(&mut self) {
        let Some(choice) = &self.editing else { return };
        let next = choice.apply_to(&self.settings);
        match self.host.change_folders(&next) {
            Ok(()) => {
                self.settings = next;
                self.editing = None;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Couldn't save: {e}")),
        }
    }

    /// The local folder with the home folder as `~`.
    pub(super) fn short_folder(&self) -> String {
        let local = &self.settings.local_folder;
        let default = self.host.default_folder();
        let home = std::path::Path::new(&default).parent().map(|p| p.display().to_string()).unwrap_or_default();
        match local.strip_prefix(&home) {
            Some(rest) if !home.is_empty() && (rest.starts_with('/') || rest.starts_with('\\')) => format!("~{rest}"),
            _ => local.clone(),
        }
    }
}
