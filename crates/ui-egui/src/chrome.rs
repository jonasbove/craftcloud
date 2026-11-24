//! The launcher's frame and parts, as the v2 design draws them (`design/README.md`, "egui
//! implementation notes"): the 48 px header of hairline-split cells, the 34 px footer, sheets with
//! a tinted hero under a 2 px accent rule, square mono buttons, HUD labels, fields and panels.
//!
//! Modelled on craft-launcher's `crates/ui-egui/src/{widgets,sheet,views}.rs` (MIT OR Apache-2.0,
//! Copyright (c) 2026 ArtCraft Team). Every colour comes from [`Tokens`].

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Color32, CornerRadius, FontId, Galley, Margin, Rect, Response, Sense, Stroke, TextFormat, Ui, UiBuilder, pos2, vec2};

use crate::theme::{Tokens, display};
use crate::widgets::logo;

/// The desktop client's parts: its header, activity list, folder table and state panels.
pub mod desk;

pub const PAD: f32 = 24.0;
pub const HEADER_H: f32 = 48.0;
pub const FOOTER_H: f32 = 34.0;
pub const CONTROL_H: f32 = 34.0;
const HUD_SIZE: f32 = 11.0;
const HUD_SPACING: f32 = HUD_SIZE * 0.15;
const BUTTON_SPACING: f32 = HUD_SIZE * 0.12;

/// `c` at `alpha` (0–1).
pub fn tint(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn shadow(t: &Tokens) -> egui::epaint::Shadow {
    egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 140 } else { 36 }) }
}

/// `text` upper case in `family`, `spacing` points between letters.
pub(crate) fn caps(ui: &Ui, text: &str, size: f32, family: egui::FontFamily, spacing: f32, color: Color32) -> Arc<Galley> {
    let format = TextFormat { font_id: FontId::new(size, family), color, extra_letter_spacing: spacing, ..Default::default() };
    ui.painter().layout_job(LayoutJob::single_section(text.to_uppercase(), format))
}

pub(crate) fn hud_galley(ui: &Ui, text: &str, color: Color32) -> Arc<Galley> {
    caps(ui, text, HUD_SIZE, egui::FontFamily::Monospace, HUD_SPACING, color)
}

/// A HUD label: 11 px Geist Mono, upper case, widely spaced.
pub fn hud(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = hud_galley(ui, text, color);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text.to_uppercase()));
    resp
}

/// A meta line: HUD parts split by "·".
pub fn meta(ui: &mut Ui, parts: &[&str]) {
    let t = Tokens::get(ui.ctx());
    meta_row(ui, |ui| {
        for (i, part) in parts.iter().enumerate() {
            // The "·" goes with the part after it, so a wrapped line never ends in one. The pair
            // asks for its measured width, so the row wraps before it rather than overflowing.
            let mut w = hud_galley(ui, part, t.muted).size().x;
            if i > 0 {
                w += hud_galley(ui, "·", t.muted).size().x + ui.spacing().item_spacing.x;
            }
            ui.allocate_ui_with_layout(vec2(w, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if i > 0 {
                    hud(ui, "·", t.muted);
                }
                hud(ui, part, t.muted);
            });
        }
    });
}

/// A wrapping row of HUD parts 10 apart, as tall as the text (not a control's 34): the meta
/// line sits on the design's spacing.
pub fn meta_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 16.0;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
            add(ui)
        })
        .inner
    })
    .inner
}

/// A display headline in lines; each line is pieces, the `true` ones in the accent ink.
pub fn headline(ui: &mut Ui, size: f32, lines: &[&[(&str, bool)]]) {
    let t = Tokens::get(ui.ctx());
    headline_toned(ui, size, lines, t.accent_ink);
}

/// [`headline`] with the marked words in `tone` instead of `accent_ink` (an error state's
/// headline takes `bad`, as its badge does).
pub fn headline_toned(ui: &mut Ui, size: f32, lines: &[&[(&str, bool)]], tone: Color32) {
    let t = Tokens::get(ui.ctx());
    // The lines sit on the design's line height (56/60, 40/44), with no item gap between them.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        headline_lines(ui, size, lines, tone, &t);
    });
}

