//! The desktop client's parts (design frames `v2-desktop-*`): the header with ACTIVITY / FOLDERS /
//! SETTINGS cells and a status cell, the footer with "Open in browser", toggle cells, the activity
//! list rows with their status badges, the folder table, compact state panels, the sign-in code box.

use egui::{Color32, FontId, Rect, Response, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::{CONTROL_H, Icon, caps, hud, hud_galley, nav_item, paint_icon, wordmark_text};
use crate::theme::Tokens;
use crate::widgets::logo;

/// Activity list rows and folder table rows.
pub const ROW_H: f32 = 40.0;
/// The folder table's header row.
pub const HEAD_H: f32 = 36.0;
/// The status badge column in the activity list.
pub const BADGE_W: f32 = 84.0;

/// A status dot, `size` across.
pub fn dot(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

/// The desktop header: the brand cell, nav cells (none before sign-in), and the status cell at the
/// right. Returns the nav cell clicked.
pub fn header(ui: &mut Ui, rect: Rect, nav: &[(&str, bool)], status: &str, status_dot: Color32) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    ui.painter().rect_filled(rect, 0.0, t.bg);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, t.hairline());
    let vrule = |ui: &Ui, x: f32| ui.painter().vline(x, rect.y_range(), t.hairline());
    // Narrow windows drop the wordmark: the logo alone names the app.
    let narrow = rect.width() < 720.0;
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(20.0);
        logo(ui, 24.0);
        if !narrow {
            ui.add_space(10.0);
            wordmark_text(ui, 18.0);
        }
        ui.add_space(20.0);
        vrule(ui, ui.cursor().left());
        if !nav.is_empty() {
            ui.add_space(12.0);
            for (i, (label, selected)) in nav.iter().enumerate() {
                if nav_item(ui, label, *selected, true).clicked() {
                    clicked = Some(i);
                }
                ui.add_space(4.0);
            }
            ui.add_space(8.0);
            vrule(ui, ui.cursor().left());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(16.0);
            let label = ui.add(egui::Label::new(egui::RichText::new(status).size(13.0).color(t.ink)).truncate());
            label.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Status, true, status));
            ui.add_space(8.0);
            dot(ui, status_dot, 8.0);
            ui.add_space(16.0);
            vrule(ui, ui.cursor().right());
        });
    });
    clicked
}

/// The 34 px footer: a HUD status on the left, "Open in browser ↗" on the right. Returns whether
/// the link was clicked.
pub fn footer(ui: &mut Ui, rect: Rect, left: &str, link: Option<&str>) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut clicked = false;
    ui.painter().rect_filled(rect, 0.0, t.bg);
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, t.hairline());
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(20.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.set_clip_rect(rect);
        hud(ui, left, t.muted);
        if let Some(link) = link {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let g = ui.painter().layout_no_wrap(format!("{link} ↗"), FontId::proportional(12.0), t.muted);
                let (r, resp) = ui.allocate_exact_size(g.size() + vec2(0.0, 8.0), Sense::click());
                let color = if resp.hovered() { t.ink_strong } else { t.muted };
                ui.painter().galley_with_override_text_color(pos2(r.left(), r.center().y - g.size().y / 2.0), g, color);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, link));
                clicked = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            });
        }
    });
    clicked
}

