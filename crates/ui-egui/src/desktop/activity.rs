//! The ACTIVITY tab (design frames `v2-desktop-activity`, `-syncing`, `-states`): the headline
//! with the state in it, the meta line, SYNC NOW / PAUSE / OPEN FOLDER, a compact panel for any
//! state that needs the person, and the activity list. Also the Conflict sheet.

use craftcloud_core::sync::{ActivityEvent, ActivityKind, FileStatus, Keep, SyncProgress, SyncState};
use egui::{Color32, Rect, Sense, Ui, vec2};

use super::{DesktopApp, DesktopHost, Overlay, folders::thousands, host_of};
use crate::chrome::{self, Hero, Icon, Kind, Part, desk};
use crate::theme::Tokens as Look;
use crate::web::bytes;

/// A headline: two lines, each pieces with the accent ones marked.
type Lines = [Vec<(String, bool)>; 2];

fn plain(s: &str) -> (String, bool) {
    (s.to_owned(), false)
}

fn accent(s: &str) -> (String, bool) {
    (s.to_owned(), true)
}

/// What the headline says about the state.
fn headline_for(p: &SyncProgress) -> Lines {
    let conflicts = p.conflicts.len();
    match &p.state {
        SyncState::Downloading if p.files_total > 0 => {
            let done = (p.files_done + 1).min(p.files_total);
            [vec![plain("Syncing")], vec![accent(&format!("{done} of {}", p.files_total)), plain(" files.")]]
        }
        SyncState::Idle | SyncState::Scanning | SyncState::Downloading => [vec![plain("Checking")], vec![plain("the "), accent("cloud"), plain("…")]],
        SyncState::UpToDate if !p.no_room.is_empty() || !p.not_synced.is_empty() => {
            [vec![plain("Almost everything")], vec![plain("is "), accent("synced"), plain(".")]]
        }
        SyncState::UpToDate if conflicts > 0 => {
            let what = if conflicts == 1 { "1 file".to_owned() } else { format!("{conflicts} files") };
            [vec![plain(&what), plain(" changed")], vec![plain("in "), accent("two places"), plain(".")]]
        }
        // Download only (docs/WEBDAV-STORAGE.md § 9): the cloud is here; changes made here stay here.
        SyncState::UpToDate if p.mode == Some(craftcloud_core::storage::Mode::DownloadOnly) => {
            [vec![plain("Everything")], vec![plain("is "), accent("downloaded"), plain(".")]]
        }
        SyncState::UpToDate => [vec![plain("Everything")], vec![plain("is "), accent("synced"), plain(".")]],
        SyncState::Paused => [vec![plain("Syncing")], vec![plain("is "), accent("paused"), plain(".")]],
        SyncState::Offline(_) => [vec![plain("Can't reach")], vec![plain("the "), accent("server"), plain(".")]],
        SyncState::SignedOut => [vec![plain("Signed")], vec![accent("out"), plain(".")]],
        SyncState::MassDelete { .. } => [vec![plain("Waiting")], vec![plain("for "), accent("you"), plain(".")]],
        SyncState::FolderMissing => [vec![plain("The folder")], vec![plain("is "), accent("gone"), plain(".")]],
        SyncState::Failed(_) => [vec![plain("Something")], vec![plain("went "), accent("wrong"), plain(".")]],
    }
}

/// The headline's marked word: `bad` for a state that stopped syncing (its badge is `bad` too),
/// the accent otherwise.
fn headline_tone(p: &SyncProgress, t: &Look) -> Color32 {
    match p.state {
        SyncState::FolderMissing | SyncState::Offline(_) | SyncState::Failed(_) => t.bad,
        _ => t.accent_ink,
    }
}

