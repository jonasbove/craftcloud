//! The launcher's list-page parts the admin pages use, kept apart from [`crate::chrome`] so other
//! pages can pick them up: the toolbar row (refresh square, joined cells, search, sort picker), the
//! table frame, initials avatars, a "…" menu, toasts and the details overlay frame.
//!
//! Sizes follow `design/README.md` ("egui implementation notes"); colours come from [`Tokens`].

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use crate::chrome::{CONTROL_H, caps, hud, hud_galley, initials, tint};
use crate::theme::Tokens;

/// A 34 px square with a drawn refresh arrow, in a hairline box.
pub fn refresh_button(ui: &mut Ui, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(CONTROL_H, CONTROL_H), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = resp.hovered() && enabled;
    ui.painter().rect(rect, 0.0, if hovered { t.sunken } else { t.bg }, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    let color = if enabled { if hovered { t.ink_strong } else { t.ink } } else { t.muted.gamma_multiply(0.6) };
    let c = rect.center();
    let r = 6.0;
    let points: Vec<egui::Pos2> = (0..=20).map(|i| 0.35 + i as f32 / 20.0 * 5.0).map(|a: f32| c + vec2(a.cos() * r, a.sin() * r)).collect();
    let end = points[points.len() - 1];
    ui.painter().add(egui::Shape::line(points, Stroke::new(1.5, color)));
    // The arrow head at the end of the arc.
    ui.painter().line_segment([end, end + vec2(-0.5, -4.0)], Stroke::new(1.5, color));
    ui.painter().line_segment([end, end + vec2(3.8, -0.8)], Stroke::new(1.5, color));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Refresh"));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// Joined toggle cells in one hairline box (the launcher's view toggle). Returns the cell clicked.
pub fn cells(ui: &mut Ui, labels: &[&str], selected: Option<usize>, enabled: bool) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let galleys: Vec<_> = labels.iter().map(|l| hud_galley(ui, l, t.ink)).collect();
    let width: f32 = galleys.iter().map(|g| g.size().x + 24.0).sum();
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::hover());
    let mut x = rect.left();
    for (i, g) in galleys.into_iter().enumerate() {
        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(g.size().x + 24.0, CONTROL_H));
        x = cell.right();
        let id = ui.scope_id().with(("cells", labels[i], i));
        let resp = ui.interact(cell, id, if enabled { Sense::click() } else { Sense::hover() });
        let on = selected == Some(i);
        let hovered = resp.hovered() && enabled && !on;
        let (fill, color) = if on {
            (t.invert_bg, t.invert_fg)
        } else if hovered {
            (t.sunken, t.ink_strong)
        } else {
            (Color32::TRANSPARENT, t.muted)
        };
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
        ui.painter().rect_filled(cell, 0.0, fade(fill));
        if i > 0 {
            ui.painter().vline(cell.left(), cell.y_range(), Stroke::new(1.0, t.line));
        }
        ui.painter().galley_with_override_text_color(cell.center() - g.size() / 2.0, g, fade(color));
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, enabled, on, labels[i].to_uppercase()));
        if resp.clicked() {
            clicked = Some(i);
        }
        if enabled {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    clicked
}

/// A sunken search field with a drawn magnifier, `width` wide.
pub fn search_box(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::hover());
    ui.painter().rect(rect, 0.0, t.sunken, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
    let c = pos2(rect.left() + 18.0, rect.center().y - 1.0);
    ui.painter().circle_stroke(c, 5.0, Stroke::new(1.5, t.muted));
    ui.painter().line_segment([c + vec2(3.6, 3.6), c + vec2(7.5, 7.5)], Stroke::new(1.5, t.muted));
    let inner = Rect::from_min_max(pos2(rect.left() + 32.0, rect.top()), rect.max);
    let resp = ui
        .scope_builder(UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(egui::RichText::new(hint).color(t.muted))
                    .frame(egui::Frame::NONE)
                    .align(egui::Align2::LEFT_CENTER)
                    .min_size(vec2(0.0, CONTROL_H))
                    .desired_width(inner.width() - 8.0)
                    .font(FontId::proportional(14.0)),
            )
        })
        .inner;
    resp.widget_info(|| egui::WidgetInfo::text_edit(true, "", value.as_str(), hint));
    resp
}

