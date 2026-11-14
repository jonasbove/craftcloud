//! The "Share link" sheet (issue #18; `design/README.md`, "Sharing"): opened from a file's or
//! folder's "…" menu ("Share link…") or the selection's SHARE, it makes a link with OxiCloud's
//! share API, as `craftcloud share create` does. The expiry follows the command line's rules
//! (`shares::expiry_at`): the presets add days to now, "On a date…" lasts to the end of that day
//! in the browser's time. The password goes in the request's body only, never in a URL, a log or
//! this module's `Debug` output; after the link is made it isn't shown again.

use std::fmt;

use craftcloud_core::api::ApiError;
use craftcloud_core::api::shares::{self, ExpiryError, NewShare, Share};
use egui::{Align2, Color32, FontId, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use jiff::civil::Date;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, ToSpan};

use super::draw::{self, Glyph};
use super::model::Item;
use super::sheets::Sheet;
use super::{Files, Key};
use crate::backend::{Backend, NetResult, WallClock};
use crate::chrome::{self, Icon, Kind as ButtonKind};
use crate::theme::Tokens;

/// How long the link works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    Never,
    Days(u16),
    /// To the end of the day in the date field.
    Date,
}

const CHOICES: [(Expiry, &str); 5] =
    [(Expiry::Never, "Never"), (Expiry::Days(1), "1 day"), (Expiry::Days(7), "7 days"), (Expiry::Days(30), "30 days"), (Expiry::Date, "On a date…")];

/// A typed password: its `Debug` output never shows it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "Secret(none)" } else { "Secret(…)" })
    }
}

/// The open share sheet.
#[derive(Debug)]
pub struct ShareSheet {
    pub item: Item,
    pub expiry: Expiry,
    /// The last day, as typed or picked ("28 Nov 2026").
    pub date_text: String,
    /// The month the date picker shows (its first day), while it is open.
    pub month: Option<Date>,
    pub password: Secret,
    /// Whether the password shows as typed.
    pub show: bool,
    /// Why the server didn't make the link, in plain words.
    pub error: Option<String>,
    /// The link, once made.
    pub created: Option<Share>,
    /// COPY was used.
    pub copied: bool,
}

impl ShareSheet {
    pub fn new(item: Item) -> Self {
        Self {
            item,
            expiry: Expiry::Days(7),
            date_text: String::new(),
            month: None,
            password: Secret::default(),
            show: false,
            error: None,
            created: None,
            copied: false,
        }
    }

    /// When the link stops working (`None`: never), or why that can't be told.
    fn until(&self, now: Option<Now>) -> Result<Option<Timestamp>, Problem> {
        let text = match self.expiry {
            Expiry::Never => return Ok(None),
            Expiry::Days(n) => format!("{n}d"),
            Expiry::Date if self.date_text.trim().is_empty() => return Err(Problem::NoDate),
            Expiry::Date => read_date(&self.date_text).ok_or(Problem::Unreadable)?.to_string(),
        };
        let now = now.ok_or(Problem::NoClock)?;
        shares::expiry_at(&text, now.at, now.offset).map(Some).map_err(|e| match e {
            ExpiryError::Past => Problem::Past,
            ExpiryError::Unreadable => Problem::Unreadable,
        })
    }

    /// The line under the expiry cells, and whether it says something is wrong.
    fn hint(&self, now: Option<Now>) -> (String, bool) {
        match (self.expiry, self.until(now)) {
            (_, Ok(None)) => ("The link works until you revoke it.".into(), false),
            (Expiry::Date, Ok(Some(_))) => {
                let day = read_date(&self.date_text).map(date_text).unwrap_or_default();
                (format!("The link stops working at the end of {day}."), false)
            }
            (_, Ok(Some(t))) => {
                let z = local(t, now);
                (format!("The link stops working on {} at {}.", z.strftime("%-d %b %Y"), z.strftime("%H:%M")), false)
            }
            (_, Err(Problem::NoDate)) => ("Pick the last day the link works.".into(), false),
            (_, Err(Problem::Unreadable)) => ("Write a date such as 28 Nov 2026, or pick one.".into(), true),
            (_, Err(Problem::Past)) => ("That day is over. Pick today or a later day.".into(), true),
            (_, Err(Problem::NoClock)) => ("This browser didn't say what time it is, so only Never works here.".into(), true),
        }
    }

