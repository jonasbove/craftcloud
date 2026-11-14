//! "Your share links" (`/links`, issue #19; `design/README.md`, "Sharing"): every link the
//! signed-in person made, with where its item is now, when it expires, whether it has a password,
//! how often it was opened and when it was made; COPY and REVOKE on each. Reached from the
//! account menu (the person's name in the header) and the share sheet's "See all your share
//! links".
//!
//! It reads OxiCloud's share API as `craftcloud share ls` does: every page of `GET /api/shares`,
//! then one lookup per link by its item's id, because OxiCloud keeps only the name a link was
//! made with. An item in the trash or deleted answers 404 and shows "No longer there" with that
//! name; a lookup that fails some other way says so, and is never taken as gone. REVOKE asks
//! first, then deletes the link (`DELETE /api/shares/{id}`, as `craftcloud share rm`); the item
//! itself is never touched.

use std::collections::HashMap;

use craftcloud_core::ServerUrl;
use craftcloud_core::api::ApiError;
use craftcloud_core::api::shares::{self, ItemPlace, Share};
use egui::{Align2, FontId, Margin, Rect, RichText, Sense, Ui, pos2, vec2};

use super::files::draw::{self, Glyph};
use super::files::share::{expiry_cell, place};
use super::files::time;
use crate::backend::{Backend, Net, NetResult, WallClock};
use crate::chrome::{self, Icon, Kind as ButtonKind};
use crate::kit;
use crate::theme::Tokens;

/// Links asked for per page (the most OxiCloud gives).
const PER_PAGE: u32 = 100;
/// Pages read at most (10 000 links), so a server that always says "more" can't loop forever.
const MAX_PAGES: u32 = 100;
/// How long a notice stays, in seconds.
const TOAST_SECS: f64 = 6.0;
/// Rows.
const ROW_H: f32 = 62.0;
const HEAD_H: f32 = 42.0;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    /// A page of the list; `gen` tells a refresh's answers from an older one's.
    List { generation: u64, page: u32 },
    /// Where the item of the link with this id is.
    Place { generation: u64, share: String },
    /// The revoke of the link with this id.
    Revoke(String),
}

/// What a link's item is known to be.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Found {
    Looking,
    Place(ItemPlace),
}

/// The filter cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Active,
    Expired,
    Gone,
}

/// A link's state; each link is in exactly one, so the counts add up to ALL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Active,
    Expired,
    /// Its item is in the trash or deleted (whether or not the link is past its expiry).
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Newest,
    Oldest,
    Name,
    Expires,
    Opened,
}

const SORTS: [(Sort, &str); 5] =
    [(Sort::Newest, "Newest"), (Sort::Oldest, "Oldest"), (Sort::Name, "Name"), (Sort::Expires, "Expires soonest"), (Sort::Opened, "Most opened")];

/// The revoke sheet.
#[derive(Debug)]
struct Revoke {
    id: String,
    error: Option<String>,
}

struct Toast {
    title: String,
    body: String,
    until: f64,
}

pub struct Links {
    server: ServerUrl,
    net: Net<Key>,
    generation: u64,
    /// Every link, once the whole list has come in.
    shares: Option<Vec<Share>>,
    /// The pages read so far of the list being loaded.
    pages: Vec<Share>,
    /// Where each link's item is, by link id.
    found: HashMap<String, Found>,
    /// Look the items up once the list is in (the page is open; the menu only counts).
    look_up: bool,
    error: Option<String>,
    pub(crate) filter: Filter,
    pub(crate) query: String,
    pub(crate) sort: Sort,
    revoke: Option<Revoke>,
    toast: Option<Toast>,
    clock: Option<WallClock>,
    now: f64,
    /// GO TO FILES was clicked: the web app opens Files.
    pub(crate) go_to_files: bool,
}

impl Links {
    pub fn new(server: ServerUrl) -> Self {
        Self {
            server,
            net: Net::default(),
            generation: 0,
            shares: None,
            pages: Vec::new(),
            found: HashMap::new(),
            look_up: false,
            error: None,
            filter: Filter::All,
            query: String::new(),
            sort: Sort::Newest,
            revoke: None,
            toast: None,
            clock: None,
            now: 0.0,
            go_to_files: false,
        }
    }

