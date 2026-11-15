//! The details panel beside the list: for the one file selected, a preview, its name and star,
//! what kind of file it is and which Craft app opens it, DOWNLOAD and the "…" menu, where it is,
//! and its last versions (each to restore, or ALL VERSIONS… for the whole history).
//!
//! It shows on wide pages only (the list keeps room for its MODIFIED column), for files (not
//! folders, not in the trash). A click on a file shows it, the close button hides it until another
//! click on a file, and the "…" menu leaves it as it was.
//!
//! Not here yet: "Open in <app>", because opening a cloud file in a Craft web app needs the page
//! that frames the app and passes the file over `craft-web-host` (docs/CRAFT-INTEGRATION.md § 6),
//! which isn't built; and "Share…", which comes with the share sheet.

use craftcloud_core::api::versions::{self, Version};
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::draw::{self, Glyph};
use super::model::Item;
use super::versions::{place, saved_when};
use super::{Files, Key, Menu, Thumb, View, time};
use crate::backend::{Backend, NetResult};
use crate::chrome;
use crate::theme::Tokens;
use crate::web::bytes;

/// The panel's width.
pub const WIDTH: f32 = 360.0;
/// The narrowest main column that gets the panel: the list keeps 640, its MODIFIED column.
pub const FROM: f32 = 640.0 + WIDTH;
/// Kept versions the panel lists under the current file (ALL VERSIONS… has the rest).
const SHOWN: usize = 2;

/// The panel's state.
#[derive(Default)]
pub struct Panel {
    /// The file whose details the person closed (or that the "…" menu picked while the panel was
    /// hidden); a click on a file clears it.
    pub hidden: Option<String>,
    /// Whether the panel was drawn last frame.
    pub shown: bool,
    /// The signed-in user, who owns everything in My files.
    pub owner: Option<String>,
    /// `(id, path, etag)` the versions below were asked for.
    asked: Option<(String, String, String)>,
    generation: u64,
    /// The kept versions, newest first; `None` while loading.
    list: Option<Result<Vec<Version>, String>>,
}

impl Files {
    /// The file the panel is about: the one selected, when it is a file and the panel isn't closed.
    pub(super) fn details_item(&self) -> Option<(usize, &Item)> {
        if self.view == View::Trash || self.selection.len() != 1 {
            return None;
        }
        let id = self.selection.first()?;
        let (index, item) = self.items.iter().enumerate().find(|(_, i)| &i.id == id)?;
        (!item.is_folder() && self.details.hidden.as_deref() != Some(id.as_str())).then_some((index, item))
    }

    /// Asks for the file's kept versions when the panel shows another file, or the file changed.
    fn details_versions(&mut self, backend: &mut dyn Backend) {
        let Some((_, item)) = self.details_item() else { return };
        if item.path.is_empty() {
            // Its path comes with the breadcrumbs.
            return;
        }
        let key = (item.id.clone(), item.path.clone(), item.etag.clone());
        if self.details.asked.as_ref() == Some(&key) {
            return;
        }
        let req = versions::list(&self.server, &key.1);
        self.details.asked = Some(key);
        self.details.generation += 1;
        self.details.list = None;
        self.net.send(backend, Key::Details(self.details.generation), req);
    }

    /// The answer to the panel's versions listing; one asked for another file is ignored.
    pub(super) fn on_details(&mut self, generation: u64, result: NetResult) {
        if generation != self.details.generation {
            return;
        }
        let list = result.and_then(|resp| versions::parse_list(&resp).map_err(|e| e.to_string()));
        self.details.list = Some(list);
    }