/// Joined toggle cells, 34 tall, in one hairline box: the chosen one inverted. Each cell is named
/// "`name`: OPTION" for screen readers (just the option when `name` is empty). `cell_w` fixes
/// the cells' width. Returns the cell clicked.
pub fn cells(ui: &mut Ui, name: &str, options: &[&str], selected: usize, enabled: bool, cell_w: Option<f32>) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let galleys: Vec<_> = options.iter().map(|o| hud_galley(ui, o, t.muted)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| cell_w.unwrap_or(g.size().x + 24.0)).collect();
    // One allocation for the whole box, so the cells read left to right in any layout.
    let (frame, _) = ui.allocate_exact_size(vec2(widths.iter().sum(), CONTROL_H), Sense::hover());
    let mut left = frame.left();
    for (i, (option, g)) in options.iter().zip(galleys).enumerate() {
        let rect = Rect::from_min_size(pos2(left, frame.top()), vec2(widths[i], CONTROL_H));
        left += widths[i];
        let on = i == selected;
        let id = ui.make_persistent_id(("cells", name, i));
        let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
        let hovered = resp.hovered() && enabled && !on;
        let (fill, color) = match (on, hovered) {
            (true, _) => (t.invert_bg, t.invert_fg),
            (false, true) => (t.sunken, t.ink_strong),
            (false, false) => (Color32::TRANSPARENT, t.muted),
        };
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
        ui.painter().rect_filled(rect, 0.0, fade(fill));
        ui.painter().galley_with_override_text_color(rect.center() - g.size() / 2.0, g, fade(color));
        if i > 0 {
            ui.painter().vline(rect.left(), rect.y_range(), t.hairline());
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, t.accent_ink), egui::StrokeKind::Inside);
        }
        let label = if name.is_empty() { option.to_uppercase() } else { format!("{name}: {}", option.to_uppercase()) };
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, on, &label));
        if resp.clicked() && !on {
            clicked = Some(i);
        }
        if enabled && !on {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
    ui.painter().rect_stroke(frame, 0.0, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    clicked
}

/// OFF | ON cells for a switch. Returns the new value when it was clicked.
pub fn off_on(ui: &mut Ui, name: &str, on: bool, enabled: bool) -> Option<bool> {
    cells(ui, name, &["Off", "On"], usize::from(on), enabled, Some(56.0)).map(|i| i == 1)
}

/// A 34 × 34 square button with only an icon (refresh), named `label` for screen readers.
pub fn square_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(CONTROL_H, CONTROL_H), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = resp.hovered() && enabled;
    let (fill, color) = if hovered { (t.invert_bg, t.invert_fg) } else { (t.bg, t.ink) };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
    ui.painter().rect(rect, 0.0, fade(fill), Stroke::new(1.0, fade(t.line_strong)), egui::StrokeKind::Inside);
    paint_icon(ui, icon, rect.center(), fade(color));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    resp.on_hover_text(label)
}

/// A status badge of a fixed width (the activity list's column): 20 tall, sunken, hairline.
pub fn badge_fixed(ui: &mut Ui, text: &str, tone: Color32, width: f32) {
    let t = Tokens::get(ui.ctx());
    let g = caps(ui, text, 10.0, egui::FontFamily::Monospace, 1.0, tone);
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(g.size().x + 12.0), 20.0), Sense::hover());
    ui.painter().rect(rect, 0.0, t.sunken, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, tone);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text.to_uppercase()));
}

/// A HUD link, underlined (the meta line's OPEN FOLDER).
pub fn hud_link(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let g = hud_galley(ui, text, t.muted);
    let (rect, resp) = ui.allocate_exact_size(g.size() + vec2(0.0, 3.0), Sense::click());
    let color = if resp.hovered() { t.ink_strong } else { t.muted };
    let width = g.size().x;
    ui.painter().galley_with_override_text_color(rect.min, g, color);
    ui.painter().hline(rect.left()..=rect.left() + width, rect.bottom() - 0.5, Stroke::new(1.0, color));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, text.to_uppercase()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A meta line whose last part may be a link: HUD parts split by "·". Returns the link's response.
pub fn meta_link(ui: &mut Ui, parts: &[&str], link: Option<&str>) -> Option<Response> {
    let t = Tokens::get(ui.ctx());
    super::meta_row(ui, |ui| {
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                hud(ui, "·", t.muted);
            }
            hud(ui, part, t.muted);
        }
        link.map(|link| {
            if !parts.is_empty() {
                hud(ui, "·", t.muted);
            }
            hud_link(ui, link)
        })
    })
}