    /// `POST /api/shares` for what the sheet says, when it says enough.
    fn request(&self, server: &craftcloud_core::ServerUrl, now: Option<Now>) -> Option<craftcloud_core::http::HttpRequest> {
        let until = self.until(now).ok()?;
        let password = (!self.password.0.is_empty()).then(|| self.password.0.clone());
        let new = NewShare {
            kind: self.item.kind,
            id: self.item.id.clone(),
            name: Some(self.item.name.clone()),
            expires_at: until.map(shares::expiry_text),
            password,
        };
        Some(shares::create(server, &new))
    }

    fn what(&self) -> &'static str {
        if self.item.is_folder() { "folder" } else { "file" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Problem {
    NoDate,
    Unreadable,
    Past,
    NoClock,
}

/// The browser's clock and offset.
#[derive(Clone, Copy)]
struct Now {
    at: Timestamp,
    offset: Offset,
}

impl Now {
    fn from(clock: Option<WallClock>) -> Option<Self> {
        let c = clock?;
        Some(Self {
            at: Timestamp::from_second(c.unix.floor() as i64).ok()?,
            offset: Offset::from_seconds(c.utc_offset_minutes.saturating_mul(60)).ok()?,
        })
    }

    fn today(self) -> Date {
        self.at.to_zoned(TimeZone::fixed(self.offset)).date()
    }
}

fn local(t: Timestamp, now: Option<Now>) -> jiff::Zoned {
    t.to_zoned(now.map_or(TimeZone::UTC, |n| TimeZone::fixed(n.offset)))
}

/// "28 Nov 2026", "28 November 2026" or "2026-11-28".
fn read_date(text: &str) -> Option<Date> {
    let t = text.trim();
    t.parse::<Date>()
        .ok()
        .or_else(|| jiff::fmt::strtime::parse("%d %b %Y", t).ok()?.to_date().ok())
        .or_else(|| jiff::fmt::strtime::parse("%d %B %Y", t).ok()?.to_date().ok())
}

/// "28 Nov 2026".
fn date_text(d: Date) -> String {
    d.strftime("%-d %b %Y").to_string()
}

/// "21 Nov 2026, 14:32 · in 7 days", "end of 28 Nov 2026 · in 14 days" or "Never".
fn expires_line(expires_at: Option<&str>, now: Option<Now>) -> String {
    let Some(t) = expires_at.and_then(|e| e.parse::<Timestamp>().ok()) else { return "Never".into() };
    let z = local(t, now);
    let when = match last_day(&z) {
        Some(d) => format!("end of {}", date_text(d)),
        None => format!("{}, {}", date_text(z.date()), z.strftime("%H:%M")),
    };
    match time_left(t, now) {
        Some(left) => format!("{when} · {left}"),
        None => when,
    }
}

/// The day a link made to last to the end of a day (it ends at midnight) works through.
fn last_day(z: &jiff::Zoned) -> Option<Date> {
    (z.hour() == 0 && z.minute() == 0).then(|| z.date().yesterday().ok()).flatten()
}

/// "today", "in 1 day", "in 7 days"; `None` when it is over or the clock isn't known.
fn time_left(t: Timestamp, now: Option<Now>) -> Option<String> {
    let now = now?;
    let secs = t.as_second() - now.at.as_second();
    let days = (secs as f64 / 86_400.0).round() as i64;
    Some(match secs {
        ..=0 => return None,
        s if s < 86_400 / 2 => "today".to_owned(),
        _ if days == 1 => "in 1 day".to_owned(),
        _ => format!("in {days} days"),
    })
}

/// A link's expiry as the list of links shows it (#19): the last day it works ("21 Nov 2026"),
/// how long is left ("in 7 days"; `None` when it is over or the clock isn't known), and whether it
/// is over. `None` for a link that never expires, or a time that can't be read.
pub(crate) fn expiry_cell(expires_at: Option<&str>, clock: Option<WallClock>) -> Option<(String, Option<String>, bool)> {
    let t = expires_at?.parse::<Timestamp>().ok()?;
    let now = Now::from(clock);
    let z = local(t, now);
    let day = last_day(&z).unwrap_or_else(|| z.date());
    let over = now.is_some_and(|n| t <= n.at);
    Some((date_text(day), time_left(t, now), over))
}

/// "My files / Documents / Reports", where an item is.
fn location(item: &Item) -> String {
    place(&item.path)
}

/// "My files / Documents / Reports": the folder a cloud path (`Documents/Reports/q3.docx`) is in.
pub(crate) fn place(path: &str) -> String {
    let mut parts = vec!["My files"];
    if let Some((dir, _)) = path.rsplit_once('/') {
        parts.extend(dir.split('/').filter(|s| !s.is_empty()));
    }
    parts.join(" / ")
}

/// What the server's refusal means, in one plain sentence.
fn failure(e: &ApiError, what: &str) -> String {
    let why = match e.status() {
        Some(401) => "you are signed out. Sign in again".to_owned(),
        Some(403) => format!("you don't have permission to share this {what}"),
        Some(404) => format!("this {what} isn't there any more"),
        _ => e.to_string().trim().trim_end_matches('.').to_owned(),
    };
    format!("The server didn't make the link: {why}. Nothing was shared. Try again, or ask your admin.")
}

enum Act {
    Cancel,
    Create,
    Copy(String),
    /// "See all your share links": the page that lists them (#19).
    AllLinks,
}

/// A link in words: accent ink, underlined, Small 13.
fn text_link(ui: &mut Ui, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    ui.add(egui::Label::new(RichText::new(label).size(13.0).underline().color(t.accent_ink)).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl Files {
    /// Opens the share sheet for `item`.
    pub(super) fn start_share(&mut self, item: Item) {
        self.menu = None;
        self.sheet = Some(Sheet::Share(Box::new(ShareSheet::new(item))));
    }

    /// The link's answer: show it, or say why there is none.
    pub(super) fn on_share_answer(&mut self, id: &str, result: NetResult) {
        let Some(Sheet::Share(s)) = &mut self.sheet else { return };
        if s.item.id != id {
            return;
        }
        match result {
            Ok(resp) => match shares::parse_share(&resp) {
                Ok(share) => {
                    s.error = None;
                    s.created = Some(share);
                    let id = s.item.id.clone();
                    for item in self.items.iter_mut().filter(|i| i.id == id) {
                        item.shared = true;
                    }
                }
                Err(e) => s.error = Some(failure(&e, s.what())),
            },
            Err(why) => s.error = Some(format!("The server couldn't be reached ({why}). Nothing was shared. Try again.")),
        }
    }

    pub(super) fn share_ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        let now = Now::from(self.clock);
        let Some(Sheet::Share(s)) = &mut self.sheet else { return };
        let busy = self.net.is_waiting(&Key::Share(s.item.id.clone()));
        let mut act: Option<Act> = None;
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if s.month.is_some() {
                s.month = None;
            } else {
                act = Some(Act::Cancel);
            }
        }
        let ready = s.created.is_some();
        let caption = format!("{} · anyone with the link can open it", s.item.name);
        let height =
            if ready { 452.0 } else { 482.0 + if s.expiry == Expiry::Date { 44.0 } else { 0.0 } + if s.error.is_some() { 74.0 } else { 0.0 } };
        // A phone's narrower sheet wraps more; it may be taller (the sheet still fits the window).
        let height = if area.width() < 600.0 { height + 160.0 } else { height };
        let mut date_field: Option<Rect> = None;
        chrome::sheet(
            ui,
            area,
            vec2(560.0, height),
            &chrome::Hero { title: if ready { "Link ready" } else { "Share link" }, caption: &caption, keep_case: false },
            |ui, part| match (part, &s.created) {
                (chrome::Part::Body, None) => {
                    item_box(ui, &s.item);
                    ui.add_space(10.0);
                    chrome::hud(ui, "Link expires", t.ink);
                    ui.add_space(2.0);
                    let labels: Vec<&str> = CHOICES.iter().map(|(_, l)| *l).collect();
                    let current = CHOICES.iter().position(|(e, _)| *e == s.expiry);
                    // A phone wraps the cells onto two rows.
                    let picked = ui.horizontal_wrapped(|ui| draw::cells(ui, &labels, current)).inner;
                    if let Some(i) = picked {
                        s.expiry = CHOICES[i].0;
                        // Choosing "On a date…" opens the picker at this month.
                        s.month = if s.expiry == Expiry::Date && s.date_text.is_empty() { now.map(|n| n.today().first_of_month()) } else { None };
                    }
                    if s.expiry == Expiry::Date {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let field = input(ui, &mut s.date_text, false, 200.0, "28 Nov 2026");
                            date_field = Some(field.rect);
                            if draw::icon_button(ui, Glyph::Calendar, "Pick a day", s.month.is_some(), true).clicked() {
                                s.month = match s.month {
                                    Some(_) => None,
                                    None => read_date(&s.date_text).or(now.map(|n| n.today())).map(|d| d.first_of_month()),
                                };
                            }
                        });
                    }
                    let (hint, bad) = s.hint(now);
                    ui.label(RichText::new(hint).size(13.0).color(if bad { t.bad } else { t.muted }));
                    ui.add_space(10.0);
                    let label = ui
                        .horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.spacing_mut().interact_size.y = 0.0;
                            let label = chrome::hud(ui, "Password", t.ink);
                            chrome::hud(ui, "Optional", t.muted);
                            label
                        })
                        .inner;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (glyph, word) = if s.show { (Glyph::EyeOff, "Hide") } else { (Glyph::Eye, "Show") };
                        let w = ui.available_width() - 8.0 - draw::icon_label_width(ui, word);
                        input(ui, &mut s.password.0, !s.show, w, "").labelled_by(label.id);
                        if draw::icon_label_button(ui, glyph, word, ButtonKind::Secondary, true).clicked() {
                            s.show = !s.show;
                        }
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "The person opening the link will need it. Send it to them separately, not in the same message as the link.",
                            )
                            .size(13.0)
                            .color(t.muted),
                        )
                        .wrap(),
                    );
                    if let Some(e) = &s.error {
                        ui.add_space(10.0);
                        error_box(ui, e);
                    }
                }
                (chrome::Part::Body, Some(share)) => {
                    chrome::hud(ui, "Link", t.ink);
                    ui.add_space(2.0);
                    let row = vec2(ui.available_width(), chrome::CONTROL_H);
                    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (glyph, word) = if s.copied { (Glyph::Check, "Copied") } else { (Glyph::Copy, "Copy") };
                        if draw::icon_label_button(ui, glyph, word, ButtonKind::Primary, true).clicked() {
                            act = Some(Act::Copy(share.url.clone()));
                        }
                        url_box(ui, &share.url);
                    });
                    ui.label(
                        RichText::new(format!("Read-only. Anyone with this link can open the {} without signing in.", s.what()))
                            .size(13.0)
                            .color(t.muted),
                    );
                    ui.add_space(14.0);
                    let shares = format!("{} / {}", location(&s.item), s.item.name);
                    let password = if share.has_password { "On · it isn't shown again" } else { "Off" };
                    let rows = [("Shares", shares), ("Expires", expires_line(share.expires_at.as_deref(), now)), ("Password", password.to_owned())];
                    chrome::panel(ui, "This link", |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        for (key, value) in &rows {
                            ui.horizontal_top(|ui| {
                                let (rect, _) = ui.allocate_exact_size(vec2(110.0, 18.0), Sense::hover());
                                let g = chrome::hud_galley(ui, key, t.muted);
                                ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.muted);
                                ui.add(egui::Label::new(RichText::new(value).size(14.0).color(t.ink_strong)).wrap());
                            });
                        }
                        if share.has_password {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(vec2(14.0, 16.0), Sense::hover());
                                draw::paint_glyph(ui, Glyph::Lock, r.center(), t.muted);
                                ui.label(RichText::new("Send the password separately.").size(12.5).color(t.muted));
                            });
                        }
                    });
                }
                (chrome::Part::ActionsLeft, None) => {
                    if chrome::button(ui, "Cancel", ButtonKind::Secondary, None, true).clicked() {
                        act = Some(Act::Cancel);
                    }
                }
                (chrome::Part::ActionsRight, None) => {
                    let ok = !busy && s.until(now).is_ok();
                    let (glyph, word) = if busy {
                        (Glyph::Link, "Creating…")
                    } else if s.error.is_some() {
                        (Glyph::Refresh, "Try again")
                    } else {
                        (Glyph::Link, "Create link")
                    };
                    if draw::icon_label_button(ui, glyph, word, ButtonKind::Primary, ok).clicked() {
                        act = Some(Act::Create);
                    }
                }
                (chrome::Part::ActionsLeft, Some(_)) => {
                    if text_link(ui, "See all your share links").clicked() {
                        act = Some(Act::AllLinks);
                    }
                }
                (chrome::Part::ActionsRight, Some(_)) => {
                    if chrome::button(ui, "Done", ButtonKind::Secondary, Some(Icon::Check), true).clicked() {
                        act = Some(Act::Cancel);
                    }
                }
                (chrome::Part::HeroRight, _) => {}
            },
        );
        if let (Some(month), Some(field), Some(now)) = (s.month, date_field, now) {
            calendar(ui, field, month, now.today(), s);
        }
        match act {
            None => {}
            Some(Act::Cancel) => self.sheet = None,
            Some(Act::AllLinks) => {
                self.sheet = None;
                self.go_to_links = true;
            }
            Some(Act::Create) => {
                s.error = None;
                s.month = None;
                if let Some(req) = s.request(&self.server, now) {
                    self.net.send(backend, Key::Share(s.item.id.clone()), req);
                }
            }
            Some(Act::Copy(url)) => {
                ui.ctx().copy_text(url);
                s.copied = true;
                self.notice_detail("Link copied", "Paste it into a message to the people you share with.");
            }
        }
    }
}

