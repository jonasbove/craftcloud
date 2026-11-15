//! The list itself: the table (rows of icon, name, app, date, size) and the grid of tiles, with
//! selection by click, Ctrl/Cmd-click, Shift-click and the check column.

use egui::{Align2, FontId, Pos2, Rect, Sense, Ui, pos2, vec2};

use super::draw::{self, Glyph};
use super::model::{self, Item, SortKey};
use super::{Files, Key, Menu, THUMBS_AT_ONCE, Thumb, View, time};
use crate::backend::Backend;
use crate::chrome;
use crate::theme::Tokens;
use crate::web::bytes;
use craftcloud_core::api::files;

impl Files {
    pub(super) fn columns(&self, width: f32) -> Vec<(Col, f32)> {
        let mut cols = vec![(Col::Check, draw::CHECK_W), (Col::Name, 0.0)];
        let in_folder = matches!(self.view, View::Folder(_));
        if width >= 900.0 {
            cols.push(if in_folder { (Col::Opens, 170.0) } else { (Col::Location, 200.0) });
        }
        if self.view == View::Trash {
            // T-048: when each item went, with its time, so two of the same name tell apart.
            // It stays on a phone, in place of the size.
            cols.push((Col::Trashed, if width >= 640.0 { 160.0 } else { 132.0 }));
            if width >= TRASH_PURGE_FROM {
                // Wide enough for "DELETED FOR GOOD ▲" and the gap before SIZE.
                cols.push((Col::Deletes, 196.0));
            }
            if width >= 640.0 {
                cols.push((Col::Size, 96.0));
            }
        } else {
            if width >= 640.0 {
                cols.push(if self.view == View::Recent { (Col::Opened, 150.0) } else { (Col::Modified, 150.0) });
            }
            if width >= 420.0 {
                cols.push((Col::Size, 96.0));
            }
        }
        cols.push((Col::More, draw::MORE_W));
        let fixed: f32 = cols.iter().map(|(_, w)| w).sum();
        if let Some(name) = cols.iter_mut().find(|(c, _)| *c == Col::Name) {
            name.1 = (width - fixed).max(120.0);
        }
        cols
    }

