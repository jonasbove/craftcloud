//! The file browser's own parts, drawn in the launcher's look: file and folder glyphs, app chips,
//! sidebar entries, breadcrumb cells, the table header and rows, icon buttons, the "…" menu and
//! the search box. Sizes follow `design/README.md` ("egui implementation notes").

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use super::apps::App;
use super::model::Item;
use crate::chrome::{self, tint};
use crate::theme::Tokens;

pub const ROW_H: f32 = 44.0;
pub const HEADER_H: f32 = 36.0;
pub const CHECK_W: f32 = 36.0;
pub const MORE_W: f32 = 40.0;

/// Mono caps text, the HUD look, as a galley.
fn caps(ui: &Ui, text: &str, size: f32, color: Color32) -> std::sync::Arc<egui::Galley> {
    let format = egui::TextFormat { font_id: FontId::monospace(size), color, extra_letter_spacing: size * 0.15, ..Default::default() };
    ui.painter().layout_job(egui::text::LayoutJob::single_section(text.to_uppercase(), format))
}

/// A folder or file glyph in `rect` (square), coloured by the app that opens it.
pub fn file_glyph(ui: &Ui, rect: Rect, item: &Item) {
    kind_glyph(ui, rect, item.is_folder(), &item.name);
}

/// A folder, or a file named `name` in its app's colour, when only the name is known (the list of
/// share links).
pub fn kind_glyph(ui: &Ui, rect: Rect, folder: bool, name: &str) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let s = rect.width();
    if folder {
        let body = Rect::from_min_max(rect.min + vec2(0.0, s * 0.22), rect.max - vec2(0.0, s * 0.1));
        let tab = Rect::from_min_size(rect.min + vec2(0.0, s * 0.12), vec2(s * 0.42, s * 0.16));
        let fill = if t.dark { Color32::from_gray(150) } else { Color32::from_gray(120) };
        p.rect_filled(tab, CornerRadius::same(1), fill);
        p.rect_filled(body, CornerRadius::same(1), fill);
        return;
    }
    let fill = super::apps::for_name(name).map_or(if t.dark { Color32::from_gray(140) } else { Color32::from_gray(130) }, super::apps::color);
    let w = s * 0.72;
    let page = Rect::from_center_size(rect.center(), vec2(w, s * 0.92));
    let fold = w * 0.32;
    let pts =
        vec![page.left_top(), pos2(page.right() - fold, page.top()), pos2(page.right(), page.top() + fold), page.right_bottom(), page.left_bottom()];
    p.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
    let corner = vec![pos2(page.right() - fold, page.top()), pos2(page.right(), page.top() + fold), pos2(page.right() - fold, page.top() + fold)];
    p.add(egui::Shape::convex_polygon(corner, Color32::from_black_alpha(70), Stroke::NONE));
}

/// An app's chip: a small square in its colour with its initial, then its name in its ink.
pub fn app_chip(ui: &mut Ui, app: &App, size: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(app.name.to_owned(), FontId::proportional(13.0), super::apps::ink(app, t.dark));
    let (rect, resp) = ui.allocate_exact_size(vec2(size + 8.0 + galley.size().x, size.max(galley.size().y)), Sense::hover());
    let square = Rect::from_min_size(pos2(rect.left(), rect.center().y - size / 2.0), vec2(size, size));
    paint_app_square(ui, square, app);
    ui.painter().galley(pos2(square.right() + 8.0, rect.center().y - galley.size().y / 2.0), galley, super::apps::ink(app, t.dark));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, app.name));
    resp
}

/// The app's square mark: its colour, its initial in white.
pub fn paint_app_square(ui: &Ui, square: Rect, app: &App) {
    ui.painter().rect_filled(square, CornerRadius::same((square.width() * 0.22) as u8), super::apps::color(app));
    let letter = app.name.chars().next().map(String::from).unwrap_or_default();
    ui.painter().text(square.center(), Align2::CENTER_CENTER, letter, FontId::new(square.width() * 0.62, crate::theme::display()), Color32::WHITE);
}

/// Drawn glyphs for icon buttons and the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    Refresh,
    List,
    Grid,
    Up,
    Plus,
    Folder,
    Clock,
    Star,
    Link,
    Trash,
    Search,
    Close,
    More,
    /// An arrow down onto a line (DOWNLOAD).
    Down,
    /// Two overlapping sheets (COPY).
    Copy,
    Check,
    /// An eye (SHOW), and one struck through (HIDE).
    Eye,
    EyeOff,
    Lock,
    Calendar,
    Prev,
    Next,
}