    /// The page is shown: (re)load the list and look every item up.
    pub fn open(&mut self, backend: &mut dyn Backend) {
        self.look_up = true;
        self.load(backend);
    }

    /// The account menu is shown: load the list for its count, once.
    pub fn count_for_menu(&mut self, backend: &mut dyn Backend) {
        if self.shares.is_none() && !self.net.any_waiting(|k| matches!(k, Key::List { .. })) {
            self.load(backend);
        }
    }

    /// How many links there are, once known.
    pub fn count(&self) -> Option<usize> {
        self.shares.as_ref().map(Vec::len)
    }

    fn load(&mut self, backend: &mut dyn Backend) {
        self.generation += 1;
        self.pages.clear();
        self.error = None;
        let generation = self.generation;
        self.net.send(backend, Key::List { generation, page: 1 }, shares::list(&self.server, 1, PER_PAGE));
    }

    pub fn is_busy(&self) -> bool {
        self.net.is_busy() || self.toast.is_some()
    }

    /// Takes this page's answers out of `answers`.
    pub fn receive(&mut self, backend: &mut dyn Backend, answers: &mut Vec<(u64, NetResult)>) {
        for (key, result) in self.net.receive(answers) {
            self.on_answer(backend, key, result);
        }
    }

    fn on_answer(&mut self, backend: &mut dyn Backend, key: Key, result: NetResult) {
        match key {
            Key::List { generation, .. } | Key::Place { generation, .. } if generation != self.generation => {}
            Key::List { page, generation } => {
                let list = result
                    .map_err(|why| format!("The server couldn't be reached ({why})."))
                    .and_then(|resp| shares::parse_list(&resp).map_err(|e| format!("The server didn't list your links: {}.", plain(&e))));
                match list {
                    Ok(list) => {
                        self.pages.extend(list.items);
                        if list.has_next && page < MAX_PAGES {
                            let next = page + 1;
                            self.net.send(backend, Key::List { generation, page: next }, shares::list(&self.server, next, PER_PAGE));
                            return;
                        }
                        let all = std::mem::take(&mut self.pages);
                        if self.look_up {
                            self.found.clear();
                            for s in &all {
                                self.found.insert(s.id.clone(), Found::Looking);
                                let key = Key::Place { generation, share: s.id.clone() };
                                self.net.send(backend, key, shares::item_info(&self.server, s));
                            }
                        }
                        self.shares = Some(all);
                    }
                    Err(why) => {
                        self.pages.clear();
                        self.error = Some(why);
                    }
                }
            }
            Key::Place { share, .. } => {
                let Some(s) = self.shares.as_ref().and_then(|all| all.iter().find(|s| s.id == share)) else { return };
                let place = result.map_or(ItemPlace::Unknown, |resp| shares::parse_item_place(s, &resp));
                self.found.insert(share, Found::Place(place));
            }
            Key::Revoke(id) => {
                let answer =
                    result.map_err(|why| format!("The server couldn't be reached ({why}). The link still works. Try again.")).and_then(|resp| {
                        match craftcloud_core::api::expect_ok(&resp) {
                            // Already gone: what the person asked for.
                            Err(e) if e.status() != Some(404) => {
                                Err(format!("The server didn't revoke the link: {}. The link still works. Try again.", plain(&e)))
                            }
                            _ => Ok(()),
                        }
                    });
                match answer {
                    Ok(()) => {
                        let name = self.shares.as_ref().and_then(|all| all.iter().find(|s| s.id == id)).map(|s| self.name(s));
                        if let Some(all) = &mut self.shares {
                            all.retain(|s| s.id != id);
                        }
                        self.found.remove(&id);
                        if self.revoke.as_ref().is_some_and(|r| r.id == id) {
                            self.revoke = None;
                        }
                        let body = match name {
                            Some(name) => format!("The link to {name} doesn't work any more."),
                            None => "It doesn't work any more.".to_owned(),
                        };
                        self.notice("Link revoked", &body);
                    }
                    Err(why) => {
                        if let Some(r) = self.revoke.as_mut().filter(|r| r.id == id) {
                            r.error = Some(why);
                        }
                    }
                }
            }
        }
    }

    fn notice(&mut self, title: &str, body: &str) {
        self.toast = Some(Toast { title: title.to_owned(), body: body.to_owned(), until: self.now + TOAST_SECS });
    }