/// A hairline-framed list (the launcher's table frame), inner margin 0 16.
pub fn list_frame<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new()
        .fill(t.bg)
        .stroke(t.hairline())
        .inner_margin(egui::Margin::symmetric(16, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .inner
}

/// A full-width row `height` tall, laid out left to right, with a hairline under it unless it is
/// the last. Hovering a clickable row tints it.
pub fn row<R>(ui: &mut Ui, height: f32, last: bool, sense: Sense, add: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    if resp.hovered() && sense.senses_click() {
        ui.painter().rect_filled(rect.expand2(vec2(16.0, 0.0)), 0.0, t.sunken);
    }
    if !last {
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, t.hairline());
    }
    let inner = ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), add).inner;
    (resp, inner)
}

/// A table header row: HUD muted titles at fixed widths (`0.0` fills what the others leave).
pub fn table_head(ui: &mut Ui, columns: &[(&str, f32)]) {
    let t = Tokens::get(ui.ctx());
    row(ui, HEAD_H, false, Sense::hover(), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let fixed: f32 = columns.iter().map(|(_, w)| *w).sum();
        let fill = (ui.available_width() - fixed).max(0.0);
        for (title, w) in columns {
            let w = if *w == 0.0 { fill } else { *w };
            let (rect, _) = ui.allocate_exact_size(vec2(w, HEAD_H), Sense::hover());
            let g = hud_galley(ui, title, t.muted);
            ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.muted);
        }
    });
}

/// A filled folder (or, with `file`, a page with a folded corner), 18 px, in the muted ink.
pub fn item_icon(ui: &mut Ui, file: bool, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 18.0), Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    if file {
        let body = vec![c + vec2(-6.0, -8.0), c + vec2(2.5, -8.0), c + vec2(6.5, -4.0), c + vec2(6.5, 8.0), c + vec2(-6.0, 8.0)];
        p.add(egui::Shape::convex_polygon(body, color, Stroke::NONE));
    } else {
        p.rect_filled(Rect::from_min_max(c + vec2(-9.0, -6.5), c + vec2(-2.0, -3.0)), 0.0, color);
        p.rect_filled(Rect::from_min_max(c + vec2(-9.0, -4.5), c + vec2(9.0, 6.5)), 0.0, color);
    }
}

/// A compact state panel (design frame `v2-desktop-states`): a tag badge, a title, a line or two
/// of body text, then the panel's buttons in a row.
pub fn state_panel(ui: &mut Ui, tag: &str, tone: Color32, title: &str, text: &str, buttons: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().fill(t.bg).stroke(t.hairline()).inner_margin(egui::Margin::symmetric(16, 14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        super::badge(ui, tag, tone);
        ui.add_space(2.0);
        ui.add(egui::Label::new(egui::RichText::new(title).size(17.0).color(t.ink_strong)).wrap());
        ui.add(egui::Label::new(egui::RichText::new(text).size(13.0).color(t.muted)).wrap());
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            buttons(ui);
        });
    });
}

/// The sign-in code in a 72 px sunken box: Geist Mono 28, widely spaced.
pub fn code_box(ui: &mut Ui, code: &str) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
    ui.painter().rect(rect, 0.0, t.sunken, t.hairline(), egui::StrokeKind::Inside);
    let format = egui::TextFormat { font_id: FontId::monospace(28.0), color: t.ink_strong, extra_letter_spacing: 28.0 * 0.08, ..Default::default() };
    let g = ui.painter().layout_job(egui::text::LayoutJob::single_section(code.to_owned(), format));
    ui.painter().galley(rect.center() - g.size() / 2.0, g, t.ink_strong);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, code));
}

/// A 4 px track with an accent segment sliding along it: waiting, with no fraction to show.
pub fn busy_meter(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.line);
    let phase = (ui.input(|i| i.time) * 0.5).fract() as f32;
    let w = rect.width() * 0.24;
    let left = rect.left() + (rect.width() + w) * phase - w;
    let seg = Rect::from_min_max(pos2(left.max(rect.left()), rect.top()), pos2((left + w).min(rect.right()), rect.bottom()));
    ui.painter().rect_filled(seg, 0.0, t.accent);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

/// A settings row: the label (Body 14) on the left, the control at the right, 34 tall at least.
pub fn setting_row(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.set_min_height(CONTROL_H);
        ui.label(egui::RichText::new(label).size(14.0).color(t.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}