pub fn paint_glyph(ui: &Ui, glyph: Glyph, c: Pos2, color: Color32) {
    let p = ui.painter();
    let s = Stroke::new(1.5, color);
    match glyph {
        Glyph::Refresh => {
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = std::f32::consts::PI * (0.25 + 1.6 * i as f32 / 20.0);
                    c + vec2(a.cos(), a.sin()) * 6.0
                })
                .collect();
            let end = *pts.last().unwrap_or(&c);
            p.add(egui::Shape::line(pts, s));
            p.line_segment([end, end + vec2(0.5, -4.0)], s);
            p.line_segment([end, end + vec2(4.0, 0.5)], s);
        }
        Glyph::List => {
            for dy in [-4.0, 0.0, 4.0] {
                p.line_segment([c + vec2(-6.0, dy), c + vec2(6.0, dy)], s);
            }
        }
        Glyph::Grid => {
            for (dx, dy) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                p.rect_filled(Rect::from_center_size(c + vec2(dx, dy), vec2(5.0, 5.0)), 0.0, color);
            }
        }
        Glyph::Up => {
            p.line_segment([c + vec2(0.0, 6.0), c + vec2(0.0, -6.0)], s);
            p.line_segment([c + vec2(-4.5, -1.5), c + vec2(0.0, -6.0)], s);
            p.line_segment([c + vec2(4.5, -1.5), c + vec2(0.0, -6.0)], s);
        }
        Glyph::Plus => {
            p.line_segment([c + vec2(-6.0, 0.0), c + vec2(6.0, 0.0)], s);
            p.line_segment([c + vec2(0.0, -6.0), c + vec2(0.0, 6.0)], s);
        }
        Glyph::Folder => {
            p.rect_stroke(Rect::from_min_max(c + vec2(-7.0, -3.5), c + vec2(7.0, 5.5)), 0.0, s, egui::StrokeKind::Middle);
            p.line_segment([c + vec2(-7.0, -3.5), c + vec2(-7.0, -5.5)], s);
            p.line_segment([c + vec2(-7.0, -5.5), c + vec2(-1.5, -5.5)], s);
            p.line_segment([c + vec2(-1.5, -5.5), c + vec2(0.0, -3.5)], s);
        }
        Glyph::Clock => {
            p.circle_stroke(c, 6.5, s);
            p.line_segment([c, c + vec2(0.0, -4.0)], s);
            p.line_segment([c, c + vec2(3.0, 1.5)], s);
        }
        Glyph::Star => star(ui, c, 7.0, None, Some(s)),
        Glyph::Link => {
            p.rect_stroke(Rect::from_center_size(c + vec2(-2.5, 2.5), vec2(7.0, 5.0)), 2.0, s, egui::StrokeKind::Middle);
            p.rect_stroke(Rect::from_center_size(c + vec2(2.5, -2.5), vec2(7.0, 5.0)), 2.0, s, egui::StrokeKind::Middle);
        }
        Glyph::Trash => {
            p.line_segment([c + vec2(-6.5, -4.0), c + vec2(6.5, -4.0)], s);
            p.line_segment([c + vec2(-2.0, -6.0), c + vec2(2.0, -6.0)], s);
            p.rect_stroke(Rect::from_min_max(c + vec2(-4.5, -4.0), c + vec2(4.5, 6.5)), 0.0, s, egui::StrokeKind::Middle);
        }
        Glyph::Search => {
            p.circle_stroke(c + vec2(-1.5, -1.5), 4.5, s);
            p.line_segment([c + vec2(1.8, 1.8), c + vec2(6.0, 6.0)], s);
        }
        Glyph::Close => {
            p.line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], s);
            p.line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], s);
        }
        Glyph::Down => {
            p.line_segment([c + vec2(0.0, -6.0), c + vec2(0.0, 3.0)], s);
            p.line_segment([c + vec2(-4.0, -1.0), c + vec2(0.0, 3.0)], s);
            p.line_segment([c + vec2(4.0, -1.0), c + vec2(0.0, 3.0)], s);
            p.line_segment([c + vec2(-6.0, 6.0), c + vec2(6.0, 6.0)], s);
        }
        Glyph::More => {
            for dx in [-5.0, 0.0, 5.0] {
                p.circle_filled(c + vec2(dx, 0.0), 1.4, color);
            }
        }
        Glyph::Copy => {
            p.rect_stroke(Rect::from_min_max(c + vec2(-2.5, -2.5), c + vec2(6.0, 6.0)), 1.0, s, egui::StrokeKind::Middle);
            p.add(egui::Shape::line(vec![c + vec2(2.5, -5.5), c + vec2(-5.5, -5.5), c + vec2(-5.5, 2.5)], s));
        }
        Glyph::Check => {
            p.line_segment([c + vec2(-5.5, 0.0), c + vec2(-1.5, 4.0)], s);
            p.line_segment([c + vec2(-1.5, 4.0), c + vec2(6.0, -4.5)], s);
        }
        Glyph::Eye | Glyph::EyeOff => {
            let lid = |sign: f32| -> Vec<Pos2> {
                (0..=12)
                    .map(|i| {
                        let x = -7.0 + 14.0 * i as f32 / 12.0;
                        c + vec2(x, sign * 4.5 * (1.0 - (x / 7.0).powi(2)))
                    })
                    .collect()
            };
            p.add(egui::Shape::line(lid(-1.0), s));
            p.add(egui::Shape::line(lid(1.0), s));
            p.circle_stroke(c, 2.0, s);
            if glyph == Glyph::EyeOff {
                p.line_segment([c + vec2(-6.0, 6.0), c + vec2(6.0, -6.0)], s);
            }
        }
        Glyph::Lock => {
            p.rect_filled(Rect::from_min_max(c + vec2(-5.0, -1.0), c + vec2(5.0, 6.0)), 1.0, color);
            let arc: Vec<Pos2> = (0..=10)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 10.0);
                    c + vec2(a.cos() * 3.2, -1.0 + a.sin() * 4.0)
                })
                .collect();
            p.add(egui::Shape::line(arc, s));
        }
        Glyph::Calendar => {
            p.rect_stroke(Rect::from_min_max(c + vec2(-6.0, -4.5), c + vec2(6.0, 6.0)), 1.0, s, egui::StrokeKind::Middle);
            p.line_segment([c + vec2(-6.0, -1.0), c + vec2(6.0, -1.0)], s);
            p.line_segment([c + vec2(-3.0, -6.5), c + vec2(-3.0, -3.0)], s);
            p.line_segment([c + vec2(3.0, -6.5), c + vec2(3.0, -3.0)], s);
        }
        Glyph::Prev => {
            p.add(egui::Shape::line(vec![c + vec2(2.0, -4.5), c + vec2(-2.5, 0.0), c + vec2(2.0, 4.5)], s));
        }
        Glyph::Next => {
            p.add(egui::Shape::line(vec![c + vec2(-2.0, -4.5), c + vec2(2.5, 0.0), c + vec2(-2.0, 4.5)], s));
        }
    }
}