fn headline_lines(ui: &mut Ui, size: f32, lines: &[&[(&str, bool)]], tone: Color32, t: &Tokens) {
    for line in lines {
        let mut job = LayoutJob::default();
        for (text, accent) in *line {
            let format = TextFormat {
                font_id: FontId::new(size, display()),
                color: if *accent { tone } else { t.ink_strong },
                extra_letter_spacing: -0.02 * size,
                line_height: Some(size * 1.07),
                ..Default::default()
            };
            job.append(text, 0.0, format);
        }
        ui.add(egui::Label::new(job).selectable(false));
    }
}

/// Body text in `size`, muted.
pub fn body(ui: &mut Ui, text: &str, size: f32) {
    let t = Tokens::get(ui.ctx());
    // The design's line heights: 18/28 for a hero paragraph, about 1.43 for the smaller sizes.
    let line = if size >= 18.0 { 28.0 } else { (size * 1.43).round() };
    ui.add(egui::Label::new(egui::RichText::new(text).size(size).line_height(Some(line)).color(t.muted)).wrap());
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Arrow,
    Check,
    Plus,
    /// Two arrows chasing each other (sync, retry).
    Refresh,
    Pause,
    Play,
    Folder,
    /// An arrow out of the corner (opens in the browser).
    External,
    /// An arrow out of a door.
    SignOut,
    Download,
    Cross,
    /// Nine squares (the Apps menu).
    Grid,
}

fn paint_icon(ui: &Ui, icon: Icon, centre: egui::Pos2, color: Color32) {
    let s = Stroke::new(1.5, color);
    let p = ui.painter();
    match icon {
        Icon::Arrow => {
            p.line_segment([centre + vec2(-6.0, 0.0), centre + vec2(6.0, 0.0)], s);
            p.line_segment([centre + vec2(1.5, -4.5), centre + vec2(6.0, 0.0)], s);
            p.line_segment([centre + vec2(1.5, 4.5), centre + vec2(6.0, 0.0)], s);
        }
        Icon::Check => {
            p.line_segment([centre + vec2(-5.5, 0.0), centre + vec2(-1.5, 4.0)], s);
            p.line_segment([centre + vec2(-1.5, 4.0), centre + vec2(6.0, -4.5)], s);
        }
        Icon::Plus => {
            p.line_segment([centre + vec2(-5.5, 0.0), centre + vec2(5.5, 0.0)], s);
            p.line_segment([centre + vec2(0.0, -5.5), centre + vec2(0.0, 5.5)], s);
        }
        Icon::Refresh => {
            let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
                (0..=8).map(|i| from + (to - from) * i as f32 / 8.0).map(|a| centre + 5.5 * vec2(a.cos(), a.sin())).collect()
            };
            use std::f32::consts::PI;
            p.add(egui::Shape::line(arc(-PI * 0.95, -PI * 0.15), s));
            p.add(egui::Shape::line(arc(PI * 0.05, PI * 0.85), s));
            let (a, b) = (centre + 5.5 * vec2((-PI * 0.15).cos(), (-PI * 0.15).sin()), centre + 5.5 * vec2((PI * 0.85).cos(), (PI * 0.85).sin()));
            p.line_segment([a, a + vec2(-3.0, -0.5)], s);
            p.line_segment([a, a + vec2(0.5, -3.0)], s);
            p.line_segment([b, b + vec2(3.0, 0.5)], s);
            p.line_segment([b, b + vec2(-0.5, 3.0)], s);
        }
        Icon::Pause => {
            p.line_segment([centre + vec2(-2.5, -5.0), centre + vec2(-2.5, 5.0)], s);
            p.line_segment([centre + vec2(2.5, -5.0), centre + vec2(2.5, 5.0)], s);
        }
        Icon::Play => {
            let pts = vec![centre + vec2(-3.5, -5.5), centre + vec2(5.0, 0.0), centre + vec2(-3.5, 5.5)];
            p.add(egui::Shape::closed_line(pts, s));
        }
        Icon::Folder => {
            let pts = vec![
                centre + vec2(-6.5, -4.5),
                centre + vec2(-2.0, -4.5),
                centre + vec2(-0.5, -3.0),
                centre + vec2(6.5, -3.0),
                centre + vec2(6.5, 5.0),
                centre + vec2(-6.5, 5.0),
            ];
            p.add(egui::Shape::closed_line(pts, s));
        }
        Icon::External => {
            p.line_segment([centre + vec2(-5.0, 5.0), centre + vec2(5.0, -5.0)], s);
            p.line_segment([centre + vec2(-1.0, -5.0), centre + vec2(5.0, -5.0)], s);
            p.line_segment([centre + vec2(5.0, -5.0), centre + vec2(5.0, 1.0)], s);
        }
        Icon::SignOut => {
            p.add(egui::Shape::line(vec![centre + vec2(0.0, -6.0), centre + vec2(-6.0, -6.0), centre + vec2(-6.0, 6.0), centre + vec2(0.0, 6.0)], s));
            p.line_segment([centre + vec2(-2.0, 0.0), centre + vec2(6.5, 0.0)], s);
            p.line_segment([centre + vec2(3.5, -3.0), centre + vec2(6.5, 0.0)], s);
            p.line_segment([centre + vec2(3.5, 3.0), centre + vec2(6.5, 0.0)], s);
        }
        Icon::Download => {
            p.line_segment([centre + vec2(0.0, -6.0), centre + vec2(0.0, 3.0)], s);
            p.line_segment([centre + vec2(-3.5, -0.5), centre + vec2(0.0, 3.0)], s);
            p.line_segment([centre + vec2(3.5, -0.5), centre + vec2(0.0, 3.0)], s);
            p.line_segment([centre + vec2(-6.0, 6.0), centre + vec2(6.0, 6.0)], s);
        }
        Icon::Cross => {
            p.line_segment([centre + vec2(-4.5, -4.5), centre + vec2(4.5, 4.5)], s);
            p.line_segment([centre + vec2(-4.5, 4.5), centre + vec2(4.5, -4.5)], s);
        }
        Icon::Grid => {
            for dy in [-4.25, 0.0, 4.25] {
                for dx in [-4.25, 0.0, 4.25] {
                    p.rect_filled(Rect::from_center_size(centre + vec2(dx, dy), vec2(2.5, 2.5)), 0.0, color);
                }
            }
        }
    }
}