/// A picker: the current choice in a 34 px box with a chevron, a menu of `options` under it.
/// Returns the option chosen this frame.
pub fn picker(ui: &mut Ui, id: &str, current: usize, options: &[&str], width: f32) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let id = ui.scope_id().with(("picker", id));
    let (rect, resp) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::click());
    let open_id = id.with("open");
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    if resp.clicked() {
        open = !open;
    }
    let hovered = resp.hovered();
    ui.painter().rect(rect, 0.0, if hovered || open { t.sunken } else { t.bg }, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    let label = options.get(current).copied().unwrap_or("");
    ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), t.ink);
    chevron(ui, pos2(rect.right() - 16.0, rect.center().y), t.muted, open);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
    let mut chosen = None;
    if open {
        let items: Vec<(&str, bool)> = options.iter().map(|o| (*o, false)).collect();
        match menu(ui, id.with("menu"), pos2(rect.left(), rect.bottom() + 2.0), width, &items, resp.clicked()) {
            MenuResult::Picked(i) => {
                chosen = Some(i);
                open = false;
            }
            MenuResult::Dismissed => open = false,
            MenuResult::Open => {}
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    chosen
}

fn chevron(ui: &Ui, c: egui::Pos2, color: Color32, up: bool) {
    let dy = if up { -2.0 } else { 2.0 };
    let s = Stroke::new(1.5, color);
    ui.painter().line_segment([c + vec2(-4.0, -dy), c + vec2(0.0, dy)], s);
    ui.painter().line_segment([c + vec2(0.0, dy), c + vec2(4.0, -dy)], s);
}

pub enum MenuResult {
    Open,
    Picked(usize),
    Dismissed,
}

/// A launcher menu at `pos` (bg, hairline, shadow; 32 px items). `(label, danger)` items.
/// `opened_now` keeps the click that opened it from closing it again.
pub fn menu(ui: &mut Ui, id: Id, pos: egui::Pos2, width: f32, items: &[(&str, bool)], opened_now: bool) -> MenuResult {
    let t = Tokens::get(ui.ctx());
    let height = items.len() as f32 * 32.0 + 2.0;
    let screen = ui.ctx().content_rect();
    // Keep it on screen: flip up or slide left when it would overflow.
    let x = pos.x.min(screen.right() - width - 4.0).max(screen.left() + 4.0);
    let y = if pos.y + height > screen.bottom() - 4.0 { (pos.y - height - 38.0).max(screen.top() + 4.0) } else { pos.y };
    let mut result = MenuResult::Open;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos2(x, y)).show(ui.ctx(), |ui| {
        let rect = Rect::from_min_size(ui.cursor().min, vec2(width, height));
        ui.painter().add(shadow(&t).as_shape(rect, CornerRadius::ZERO));
        ui.painter().rect(rect, 0.0, t.bg, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
        for (i, (label, danger)) in items.iter().enumerate() {
            let item = Rect::from_min_size(pos2(rect.left() + 1.0, rect.top() + 1.0 + i as f32 * 32.0), vec2(width - 2.0, 32.0));
            let resp = ui.interact(item, id.with(i), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(item, 0.0, t.sunken);
            }
            let color = if *danger { t.bad } else { t.ink };
            ui.painter().text(pos2(item.left() + 12.0, item.center().y), Align2::LEFT_CENTER, *label, FontId::proportional(13.0), color);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, true, *label));
            if resp.clicked() {
                result = MenuResult::Picked(i);
            }
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        ui.allocate_rect(rect, Sense::hover());
    });
    let clicked_outside = ui.input(|i| i.pointer.any_click()) && !area.response.contains_pointer() && !opened_now;
    if matches!(result, MenuResult::Open) && (clicked_outside || ui.input(|i| i.key_pressed(egui::Key::Escape))) {
        result = MenuResult::Dismissed;
    }
    result
}

pub fn shadow(t: &Tokens) -> egui::epaint::Shadow {
    egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 140 } else { 36 }) }
}