/// A five-pointed star of radius `r`.
pub fn star(ui: &Ui, c: Pos2, r: f32, fill: Option<Color32>, stroke: Option<Stroke>) {
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rr = if i % 2 == 0 { r } else { r * 0.45 };
            c + vec2(a.cos(), a.sin()) * rr
        })
        .collect();
    if let Some(fill) = fill {
        // Not convex: fan from the centre.
        for i in 0..10 {
            ui.painter().add(egui::Shape::convex_polygon(vec![c, pts[i], pts[(i + 1) % 10]], fill, Stroke::NONE));
        }
    }
    if let Some(s) = stroke {
        let mut closed = pts.clone();
        closed.push(pts[0]);
        ui.painter().add(egui::Shape::line(closed, s));
    }
}

/// A 34×34 square button with a glyph, in a hairline box; `on` draws it inverted.
pub fn icon_button(ui: &mut Ui, glyph: Glyph, label: &str, on: bool, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(chrome::CONTROL_H, chrome::CONTROL_H), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = resp.hovered() && enabled;
    let (fill, color) = if on {
        (t.invert_bg, t.invert_fg)
    } else if hovered {
        (t.sunken, t.ink_strong)
    } else {
        (t.bg, t.ink)
    };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
    ui.painter().rect(rect, 0.0, fill, Stroke::new(1.0, fade(t.line_strong)), egui::StrokeKind::Inside);
    paint_glyph(ui, glyph, rect.center(), fade(color));
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, label));
    let resp = resp.on_hover_text(label);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A sidebar entry: 36 tall, glyph and HUD label; the current one inverted.