/// Draws `icon` 14 px square around `centre` (for parts outside buttons).
pub fn icon(ui: &Ui, icon: Icon, centre: egui::Pos2, color: Color32) {
    paint_icon(ui, icon, centre, color);
}

/// A square button: 34 tall, mono caps, an optional drawn icon before the label.
pub fn button(ui: &mut Ui, text: &str, kind: Kind, icon: Option<Icon>, enabled: bool) -> Response {
    button_sized(ui, text, kind, icon, enabled, None)
}

/// A [`button`] `width` points wide (at least its content), with the icon and label centred: the
/// sign-in sheet's "Sign in with …", which spans the form.
pub fn wide_button(ui: &mut Ui, text: &str, kind: Kind, icon: Option<Icon>, enabled: bool, width: f32) -> Response {
    button_sized(ui, text, kind, icon, enabled, Some(width))
}

fn button_sized(ui: &mut Ui, text: &str, kind: Kind, icon: Option<Icon>, enabled: bool, width: Option<f32>) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = caps(ui, text, HUD_SIZE, egui::FontFamily::Monospace, BUTTON_SPACING, t.ink);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let content = galley.size().x + 32.0 + icon_w;
    let size = vec2(width.map_or(content, |w| w.max(content)), CONTROL_H);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hovered = resp.hovered() && enabled;
    let (fill, stroke, color) = match kind {
        Kind::Primary => (t.invert_bg.gamma_multiply(if hovered { 0.8 } else { 1.0 }), Stroke::NONE, t.invert_fg),
        Kind::Danger => (t.bad.gamma_multiply(if hovered { 0.8 } else { 1.0 }), Stroke::NONE, if t.dark { t.invert_fg } else { Color32::WHITE }),
        Kind::Secondary if hovered => (t.invert_bg, Stroke::NONE, t.invert_fg),
        Kind::Secondary => (t.bg, Stroke::new(1.0, t.line_strong), t.ink),
    };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
    ui.painter().rect(rect, CornerRadius::ZERO, fade(fill), Stroke::new(stroke.width, fade(stroke.color)), egui::StrokeKind::Inside);
    // The content's left edge: 16 in from the left, or centred in a wider button.
    let left = rect.left() + ((rect.width() - content) / 2.0).max(0.0);
    let text_left = left + 16.0 + icon_w;
    if let Some(icon) = icon {
        paint_icon(ui, icon, pos2(left + 16.0 + 7.0, rect.center().y), fade(color));
    }
    ui.painter().galley_with_override_text_color(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, fade(color));
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 0.0, Stroke::new(2.0, t.accent_ink), egui::StrokeKind::Outside);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text.to_uppercase()));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A labelled field: HUD label, a 34 px sunken box, an optional hint under it.
pub fn field(ui: &mut Ui, label: &str, value: &mut String, password: bool, hint: Option<&str>) -> Response {
    let t = Tokens::get(ui.ctx());
    let label = hud(ui, label, t.ink);
    ui.add_space(2.0);
    let resp = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.extreme_bg_color = t.sunken;
            v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.line);
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.line_strong);
            v.selection.stroke = Stroke::new(2.0, t.accent_ink);
            ui.add(
                egui::TextEdit::singleline(value)
                    .password(password)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(10, 8))
                    .min_size(vec2(0.0, CONTROL_H))
                    .font(FontId::proportional(14.0)),
            )
        })
        .inner
        .labelled_by(label.id);
    if let Some(hint) = hint {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(hint).size(12.0).color(t.muted));
    }
    resp
}