/// The badge for an activity event.
pub(super) fn kind_badge(kind: ActivityKind, t: &Look) -> (&'static str, Color32) {
    match kind {
        ActivityKind::Up => ("Up", t.good),
        ActivityKind::Down => ("Down", t.accent_ink),
        ActivityKind::Deleted => ("Deleted", t.bad),
        ActivityKind::Moved => ("Moved", t.accent_ink),
        ActivityKind::Conflict => ("Conflict", t.warn),
        ActivityKind::Error => ("Error", t.bad),
        ActivityKind::Skipped => ("Not synced", t.muted),
        ActivityKind::Warning => ("Warning", t.warn),
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "14:32" today, "12 Nov" before.
fn when(e: &ActivityEvent, today: &str) -> String {
    if e.date == today || e.date.len() < 10 {
        return e.time.clone();
    }
    let month = e.date[5..7].parse::<usize>().ok().and_then(|m| MONTHS.get(m.wrapping_sub(1))).copied().unwrap_or("");
    format!("{} {month}", e.date[8..10].trim_start_matches('0'))
}

/// The row's right-hand text: size, then a few words.
fn extra(e: &ActivityEvent) -> String {
    match (e.size, e.detail.is_empty()) {
        (Some(s), true) => bytes(s),
        (Some(s), false) => format!("{} · {}", bytes(s), e.detail),
        (None, _) => e.detail.clone(),
    }
}

/// A state panel's button: what it asks for, its label, kind and icon.
type Offer = (Ask, &'static str, Kind, Option<Icon>);

/// Whether the state needs a panel under the hero (see [`DesktopApp::state_panel_ui`]).
fn needs_panel(p: &SyncProgress) -> bool {
    !matches!(p.state, SyncState::Idle | SyncState::Scanning | SyncState::Downloading | SyncState::UpToDate)
        || !p.conflicts.is_empty()
        || !p.no_room.is_empty()
}

/// One row of the activity list.
struct Line {
    when: String,
    tag: (&'static str, Color32),
    path: String,
    extra: String,
    /// `None` for a file still on its way.
    kind: Option<ActivityKind>,
}

/// How wide the path and the extra are drawn, out of the `room` the row leaves them (a 14 gap
/// between them). Both fit when they can; otherwise the path keeps up to 40% of the room (less
/// if it is shorter), the extra gets the rest, and each is cut with an ellipsis. Never wider than
/// the room: a long error detail used to spill out of the panel over the time and the badge.
fn split_row(room: f32, path: f32, extra: f32) -> (f32, f32) {
    const GAP: f32 = 14.0;
    let room = (room - GAP).max(0.0);
    if path + extra <= room {
        return (room - extra, extra);
    }
    let extra_w = extra.min(room - path.min(room * 0.4));
    (room - extra_w, extra_w)
}

/// Time · badge · path (fills, truncated) · extra (truncated, full text on hover), 14 apart.
fn line_ui(ui: &mut Ui, line: &Line, t: &Look) {
    ui.spacing_mut().item_spacing.x = 14.0;
    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 20.0), Sense::hover());
    let g = ui.painter().layout_no_wrap(line.when.clone(), egui::FontId::monospace(12.0), t.muted);
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.muted);
    desk::badge_fixed(ui, line.tag.0, line.tag.1, desk::BADGE_W);
    let path_text = egui::RichText::new(&line.path).size(13.0).color(t.ink);
    let extra_text = egui::RichText::new(&line.extra).size(12.0).color(t.muted);
    let path_g = ui.painter().layout_no_wrap(line.path.clone(), egui::FontId::proportional(13.0), t.ink);
    let extra_g = ui.painter().layout_no_wrap(line.extra.clone(), egui::FontId::proportional(12.0), t.muted);
    let (path_w, extra_w) = split_row(ui.available_width(), path_g.size().x, extra_g.size().x);
    ui.allocate_ui_with_layout(vec2(path_w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(path_w);
        ui.add(egui::Label::new(path_text).truncate().selectable(false));
    });
    if !line.extra.is_empty() {
        ui.allocate_ui_with_layout(vec2(extra_w, 20.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.set_width(extra_w);
            ui.add(egui::Label::new(extra_text).truncate().selectable(false));
        });
    }
}

/// What a state panel offers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ask {
    Resume,
    Retry,
    SignIn,
    DeleteThem,
    Keep,
    Recreate,
    Review,
}