pub fn side_item(ui: &mut Ui, glyph: Glyph, label: &str, selected: bool, count: Option<usize>) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let hovered = resp.hovered() && !selected;
    let (fill, color) = if selected {
        (t.invert_bg, t.invert_fg)
    } else if hovered {
        (t.sunken, t.ink_strong)
    } else {
        (Color32::TRANSPARENT, t.ink)
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    paint_glyph(ui, glyph, pos2(rect.left() + 20.0, rect.center().y), color);
    let g = caps(ui, label, 11.0, color);
    ui.painter().galley(pos2(rect.left() + 40.0, rect.center().y - g.size().y / 2.0), g, color);
    if let Some(n) = count.filter(|n| *n > 0) {
        let g = caps(ui, &n.to_string(), 10.0, if selected { color } else { t.muted });
        ui.painter().galley(pos2(rect.right() - 12.0 - g.size().x, rect.center().y - g.size().y / 2.0), g, color);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label.to_uppercase()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Breadcrumb cells `FILES › DOCUMENTS › REPORTS`; the last one is the current folder. Returns the
/// clicked index. When they don't fit, the first ones collapse to "…".
pub fn crumbs(ui: &mut Ui, names: &[String], max_width: f32) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let widths: Vec<f32> = names.iter().map(|n| caps(ui, n, 11.0, t.ink).size().x.min(180.0) + 28.0).collect();
    let mut first = 0;
    while first + 1 < names.len() && widths[first..].iter().sum::<f32>() + if first > 0 { 40.0 } else { 0.0 } > max_width {
        first += 1;
    }
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut shown: Vec<(usize, String)> = Vec::new();
        if first > 0 {
            shown.push((first - 1, "…".into()));
        }
        shown.extend(names.iter().enumerate().skip(first).map(|(i, n)| (i, n.clone())));
        for (i, name) in shown {
            let last = i + 1 == names.len() && name != "…";
            let color = if last { t.invert_fg } else { t.muted };
            // "My files" and the view names are HUD labels; a folder's own name keeps its case
            // ("Notat é", not "NOTAT É").
            let own = i > 0 && name != "…";
            let text =
                |s: &str| if own { ui.painter().layout_no_wrap(s.to_owned(), FontId::proportional(13.0), color) } else { caps(ui, s, 11.0, color) };
            let mut g = text(&name);
            if g.size().x > 180.0 {
                let mut short: String = name.chars().take(18).collect();
                short.push('…');
                g = text(&short);
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 28.0, chrome::CONTROL_H), if last { Sense::hover() } else { Sense::click() });
            let hovered = resp.hovered() && !last;
            let fill = if last {
                t.invert_bg
            } else if hovered {
                t.sunken
            } else {
                t.bg
            };
            ui.painter().rect(rect, 0.0, fill, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
            ui.painter().galley_with_override_text_color(rect.center() - g.size() / 2.0, g, if hovered { t.ink_strong } else { color });
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, format!("{} folder", name.to_uppercase())));
            if !last && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// Column headings of the table; returns the clicked column index.
/// The least clear space after a column header's label (and its sort arrow).
pub const HEADER_GAP: f32 = 16.0;

pub fn table_header(ui: &mut Ui, rect: Rect, cols: &[(f32, f32, &str, Option<bool>)]) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, t.hairline());
    let mut clicked = None;
    for (i, (x, w, label, sorted)) in cols.iter().enumerate() {
        if label.is_empty() {
            continue;
        }
        let cell = Rect::from_min_size(pos2(*x, rect.top()), vec2(*w, rect.height()));
        let resp = ui.interact(cell, ui.scope_id().with(("col", i)), Sense::click());
        let color = if sorted.is_some() || resp.hovered() { t.ink_strong } else { t.muted };
        // A label never runs into the next column: it keeps HEADER_GAP clear (and room for the
        // sort arrow), and is cut with "…" when the column is too narrow for it.
        let room = (*w - HEADER_GAP - if sorted.is_some() { 18.0 } else { 0.0 }).max(12.0);
        let mut job = egui::text::LayoutJob::single_section(
            label.to_uppercase(),
            egui::TextFormat { font_id: FontId::monospace(11.0), color, extra_letter_spacing: 11.0 * 0.15, ..Default::default() },
        );
        job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let g = ui.painter().layout_job(job);
        let pos = pos2(cell.left(), cell.center().y - g.size().y / 2.0);
        let right = pos.x + g.size().x;
        ui.painter().galley(pos, g, color);
        if let Some(asc) = sorted {
            let c = pos2(right + 9.0, cell.center().y);
            let (a, b) = if *asc { (-3.0, 2.0) } else { (3.0, -2.0) };
            ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-4.0, b), c + vec2(4.0, b), c + vec2(0.0, a)], color, Stroke::NONE));
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColumnHeader, true, format!("Sort by {label}")));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A row's background: sunken on hover, an accent wash and rule when selected; the hairline under it.
pub fn row_background(ui: &Ui, rect: Rect, hovered: bool, selected: bool) {
    let t = Tokens::get(ui.ctx());
    if selected {
        ui.painter().rect_filled(rect, 0.0, tint(t.accent, if t.dark { 0.16 } else { 0.12 }));
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(2.0, rect.height())), 0.0, t.accent);
    } else if hovered {
        ui.painter().rect_filled(rect, 0.0, t.sunken);
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, t.hairline());
}