/// A hairline box with a HUD title (the launcher's panel).
pub fn panel(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().stroke(t.hairline()).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        hud(ui, title, t.muted);
        ui.add_space(8.0);
        add(ui);
    });
}

/// Rows of HUD key and mono value, in a [`panel`].
pub fn facts(ui: &mut Ui, title: &str, rows: &[(&str, String)]) {
    let t = Tokens::get(ui.ctx());
    // As the design's device facts: HUD keys in a 110 px column, values in Body 14 that wrap
    // (a long address stays inside the panel).
    panel(ui, title, |ui| {
        ui.spacing_mut().interact_size.y = 20.0;
        ui.spacing_mut().item_spacing.y = 8.0;
        for (key, value) in rows {
            ui.horizontal_top(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(110.0, 18.0), Sense::hover());
                let g = hud_galley(ui, key, t.muted);
                ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.muted);
                ui.add(egui::Label::new(egui::RichText::new(value).size(14.0).color(t.ink_strong)).wrap());
            });
        }
    });
}

/// A 4 px meter, `fraction` filled in the accent.
pub fn meter(ui: &mut Ui, fraction: f32) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.line);
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * fraction.clamp(0.0, 1.0), 4.0));
    ui.painter().rect_filled(fill, 0.0, t.accent);
}

/// A small status badge (20 tall, sunken, a hairline, text in its tone).
pub fn badge(ui: &mut Ui, text: &str, tone: Color32) {
    let t = Tokens::get(ui.ctx());
    let g = caps(ui, text, 10.0, egui::FontFamily::Monospace, 1.0, tone);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 12.0, 20.0), Sense::hover());
    ui.painter().rect(rect, 0.0, t.sunken, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, tone);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text.to_uppercase()));
}

/// The setup steps as joined cells: done ones in the accent ink, the current one inverted and named.
pub fn step_cells(ui: &mut Ui, names: &[&str], current: usize) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.x = 0.0;
    // Laid out right to left by the caller: draw the last step first.
    for i in (0..names.len()).rev() {
        let text = if i == current { format!("{:02} {}", i + 1, names[i]) } else { format!("{:02}", i + 1) };
        let (fill, color) = if i == current {
            (t.invert_bg, t.invert_fg)
        } else if i < current {
            (Color32::TRANSPARENT, t.accent_ink)
        } else {
            (Color32::TRANSPARENT, t.muted)
        };
        let g = hud_galley(ui, &text, color);
        let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 24.0, 34.0), Sense::hover());
        ui.painter().rect(rect, 0.0, fill, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
        ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
    }
}

/// What the header shows.
pub struct HeaderInfo<'a> {
    /// `(label, selected, enabled, tag)`; empty when nobody is signed in.
    pub nav: &'a [(&'a str, bool, bool, Option<&'a str>)],
    /// The signed-in person's name.
    pub user: Option<&'a str>,
    /// The person's cell is drawn sunken (their account menu is open, or one of its pages).
    pub user_selected: bool,
    /// The nav entry that opens a menu rather than a page (APPS): drawn with the app-grid glyph,
    /// as a button that says whether its menu is open.
    pub menu: Option<usize>,
    /// That menu is open (the entry is drawn sunken).
    pub menu_open: bool,
}