/// A square button with a drawn "…", `size` points.
pub fn more_button(ui: &mut Ui, label: &str, open: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(CONTROL_H, CONTROL_H), Sense::click());
    let on = open || resp.hovered();
    let (fill, color) = if on { (t.invert_bg, t.invert_fg) } else { (t.bg, t.ink) };
    ui.painter().rect(rect, 0.0, fill, Stroke::new(1.0, if on { t.invert_bg } else { t.line_strong }), egui::StrokeKind::Inside);
    for dx in [-5.0, 0.0, 5.0] {
        ui.painter().circle_filled(rect.center() + vec2(dx, 3.0), 1.2, color);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A square initials avatar: sunken, a strong hairline, mono initials.
pub fn avatar(ui: &mut Ui, name: &str, size: f32, faded: bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let ink = if faded { t.muted } else { t.ink };
    ui.painter().rect(r, 0.0, t.sunken, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    let g = caps(ui, &initials(name), (size * 0.32).clamp(10.0, 20.0), egui::FontFamily::Monospace, 0.5, ink);
    ui.painter().galley(r.center() - g.size() / 2.0, g, ink);
}

/// A table header cell: HUD muted, or ink with an arrow when it is the sorted column.
/// Returns its response (click to sort).
pub fn header_cell(ui: &mut Ui, rect: Rect, label: &str, sorted: Option<bool>) -> Response {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, ui.scope_id().with(("th", label)), Sense::click());
    let color = if sorted.is_some() || resp.hovered() { t.ink } else { t.muted };
    let g = hud_galley(ui, label, color);
    let pos = pos2(rect.left(), rect.center().y - g.size().y / 2.0);
    let w = g.size().x;
    ui.painter().galley(pos, g, color);
    if let Some(ascending) = sorted {
        let c = pos2(rect.left() + w + 9.0, rect.center().y);
        let (a, b, d) = if ascending {
            (c + vec2(-3.5, 2.0), c + vec2(3.5, 2.0), c + vec2(0.0, -2.5))
        } else {
            (c + vec2(-3.5, -2.0), c + vec2(3.5, -2.0), c + vec2(0.0, 2.5))
        };
        ui.painter().add(egui::Shape::convex_polygon(vec![a, b, d], color, Stroke::NONE));
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColumnHeader, true, label.to_uppercase()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A toast at the bottom right of `area`: a good-tone rule on the left, a title, a line under
/// it, and a close cross. Returns `true` when closed.
pub fn toast(ui: &mut Ui, area: Rect, title: &str, body: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let w = (area.width() - 32.0).min(420.0);
    let font = FontId::proportional(12.5);
    let body_g = ui.painter().layout(body.to_owned(), font, t.muted, w - 64.0);
    let h = 44.0 + body_g.size().y;
    let rect = Rect::from_min_size(pos2(area.right() - w - 24.0, area.bottom() - h - 24.0), vec2(w, h));
    let mut closed = false;
    egui::Area::new(Id::unique("craftcloud-toast")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
        ui.painter().add(shadow(&t).as_shape(rect, CornerRadius::ZERO));
        ui.painter().rect(rect, 0.0, t.bg, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(2.0, h)), 0.0, t.good);
        let title_g = ui.painter().layout_no_wrap(title.to_owned(), FontId::proportional(14.0), t.ink_strong);
        ui.painter().galley(rect.min + vec2(16.0, 14.0), title_g, t.ink_strong);
        ui.painter().galley(rect.min + vec2(16.0, 36.0), body_g, t.muted);
        let x = Rect::from_center_size(pos2(rect.right() - 22.0, rect.top() + 22.0), vec2(28.0, 28.0));
        let resp = ui.interact(x, Id::unique("craftcloud-toast-close"), Sense::click());
        cross(ui, x.center(), if resp.hovered() { t.ink_strong } else { t.muted });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close"));
        closed = resp.clicked();
        ui.allocate_rect(rect, Sense::hover());
        // Screen readers and tests see the message.
        let info = ui.interact(rect.shrink(30.0), Id::unique("craftcloud-toast-text"), Sense::hover());
        info.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, title));
    });
    closed
}

/// A drawn 10 px cross.
pub fn cross(ui: &Ui, c: egui::Pos2, color: Color32) {
    let s = Stroke::new(1.5, color);
    ui.painter().line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], s);
    ui.painter().line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], s);
}