impl<H: DesktopHost> DesktopApp<H> {
    pub(super) fn activity_ui(&mut self, ui: &mut Ui, p: &SyncProgress) {
        // A tall window keeps the hero in place and scrolls the list inside its frame; a short
        // one scrolls the whole tab, so the list never shrinks to nothing under a state panel.
        if ui.available_height() < 460.0 {
            egui::ScrollArea::vertical().id_salt("craftcloud-activity").auto_shrink([false, false]).show(ui, |ui| {
                self.hero_ui(ui, p);
                self.list_ui(ui, p, None);
            });
        } else {
            self.hero_ui(ui, p);
            let rest = ui.available_height();
            self.list_ui(ui, p, Some(rest));
        }
    }

    fn hero_ui(&mut self, ui: &mut Ui, p: &SyncProgress) {
        {
            ui.spacing_mut().item_spacing.y = 0.0;
            let lines = headline_for(p);
            let pieces: Vec<Vec<(&str, bool)>> = lines.iter().map(|l| l.iter().map(|(s, a)| (s.as_str(), *a)).collect()).collect();
            let refs: Vec<&[(&str, bool)]> = pieces.iter().map(Vec::as_slice).collect();
            let t = Look::get(ui.ctx());
            chrome::headline_toned(ui, 40.0, &refs, headline_tone(p, &t));
            ui.add_space(12.0);
            self.meta_ui(ui, p);
            if matches!(p.state, SyncState::Downloading) && p.files_total > 0 {
                ui.add_space(14.0);
                let t = Look::get(ui.ctx());
                let current = p.current.split_once(' ').map_or(p.current.as_str(), |(_, path)| path);
                let caption = if current.is_empty() { format!("{} of {} files", p.files_done, p.files_total) } else { current.to_owned() };
                ui.add(egui::Label::new(egui::RichText::new(caption).size(12.0).color(t.ink)).truncate());
                ui.add_space(6.0);
                chrome::meter(ui, p.files_done as f32 / p.files_total as f32);
            }
            ui.add_space(16.0);
            let panel = needs_panel(p);
            self.actions_row(ui, p, panel);
            if let Some(ask) = self.state_panel_ui(ui, p) {
                self.on_ask(ask, p);
            }
            ui.add_space(24.0);
        }
    }

    fn meta_ui(&mut self, ui: &mut Ui, p: &SyncProgress) {
        let folder = self.short_folder();
        let host = self.server.as_ref().map(host_of).unwrap_or_default();
        let files = p.files_synced();
        let count = format!("{} files", thousands(files));
        let mut parts = vec![folder.as_str(), host.as_str()];
        if files > 0 {
            parts.push(&count);
        }
        // A missing folder can't be opened: the meta line keeps the place but drops the link.
        let link = (p.state != SyncState::FolderMissing).then_some("Open folder");
        if desk::meta_link(ui, &parts, link).is_some_and(|r| r.clicked()) {
            let path = self.settings.local_folder.clone();
            self.host.open_folder(&path);
        }
    }