/// What was clicked in the header.
#[derive(Default)]
pub struct HeaderClicks {
    pub nav: Option<usize>,
    pub sign_out: bool,
    /// The person's cell (name and initials) was clicked: it opens the account menu.
    pub user: bool,
    /// Where the person's cell is, for the menu to hang under.
    pub user_rect: Option<Rect>,
    /// Where the menu entry ([`HeaderInfo::menu`]) is, and its id (focus goes back to it).
    pub menu: Option<(Rect, egui::Id)>,
}

/// The 48 px header: brand cell, "CRAFT APP SUITE", nav, and the person and SIGN OUT at the right.
pub fn header(ui: &mut Ui, rect: Rect, info: &HeaderInfo<'_>) -> HeaderClicks {
    let t = Tokens::get(ui.ctx());
    let mut clicks = HeaderClicks::default();
    ui.painter().rect_filled(rect, 0.0, t.bg);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, t.hairline());
    let vrule = |ui: &Ui, x: f32| ui.painter().vline(x, rect.y_range(), t.hairline());
    // Narrow windows (tablets, phones) drop the suite cell, then the wordmark, the person's name, the
    // disabled entries and the SIGN OUT words (an icon then), so the header never overflows.
    let w = rect.width();
    let (suite, wordmark, name, compact) = (w >= 900.0, w >= 560.0, w >= 700.0, w < 520.0);
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.set_clip_rect(rect);
        ui.spacing_mut().item_spacing.x = 0.0;
        // Brand cell.
        ui.add_space(if compact { 14.0 } else { 20.0 });
        logo(ui, 24.0);
        if wordmark {
            ui.add_space(10.0);
            wordmark_text(ui, 18.0);
        }
        ui.add_space(if compact { 14.0 } else { 20.0 });
        vrule(ui, ui.cursor().left());
        if suite {
            ui.add_space(16.0);
            hud(ui, "Craft App Suite", t.muted);
            ui.add_space(16.0);
            vrule(ui, ui.cursor().left());
        }
        if !info.nav.is_empty() {
            ui.add_space(if compact { 6.0 } else { 12.0 });
            for (i, (label, selected, enabled, tag)) in info.nav.iter().enumerate() {
                if compact && !*enabled {
                    continue;
                }
                let resp = if info.menu == Some(i) {
                    let resp = menu_item(ui, label, info.menu_open, compact);
                    clicks.menu = Some((resp.rect, resp.id));
                    resp
                } else {
                    nav_item(ui, label, *selected, *enabled)
                };
                if resp.clicked() {
                    clicks.nav = Some(i);
                }
                if let Some(tag) = tag {
                    ui.add_space(2.0);
                    let g = caps(ui, tag, 10.0, egui::FontFamily::Monospace, 1.0, t.muted);
                    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 8.0, 16.0), Sense::hover());
                    ui.painter().rect_stroke(r, 0.0, t.hairline(), egui::StrokeKind::Inside);
                    ui.painter().galley(r.center() - g.size() / 2.0, g, t.muted);
                }
                ui.add_space(4.0);
            }
            ui.add_space(if compact { 2.0 } else { 8.0 });
            vrule(ui, ui.cursor().left());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(user) = info.user {
                let pad = if compact { 10.0 } else { 16.0 };
                ui.add_space(pad);
                // A phone shows the door icon alone (the words "OUT" read as a typo); same label.
                let out = if compact { header_icon(ui, Icon::SignOut, "Sign out") } else { header_link(ui, "Sign out") };
                if out.clicked() {
                    clicks.sign_out = true;
                }
                ui.add_space(pad);
                vrule(ui, ui.cursor().right());
                // The person's cell: name and initials, a button that opens the account menu.
                let cell_right = ui.cursor().right();
                let background = ui.painter().add(egui::Shape::Noop);
                ui.add_space(pad);
                if name {
                    ui.add(egui::Label::new(egui::RichText::new(user).size(13.0).color(t.ink)).selectable(false));
                    ui.add_space(10.0);
                }
                let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                ui.painter().rect(r, 0.0, t.sunken, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
                let g = caps(ui, &initials(user), 10.0, egui::FontFamily::Monospace, 0.5, t.ink);
                ui.painter().galley(r.center() - g.size() / 2.0, g, t.ink);
                ui.add_space(pad);
                let cell = Rect::from_x_y_ranges(ui.cursor().right()..=cell_right, rect.y_range());
                let resp = ui.interact(cell, ui.scope_id().with("account"), Sense::click());
                if info.user_selected || resp.hovered() {
                    ui.painter().set(background, egui::Shape::rect_filled(cell, 0.0, t.sunken));
                }
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, info.user_selected, format!("Account: {user}")));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    clicks.user = true;
                }
                clicks.user_rect = Some(cell);
                vrule(ui, ui.cursor().right());
            }
        });
    });
    clicks
}

