//! The Apps menu in the header: when the server hosts the Craft web apps, the APPS entry drops a
//! grid of the apps switched on, and a tile opens that app in a new tab (`/apps/<id>/`). Design
//! frames: `v2-web-apps-menu` (and `-light`, `-phone`).
//!
//! It follows the disclosure pattern: APPS is a button that says whether the menu is expanded,
//! and the tiles are links. Opening it puts the focus on the first tile; the arrow keys move
//! between tiles, Tab and Shift+Tab too, Enter or Space opens one, and Esc closes the menu and
//! puts the focus back on APPS. A click outside it, or the focus moving elsewhere, closes it.

use craftcloud_core::catalog::{self, CraftApp};
use craftcloud_core::{RuntimeConfig, ServerUrl};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Id, Key, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::chrome::{self, Icon};
use crate::theme::Tokens;

/// Tiles to a row.
const COLS: usize = 4;
const TILE_W: f32 = 96.0;
const TILE_H: f32 = 88.0;
const GRID_PAD: f32 = 8.0;
const MENU_W: f32 = COLS as f32 * TILE_W + 2.0 * GRID_PAD;
const ICON: f32 = 40.0;
const HEAD_H: f32 = 40.0;
const FOOT_H: f32 = 34.0;
/// The space kept between the menu and the window's edges.
const EDGE: f32 = 8.0;

/// The Craft apps the menu offers: none unless someone is signed in and the server has the web
/// apps on; then the ones switched on (`enabled_apps`) that CraftCloud knows, in the server's
/// order. The header shows APPS only when this isn't empty.
pub(super) fn offered(config: Option<&RuntimeConfig>, signed_in: bool) -> Vec<&'static CraftApp> {
    let Some(config) = config.filter(|c| signed_in && c.web_apps) else { return Vec::new() };
    let mut apps: Vec<&'static CraftApp> = Vec::new();
    for app in config.enabled_apps.iter().filter_map(|id| catalog::app(id)) {
        if !apps.iter().any(|a| a.id == app.id) {
            apps.push(app);
        }
    }
    apps
}

/// Where an app's web build is served (Caddy sends `/apps/<id>/` on to its `index.html`).
pub(super) fn app_url(server: &ServerUrl, id: &str) -> String {
    server.join(&format!("apps/{id}/"))
}

#[derive(Default)]
pub(super) struct AppsMenu {
    pub(super) open: bool,
    /// The focus goes to the first tile on the next draw (the menu was just opened).
    focus_first: bool,
}

impl AppsMenu {
    pub(super) fn toggle(&mut self) {
        self.open = !self.open;
        self.focus_first = self.open;
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.focus_first = false;
    }

    /// Draws the open menu hung from `anchor` (the APPS entry), its top at `top`; returns the app
    /// chosen. `trigger` is the APPS entry's id; `opened_now`: the click that opened it is not a
    /// click outside it.
    pub(super) fn ui(
        &mut self,
        ui: &mut Ui,
        anchor: Rect,
        top: f32,
        trigger: Id,
        apps: &[&'static CraftApp],
        opened_now: bool,
    ) -> Option<&'static str> {
        let t = Tokens::get(ui.ctx());
        let screen = ui.ctx().content_rect();
        let width = MENU_W.min(screen.width() - 2.0 * EDGE).max(COLS as f32 * 40.0 + 2.0 * GRID_PAD);
        let x = anchor.left().min(screen.right() - width - EDGE).max(screen.left() + EDGE);
        let ids: Vec<Id> = apps.iter().map(|a| Id::unique(("craftcloud-apps-tile", a.id))).collect();
        // From the frame after the one that opened it (the Enter that opened the menu would
        // otherwise open the first app too), and again until a tile has it: egui lays a new area
        // out unseen on its first frame, and a widget there gives the focus straight back.
        if self.focus_first
            && !opened_now
            && let Some(first) = ids.first()
        {
            ui.memory_mut(|m| m.request_focus(*first));
        }
        let mut chosen = None;
        let mut focused = None;
        let area = egui::Area::new(Id::unique("craftcloud-apps-menu")).order(egui::Order::Foreground).fixed_pos(pos2(x, top)).show(ui.ctx(), |ui| {
            egui::Frame::new().fill(t.bg).stroke(Stroke::new(1.0, t.line_strong)).shadow(crate::kit::shadow(&t)).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                // Head: what this is, and how many.
                let (head, _) = ui.allocate_exact_size(vec2(width, HEAD_H), Sense::hover());
                let title = chrome::hud_galley(ui, "Craft apps in the browser", t.ink);
                ui.painter().galley(pos2(head.left() + 14.0, head.center().y - title.size().y / 2.0), title, t.ink);
                let count = chrome::hud_galley(ui, &apps.len().to_string(), t.muted);
                ui.painter().galley(pos2(head.right() - 14.0 - count.size().x, head.center().y - count.size().y / 2.0), count, t.muted);
                ui.painter().hline(head.x_range(), head.bottom() - 0.5, t.hairline());
                // The tiles.
                let rows = apps.len().div_ceil(COLS);
                let (grid, _) = ui.allocate_exact_size(vec2(width, rows as f32 * TILE_H + 2.0 * GRID_PAD), Sense::hover());
                let tile_w = (width - 2.0 * GRID_PAD) / COLS as f32;
                for (i, (app, id)) in apps.iter().zip(&ids).enumerate() {
                    let min = grid.min + vec2(GRID_PAD + (i % COLS) as f32 * tile_w, GRID_PAD + (i / COLS) as f32 * TILE_H);
                    let rect = Rect::from_min_size(min, vec2(tile_w, TILE_H));
                    let resp = tile(ui, rect, *id, app);
                    if resp.has_focus() {
                        focused = Some(i);
                    }
                    if resp.clicked() {
                        chosen = Some(app.id);
                    }
                }
                // Foot: where the apps open.
                let (foot, _) = ui.allocate_exact_size(vec2(width, FOOT_H), Sense::hover());
                ui.painter().hline(foot.x_range(), foot.top() + 0.5, t.hairline());
                ui.painter().text(
                    pos2(foot.left() + 14.0, foot.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Each app opens in a new tab.",
                    FontId::proportional(12.0),
                    t.muted,
                );
                chrome::icon(ui, Icon::External, pos2(foot.right() - 14.0 - 7.0, foot.center().y), t.muted);
            });
        });