    /// Draws the panel in `rect` (the right of the page, under the header).
    pub(super) fn details_ui(&mut self, ui: &mut Ui, rect: Rect, backend: &mut dyn Backend) {
        self.details_versions(backend);
        let Some((index, item)) = self.details_item().map(|(i, item)| (i, item.clone())) else { return };
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_filled(rect, 0.0, t.bg);
        ui.painter().vline(rect.left() + 0.5, rect.y_range(), t.hairline());
        let inner = Rect::from_min_max(rect.min + vec2(24.0, 20.0), rect.max - vec2(24.0, 0.0));
        let mut child =
            ui.new_child(egui::UiBuilder::new().id_salt("files-details").max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("files-details-scroll").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
            let w = inner.width();
            ui.set_width(w);
            self.details_head(ui, w);
            self.details_preview(ui, backend, w, &item);
            self.details_name(ui, backend, w, &item);
            self.details_actions(ui, backend, w, index);
            self.details_facts(ui, &item);
            self.details_history(ui, backend, index, &item);
            ui.add_space(8.0);
        });
    }

    fn details_head(&mut self, ui: &mut Ui, w: f32) {
        let t = Tokens::get(ui.ctx());
        let (row, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
        let g = chrome::hud_galley(ui, "Details", t.muted);
        ui.painter().galley(pos2(row.left(), row.center().y - g.size().y / 2.0), g, t.muted);
        let close = Rect::from_center_size(pos2(row.right() - 10.0, row.center().y), vec2(24.0, 24.0));
        let resp = ui.interact(close, ui.scope_id().with("details-close"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(close, 0.0, t.sunken);
        }
        draw::paint_glyph(ui, Glyph::Close, close.center(), if resp.hovered() { t.ink_strong } else { t.muted });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close details"));
        if resp.on_hover_text("Close details").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.details.hidden = self.selection.first().cloned();
        }
    }

    /// The thumbnail of an image, else the app's mark (or the file's glyph), on the sunken well.
    fn details_preview(&mut self, ui: &mut Ui, backend: &mut dyn Backend, w: f32, item: &Item) {
        let t = Tokens::get(ui.ctx());
        let (well, _) = ui.allocate_exact_size(vec2(w, 148.0), Sense::hover());
        ui.painter().rect(well, 0.0, t.sunken, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
        if item.has_thumbnail()
            && let Some(Thumb::Ready(tex)) = self.thumb(ui.ctx(), backend, item)
        {
            let size = tex.size_vec2();
            let room = well.shrink(12.0);
            let scale = (room.width() / size.x.max(1.0)).min(room.height() / size.y.max(1.0));
            let shown = Rect::from_center_size(well.center(), size * scale);
            ui.painter().image(tex.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            return;
        }
        match item.app() {
            Some(app) => draw::paint_app_square(ui, Rect::from_center_size(well.center(), vec2(56.0, 56.0)), app),
            None => draw::file_glyph(ui, Rect::from_center_size(well.center(), vec2(48.0, 48.0)), item),
        }
    }

    /// The name and its star, then "DOCX document · 248 KB · opens in WordCraft".
    fn details_name(&mut self, ui: &mut Ui, backend: &mut dyn Backend, w: f32, item: &Item) {
        let t = Tokens::get(ui.ctx());
        let spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 4.0;
        let (row, name) = ui.allocate_exact_size(vec2(w, chrome::CONTROL_H), Sense::hover());
        let g = draw::fit_text(ui, &item.name, FontId::new(22.0, crate::theme::display()), t.ink_strong, w - chrome::CONTROL_H - 10.0);
        ui.painter().galley(pos2(row.left(), row.center().y - g.size().y / 2.0), g, t.ink_strong);
        name.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &item.name));
        let star = Rect::from_min_size(pos2(row.right() - chrome::CONTROL_H, row.top()), vec2(chrome::CONTROL_H, chrome::CONTROL_H));
        let label = if item.starred { "Remove star" } else { "Star" };
        let resp = ui.interact(star, ui.scope_id().with("details-star"), Sense::click());
        ui.painter().rect(star, 0.0, if resp.hovered() { t.sunken } else { t.bg }, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
        if item.starred {
            draw::star(ui, star.center(), 7.0, Some(t.warn), None);
        } else {
            draw::star(ui, star.center(), 7.0, None, Some(Stroke::new(1.5, t.ink)));
        }
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, item.starred, label));
        if resp.on_hover_text(label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.set_starred(backend, std::slice::from_ref(item), !item.starred);
        }
        // The panel's own gap after the kind line.
        ui.spacing_mut().item_spacing.y = spacing;
        ui.add(egui::Label::new(egui::RichText::new(kind_line(item)).size(12.5).color(t.muted)).wrap());
    }

    /// DOWNLOAD across the panel, then the "…" menu.
    fn details_actions(&mut self, ui: &mut Ui, backend: &mut dyn Backend, w: f32, index: usize) {
        let (row, _) = ui.allocate_exact_size(vec2(w, chrome::CONTROL_H), Sense::hover());
        let button = Rect::from_min_max(row.min, pos2(row.right() - chrome::CONTROL_H - 8.0, row.bottom()));
        if wide_button(ui, button, Glyph::Down, "Download").clicked() {
            self.download(backend, &[index]);
        }
        let more = Rect::from_min_size(pos2(row.right() - chrome::CONTROL_H, row.top()), vec2(chrome::CONTROL_H, chrome::CONTROL_H));
        let resp = ui.put(more, |ui: &mut Ui| draw::icon_button(ui, Glyph::More, "More actions", false, true));
        if resp.clicked()
            && let Some(item) = self.items.get(index)
        {
            self.menu = Some(Menu { id: item.id.clone(), at: pos2(more.right() - 220.0, more.bottom() + 4.0) });
        }
    }

    fn details_facts(&self, ui: &mut Ui, item: &Item) {
        let t = Tokens::get(ui.ctx());
        let mut rows: Vec<(&str, String)> = vec![("Modified", time::when_exact(&item.modified, self.clock)), ("Location", place(&item.path))];
        if let Some(owner) = &self.details.owner {
            rows.push(("Owner", owner.clone()));
        }
        rows.push(("Shared", if item.shared { "By link".into() } else { "Not shared".into() }));
        chrome::panel(ui, "About this file", |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let w = ui.available_width();
            for (key, value) in &rows {
                let (r, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                let g = chrome::hud_galley(ui, key, t.muted);
                ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, t.muted);
                let v = draw::fit_text(ui, value, FontId::proportional(13.0), t.ink, w - 92.0);
                ui.painter().galley(pos2(r.left() + 92.0, r.center().y - v.size().y / 2.0), v, t.ink);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{key}: {value}")));
            }
        });
    }

    /// The current file and the newest kept versions, each to restore; ALL VERSIONS… opens the sheet.
    fn details_history(&mut self, ui: &mut Ui, backend: &mut dyn Backend, index: usize, item: &Item) {
        let t = Tokens::get(ui.ctx());
        let clock = self.clock;
        let restoring = self.restoring().map(|(p, s)| (p.to_owned(), s.to_owned()));
        let mut restore = None;
        let mut all = false;
        // Only the versions asked for this file (another file's stay out while its path is unknown).
        let mine = self.details.asked.as_ref().is_some_and(|(id, _, _)| *id == item.id);
        let list = if mine { self.details.list.as_ref() } else { None };
        let kept: Vec<Version> = match list {
            Some(Ok(list)) => list.iter().take(SHOWN).cloned().collect(),
            _ => Vec::new(),
        };
        let note = match list {
            None if !item.path.is_empty() => Some("Loading…"),
            None => None,
            Some(Err(_)) => Some("Couldn't list the versions."),
            Some(Ok(list)) if list.is_empty() => Some("No earlier versions yet."),
            Some(Ok(_)) => None,
        };
        chrome::panel(ui, "Versions · last 5 kept", |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let w = ui.available_width();
            let rows = std::iter::once((time::when_exact(&item.modified, clock), item.size.unwrap_or(0), None))
                .chain(kept.iter().map(|v| (saved_when(v, clock), v.size, Some(v))));
            for (i, (when, size, version)) in rows.enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &when));
                if i > 0 {
                    ui.painter().hline(r.x_range(), r.top() + 0.5, t.hairline());
                }
                // The current file in the stronger ink, as the frame's medium weight.
                let ink = if version.is_none() { t.ink_strong } else { t.ink };
                let g = draw::fit_text(ui, &when, FontId::proportional(13.0), ink, w * 0.5);
                let gw = g.size().x;
                ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, ink);
                ui.painter().text(pos2(r.left() + gw + 8.0, r.center().y), Align2::LEFT_CENTER, bytes(size), FontId::monospace(12.0), t.muted);
                let right = Rect::from_min_max(pos2(r.left() + w * 0.5, r.top()), r.max);
                let mut cell = ui.new_child(
                    egui::UiBuilder::new().id_salt(("details-version", i)).max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                match version {
                    None => chrome::badge(&mut cell, "Current", t.good),
                    Some(v) => {
                        let busy = restoring.as_ref().is_some_and(|(p, s)| *p == item.path && *s == v.stamp);
                        if busy {
                            chrome::hud(&mut cell, "Restoring…", t.muted);
                        } else if draw::text_action(&mut cell, "Restore", false).clicked() && restoring.is_none() {
                            restore = Some(v.stamp.clone());
                        }
                    }
                }
            }
            if let Some(note) = note {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(note).size(13.0).color(t.muted));
            }
            ui.add_space(6.0);
            all = draw::text_action(ui, "All versions…", false).clicked();
        });
        if let Some(stamp) = restore {
            let known = match list {
                Some(Ok(list)) => list.clone(),
                _ => Vec::new(),
            };
            self.start_restore(backend, item.path.clone(), stamp, known);
        }
        if all {
            self.open_versions(backend, index);
        }
    }
}