/// A 16 px check square, ticked or not.
pub fn check(ui: &Ui, c: Pos2, on: bool, strong: bool) {
    let t = Tokens::get(ui.ctx());
    let r = Rect::from_center_size(c, vec2(16.0, 16.0));
    if on {
        ui.painter().rect_filled(r, 0.0, t.accent);
        let s = Stroke::new(1.8, Color32::WHITE);
        ui.painter().line_segment([c + vec2(-4.0, 0.0), c + vec2(-1.0, 3.5)], s);
        ui.painter().line_segment([c + vec2(-1.0, 3.5), c + vec2(4.5, -3.5)], s);
    } else {
        ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, if strong { t.line_strong } else { t.line }), egui::StrokeKind::Inside);
    }
}

/// Text cut to `width` with "…".
pub fn fit_text(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

/// A menu entry: `(label, danger, enabled)`, or `None` for a rule.
pub type MenuEntry<'a> = Option<(&'a str, bool, bool)>;

/// The floating menu's frame and entries. Returns the chosen index, and whether the menu should
/// close.
pub fn menu(ui: &mut Ui, at: Pos2, entries: &[MenuEntry<'_>]) -> (Option<usize>, bool) {
    let t = Tokens::get(ui.ctx());
    let mut chosen = None;
    let screen = ui.ctx().content_rect();
    let width = 220.0;
    let height: f32 = entries.iter().map(|e| if e.is_some() { 32.0 } else { 9.0 }).sum::<f32>() + 8.0;
    let x = at.x.min(screen.right() - width - 8.0).max(screen.left() + 8.0);
    let y = if at.y + height > screen.bottom() - 8.0 { (at.y - height).max(screen.top() + 8.0) } else { at.y };
    let area = egui::Area::new(ui.scope_id().with("files-menu")).order(egui::Order::Foreground).fixed_pos(pos2(x, y)).show(ui.ctx(), |ui| {
        egui::Frame::new()
            .fill(t.bg)
            .stroke(Stroke::new(1.0, t.line))
            .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 140 } else { 36 }) })
            .inner_margin(egui::Margin::symmetric(0, 4))
            .show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, entry) in entries.iter().enumerate() {
                    let Some((label, danger, enabled)) = entry else {
                        let (r, _) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
                        ui.painter().hline(r.x_range(), r.center().y, t.hairline());
                        continue;
                    };
                    let (r, resp) = ui.allocate_exact_size(vec2(width, 32.0), if *enabled { Sense::click() } else { Sense::hover() });
                    if resp.hovered() && *enabled {
                        ui.painter().rect_filled(r, 0.0, t.sunken);
                    }
                    let color = if !*enabled {
                        t.muted.gamma_multiply(0.6)
                    } else if *danger {
                        t.bad
                    } else {
                        t.ink
                    };
                    ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, *label, FontId::proportional(13.0), color);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, *enabled, *label));
                    if resp.clicked() {
                        chosen = Some(i);
                    }
                }
            });
    });
    let outside = ui.input(|i| i.pointer.any_pressed())
        && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p))
        && chosen.is_none();
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    (chosen, chosen.is_some() || outside || escape)
}