/// "Craft" in strong ink, "Cloud" in the accent ink, in the display face.
pub fn wordmark_text(ui: &mut Ui, size: f32) {
    let t = Tokens::get(ui.ctx());
    let mut job = LayoutJob::default();
    for (text, color) in [("Craft", t.ink_strong), ("Cloud", t.accent_ink)] {
        job.append(text, 0.0, TextFormat { font_id: FontId::new(size, display()), color, extra_letter_spacing: -0.02 * size, ..Default::default() });
    }
    ui.add(egui::Label::new(job).selectable(false));
}

pub(crate) fn initials(name: &str) -> String {
    let mut parts = name.split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-').filter(|p| !p.is_empty());
    let first = parts.next().and_then(|p| p.chars().next());
    let second = parts.next().and_then(|p| p.chars().next());
    match (first, second) {
        (Some(a), Some(b)) => format!("{a}{b}"),
        (Some(_), None) => name.chars().take(2).collect(),
        _ => "?".into(),
    }
    .to_uppercase()
}

fn nav_item(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let color = if selected { t.invert_fg } else { t.muted };
    let g = hud_galley(ui, label, color);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = resp.hovered() && enabled && !selected;
    if selected {
        ui.painter().rect_filled(rect, 0.0, t.invert_bg);
    } else if hovered {
        ui.painter().rect_filled(rect, 0.0, t.sunken);
    }
    let color = if hovered { t.ink_strong } else { color };
    let color = if enabled { color } else { color.gamma_multiply(0.5) };
    ui.painter().galley_with_override_text_color(rect.center() - g.size() / 2.0, g, color);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, enabled, selected, label.to_uppercase()));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// The nav entry that opens a menu (APPS): the app-grid glyph and the label, or the glyph alone on
/// a phone. Sunken while its menu is open. A button that says whether the menu is expanded.
fn menu_item(ui: &mut Ui, label: &str, open: bool, compact: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let g = hud_galley(ui, label, t.muted);
    let width = if compact { 24.0 } else { 8.0 + 14.0 + 6.0 + g.size().x + 8.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let lit = open || resp.hovered();
    if lit {
        ui.painter().rect_filled(rect, 0.0, t.sunken);
    }
    let color = if lit { t.ink_strong } else { t.muted };
    let glyph_x = if compact { rect.center().x } else { rect.left() + 8.0 + 7.0 };
    paint_icon(ui, Icon::Grid, pos2(glyph_x, rect.center().y), color);
    if !compact {
        ui.painter().galley_with_override_text_color(pos2(rect.left() + 8.0 + 14.0 + 6.0, rect.center().y - g.size().y / 2.0), g, color);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 0.0, Stroke::new(2.0, t.accent_ink), egui::StrokeKind::Outside);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.to_uppercase()));
    ui.ctx().accesskit_node_builder(resp.id, |node| node.set_expanded(open));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if compact { resp.on_hover_text(label) } else { resp }
}

/// A header icon cell's content: a 14 px icon in a 24×24 target, muted, strong on hover.
fn header_icon(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let color = if resp.hovered() { t.ink_strong } else { t.muted };
    paint_icon(ui, icon, rect.center(), color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.to_uppercase()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label)
}

