//! A grid of Craft app cards that switch on and off with a click anywhere on the card (setup step
//! 4, Admin › Server), after the launcher's app card (`views.rs` `card`) at the v2 size: a version
//! strip, the 56 px icon, the two-tone wordmark, the tagline, a meta row and the Off/On cells.
//!
//! Off, a card shows its icon in grey and its content at 55 %. The design blurs it too; egui has
//! no blur, so the faded, unblurred look is the egui version (design/README.md, "Card off state").
//!
//! The icons are the Craft apps' own (craft-launcher `assets/app-icons/`, MIT OR Apache-2.0),
//! built in. An app without one gets a tile in its colour with its initial.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::chrome::{self, CONTROL_H};
use crate::theme::Tokens;

pub struct Card<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub tagline: &'a str,
    /// The pinned version, shown in the strip.
    pub version: &'a str,
    /// The app's surface colour.
    pub color: Color32,
    /// The app's text colour on this appearance.
    pub ink: Color32,
    /// The extensions it opens, without dots (the first few are shown).
    pub opens: &'a [String],
    pub experimental: bool,
}

const MIN_WIDTH: f32 = 320.0;
const HEIGHT: f32 = 252.0;
const STRIP: f32 = 40.0;
const PAD_X: f32 = 20.0;
const ICON: f32 = 56.0;
/// The content block's opacity on a card that is off.
const OFF_FADE: f32 = 0.55;

/// Draws the grid; returns the ids of the cards clicked this frame.
pub fn app_grid(ui: &mut Ui, cards: &[Card<'_>], is_on: impl Fn(&str) -> bool) -> Vec<String> {
    let width = ui.available_width();
    let cols = ((width / MIN_WIDTH).floor() as usize).clamp(1, 6);
    // Cards share their 1 px rules: each is one px wider and taller than its step, so neighbours
    // draw their edges on the same pixels. Column edges are rounded, so the grid fills the width.
    let width = width.floor();
    let edge = |col: usize| (col as f32 * (width - 1.0) / cols as f32).round();
    let rows = cards.len().div_ceil(cols);
    let (grid, _) = ui.allocate_exact_size(vec2(width, HEIGHT * rows as f32 + 1.0), Sense::hover());
    let mut clicked = Vec::new();
    for (i, card) in cards.iter().enumerate() {
        let (row, col) = (i / cols, i % cols);
        let (x0, x1) = (edge(col), edge(col + 1));
        let rect = Rect::from_min_size(grid.min + vec2(x0, row as f32 * HEIGHT), vec2(x1 - x0 + 1.0, HEIGHT + 1.0));
        if ui.is_rect_visible(rect) && draw_card(ui, rect, card, is_on(card.id)).clicked() {
            clicked.push(card.id.to_owned());
        }
    }
    clicked
}

fn draw_card(ui: &mut Ui, rect: Rect, card: &Card<'_>, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let response = ui.interact(rect, ui.scope_id().with(("app-card", card.id)), Sense::click());
    // Screen readers (and tests) see one named on/off toggle; the Off/On cells are drawn only.
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, card.name));
    let hovered = response.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter_at(rect);
    let fade = |c: Color32| if on { c } else { c.gamma_multiply(OFF_FADE) };

    if hovered {
        painter.rect_filled(rect, 0.0, chrome::tint(card.color, 0.05));
    }
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);

    // The strip: version left, state right, a hairline under it.
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), STRIP));
    painter.hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, t.line));
    let version = chrome::hud_galley(ui, card.version, if on { card.ink } else { t.muted });
    painter.galley(pos2(strip.left() + PAD_X, strip.center().y - version.size().y / 2.0), version, t.muted);
    let (state, tone) = if on { ("In the browser", t.good) } else { ("Off", t.muted) };
    let state = chrome::hud_galley(ui, state, tone);
    painter.galley(pos2(strip.right() - PAD_X - state.size().x, strip.center().y - state.size().y / 2.0), state, tone);
    if hovered {
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), 2.0)), 0.0, if on { card.color } else { t.line_strong });
    }

    // Icon, with the wordmark and tagline beside it.
    let x = rect.left() + PAD_X;
    let top = strip.bottom() + 18.0;
    let icon_rect = Rect::from_min_size(pos2(x, top - if hovered { 4.0 } else { 0.0 }), vec2(ICON, ICON));
    app_icon(ui, &painter, icon_rect, card, on);
    let text_x = icon_rect.right() + 14.0;
    let text_w = (rect.right() - PAD_X - text_x).max(40.0);
    let (first, craft) = card.name.strip_suffix("Craft").map_or((card.name, ""), |f| (f, "Craft"));
    let mut job = LayoutJob::default();
    let word =
        |c: Color32| TextFormat { font_id: FontId::new(22.0, crate::theme::display()), color: c, extra_letter_spacing: -0.44, ..Default::default() };
    job.append(first, 0.0, word(fade(t.ink_strong)));
    job.append(craft, 0.0, word(if on { card.ink } else { t.muted.gamma_multiply(OFF_FADE) }));
    job.wrap = one_line(text_w);
    let wordmark = painter.layout_job(job);
    let mark_h = wordmark.size().y;
    painter.galley(pos2(text_x, top + 2.0), wordmark, t.ink_strong);
    let tagline = painter.layout_job(LayoutJob {
        wrap: one_line(text_w),
        ..LayoutJob::single_section(card.tagline.to_owned(), TextFormat::simple(FontId::proportional(13.5), fade(t.muted)))
    });
    painter.galley(pos2(text_x, top + 2.0 + mark_h + 2.0), tagline, t.muted);

    // Meta row: EXPERIMENTAL, then what it opens.
    let mut mx = x;
    let my = top + ICON + 14.0;
    if card.experimental {
        let g = chrome::caps(ui, "Experimental", 10.0, egui::FontFamily::Monospace, 1.0, fade(t.warn));
        let badge = Rect::from_min_size(pos2(mx, my), vec2(g.size().x + 12.0, 20.0));
        painter.rect(badge, 0.0, fade(t.sunken), Stroke::new(1.0, fade(t.line)), egui::StrokeKind::Inside);
        painter.galley(badge.center() - g.size() / 2.0, g, t.warn);
        mx = badge.right() + 10.0;
    }
    if !card.opens.is_empty() {
        let mut opens = format!("Opens {}", card.opens.iter().take(3).map(|e| format!(".{e}")).collect::<Vec<_>>().join(" "));
        if card.opens.len() > 3 {
            opens.push_str(" …");
        }
        let g = chrome::hud_galley(ui, &opens, fade(t.muted));
        painter.galley(pos2(mx, my + 10.0 - g.size().y / 2.0), g, t.muted);
    }

    // Off/On cells at the bottom, drawn only (the whole card is the control).
    let labels = ["Off", "On"];
    let galleys: Vec<_> = labels.iter().map(|l| chrome::hud_galley(ui, l, t.muted)).collect();
    let cells_w: f32 = galleys.iter().map(|g| g.size().x + 24.0).sum();
    let cells = Rect::from_min_size(pos2(x, rect.bottom() - 1.0 - 20.0 - CONTROL_H), vec2(cells_w, CONTROL_H));
    let mut cx = cells.left();
    for (i, g) in galleys.into_iter().enumerate() {
        let cell = Rect::from_min_size(pos2(cx, cells.top()), vec2(g.size().x + 24.0, CONTROL_H));
        cx = cell.right();
        let selected = (i == 1) == on;
        let color = if selected {
            painter.rect_filled(cell, 0.0, t.invert_bg);
            t.invert_fg
        } else {
            t.muted
        };
        if i > 0 {
            painter.vline(cell.left(), cell.y_range(), Stroke::new(1.0, t.line));
        }
        painter.galley_with_override_text_color(cell.center() - g.size() / 2.0, g, color);
    }
    painter.rect_stroke(cells, 0.0, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    response
}