/// A strip of joined cells (the toggle-cell look); returns the clicked index.
pub fn cells(ui: &mut Ui, labels: &[&str], current: Option<usize>) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, label) in labels.iter().enumerate() {
            let on = current == Some(i);
            let g = caps(ui, label, 11.0, t.ink);
            let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 24.0, chrome::CONTROL_H), Sense::click());
            let hovered = resp.hovered() && !on;
            let (fill, color) = if on {
                (t.invert_bg, t.invert_fg)
            } else if hovered {
                (t.sunken, t.ink_strong)
            } else {
                (t.bg, t.muted)
            };
            ui.painter().rect(rect, 0.0, fill, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
            ui.painter().galley_with_override_text_color(rect.center() - g.size() / 2.0, g, color);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, label.to_uppercase()));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// A search box: sunken, hairline, a glyph, the text field. Returns the field's response.
pub fn search_box(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, chrome::CONTROL_H), Sense::hover());
    ui.painter().rect(rect, 0.0, t.sunken, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
    paint_glyph(ui, Glyph::Search, pos2(rect.left() + 18.0, rect.center().y), t.muted);
    // The magnifier names the field and focuses it.
    let glyph = ui.interact(Rect::from_min_size(rect.min, vec2(32.0, rect.height())), ui.scope_id().with(("search", hint)), Sense::click());
    glyph.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, hint));
    let inner = Rect::from_min_max(pos2(rect.left() + 32.0, rect.top() + 1.0), pos2(rect.right() - 4.0, rect.bottom() - 1.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let v = child.visuals_mut();
    v.extreme_bg_color = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.selection.stroke = Stroke::NONE;
    let field = child
        .add(
            egui::TextEdit::singleline(text)
                .frame(egui::Frame::NONE)
                .hint_text(egui::RichText::new(hint).color(t.muted))
                .desired_width(inner.width())
                // Centred in the 34 px box, as the admin search (kit::search_box).
                .align(egui::Align2::LEFT_CENTER)
                .min_size(vec2(0.0, inner.height()))
                .font(FontId::proportional(14.0)),
        )
        .labelled_by(glyph.id);
    if glyph.clicked() {
        field.request_focus();
    }
    field
}

/// A small text link in the accent ink (mono caps), e.g. RESTORE.
pub fn text_action(ui: &mut Ui, label: &str, danger: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let color = if danger { t.bad } else { t.accent_ink };
    let g = caps(ui, label, 10.5, color);
    let (rect, resp) = ui.allocate_exact_size(g.size() + vec2(8.0, 10.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, t.sunken);
    }
    ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.to_uppercase()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// How wide [`icon_label_button`] draws `label`.
pub fn icon_label_width(ui: &Ui, label: &str) -> f32 {
    caps(ui, label, 11.0, Color32::WHITE).size().x + 32.0 + 22.0
}

/// A launcher button with a drawn glyph before its label (UPLOAD, NEW FOLDER).
pub fn icon_label_button(ui: &mut Ui, glyph: Glyph, label: &str, kind: chrome::Kind, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let g = caps(ui, label, 11.0, t.ink);
    let (rect, resp) =
        ui.allocate_exact_size(vec2(g.size().x + 32.0 + 22.0, chrome::CONTROL_H), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = resp.hovered() && enabled;
    let (fill, stroke, color) = match kind {
        chrome::Kind::Primary => (t.invert_bg.gamma_multiply(if hovered { 0.8 } else { 1.0 }), Stroke::NONE, t.invert_fg),
        chrome::Kind::Danger => (t.bad, Stroke::NONE, Color32::WHITE),
        chrome::Kind::Secondary if hovered => (t.invert_bg, Stroke::NONE, t.invert_fg),
        chrome::Kind::Secondary => (t.bg, Stroke::new(1.0, t.line_strong), t.ink),
    };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
    ui.painter().rect(rect, 0.0, fade(fill), Stroke::new(stroke.width, fade(stroke.color)), egui::StrokeKind::Inside);
    paint_glyph(ui, glyph, pos2(rect.left() + 16.0 + 7.0, rect.center().y), fade(color));
    ui.painter().galley_with_override_text_color(pos2(rect.left() + 16.0 + 22.0, rect.center().y - g.size().y / 2.0), g, fade(color));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label.to_uppercase()));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}