fn header_link(ui: &mut Ui, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let g = hud_galley(ui, label, t.muted);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x, 24.0), Sense::click());
    let color = if resp.hovered() { t.ink_strong } else { t.muted };
    ui.painter().galley_with_override_text_color(rect.left_center() - vec2(0.0, g.size().y / 2.0), g, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.to_uppercase()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The 34 px footer: a HUD status on the left, small links on the right.
pub fn footer(ui: &mut Ui, rect: Rect, left: &str, links: &[(&str, &str)]) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.bg);
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, t.hairline());
    let narrow = rect.width() < 760.0;
    let inner = rect.shrink2(vec2(if narrow { 16.0 } else { PAD }, 0.0));
    ui.scope_builder(UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.set_clip_rect(rect);
        if !narrow {
            hud(ui, left, t.muted);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            // Narrow: only the links without an address (the attribution) and the last link.
            let shown: Vec<&(&str, &str)> = if narrow { links.iter().filter(|(_, url)| url.is_empty()).collect() } else { links.iter().collect() };
            for (text, url) in shown.into_iter().rev() {
                if url.is_empty() {
                    ui.label(egui::RichText::new(*text).size(12.0).color(t.muted));
                } else {
                    let link = ui.add(egui::Label::new(egui::RichText::new(*text).size(12.0).color(t.muted)).sense(Sense::click()));
                    if link.clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(*url));
                    }
                    link.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            }
        });
    });
}

/// An address in a sunken hairline box (cut off at its edge; the whole text is its label) with a
/// COPY button that says COPIED once used.
pub fn copy_row(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let id = ui.make_persistent_id(("copied", text));
    let copied = ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w = (ui.available_width() - 110.0).max(120.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(w, CONTROL_H), Sense::hover());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
        ui.painter().rect(rect, 0.0, t.sunken, t.hairline(), egui::StrokeKind::Inside);
        let inner = rect.shrink2(vec2(10.0, 0.0));
        ui.painter().with_clip_rect(inner).text(inner.left_center(), egui::Align2::LEFT_CENTER, text, FontId::proportional(14.0), t.ink);
        if button(ui, if copied { "Copied" } else { "Copy" }, Kind::Secondary, Some(Icon::Check), true).clicked() {
            ui.ctx().copy_text(text.to_owned());
            ui.ctx().data_mut(|d| d.insert_temp(id, true));
        }
    });
}

/// A sheet's hero: logo, title, caption.
pub struct Hero<'a> {
    pub title: &'a str,
    pub caption: &'a str,
    /// The caption is a file or folder name: drawn in its own case (Small 13 muted), not in HUD
    /// caps, which would turn "notat é.txt" into "NOTAT É.TXT".
    pub keep_case: bool,
}

/// The parts of a [`sheet`] the caller fills, in drawing order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// At the hero's right edge (laid out right to left).
    HeroRight,
    /// The body, which scrolls.
    Body,
    /// The action row's left side (secondary actions).
    ActionsLeft,
    /// The action row's right side (laid out right to left: the primary first).
    ActionsRight,
}

/// A sheet `size` points large, centred in `area` (shrunk to fit), over a dim of the page.
/// `fill` is called once for each [`Part`].
pub fn sheet(ui: &mut Ui, area: Rect, size: egui::Vec2, hero: &Hero<'_>, fill: impl FnMut(&mut Ui, Part)) {
    sheet_with(ui, area, size, hero, true, fill);
}

/// A [`sheet`] without the action row: a short notice while something happens on its own
/// ("Signing you in…"). `fill` is called for [`Part::HeroRight`] and [`Part::Body`] only.
pub fn notice_sheet(ui: &mut Ui, area: Rect, size: egui::Vec2, hero: &Hero<'_>, fill: impl FnMut(&mut Ui, Part)) {
    sheet_with(ui, area, size, hero, false, fill);
}