/// The item being shared: its glyph, name, and "<format> · <size> · in <place>".
fn item_box(ui: &mut Ui, item: &Item) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().fill(t.sunken).stroke(t.hairline()).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 38.0), Sense::hover());
            draw::file_glyph(ui, Rect::from_center_size(r.center(), vec2(20.0, 20.0)), item);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(RichText::new(&item.name).size(14.0).color(t.ink_strong)).truncate());
                let mut facts = vec![item.kind_label()];
                if let Some(size) = item.size {
                    facts.push(crate::web::bytes(size));
                }
                facts.push(format!("in {}", location(item)));
                ui.add(egui::Label::new(RichText::new(facts.join(" · ")).size(12.0).color(t.muted)).truncate());
            });
        });
    });
}

/// A 34 px sunken text field `width` wide (the password is masked, also to screen readers).
fn input(ui: &mut Ui, value: &mut String, password: bool, width: f32, hint: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.extreme_bg_color = t.sunken;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.line);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.line_strong);
        v.selection.stroke = Stroke::new(2.0, t.accent_ink);
        let mut edit = egui::TextEdit::singleline(value).password(password).margin(Margin::symmetric(10, 8)).font(FontId::proportional(14.0));
        if !hint.is_empty() {
            edit = edit.hint_text(RichText::new(hint).color(t.faint));
        }
        ui.add_sized(vec2(width.max(80.0), chrome::CONTROL_H), edit)
    })
    .inner
}