/// "DOCX document · 248 KB · opens in WordCraft"; a file no Craft app claims says its kind and size.
pub fn kind_line(item: &Item) -> String {
    let mut line = format!("{} · {}", item.kind_label(), bytes(item.size.unwrap_or(0)));
    if let Some(app) = item.app() {
        line.push_str(" · opens in ");
        line.push_str(app.name);
    }
    line
}

/// A primary button across `rect`, a glyph before its label.
fn wide_button(ui: &mut Ui, rect: Rect, glyph: Glyph, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, ui.scope_id().with(("details-button", label)), Sense::click());
    let fill = t.invert_bg.gamma_multiply(if resp.hovered() { 0.8 } else { 1.0 });
    ui.painter().rect_filled(rect, 0.0, fill);
    let format = egui::TextFormat { font_id: FontId::monospace(11.0), color: t.invert_fg, extra_letter_spacing: 11.0 * 0.15, ..Default::default() };
    let g = ui.painter().layout_job(egui::text::LayoutJob::single_section(label.to_uppercase(), format));
    let total = 14.0 + 8.0 + g.size().x;
    let left = rect.center().x - total / 2.0;
    draw::paint_glyph(ui, glyph, pos2(left + 7.0, rect.center().y), t.invert_fg);
    ui.painter().galley(pos2(left + 22.0, rect.center().y - g.size().y / 2.0), g, t.invert_fg);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.to_uppercase()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::kind_line;
    use crate::web::files::model::Item;
    use craftcloud_core::api::resources::Kind;

    fn file(name: &str, size: u64) -> Item {
        Item {
            kind: Kind::File,
            id: "f1".into(),
            name: name.into(),
            size: Some(size),
            modified: "2026-11-13T11:42:00Z".into(),
            mime: String::new(),
            category: String::new(),
            starred: false,
            shared: false,
            path: String::new(),
            etag: String::new(),
            parent: None,
            expires: None,
            trashed: None,
            opened: None,
        }
    }

    #[test]
    fn the_kind_line_names_the_format_the_size_and_the_app() {
        assert_eq!(kind_line(&file("Q3 report.docx", 248_000)), "DOCX document · 248 KB · opens in WordCraft");
        assert_eq!(kind_line(&file("archive.zip", 96_300_000)), "Archive · 96.3 MB");
    }
}