    /// SYNC NOW / PAUSE / OPEN FOLDER. With a state panel below, its action is the primary one:
    /// SYNC NOW steps back to secondary, and while paused only the panel offers RESUME.
    /// While the folder is missing, SYNC NOW and OPEN FOLDER are off (the panel's LOOK AGAIN
    /// is the way on), and say why on hover.
    fn actions_row(&mut self, ui: &mut Ui, p: &SyncProgress, panel: bool) {
        let paused = p.state == SyncState::Paused;
        let missing = p.state == SyncState::FolderMissing;
        const WHY: &str = "The CraftCloud folder is missing. Put it back, then Look again.";
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            if !paused {
                let kind = if panel { Kind::Secondary } else { Kind::Primary };
                let sync = chrome::button(ui, "Sync now", kind, Some(Icon::Refresh), !p.state.is_busy() && !missing);
                if missing {
                    sync.on_disabled_hover_text(WHY);
                } else if sync.clicked() {
                    self.host.sync_now();
                }
                if chrome::button(ui, "Pause", Kind::Secondary, Some(Icon::Pause), true).clicked() {
                    self.host.pause(true);
                }
            }
            let open = chrome::button(ui, "Open folder", Kind::Secondary, Some(Icon::Folder), !missing);
            if missing {
                open.on_disabled_hover_text(WHY);
            } else if open.clicked() {
                let path = self.settings.local_folder.clone();
                self.host.open_folder(&path);
            }
            if missing {
                let t = Look::get(ui.ctx());
                ui.label(egui::RichText::new("Off until the folder is back").size(12.0).color(t.muted));
            }
        });
    }

    /// The compact panel for a state that needs the person; returns what they picked.
    fn state_panel_ui(&mut self, ui: &mut Ui, p: &SyncProgress) -> Option<Ask> {
        let t = Look::get(ui.ctx());
        let host = self.server.as_ref().map(host_of).unwrap_or_default();
        let folder = self.short_folder();
        let (tag, tone, title, text, asks): (&str, Color32, String, String, &[Offer]) = match &p.state {
            SyncState::Paused => (
                "Paused",
                t.muted,
                "Syncing is paused".into(),
                "Nothing goes up or down until you resume. Changes made meanwhile are kept and sync then.".into(),
                &[(Ask::Resume, "Resume", Kind::Primary, Some(Icon::Play))],
            ),
            SyncState::Offline(_) => (
                "Offline",
                t.bad,
                "Server unreachable".into(),
                format!("Can't reach {host}. Local changes are safe; the client tries again every 15 seconds."),
                &[(Ask::Retry, "Retry now", Kind::Primary, Some(Icon::Refresh))],
            ),
            SyncState::SignedOut => (
                "Signed out",
                t.warn,
                "Sign in again".into(),
                "The session ended (the password changed, or this device was removed). Nothing was deleted.".into(),
                &[(Ask::SignIn, "Sign in again", Kind::Primary, Some(Icon::Arrow))],
            ),
            SyncState::MassDelete { deletes } => (
                "Mass delete",
                t.warn,
                format!("This would delete {deletes} files"),
                format!("{deletes} files vanished at once, here or in the cloud. Their deletion waits until you decide; everything else keeps syncing. Keep brings them back."),
                &[(Ask::DeleteThem, "Delete them", Kind::Danger, Some(Icon::Cross)), (Ask::Keep, "Keep", Kind::Secondary, Some(Icon::Check))],
            ),
            SyncState::FolderMissing => (
                "Folder missing",
                t.bad,
                format!("{folder} is gone"),
                "Your CraftCloud folder was moved or deleted, or it's on a drive that isn't plugged in. Nothing was deleted in the cloud. Put it back where it was (or plug the drive in) and look again; or download everything again into a new, empty folder there; or sign out to choose another folder.".into(),
                &[
                    (Ask::Retry, "Look again", Kind::Primary, Some(Icon::Refresh)),
                    (Ask::Recreate, "Download everything again", Kind::Secondary, None),
                    (Ask::SignIn, "Sign out", Kind::Secondary, None),
                ],
            ),
            SyncState::Failed(why) => (
                "Error",
                t.bad,
                "Syncing stopped".into(),
                format!("{} It's tried again at the next cycle.", sentence(why)),
                &[(Ask::Retry, "Try again", Kind::Primary, Some(Icon::Refresh))],
            ),
            _ if !p.no_room.is_empty() => {
                let n = p.no_room.len();
                let first = name_of(&p.no_room[0]).to_owned();
                (
                    "No room",
                    t.warn,
                    if n == 1 { format!("{first} doesn't fit in the cloud") } else { format!("{n} files don't fit in the cloud") },
                    "The cloud's storage quota is full. They stay on this computer and sync once there is room (free some space, or ask your admin for more), or when you try again.".into(),
                    &[(Ask::Retry, "Try again", Kind::Primary, Some(Icon::Refresh))],
                )
            }
            _ if !p.conflicts.is_empty() => {
                let c = &p.conflicts[0];
                let more = if p.conflicts.len() > 1 { format!(" (and {} more)", p.conflicts.len() - 1) } else { String::new() };
                (
                    "Conflict",
                    t.warn,
                    format!("{} changed in two places{more}", name_of(&c.original)),
                    "Both versions are kept. Decide which one stays, or keep both as separate files.".into(),
                    &[(Ask::Review, "Review", Kind::Primary, Some(Icon::Arrow))],
                )
            }
            _ => return None,
        };
        ui.add_space(16.0);
        let mut picked = None;
        desk::state_panel(ui, tag, tone, &title, &text, |ui| {
            for (ask, label, kind, icon) in asks {
                if chrome::button(ui, label, *kind, *icon, true).clicked() {
                    picked = Some(*ask);
                }
            }
        });
        picked
    }

    fn on_ask(&mut self, ask: Ask, p: &SyncProgress) {
        match ask {
            Ask::Resume => self.host.pause(false),
            Ask::Retry => self.host.sync_now(),
            Ask::Keep => self.host.discard_deletes(),
            Ask::Recreate => self.host.recreate_folder(),
            // The server ended the sign-in already: nothing to end there.
            Ask::SignIn if matches!(p.state, SyncState::SignedOut) => self.sign_out(),
            // "Sign out" to choose another folder: the sign-in ends on the server too.
            Ask::SignIn => self.sign_out_everywhere(),
            Ask::DeleteThem => self.host.confirm_deletes(),
            Ask::Review => {
                if let Some(c) = p.conflicts.first() {
                    self.overlay = Some(Overlay::Conflict(c.copy.clone()));
                }
            }
        }
    }

    /// The activity list; `height` is the room it has (it scrolls inside its frame), `None` when
    /// the whole tab scrolls.
    fn list_ui(&mut self, ui: &mut Ui, p: &SyncProgress, height: Option<f32>) {
        let t = Look::get(ui.ctx());
        let today = self.host.clock().date;
        let all_today = p.activity.iter().all(|e| e.date == today);
        chrome::meta(ui, &["Activity", if all_today { "Today" } else { "Recent" }]);
        ui.add_space(8.0);
        // Files on their way first (at most a handful), then what happened, newest first.
        let mut lines: Vec<Line> = p
            .unsynced
            .iter()
            .filter(|(_, s)| **s == FileStatus::Syncing)
            .take(5)
            .map(|(path, _)| Line {
                when: "now".into(),
                tag: ("Syncing", t.accent_ink),
                path: path.clone(),
                extra: if p.current.ends_with(path.as_str()) { "in progress".into() } else { "waiting".into() },
                kind: None,
            })
            .collect();
        lines.extend(p.activity.iter().rev().map(|e| Line {
            when: when(e, &today),
            tag: kind_badge(e.kind, &t),
            path: if e.path.is_empty() { "—".into() } else { e.path.clone() },
            extra: extra(e),
            kind: Some(e.kind),
        }));
        if lines.is_empty() {
            desk::list_frame(ui, |ui| {
                desk::row(ui, 64.0, true, Sense::hover(), |ui| {
                    let folder = self.short_folder();
                    ui.label(
                        egui::RichText::new(format!("Nothing yet. Files you add to {folder} show up here as they sync.")).size(13.0).color(t.muted),
                    );
                });
            });
            return;
        }
        let mut clicked: Option<(String, Option<ActivityKind>)> = None;
        let mut rows = |ui: &mut Ui| {
            let n = lines.len();
            for (i, line) in lines.iter().enumerate() {
                let (resp, ()) = desk::row(ui, desk::ROW_H, i + 1 == n, Sense::click(), |ui| line_ui(ui, line, &t));
                let what = if line.kind == Some(ActivityKind::Conflict) { "Review" } else { "Open" };
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{what} {}", line.path)));
                let resp = resp.on_hover_text(&line.path);
                if resp.clicked() {
                    clicked = Some((line.path.clone(), line.kind));
                }
            }
        };
        desk::list_frame(ui, |ui| match height {
            Some(h) => {
                // The meta line and its gap above take 28.
                let h = (h - 30.0).max(desk::ROW_H * 3.0);
                egui::ScrollArea::vertical().id_salt("craftcloud-activity-list").max_height(h).auto_shrink([false, true]).show(ui, rows);
            }
            None => rows(ui),
        });
        match clicked {
            Some((path, Some(ActivityKind::Conflict))) => {
                if let Some(c) = p.conflicts.iter().find(|c| c.original == path) {
                    self.overlay = Some(Overlay::Conflict(c.copy.clone()));
                }
            }
            Some((path, kind)) if !matches!(kind, Some(ActivityKind::Deleted)) => self.host.open_file(&path),
            _ => {}
        }
    }

    /// The Conflict sheet over Activity (design frame `v2-desktop-conflict`).
    pub(super) fn conflict_ui(&mut self, ui: &mut Ui, area: Rect, p: &SyncProgress, copy: &str) {
        let Some(conflict) = p.conflicts.iter().find(|c| c.copy == copy).cloned() else {
            // Settled meanwhile (or from another computer).
            self.overlay = None;
            return;
        };
        let t = Look::get(ui.ctx());
        let name = name_of(&conflict.original).to_owned();
        let title = format!("{name} changed in two places");
        let cloud = self.host.local_file(&conflict.original);
        let mine = self.host.local_file(&conflict.copy);
        let device = self.device.clone();
        let mut picked: Option<Option<Keep>> = None;
        chrome::sheet(ui, area, vec2(720.0, 470.0), &Hero { title: &title, caption: "Conflict", keep_case: false }, |ui, part| match part {
            Part::HeroRight => {}
            Part::Body => {
                chrome::body(ui, "It was saved on this computer and in the cloud before either change reached the other. Nothing is lost.", 14.0);
                ui.add_space(8.0);
                ui.columns(2, |cols| {
                    version_panel(&mut cols[0], "Cloud", "The cloud's version", cloud.as_ref());
                    version_panel(&mut cols[1], "This computer", &format!("Yours, on {device}"), mine.as_ref());
                });
                ui.add_space(8.0);
                egui::Frame::new().fill(t.sunken).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new("Keep both leaves the cloud file as it is and saves yours next to it as").size(13.0).color(t.ink));
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(egui::RichText::new(name_of(&conflict.copy)).font(egui::FontId::monospace(12.0)).color(t.ink_strong)).wrap(),
                    );
                });
                ui.add_space(8.0);
                chrome::body(
                    ui,
                    "Keep mine or Keep cloud replaces the other version; the replaced cloud file is kept in .craftcloud/versions/.",
                    12.5,
                );
            }
            Part::ActionsLeft => {
                if chrome::button(ui, "Decide later", Kind::Secondary, None, true).clicked() {
                    picked = Some(None);
                }
            }
            Part::ActionsRight => {
                if chrome::button(ui, "Keep both", Kind::Primary, Some(Icon::Check), true).clicked() {
                    picked = Some(Some(Keep::Both));
                }
                if chrome::button(ui, "Keep mine", Kind::Secondary, None, true).clicked() {
                    picked = Some(Some(Keep::Mine));
                }
                if chrome::button(ui, "Keep cloud", Kind::Secondary, None, true).clicked() {
                    picked = Some(Some(Keep::Cloud));
                }
            }
        });
        if let Some(keep) = picked {
            if let Some(keep) = keep {
                self.host.resolve(&conflict, keep);
            }
            self.overlay = None;
        }
    }
}

fn version_panel(ui: &mut Ui, title: &str, place: &str, facts: Option<&super::FileFacts>) {
    let t = Look::get(ui.ctx());
    chrome::panel(ui, title, |ui| {
        ui.label(egui::RichText::new(place).size(16.0).color(t.ink_strong));
        ui.add_space(4.0);
        match facts {
            Some(f) => chrome::meta(ui, &[&format!("Saved {}", f.modified), &bytes(f.size)]),
            None => chrome::meta(ui, &["Not on disk"]),
        }
    });
}

/// `why` as a sentence: it ends with one full stop, never two. Engine messages may already end
/// with one (often right after a path, as in "is already syncing C:\Users\john\CraftCloud.").
fn sentence(why: &str) -> String {
    let why = why.trim_end();
    if why.ends_with(['.', '!', '?']) { why.to_owned() } else { format!("{why}.") }
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