/// The read-only link: Mono 12 in a sunken box, cut with an ellipsis; the whole URL is its label.
fn url_box(ui: &mut Ui, url: &str) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), chrome::CONTROL_H), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, url));
    ui.painter().rect(rect, 0.0, t.sunken, t.hairline(), egui::StrokeKind::Inside);
    let g = draw::fit_text(ui, url, FontId::monospace(12.0), t.ink, rect.width() - 20.0);
    ui.painter().galley(pos2(rect.left() + 10.0, rect.center().y - g.size().y / 2.0), g, t.ink);
}

/// The server's refusal, with the NOT MADE badge.
fn error_box(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().fill(t.sunken).stroke(t.hairline()).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            chrome::badge(ui, "Not made", t.bad);
            ui.add(egui::Label::new(RichText::new(text).size(13.0).color(t.bad)).wrap());
        });
    });
}

/// The date picker under the date field: the month, Monday first; past days can't be picked.
fn calendar(ui: &mut Ui, field: Rect, month: Date, today: Date, s: &mut ShareSheet) {
    let t = Tokens::get(ui.ctx());
    let (cell, pad) = (vec2(34.0, 30.0), 12.0);
    let picked = read_date(&s.date_text);
    egui::Area::new(ui.scope_id().with("share-calendar")).order(egui::Order::Foreground).fixed_pos(field.left_bottom() + vec2(0.0, 4.0)).show(
        ui.ctx(),
        |ui| {
            egui::Frame::new()
                .fill(t.bg)
                .stroke(Stroke::new(1.0, t.line_strong))
                .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 140 } else { 36 }) })
                .inner_margin(Margin::same(pad as i8))
                .show(ui, |ui| {
                    let w = cell.x * 7.0;
                    ui.set_width(w);
                    ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
                    // Month row.
                    let (row, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                    let before = month.checked_sub(1.month()).ok().filter(|m| *m >= today.first_of_month());
                    let after = month.checked_add(1.month()).ok();
                    for (r, glyph, label, to) in [
                        (Rect::from_min_size(row.min, vec2(28.0, 28.0)), Glyph::Prev, "Previous month", before),
                        (Rect::from_min_size(pos2(row.right() - 28.0, row.top()), vec2(28.0, 28.0)), Glyph::Next, "Next month", after),
                    ] {
                        let resp = ui.interact(r, ui.scope_id().with(label), if to.is_some() { Sense::click() } else { Sense::hover() });
                        ui.painter().rect(r, 0.0, if resp.hovered() { t.sunken } else { t.bg }, Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
                        draw::paint_glyph(ui, glyph, r.center(), if to.is_some() { t.ink } else { t.faint });
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, to.is_some(), label));
                        if resp.clicked() {
                            s.month = to;
                        }
                    }
                    let title = month.strftime("%B %Y").to_string();
                    ui.painter().text(row.center(), Align2::CENTER_CENTER, &title, FontId::proportional(14.0), t.ink_strong);
                    ui.add_space(4.0);
                    // Weekdays.
                    let (days_row, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                    for (i, d) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
                        let c = pos2(days_row.left() + cell.x * (i as f32 + 0.5), days_row.center().y);
                        let g = chrome::hud_galley(ui, d, t.muted);
                        ui.painter().galley(c - g.size() / 2.0, g, t.muted);
                    }
                    // Days.
                    let lead = i32::from(month.weekday().to_monday_zero_offset());
                    let count = i32::from(month.days_in_month());
                    let rows = (lead + count + 6) / 7;
                    let (grid, _) = ui.allocate_exact_size(vec2(w, cell.y * rows as f32 + 6.0 * (rows - 1) as f32), Sense::hover());
                    for day in 1..=count {
                        let Ok(date) = month.with().day(day as i8).build() else { continue };
                        let n = lead + day - 1;
                        let r = Rect::from_min_size(grid.min + vec2(cell.x * (n % 7) as f32, (cell.y + 6.0) * (n / 7) as f32), cell);
                        let past = date < today;
                        let resp = ui.interact(r, ui.scope_id().with(("day", day)), if past { Sense::hover() } else { Sense::click() });
                        let on = picked == Some(date);
                        let (fill, color) = if on {
                            (t.invert_bg, t.invert_fg)
                        } else if resp.hovered() && !past {
                            (t.sunken, t.ink_strong)
                        } else {
                            (Color32::TRANSPARENT, if past { t.faint } else { t.ink })
                        };
                        ui.painter().rect_filled(r, 0.0, fill);
                        if date == today && !on {
                            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
                        }
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, day.to_string(), FontId::proportional(13.0), color);
                        let name = date_text(date);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, !past, on, &name));
                        if resp.clicked() && !past {
                            s.date_text = name;
                            s.month = None;
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new("Past days can't be picked.").size(12.0).color(t.muted));
                });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{Expiry, Now, Secret, ShareSheet, expires_line, read_date};
    use crate::backend::WallClock;

    fn now() -> Option<Now> {
        // 2026-11-14 13:32 UTC, seen at UTC+1.
        Now::from(Some(WallClock { unix: 1_794_663_120.0, utc_offset_minutes: 60 }))
    }

    #[test]
    fn dates_read_as_written() {
        let d = read_date("28 Nov 2026");
        assert_eq!(d, read_date("2026-11-28"));
        assert_eq!(d, read_date(" 28 November 2026 "));
        assert_eq!(read_date("someday"), None);
    }

    fn sheet() -> ShareSheet {
        ShareSheet::new(super::Item {
            kind: craftcloud_core::api::resources::Kind::File,
            id: "f1".into(),
            name: "a.txt".into(),
            size: Some(8),
            modified: String::new(),
            mime: String::new(),
            category: String::new(),
            starred: false,
            shared: false,
            path: "Documents/a.txt".into(),
            etag: String::new(),
            parent: None,
            expires: None,
            trashed: None,
            opened: None,
        })
    }

    #[test]
    fn the_password_stays_out_of_debug_output() {
        let mut s = sheet();
        s.password = Secret("maple-harbour-41".into());
        assert!(!format!("{s:?}").contains("maple"), "{s:?}");
        assert_eq!(super::location(&s.item), "My files / Documents");
    }

    #[test]
    fn hints_say_when_the_link_stops() {
        let mut s = sheet();
        assert_eq!(s.hint(now()).0, "The link stops working on 21 Nov 2026 at 14:32.");
        s.expiry = Expiry::Never;
        assert_eq!(s.hint(now()).0, "The link works until you revoke it.");
        s.expiry = Expiry::Date;
        s.date_text = "28 Nov 2026".into();
        assert_eq!(s.hint(now()).0, "The link stops working at the end of 28 Nov 2026.");
        s.date_text = "13 Nov 2026".into();
        assert!(s.hint(now()).1, "a past day is wrong");
        assert_eq!(expires_line(Some("2026-11-21T13:32:00Z"), now()), "21 Nov 2026, 14:32 · in 7 days");
        assert_eq!(expires_line(Some("2026-11-28T23:00:00Z"), now()), "end of 28 Nov 2026 · in 14 days");
        assert_eq!(expires_line(None, now()), "Never");
    }
}