/// A square close button with a drawn cross (34 px, hairline box).
pub fn close_button(ui: &mut Ui) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(CONTROL_H, CONTROL_H), Sense::click());
    let on = resp.hovered();
    ui.painter().rect(rect, 0.0, if on { t.invert_bg } else { t.bg }, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    cross(ui, rect.center(), if on { t.invert_fg } else { t.ink });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close"));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Dims `area`, swallows clicks on it, and draws an overlay card of `size` (shrunk to fit) with the
/// sheet's shadow, strong hairline and 2 px accent rule. Returns the card's rect.
pub fn overlay(ui: &mut Ui, area: Rect, size: egui::Vec2, id: &str) -> Rect {
    let t = Tokens::get(ui.ctx());
    block(ui, area, id);
    let margin = if area.width() < 600.0 { 8.0 } else { 32.0 };
    let w = size.x.min(area.width() - 2.0 * margin).max(1.0);
    let h = size.y.min(area.height() - 2.0 * if area.width() < 600.0 { 8.0 } else { 20.0 }).max(1.0);
    let rect = Rect::from_center_size(area.center(), vec2(w, h));
    ui.painter().add(shadow(&t).as_shape(rect, CornerRadius::ZERO));
    ui.painter().rect(rect, 0.0, t.bg, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Outside);
    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(w, 2.0)), 0.0, t.accent);
    rect
}

/// Dims `area` and takes every click on it, so what is under a sheet can't be used.
pub fn block(ui: &mut Ui, area: Rect, id: &str) {
    let t = Tokens::get(ui.ctx());
    ui.interact(area, ui.scope_id().with(("block", id)), Sense::click_and_drag());
    ui.painter().rect_filled(area, 0.0, Color32::from_black_alpha(if t.dark { 90 } else { 30 }));
}

/// The hero wash of a sheet, as a fill.
pub fn hero_fill(t: &Tokens) -> Color32 {
    tint(t.accent, if t.dark { 0.09 } else { 0.07 })
}

/// A strip of facts in joined cells (HUD key over a mono value), wrapping into rows of
/// `per_row` when narrow.
pub fn facts_strip(ui: &mut Ui, facts: &[(&str, String)]) {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width();
    let per_row = if width >= 900.0 {
        facts.len().max(1)
    } else if width >= 480.0 {
        2
    } else {
        1
    };
    let cell_w = (width / per_row as f32).floor();
    let rows = facts.len().div_ceil(per_row);
    let cell_h = 58.0;
    let (rect, _) = ui.allocate_exact_size(vec2(cell_w * per_row as f32, cell_h * rows as f32), Sense::hover());
    for (i, (key, value)) in facts.iter().enumerate() {
        let (row, col) = (i / per_row, i % per_row);
        let cell = Rect::from_min_size(rect.min + vec2(col as f32 * cell_w, row as f32 * cell_h), vec2(cell_w, cell_h));
        ui.painter().rect_stroke(cell, 0.0, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
        let k = hud_galley(ui, key, t.muted);
        ui.painter().galley(cell.min + vec2(16.0, 12.0), k, t.muted);
        let v = ui.painter().layout(value.clone(), FontId::monospace(13.0), t.ink_strong, cell_w - 32.0);
        ui.painter().galley(cell.min + vec2(16.0, 32.0), v, t.ink_strong);
        let resp = ui.interact(cell, ui.scope_id().with(("fact", i)), Sense::hover());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{}: {value}", key.to_uppercase())));
    }
}

/// A 4 px meter whose fill turns warn above 80 % and bad above 95 %.
pub fn storage_meter(ui: &mut Ui, width: f32, fraction: Option<f32>) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.line);
    if let Some(f) = fraction {
        let f = f.clamp(0.0, 1.0);
        let color = if f > 0.95 {
            t.bad
        } else if f > 0.8 {
            t.warn
        } else {
            t.accent
        };
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(rect.width() * f, 4.0)), 0.0, color);
    }
}

/// A HUD line with a leading rule, used as a section title in long pages.
pub fn section_title(ui: &mut Ui, title: &str, note: Option<&str>) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        hud(ui, title, t.ink);
        if let Some(note) = note {
            ui.add_space(10.0);
            hud(ui, note, t.muted);
        }
    });
}