    pub(super) fn table_ui(&mut self, ui: &mut Ui, rect: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        let cols = self.columns(rect.width());
        let mut xs = Vec::with_capacity(cols.len());
        let mut x = rect.left();
        for (_, w) in &cols {
            xs.push(x);
            x += w;
        }
        // Header.
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), draw::HEADER_H));
        ui.painter().hline(header.x_range(), header.top() + 0.5, t.hairline());
        let heads: Vec<(f32, f32, &str, Option<bool>)> = cols
            .iter()
            .zip(&xs)
            .map(|((c, w), x)| {
                let sorted = c.sort_key().filter(|k| *k == self.sort.key).map(|_| self.sort.ascending);
                (*x + c.inset(), *w - c.inset(), c.label(), sorted)
            })
            .collect();
        if let Some(i) = draw::table_header(ui, header, &heads)
            && let Some(key) = cols[i].0.sort_key()
        {
            self.sort = self.sort.click(key);
            model::sort_items(&mut self.items, self.sort);
        }
        // Select-all square in the header's check column.
        let all = !self.items.is_empty() && self.selection.len() == self.items.len();
        let check_rect = Rect::from_min_size(header.min, vec2(draw::CHECK_W, header.height()));
        let resp = ui.interact(check_rect, ui.scope_id().with("select-all"), Sense::click());
        draw::check(ui, check_rect.center(), all, resp.hovered() || !self.selection.is_empty());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, all, "Select all"));
        if resp.clicked() {
            self.selection = if all { Vec::new() } else { self.items.iter().map(|i| i.id.clone()).collect() };
        }

        let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
        let mut action: Option<RowAction> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, draw::ROW_H, self.items.len(), |ui, range| {
                for index in range {
                    if let Some(a) = self.row_ui(ui, index, &cols, backend) {
                        action = Some(a);
                    }
                }
            });
        });
        self.keyboard(ui, backend);
        if let Some(a) = action {
            self.row_action(backend, a);
        }
    }

    fn row_ui(&mut self, ui: &mut Ui, index: usize, cols: &[(Col, f32)], backend: &mut dyn Backend) -> Option<RowAction> {
        let t = Tokens::get(ui.ctx());
        let item = self.items[index].clone();
        let selected = self.selection.contains(&item.id);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), draw::ROW_H), Sense::click());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        // Without the DELETED FOR GOOD column, the row says it on hover.
        let resp = match item.expires.as_deref() {
            Some(e) if !cols.iter().any(|(c, _)| *c == Col::Deletes) => resp.on_hover_text(format!("Deleted for good {}", time::when(e, self.clock))),
            _ => resp,
        };
        let hovered = resp.hovered();
        draw::row_background(ui, rect, hovered, selected);
        let mut action = None;
        let mut x = rect.left();
        for (col, w) in cols {
            let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, rect.height()));
            x += w;
            match col {
                Col::Check => {
                    let r = ui.interact(cell, ui.scope_id().with(("check", &item.id)), Sense::click());
                    draw::check(ui, cell.center(), selected, hovered || r.hovered() || !self.selection.is_empty());
                    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, selected, format!("Select {}", item.name)));
                    if r.clicked() {
                        action = Some(RowAction::Toggle(index));
                    }
                }
                Col::Name => {
                    let icon = Rect::from_center_size(pos2(cell.left() + 12.0, cell.center().y), vec2(22.0, 22.0));
                    self.paint_icon(ui, backend, icon, &item);
                    let mut right = cell.right() - 12.0;
                    if item.shared {
                        right -= 16.0;
                        draw::paint_glyph(ui, Glyph::Link, pos2(right + 8.0, cell.center().y), t.muted);
                    }
                    if item.starred {
                        right -= 18.0;
                        draw::star(ui, pos2(right + 9.0, cell.center().y), 6.5, Some(t.warn), None);
                    }
                    let g = draw::fit_text(ui, &item.name, FontId::proportional(14.0), t.ink_strong, right - (icon.right() + 12.0) - 4.0);
                    let gx = icon.right() + 12.0;
                    ui.painter().galley(pos2(gx, cell.center().y - g.size().y / 2.0), g, t.ink_strong);
                }
                Col::Opens => {
                    if let Some(app) = item.app() {
                        let mut child = ui.new_child(
                            egui::UiBuilder::new().max_rect(cell.shrink2(vec2(0.0, 12.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        draw::app_chip(&mut child, app, 16.0);
                    } else {
                        let text = if item.is_folder() { "—" } else { "Download" };
                        ui.painter().text(pos2(cell.left(), cell.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), t.muted);
                    }
                }
                Col::Location => {
                    let folder = item.path.rsplit_once('/').map_or("My files".to_owned(), |(f, _)| format!("My files/{f}"));
                    let g = draw::fit_text(ui, &folder, FontId::proportional(13.0), t.muted, cell.width() - 16.0);
                    ui.painter().galley(pos2(cell.left(), cell.center().y - g.size().y / 2.0), g, t.muted);
                }
                Col::Trashed => {
                    let text = item.trashed.as_deref().map(|e| time::when_exact(e, self.clock)).unwrap_or_default();
                    ui.painter().text(pos2(cell.left(), cell.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), t.muted);
                }
                Col::Deletes => {
                    let text = item.expires.as_deref().map(|e| time::when(e, self.clock)).unwrap_or_default();
                    ui.painter().text(pos2(cell.left(), cell.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), t.muted);
                }
                Col::Modified | Col::Opened => {
                    let at = if *col == Col::Opened { item.opened.as_deref().unwrap_or_default() } else { item.modified.as_str() };
                    let text = time::when(at, self.clock);
                    ui.painter().text(pos2(cell.left(), cell.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), t.muted);
                }
                Col::Size => {
                    let text = item.size.map_or_else(|| "—".to_owned(), bytes);
                    ui.painter().text(pos2(cell.left(), cell.center().y), Align2::LEFT_CENTER, text, FontId::monospace(12.0), t.muted);
                }
                Col::More => {
                    let r = ui.interact(cell, ui.scope_id().with(("more", &item.id)), Sense::click());
                    let c = cell.center();
                    if r.hovered() {
                        ui.painter().rect_filled(Rect::from_center_size(c, vec2(28.0, 28.0)), 0.0, t.line);
                    }
                    draw::paint_glyph(ui, Glyph::More, c, if hovered || selected { t.ink_strong } else { t.muted });
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("More for {}", item.name)));
                    if r.clicked() {
                        action = Some(RowAction::Menu(index, pos2(cell.right() - 8.0, cell.bottom())));
                    }
                }
            }
        }
        let name = item.name.clone();
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Row, true, selected, &name));
        if action.is_some() {
            return action;
        }
        if resp.double_clicked() {
            Some(RowAction::Open(index))
        } else if resp.secondary_clicked() {
            Some(RowAction::Menu(index, resp.interact_pointer_pos().unwrap_or(rect.center())))
        } else if resp.clicked() {
            Some(RowAction::Select(index, ui.input(|i| i.modifiers)))
        } else {
            None
        }
    }

    pub(super) fn grid_ui(&mut self, ui: &mut Ui, rect: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        let tile = vec2(176.0, 168.0);
        let gap = 12.0;
        let per_row = (((rect.width() + gap) / (tile.x + gap)).floor() as usize).max(1);
        let w = ((rect.width() - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
        let rows = self.items.len().div_ceil(per_row);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, t.hairline());
        let mut action: Option<RowAction> = None;
        let body = rect.shrink2(vec2(0.0, 0.0)).with_min_y(rect.top() + 12.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, tile.y, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        for index in row * per_row..((row + 1) * per_row).min(self.items.len()) {
                            if let Some(a) = self.tile_ui(ui, index, vec2(w, tile.y), backend) {
                                action = Some(a);
                            }
                        }
                    });
                }
            });
        });
        self.keyboard(ui, backend);
        if let Some(a) = action {
            self.row_action(backend, a);
        }
    }

    fn tile_ui(&mut self, ui: &mut Ui, index: usize, size: egui::Vec2, backend: &mut dyn Backend) -> Option<RowAction> {
        let t = Tokens::get(ui.ctx());
        let item = self.items[index].clone();
        let selected = self.selection.contains(&item.id);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let hovered = resp.hovered();
        let fill = if selected {
            chrome::tint(t.accent, if t.dark { 0.16 } else { 0.12 })
        } else if hovered {
            t.sunken
        } else {
            t.bg
        };
        ui.painter().rect(rect, 0.0, fill, egui::Stroke::new(1.0, if selected { t.accent } else { t.line }), egui::StrokeKind::Inside);
        let preview = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(rect.width() - 2.0, rect.height() - 48.0));
        ui.painter().rect_filled(preview, 0.0, if t.dark { t.sunken } else { egui::Color32::from_gray(228) });
        if let Some(Thumb::Ready(tex)) = item.has_thumbnail().then(|| self.thumb(ui.ctx(), backend, &item)).flatten() {
            let size = tex.size_vec2();
            let scale = (preview.width() / size.x).min(preview.height() / size.y);
            let r = Rect::from_center_size(preview.center(), size * scale);
            ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        } else {
            draw::file_glyph(ui, Rect::from_center_size(preview.center(), vec2(48.0, 48.0)), &item);
        }
        let name_y = preview.bottom() + 14.0;
        let g = draw::fit_text(ui, &item.name, FontId::proportional(13.5), t.ink_strong, rect.width() - 24.0);
        ui.painter().galley(pos2(rect.left() + 12.0, name_y), g, t.ink_strong);
        let sub = if item.is_folder() { "Folder".to_owned() } else { item.size.map(bytes).unwrap_or_default() };
        ui.painter().text(pos2(rect.left() + 12.0, name_y + 20.0), Align2::LEFT_TOP, sub, FontId::monospace(11.0), t.muted);
        if item.starred {
            draw::star(ui, pos2(rect.right() - 14.0, preview.top() + 14.0), 6.5, Some(t.warn), None);
        }
        let name = item.name.clone();
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Row, true, selected, &name));
        if resp.double_clicked() {
            Some(RowAction::Open(index))
        } else if resp.secondary_clicked() {
            Some(RowAction::Menu(index, resp.interact_pointer_pos().unwrap_or(rect.center())))
        } else if resp.clicked() {
            Some(RowAction::Select(index, ui.input(|i| i.modifiers)))
        } else {
            None
        }
    }

    /// The thumbnail of an image, asked for when first needed.
    pub(super) fn thumb(&mut self, ctx: &egui::Context, backend: &mut dyn Backend, item: &Item) -> Option<&Thumb> {
        if let Some(Thumb::Image(image)) = self.thumbs.get_mut(&item.id) {
            let image = std::mem::take(image);
            let tex = ctx.load_texture(format!("thumb-{}", item.id), image, egui::TextureOptions::LINEAR);
            self.thumbs.insert(item.id.clone(), Thumb::Ready(tex));
        }
        if !self.thumbs.contains_key(&item.id) {
            let loading = self.thumbs.values().filter(|t| matches!(t, Thumb::Loading)).count();
            if loading >= THUMBS_AT_ONCE {
                return None;
            }
            self.thumbs.insert(item.id.clone(), Thumb::Loading);
            let req = files::thumbnail(&self.server, &item.id, files::ThumbnailSize::Preview);
            self.net.send(backend, Key::Thumb(item.id.clone()), req);
        }
        self.thumbs.get(&item.id)
    }

    fn paint_icon(&mut self, ui: &Ui, backend: &mut dyn Backend, rect: Rect, item: &Item) {
        if item.has_thumbnail()
            && let Some(Thumb::Ready(tex)) = self.thumb(ui.ctx(), backend, item)
        {
            let size = tex.size_vec2();
            let side = size.x.min(size.y);
            let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(side / size.x, side / size.y));
            ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
            return;
        }
        draw::file_glyph(ui, rect, item);
    }

    pub(super) fn row_action(&mut self, backend: &mut dyn Backend, action: RowAction) {
        match action {
            RowAction::Toggle(i) => {
                // A click on a file shows its details again.
                self.details.hidden = None;
                self.toggle(i);
            }
            RowAction::Select(i, m) => {
                self.menu = None;
                self.details.hidden = None;
                self.select_click(i, m);
            }
            RowAction::Open(i) => self.open_item(backend, i),
            RowAction::Menu(i, at) => {
                if let Some(item) = self.items.get(i) {
                    if !self.selection.contains(&item.id) {
                        self.selection = vec![item.id.clone()];
                        self.anchor = Some(i);
                        // The menu leaves the details as they were: a hidden panel stays hidden,
                        // so the page doesn't shift under the open menu.
                        if !self.details.shown {
                            self.details.hidden = Some(item.id.clone());
                        }
                    }
                    self.menu = Some(Menu { id: item.id.clone(), at });
                }
            }
        }
    }

    /// Keys on the list when no text field has focus.
    pub(super) fn keyboard(&mut self, ui: &mut Ui, backend: &mut dyn Backend) {
        if ui.ctx().memory(|m| m.focused().is_some()) || self.menu.is_some() {
            return;
        }
        let (enter, escape, all) =
            ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape), i.modifiers.command && i.key_pressed(egui::Key::A)));
        if escape {
            self.selection.clear();
        }
        if all {
            self.selection = self.items.iter().map(|i| i.id.clone()).collect();
        }
        if enter && let [one] = self.selected_indices().as_slice() {
            self.open_item(backend, *one);
        }
        let (f2, delete) = ui.input(|i| (i.key_pressed(egui::Key::F2), i.key_pressed(egui::Key::Delete)));
        if f2
            && self.view != View::Trash
            && let [one] = self.selected_indices().as_slice()
        {
            self.start_rename(*one);
        }
        if delete && self.view != View::Trash {
            self.trash_selection(backend);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Col {
    Check,
    Name,
    Opens,
    Location,
    Modified,
    Opened,
    /// When it went to the trash.
    Trashed,
    /// When the trash deletes it for good.
    Deletes,
    Size,
    More,
}

/// The trash shows DELETED FOR GOOD as a column from this width; under it, as the row's tooltip.
const TRASH_PURGE_FROM: f32 = 760.0;

impl Col {
    fn label(self) -> &'static str {
        match self {
            Self::Check | Self::More => "",
            Self::Name => "Name",
            Self::Opens => "Opens in",
            Self::Location => "Location",
            Self::Modified => "Modified",
            Self::Opened => "Opened",
            Self::Trashed => "Deleted",
            Self::Deletes => "Deleted for good",
            Self::Size => "Size",
        }
    }

    fn sort_key(self) -> Option<SortKey> {
        match self {
            Self::Name => Some(SortKey::Name),
            Self::Modified => Some(SortKey::Modified),
            Self::Opened => Some(SortKey::Opened),
            Self::Trashed => Some(SortKey::Trashed),
            Self::Deletes => Some(SortKey::Expires),
            Self::Size => Some(SortKey::Size),
            _ => None,
        }
    }

    /// Where the column's text starts within it.
    fn inset(self) -> f32 {
        if self == Self::Name { 34.0 } else { 0.0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum RowAction {
    Toggle(usize),
    Select(usize, egui::Modifiers),
    Open(usize),
    Menu(usize, Pos2),
}