/// One line, cut with "…" when it doesn't fit.
fn one_line(width: f32) -> TextWrapping {
    TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') }
}

/// The 56 px icon with its soft shadow; grey when the card is off.
fn app_icon(ui: &Ui, painter: &egui::Painter, rect: Rect, card: &Card<'_>, on: bool) {
    let t = Tokens::get(ui.ctx());
    let radius = egui::CornerRadius::same(12);
    if on {
        let shadow = egui::epaint::Shadow { offset: [0, 6], blur: 14, spread: 0, color: Color32::from_black_alpha(if t.dark { 71 } else { 36 }) };
        painter.add(shadow.as_shape(rect, radius));
    }
    match icon_texture(ui.ctx(), card.id, !on) {
        Some(texture) => {
            let tint = if on { Color32::WHITE } else { Color32::WHITE.gamma_multiply(OFF_FADE) };
            painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        None => {
            let fill = if on { card.color } else { grey(card.color).gamma_multiply(OFF_FADE) };
            painter.rect_filled(rect, radius, fill);
            let initial: String = card.name.chars().take(1).collect();
            let g = painter.layout_no_wrap(initial, FontId::new(28.0, crate::theme::display()), Color32::WHITE);
            painter.galley(rect.center() - g.size() / 2.0, g, Color32::WHITE);
        }
    }
}

fn grey(c: Color32) -> Color32 {
    let l = (0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b())).round() as u8;
    Color32::from_rgba_unmultiplied(l, l, l, c.a())
}

/// The built-in icon of the app `id`, if there is one.
fn icon_png(id: &str) -> Option<&'static [u8]> {
    macro_rules! icons {
        ($($name:literal),*) => {
            match id {
                $($name => Some(include_bytes!(concat!("../assets/app-icons/", $name, ".png")).as_slice()),)*
                _ => None,
            }
        };
    }
    icons!(
        "cadcraft",
        "deckcraft",
        "designcraft",
        "effectcraft",
        "figcraft",
        "filmcraft",
        "gridcraft",
        "lightcraft",
        "materialcraft",
        "pdfcraft",
        "photocraft",
        "soundcraft",
        "texturecraft",
        "vectorcraft",
        "wordcraft"
    )
}

/// The icon's texture, in colour or in grey, decoded once per context.
fn icon_texture(ctx: &egui::Context, id: &str, grey_out: bool) -> Option<egui::TextureHandle> {
    let key = egui::Id::unique(("craftcloud-app-icon", id, grey_out));
    if let Some(found) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(key)) {
        return found;
    }
    let made = icon_png(id).and_then(|png| image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()).map(|img| {
        let mut rgba = img.into_rgba8();
        if grey_out {
            for p in rgba.pixels_mut() {
                let g = grey(Color32::from_rgb(p[0], p[1], p[2]));
                p[0] = g.r();
                p[1] = g.r();
                p[2] = g.r();
            }
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
        ctx.load_texture(format!("app-icon-{id}-{grey_out}"), image, egui::TextureOptions::LINEAR)
    });
    ctx.data_mut(|d| d.insert_temp(key, made.clone()));
    made
}