        // The arrow keys move between the tiles (egui's own arrow focus is held off on a tile, so
        // the focus never wanders into the page under the menu).
        if let Some(i) = focused {
            self.focus_first = false;
            let lock = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() };
            ui.memory_mut(|m| m.set_focus_lock_filter(ids[i], lock));
            let last = ids.len() - 1;
            let next = ui.input(|inp| {
                if inp.key_pressed(Key::ArrowRight) {
                    Some((i + 1).min(last))
                } else if inp.key_pressed(Key::ArrowLeft) {
                    Some(i.saturating_sub(1))
                } else if inp.key_pressed(Key::ArrowDown) {
                    Some((i + COLS).min(last))
                } else if inp.key_pressed(Key::ArrowUp) {
                    Some(i.checked_sub(COLS).unwrap_or(i))
                } else if inp.key_pressed(Key::Home) {
                    Some(0)
                } else if inp.key_pressed(Key::End) {
                    Some(last)
                } else {
                    None
                }
            });
            if let Some(next) = next.filter(|n| *n != i) {
                ui.memory_mut(|m| m.request_focus(ids[next]));
            }
        }

        let outside = !opened_now
            && ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p));
        let focus_left = ui.memory(|m| m.focused()).is_some_and(|f| f != trigger && !ids.contains(&f));
        if chosen.is_some() || outside || focus_left {
            self.close();
        } else if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.close();
            ui.memory_mut(|m| m.request_focus(trigger));
        }
        chosen
    }
}

/// One app: its icon and name, a link. Hover and focus as the app cards: sunken, with a 2 px rule
/// in the app's colour along the top; focus adds the accent ring.
fn tile(ui: &mut Ui, rect: Rect, id: Id, app: &CraftApp) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, id, Sense::click());
    let color = catalog::rgb(app.color).map_or(t.accent, |(r, g, b)| Color32::from_rgb(r, g, b));
    let lit = resp.hovered() || resp.has_focus();
    let painter = ui.painter_at(rect);
    if lit {
        painter.rect_filled(rect, 0.0, t.sunken);
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), 2.0)), 0.0, color);
    }
    let icon = Rect::from_min_size(pos2(rect.center().x - ICON / 2.0, rect.top() + 14.0), vec2(ICON, ICON));
    crate::app_grid::paint_icon(ui.ctx(), &painter, icon, app.id, app.name, color);
    let name = painter.layout_job(LayoutJob {
        wrap: TextWrapping { max_width: rect.width() - 12.0, max_rows: 1, break_anywhere: true, overflow_character: Some('…') },
        ..LayoutJob::single_section(app.name.to_owned(), TextFormat::simple(FontId::proportional(12.0), if lit { t.ink_strong } else { t.ink }))
    });
    painter.galley(pos2(rect.center().x - name.size().x / 2.0, icon.bottom() + 8.0), name, t.ink);
    if resp.has_focus() {
        painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, t.accent_ink), egui::StrokeKind::Inside);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, app.name));
    ui.ctx().accesskit_node_builder(id, |node| node.set_description("Opens in a new tab"));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests;