fn sheet_with(ui: &mut Ui, area: Rect, size: egui::Vec2, hero: &Hero<'_>, with_actions: bool, mut fill: impl FnMut(&mut Ui, Part)) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(area, 0.0, Color32::from_black_alpha(if t.dark { 90 } else { 30 }));
    let margin = if area.width() < 600.0 { 12.0 } else { 32.0 };
    let w = size.x.min(area.width() - 2.0 * margin).max(1.0);
    let h = size.y.min(area.height() - 2.0 * 24.0).max(1.0);
    let rect = Rect::from_center_size(area.center(), vec2(w, h));
    ui.painter().add(shadow(&t).as_shape(rect, CornerRadius::ZERO));
    ui.painter().rect(rect, 0.0, t.bg, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Outside);
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let hero_frame =
            egui::Frame::new().fill(tint(t.accent, if t.dark { 0.09 } else { 0.07 })).inner_margin(Margin::symmetric(24, 18)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    logo(ui, 44.0);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(egui::RichText::new(hero.title).size(26.0).family(display()).color(t.ink_strong).extra_letter_spacing(-0.52));
                        if hero.keep_case {
                            ui.add(egui::Label::new(egui::RichText::new(hero.caption).size(13.0).color(t.muted)).truncate());
                        } else if !hero.caption.is_empty() {
                            // As a meta line, so a long caption wraps on a phone.
                            meta(ui, &hero.caption.split(" · ").collect::<Vec<_>>());
                        }
                    });
                    // A narrow sheet keeps its title: the caption already says which step it is.
                    if w >= 560.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| fill(ui, Part::HeroRight));
                    }
                });
            });
        let r = hero_frame.response.rect;
        ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 2.0)), 0.0, t.accent);
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, t.hairline());
        let actions_h = if with_actions { CONTROL_H + 28.0 } else { 0.0 };
        let body_h = (rect.bottom() - r.bottom() - actions_h).max(0.0);
        let (body_rect, _) = ui.allocate_exact_size(vec2(rect.width(), body_h), Sense::hover());
        ui.scope_builder(UiBuilder::new().max_rect(body_rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(body_h).show(ui, |ui| {
                egui::Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    fill(ui, Part::Body);
                });
            });
        });
        if !with_actions {
            return;
        }
        let actions = Rect::from_min_max(pos2(rect.left(), body_rect.bottom()), rect.max);
        ui.painter().hline(actions.x_range(), actions.top() + 0.5, t.hairline());
        ui.scope_builder(
            UiBuilder::new().max_rect(actions.shrink2(vec2(24.0, 14.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                fill(ui, Part::ActionsLeft);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| fill(ui, Part::ActionsRight));
            },
        );
    });
}

/// Splits the page into header, middle and footer.
pub fn page_rects(full: Rect) -> (Rect, Rect, Rect) {
    let header = Rect::from_min_max(full.min, pos2(full.right(), full.top() + HEADER_H));
    let footer = Rect::from_min_max(pos2(full.left(), full.bottom() - FOOTER_H), full.max);
    let middle = Rect::from_min_max(pos2(full.left(), header.bottom()), pos2(full.right(), footer.top()));
    (header, middle, footer)
}

#[cfg(test)]
mod tests {
    use super::{Hero, Kind, Part, initials, notice_sheet, wide_button};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn a_wide_button_spans_the_width_it_is_given() {
        let mut h = Harness::builder().with_size(egui::vec2(600.0, 200.0)).build_ui(|ui| {
            wide_button(ui, "Sign in with Example ID", Kind::Secondary, None, true, 430.0);
            wide_button(ui, "Sign in with a provider whose name is long", Kind::Secondary, None, true, 40.0);
        });
        h.run();
        assert_eq!(h.get_by_label("SIGN IN WITH EXAMPLE ID").rect().width(), 430.0);
        assert!(h.get_by_label("SIGN IN WITH A PROVIDER WHOSE NAME IS LONG").rect().width() > 40.0, "never narrower than its label");
    }

    #[test]
    fn a_notice_sheet_has_no_action_row() {
        let parts = std::cell::RefCell::new(Vec::new());
        let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui(|ui| {
            crate::theme::install(ui.ctx());
            let area = ui.max_rect();
            let hero = Hero { title: "Signing you in…", caption: "", keep_case: false };
            notice_sheet(ui, area, egui::vec2(480.0, 260.0), &hero, |_, part| parts.borrow_mut().push(part));
        });
        h.run();
        drop(h);
        let parts = parts.into_inner();
        assert!(parts.contains(&Part::Body));
        assert!(!parts.contains(&Part::ActionsLeft) && !parts.contains(&Part::ActionsRight), "{parts:?}");
    }

    #[test]
    fn initials_from_names() {
        assert_eq!(initials("John Doe"), "JD");
        assert_eq!(initials("john"), "JO");
        assert_eq!(initials("jane.smith"), "JS");
    }
}