    fn place_of(&self, s: &Share) -> Option<&ItemPlace> {
        match self.found.get(&s.id) {
            Some(Found::Place(p)) => Some(p),
            _ => None,
        }
    }

    fn state(&self, s: &Share) -> State {
        if self.place_of(s) == Some(&ItemPlace::Gone) {
            State::Gone
        } else if expiry_cell(s.expires_at.as_deref(), self.clock).is_some_and(|(_, _, over)| over) {
            State::Expired
        } else {
            State::Active
        }
    }

    /// The item's name: from where it is now, else the one the link was made with.
    fn name(&self, s: &Share) -> String {
        if let Some(ItemPlace::At(path)) = self.place_of(s) {
            return path.rsplit('/').next().unwrap_or(path).to_owned();
        }
        s.item_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("A {} with no name", s.item_type))
    }

    /// `[all, active, expired, gone]`.
    fn counts(&self) -> [usize; 4] {
        let mut n = [0; 4];
        for s in self.shares.iter().flatten() {
            n[0] += 1;
            n[match self.state(s) {
                State::Active => 1,
                State::Expired => 2,
                State::Gone => 3,
            }] += 1;
        }
        n
    }

    /// The links the filter and the search let through, in the chosen order.
    pub(crate) fn rows(&self) -> Vec<&Share> {
        let q = self.query.trim().to_lowercase();
        let mut rows: Vec<&Share> = self
            .shares
            .iter()
            .flatten()
            .filter(|s| match self.filter {
                Filter::All => true,
                Filter::Active => self.state(s) == State::Active,
                Filter::Expired => self.state(s) == State::Expired,
                Filter::Gone => self.state(s) == State::Gone,
            })
            .filter(|s| {
                q.is_empty()
                    || self.name(s).to_lowercase().contains(&q)
                    || matches!(self.place_of(s), Some(ItemPlace::At(p)) if p.to_lowercase().contains(&q))
            })
            .collect();
        let made = |s: &Share| time::parse_rfc3339(&s.created_at).unwrap_or(0);
        match self.sort {
            Sort::Newest => rows.sort_by_key(|s| std::cmp::Reverse(made(s))),
            Sort::Oldest => rows.sort_by_key(|s| made(s)),
            Sort::Name => rows.sort_by_cached_key(|s| self.name(s).to_lowercase()),
            // Links that never expire last.
            Sort::Expires => rows.sort_by_key(|s| s.expires_at.as_deref().and_then(time::parse_rfc3339).unwrap_or(i64::MAX)),
            Sort::Opened => rows.sort_by_key(|s| std::cmp::Reverse(s.access_count)),
        }
        rows
    }

    // ─── Drawing ────────────────────────────────────────────────────────────────────────────────

    pub fn ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend) {
        self.clock = backend.wall_clock();
        self.now = backend.now();
        if self.is_busy() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let small = area.width() < 600.0;
        let pad = if small { 16.0 } else { chrome::PAD };
        let inner = area.shrink2(vec2(pad, 0.0));
        let mut y = area.top() + if small { 16.0 } else { 24.0 };

        let bar_h = if small { chrome::CONTROL_H * 2.0 + 10.0 } else { chrome::CONTROL_H };
        let bar = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), bar_h));
        self.toolbar_ui(ui, bar, small, backend);
        y = bar.bottom() + if small { 16.0 } else { 26.0 };

        let rest = Rect::from_min_max(pos2(inner.left(), y), pos2(inner.right(), area.bottom()));
        let mut copy: Option<(String, String)> = None;
        let mut revoke: Option<String> = None;
        if let Some(why) = self.error.clone() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rest), |ui| {
                crate::widgets::error_line(ui, &why);
                ui.add_space(10.0);
                if chrome::button(ui, "Try again", ButtonKind::Secondary, Some(Icon::Refresh), !self.net.is_busy()).clicked() {
                    self.load(backend);
                }
            });
        } else if self.shares.is_none() {
            ui.painter().text(
                pos2(rest.center().x, rest.top() + 120.0),
                Align2::CENTER_CENTER,
                "LOADING…",
                FontId::monospace(11.0),
                Tokens::get(ui.ctx()).muted,
            );
        } else if self.count() == Some(0) {
            self.empty_ui(ui, rest);
        } else {
            (copy, revoke) = self.table_ui(ui, rest, small);
        }
        if let Some((url, name)) = copy {
            ui.ctx().copy_text(url);
            self.notice("Link copied", &format!("The link to {name} is on your clipboard."));
        }
        if let Some(id) = revoke {
            self.revoke = Some(Revoke { id, error: None });
        }
        self.toast_ui(ui, area);
        if self.revoke.is_some() {
            self.revoke_ui(ui, area, backend);
        }
    }

    fn toolbar_ui(&mut self, ui: &mut Ui, bar: Rect, small: bool, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        let [all, active, expired, gone] = self.counts();
        let mut cells =
            vec![(Filter::All, format!("All {all}")), (Filter::Active, format!("Active {active}")), (Filter::Expired, format!("Expired {expired}"))];
        // "No longer there" only when there is something to show under it.
        if gone > 0 || self.filter == Filter::Gone {
            // A phone has room for the short word only.
            cells.push((Filter::Gone, if small { format!("Gone {gone}") } else { format!("No longer there {gone}") }));
        }
        let row1 = Rect::from_min_size(bar.min, vec2(bar.width(), chrome::CONTROL_H));
        let row2 = if small { Rect::from_min_size(pos2(bar.left(), bar.bottom() - chrome::CONTROL_H), row1.size()) } else { row1 };
        // On a phone the refresh square starts the second row, so the cells fit the first.
        let refresh_row = if small { row2 } else { row1 };
        let refresh = Rect::from_min_size(refresh_row.min, vec2(chrome::CONTROL_H, chrome::CONTROL_H));
        ui.scope_builder(egui::UiBuilder::new().max_rect(refresh), |ui| {
            if kit::refresh_button(ui, !self.net.any_waiting(|k| matches!(k, Key::List { .. }))).clicked() {
                self.open(backend);
            }
        });
        let cells_left = if small { row1.left() } else { row1.left() + chrome::CONTROL_H + 8.0 };
        let cells_row = Rect::from_min_max(pos2(cells_left, row1.top()), row1.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(cells_row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let labels: Vec<&str> = cells.iter().map(|(_, l)| l.as_str()).collect();
            let current = cells.iter().position(|(f, _)| *f == self.filter);
            if let Some(i) = kit::cells(ui, &labels, current, true) {
                self.filter = cells[i].0;
            }
        });
        let right = if small { Rect::from_min_max(pos2(row2.left() + chrome::CONTROL_H + 8.0, row2.top()), row2.max) } else { row1 };
        ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let current = SORTS.iter().position(|(s, _)| *s == self.sort).unwrap_or(0);
            let names: Vec<&str> = SORTS.iter().map(|(_, n)| *n).collect();
            if let Some(i) = kit::picker(ui, "links-sort", current, &names, if small { 130.0 } else { 150.0 }) {
                self.sort = SORTS[i].0;
            }
            if !small {
                chrome::hud(ui, "Sort", t.muted);
            }
            let w = if small { ui.available_width() } else { 200.0 };
            kit::search_box(ui, &mut self.query, "Search links", w);
        });
    }

    fn empty_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        let body = Rect::from_min_size(rect.min + vec2(0.0, 18.0), vec2(rect.width().min(640.0), rect.height()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            chrome::hud(ui, "Your share links", t.muted);
            ui.add_space(14.0);
            chrome::headline(ui, if rect.width() < 600.0 { 36.0 } else { 48.0 }, &[&[("No share ", false), ("links", true)], &[("yet.", false)]]);
            ui.add_space(14.0);
            chrome::body(
                ui,
                "Make one from Files: open a file's or folder's menu and choose Share link…, or select it and press Share. Every link you make shows here, with where the item is now and when the link expires.",
                18.0,
            );
            ui.add_space(18.0);
            if chrome::button(ui, "Go to files", ButtonKind::Primary, Some(Icon::Folder), true).clicked() {
                self.go_to_files = true;
            }
            ui.add_space(16.0);
            chrome::meta(ui, &["Links work without signing in", "You can revoke one at any time"]);
        });
    }

    /// The summary line, the table and the note under it. Returns a link to copy `(url, name)` and
    /// a link to revoke.
    fn table_ui(&mut self, ui: &mut Ui, rect: Rect, small: bool) -> (Option<(String, String)>, Option<String>) {
        let t = Tokens::get(ui.ctx());
        let mut copy = None;
        let mut revoke = None;
        let rows: Vec<Share> = self.rows().into_iter().cloned().collect();
        let with_password = rows.iter().filter(|s| s.has_password).count();
        let opened: u64 = rows.iter().map(|s| s.access_count).sum();
        let summary = [count(rows.len(), "link"), format!("{with_password} with a password"), format!("Opened {}", times(opened, true))];
        // A phone wraps the summary onto two lines.
        let meta = ui
            .scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.min, vec2(rect.width(), 40.0))), |ui| {
                chrome::meta(ui, &summary.iter().map(String::as_str).collect::<Vec<_>>());
            })
            .response
            .rect;

        // Columns: (left, width) after the item column; narrow windows drop the minor ones.
        let wide = rect.width() >= 1100.0;
        let medium = rect.width() >= 760.0;
        let actions_w = if small { 96.0 } else { 230.0 };
        let mut cols: Vec<(Col, f32)> = vec![(Col::Item, 0.0)];
        if medium {
            cols.push((Col::Expires, 170.0));
            cols.push((Col::Password, 110.0));
        }
        if wide {
            cols.push((Col::Opened, 100.0));
            cols.push((Col::Made, 120.0));
        }
        cols.push((Col::Actions, actions_w));
        let frame = Rect::from_min_max(pos2(rect.left(), meta.bottom() + 14.0), rect.max);
        let fixed: f32 = cols.iter().map(|(_, w)| w).sum();
        let x0 = frame.left() + 16.0;
        let item_w = (frame.width() - 32.0 - fixed).max(120.0);
        let mut x = x0;
        let spans: Vec<(Col, f32, f32)> = cols
            .iter()
            .map(|(c, w)| {
                let w = if *c == Col::Item { item_w } else { *w };
                let span = (*c, x, w);
                x += w;
                span
            })
            .collect();
        let col = |c: Col| spans.iter().find(|(k, _, _)| *k == c).map(|(_, x, w)| (*x, *w));

        let note = "A link opens its item wherever it is now: a moved or renamed file keeps its link. A link whose item is in the trash or deleted shows the name it was shared with. Revoking a link doesn't touch the item.";
        let note_h = if small { 84.0 } else { 44.0 };
        let body_h = if rows.is_empty() { 120.0 } else { rows.len() as f32 * ROW_H };
        // The frame's hairlines take a point at the top and bottom of the rows.
        let table_h = (HEAD_H + body_h + 2.0).min(frame.height() - note_h).max(HEAD_H + ROW_H);
        let table = Rect::from_min_size(frame.min, vec2(frame.width(), table_h));
        ui.painter().rect_stroke(table, 0.0, t.hairline(), egui::StrokeKind::Inside);

        // Header.
        let head = Rect::from_min_max(pos2(x0, table.top()), pos2(table.right() - 16.0, table.top() + HEAD_H));
        ui.painter().hline(head.x_range(), head.bottom() - 0.5, t.hairline());
        for (c, x, w) in &spans {
            let label = match c {
                Col::Item => "Shared item",
                Col::Expires => "Expires",
                Col::Password => "Password",
                Col::Opened => "Opened",
                Col::Made => "Made",
                Col::Actions => continue,
            };
            let sorted = match (c, self.sort) {
                (Col::Made, Sort::Newest) => Some(false),
                (Col::Made, Sort::Oldest) => Some(true),
                (Col::Item, Sort::Name) | (Col::Expires, Sort::Expires) => Some(true),
                (Col::Opened, Sort::Opened) => Some(false),
                _ => None,
            };
            let cell = Rect::from_min_size(pos2(*x, head.top()), vec2(*w, HEAD_H));
            if kit::header_cell(ui, cell, label, sorted).clicked() {
                self.sort = match c {
                    Col::Item => Sort::Name,
                    Col::Expires => Sort::Expires,
                    Col::Opened => Sort::Opened,
                    _ if self.sort == Sort::Newest => Sort::Oldest,
                    _ => Sort::Newest,
                };
            }
        }

        // Rows.
        let body = Rect::from_min_max(pos2(table.left() + 1.0, head.bottom()), pos2(table.right() - 1.0, table.bottom() - 1.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            if rows.is_empty() {
                let r = ui.painter().text(body.center(), Align2::CENTER_CENTER, "No link matches.", FontId::proportional(14.0), t.muted);
                let label = ui.interact(r, ui.scope_id().with("no-match"), Sense::hover());
                label.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, "No link matches."));
                return;
            }
            egui::ScrollArea::vertical().id_salt("links-rows").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for s in &rows {
                    let (row, resp) = ui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::hover());
                    let line = Rect::from_min_max(pos2(x0, row.top()), pos2(head.right(), row.bottom()));
                    if resp.hovered() {
                        ui.painter().rect_filled(row, 0.0, t.sunken);
                    }
                    ui.painter().hline(line.x_range(), row.bottom() - 0.5, t.hairline());
                    match self.row_ui(ui, s, row, &col, small) {
                        Some(RowAct::Copy) => copy = Some((s.url.clone(), self.name(s))),
                        Some(RowAct::Revoke) => revoke = Some(s.id.clone()),
                        None => {}
                    }
                }
            });
        });
        let note_rect = Rect::from_min_size(pos2(frame.left(), table.bottom() + 14.0), vec2(frame.width(), note_h));
        ui.scope_builder(egui::UiBuilder::new().max_rect(note_rect), |ui| {
            ui.add(egui::Label::new(RichText::new(note).size(13.0).color(t.muted)).wrap());
        });
        (copy, revoke)
    }

    fn row_ui(&self, ui: &mut Ui, s: &Share, row: Rect, col: &dyn Fn(Col) -> Option<(f32, f32)>, small: bool) -> Option<RowAct> {
        let t = Tokens::get(ui.ctx());
        let state = self.state(s);
        let gone = state == State::Gone;
        let name = self.name(s);
        let folder = s.item_type == "folder";
        let cy = row.center().y;
        let mut act = None;

        // Shared item: glyph, name, where it is now.
        if let Some((x, w)) = col(Col::Item) {
            let glyph = Rect::from_center_size(pos2(x + 10.0, cy), vec2(20.0, 20.0));
            draw::kind_glyph(ui, glyph, folder, &name);
            if gone {
                // Faded: the item isn't there.
                ui.painter().rect_filled(glyph.expand(1.0), 0.0, t.bg.gamma_multiply(0.45));
            }
            let text_x = x + 30.0;
            let text_w = w - 30.0 - 16.0;
            let g = draw::fit_text(ui, &name, FontId::proportional(16.0), if gone { t.muted } else { t.ink_strong }, text_w);
            ui.painter().galley(pos2(text_x, cy - 20.0), g.clone(), t.ink_strong);
            let label = ui.interact(Rect::from_min_size(pos2(text_x, cy - 20.0), g.size()), ui.scope_id().with(("name", &s.id)), Sense::hover());
            label.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &name));
            let second = Rect::from_min_size(pos2(text_x, cy + 2.0), vec2(text_w, 18.0));
            match self.found.get(&s.id) {
                Some(Found::Place(ItemPlace::Gone)) => {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(second).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        chrome::badge(ui, "No longer there", t.warn);
                        let shared_as = s.item_name.as_deref().filter(|n| !n.trim().is_empty());
                        let line = match shared_as {
                            Some(n) => format!("Shared as {n} · moved to the trash or deleted"),
                            None => "Moved to the trash or deleted".to_owned(),
                        };
                        ui.add(egui::Label::new(RichText::new(line).size(12.5).color(t.muted)).truncate());
                    });
                }
                found => {
                    let line = match found {
                        Some(Found::Place(ItemPlace::At(path))) => place(path),
                        Some(Found::Place(ItemPlace::Unknown)) => "Where it is now couldn't be looked up".to_owned(),
                        _ => "Looking up where it is…".to_owned(),
                    };
                    let g = draw::fit_text(ui, &line, FontId::proportional(12.5), t.muted, text_w);
                    let r = ui.interact(Rect::from_min_size(second.min, g.size()), ui.scope_id().with(("where", &s.id)), Sense::hover());
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &line));
                    ui.painter().galley(second.min, g, t.muted);
                }
            }
        }

        // Expires.
        if let Some((x, _)) = col(Col::Expires) {
            let at = pos2(x, cy);
            match expiry_cell(s.expires_at.as_deref(), self.clock) {
                None => text(ui, at, "Never", 14.0, t.muted, ("never", &s.id)),
                Some((day, _, true)) => {
                    let badge = Rect::from_min_size(pos2(x, cy - 20.0), vec2(120.0, 18.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(badge).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        chrome::badge(ui, "Expired", t.bad);
                    });
                    text(ui, pos2(x, cy + 11.0), &day, 12.5, t.muted, ("day", &s.id));
                }
                Some((day, left, false)) => {
                    text(ui, pos2(x, cy - 10.0), &day, 14.0, t.ink_strong, ("day", &s.id));
                    if let Some(left) = left {
                        text(ui, pos2(x, cy + 11.0), &left, 12.5, t.muted, ("left", &s.id));
                    }
                }
            }
        }

        // Password.
        if let Some((x, _)) = col(Col::Password) {
            if s.has_password {
                draw::paint_glyph(ui, Glyph::Lock, pos2(x + 7.0, cy), t.muted);
                text(ui, pos2(x + 24.0, cy), "Yes", 14.0, t.ink_strong, ("pw", &s.id));
            } else {
                text(ui, pos2(x, cy), "No", 14.0, t.muted, ("pw", &s.id));
            }
        }

        if let Some((x, _)) = col(Col::Opened) {
            let label = times(s.access_count, false);
            let g = ui.painter().layout_no_wrap(label.clone(), FontId::monospace(12.0), t.muted);
            let r = ui.interact(Rect::from_min_size(pos2(x, cy - g.size().y / 2.0), g.size()), ui.scope_id().with(("opened", &s.id)), Sense::hover());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
            ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, t.muted);
        }

        if let Some((x, _)) = col(Col::Made) {
            text(ui, pos2(x, cy), &time::when(&s.created_at, self.clock), 13.0, t.muted, ("made", &s.id));
        }

        // COPY and REVOKE, right-aligned.
        if let Some((x, w)) = col(Col::Actions) {
            let cell = Rect::from_min_size(pos2(x, row.top()), vec2(w, ROW_H));
            ui.scope_builder(egui::UiBuilder::new().max_rect(cell).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // Each button names its link for screen readers and tests.
                let revoke =
                    if small { draw::text_action(ui, "Revoke", true) } else { chrome::button(ui, "Revoke", ButtonKind::Secondary, None, true) };
                let revoke = revoke.on_hover_text(format!("Revoke the link to {name}"));
                if revoke.clicked() {
                    act = Some(RowAct::Revoke);
                }
                let can_copy = state == State::Active;
                let copy = if small {
                    if can_copy { Some(draw::text_action(ui, "Copy", false)) } else { None }
                } else {
                    Some(draw::icon_label_button(ui, Glyph::Copy, "Copy", ButtonKind::Secondary, can_copy))
                };
                if copy.is_some_and(|c| c.clicked()) && can_copy {
                    act = Some(RowAct::Copy);
                }
            });
        }
        act
    }

    fn toast_ui(&mut self, ui: &mut Ui, area: Rect) {
        let Some(toast) = &self.toast else { return };
        if self.now > toast.until {
            self.toast = None;
            return;
        }
        if kit::toast(ui, area, &toast.title, &toast.body) {
            self.toast = None;
        }
    }

    fn revoke_ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        let Some(r) = &self.revoke else { return };
        let Some(s) = self.shares.as_ref().and_then(|all| all.iter().find(|s| s.id == r.id)).cloned() else {
            self.revoke = None;
            return;
        };
        let error = r.error.clone();
        let busy = self.net.is_waiting(&Key::Revoke(s.id.clone()));
        let name = self.name(&s);
        let what = if s.item_type == "folder" { "folder" } else { "file" };
        let mut facts = vec![match self.place_of(&s) {
            Some(ItemPlace::At(path)) => place(path),
            Some(ItemPlace::Gone) => "No longer there".to_owned(),
            _ => "Where it is now isn't known".to_owned(),
        }];
        facts.push(format!("opened {}", times(s.access_count, false).to_lowercase()));
        facts.push(if s.has_password { "password on".to_owned() } else { "no password".to_owned() });
        let caption = format!("{name} · made {}", time::when(&s.created_at, self.clock));
        let mut height = 340.0 + if error.is_some() { 70.0 } else { 0.0 };
        if area.width() < 600.0 {
            height += 100.0;
        }
        // What's under the sheet can't be used meanwhile.
        ui.interact(area, ui.scope_id().with("links-revoke-block"), Sense::click_and_drag());
        let mut cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let mut confirm = false;
        chrome::sheet(
            ui,
            area,
            vec2(520.0, height),
            &chrome::Hero { title: "Revoke this link?", caption: &caption, keep_case: false },
            |ui, part| match part {
                chrome::Part::Body => {
                    egui::Frame::new().fill(t.sunken).stroke(t.hairline()).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let (g, _) = ui.allocate_exact_size(vec2(20.0, 38.0), Sense::hover());
                            draw::kind_glyph(ui, Rect::from_center_size(g.center(), vec2(20.0, 20.0)), s.item_type == "folder", &name);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(egui::Label::new(RichText::new(&name).size(14.0).color(t.ink_strong)).truncate());
                                ui.add(egui::Label::new(RichText::new(facts.join(" · ")).size(12.0).color(t.muted)).truncate());
                            });
                        });
                    });
                    ui.add_space(14.0);
                    ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Anyone who opens it afterwards sees \"This link doesn't work\". The {what} stays where it is, and you can make a new link later; it gets a new address."
                        ))
                        .size(14.0)
                        .color(t.ink),
                    )
                    .wrap(),
                );
                    if let Some(e) = &error {
                        ui.add_space(12.0);
                        egui::Frame::new().fill(t.sunken).stroke(t.hairline()).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                chrome::badge(ui, "Not revoked", t.bad);
                                ui.add(egui::Label::new(RichText::new(e).size(13.0).color(t.bad)).wrap());
                            });
                        });
                    }
                }
                chrome::Part::ActionsLeft => {
                    if chrome::button(ui, "Cancel", ButtonKind::Secondary, None, !busy).clicked() {
                        cancel = true;
                    }
                }
                chrome::Part::ActionsRight => {
                    let label = if busy {
                        "Revoking…"
                    } else if error.is_some() {
                        "Try again"
                    } else {
                        "Revoke link"
                    };
                    if chrome::button(ui, label, ButtonKind::Danger, Some(Icon::Cross), !busy).clicked() {
                        confirm = true;
                    }
                }
                chrome::Part::HeroRight => {}
            },
        );
        if confirm && !busy {
            if let Some(r) = &mut self.revoke {
                r.error = None;
            }
            self.net.send(backend, Key::Revoke(s.id.clone()), shares::delete(&self.server, &s.id));
        } else if cancel && !busy {
            self.revoke = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Col {
    Item,
    Expires,
    Password,
    Opened,
    Made,
    Actions,
}

enum RowAct {
    Copy,
    Revoke,
}

/// A line of text left-centred at `at`, with an accessible label.
fn text(ui: &mut Ui, at: egui::Pos2, s: &str, size: f32, color: egui::Color32, id: (&str, &str)) {
    let g = ui.painter().layout_no_wrap(s.to_owned(), FontId::proportional(size), color);
    let r = Rect::from_min_size(pos2(at.x, at.y - g.size().y / 2.0), g.size());
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, s));
    ui.painter().galley(r.min, g, color);
}

/// "Not yet", "1 time", "3 times"; in the summary line, "27 times in all".
fn times(n: u64, in_all: bool) -> String {
    match (n, in_all) {
        (0, false) => "Not yet".to_owned(),
        (0, true) => "0 times in all".to_owned(),
        (1, false) => "1 time".to_owned(),
        (1, true) => "1 time in all".to_owned(),
        (n, false) => format!("{n} times"),
        (n, true) => format!("{n} times in all"),
    }
}

/// "1 link", "5 links".
fn count(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// A refusal in plain words, without a trailing full stop.
fn plain(e: &ApiError) -> String {
    match e.status() {
        Some(401) => "you are signed out. Sign in again".to_owned(),
        Some(403) => "you don't have permission".to_owned(),
        _ => e.to_string().trim().trim_end_matches('.').to_owned(),
    }
}

#[cfg(test)]
mod tests;
