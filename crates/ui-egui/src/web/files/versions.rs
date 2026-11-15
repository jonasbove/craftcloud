//! A file's version history (the "Versions…" sheet): the current file and the versions CraftCloud
//! kept in `.craftcloud/versions/<path>/` (the newest 5), each to download or restore.
//!
//! Restoring never loses the current file, and never overwrites a change made meanwhile:
//! 1. `PROPFIND` the file for its current ETag,
//! 2. `GET` the version's bytes (before anything is pruned),
//! 3. keep the current file as a version of its own (the `MKCOL`s, then a `COPY` that never
//!    overwrites), unless the newest kept version already holds its bytes,
//! 4. prune the history to the newest 5,
//! 5. `PUT` the version's bytes with `If-Match: <etag>`: a 412 means the file changed meanwhile,
//!    and nothing is written.
//!
//! If a step before the `PUT` fails, the restore stops there and the file stays as it was.

use std::collections::VecDeque;

use craftcloud_core::api::dav::{self, Uploaded, Write};
use craftcloud_core::api::versions::{self, Version};
use craftcloud_core::http::{HttpRequest, Method};
use egui::{Align2, FontId, Rect, RichText, Sense, Ui, pos2, vec2};

use super::model::{Item, guess_mime};
use super::sheets::Sheet;
use super::{Files, Key, time};
use crate::backend::{Backend, NetResult, WallClock};
use crate::chrome::{self, Kind as ButtonKind};
use crate::theme::Tokens;
use crate::widgets::error_line;

/// The open "Versions…" sheet.
#[derive(Debug)]
pub struct History {
    pub item: Item,
    /// The kept versions, newest first; `None` while loading.
    pub list: Option<Vec<Version>>,
    pub error: Option<String>,
    /// The version being restored.
    pub restoring: Option<String>,
}

/// What the person picked in the sheet.
pub enum Pick {
    /// Download the current file (`None`) or a kept version.
    Download(Option<String>),
    Restore(String),
}

/// One step of a restore.
#[derive(Clone, Debug)]
enum Step {
    Stat,
    Fetch,
    /// A `MKCOL` (201, or 405 when it is there) or the snapshot's `COPY`.
    Plain(HttpRequest),
    Prune,
    Put,
}

/// A restore under way.
#[derive(Default)]
pub struct Run {
    path: String,
    stamp: String,
    steps: VecDeque<Step>,
    etag: Option<String>,
    bytes: Vec<u8>,
    /// The steps that keep the current file, dropped when it is kept already.
    keep: Vec<HttpRequest>,
    /// The kept versions when the restore started, for [`versions::already_kept`].
    known: Vec<Version>,
    when: String,
}

impl Files {
    /// Opens the version history of the file at `index`.
    pub(super) fn open_versions(&mut self, backend: &mut dyn Backend, index: usize) {
        let Some(item) = self.items.get(index).cloned() else { return };
        if item.is_folder() || item.path.is_empty() {
            return;
        }
        let req = versions::list(&self.server, &item.path);
        self.sheet_error = None;
        self.sheet = Some(Sheet::Versions(Box::new(History { item, list: None, error: None, restoring: None })));
        self.versions_generation += 1;
        self.net.send(backend, Key::Versions(self.versions_generation), req);
    }

    /// The answer to the versions listing.
    pub(super) fn on_versions(&mut self, generation: u64, result: NetResult) {
        if generation != self.versions_generation {
            return;
        }
        let Some(Sheet::Versions(h)) = &mut self.sheet else { return };
        match result.and_then(|resp| versions::parse_list(&resp).map_err(|e| e.to_string())) {
            Ok(list) => h.list = Some(list),
            Err(why) => {
                h.list = Some(Vec::new());
                h.error = Some(format!("Couldn't list the versions: {why}"));
            }
        }
    }

    /// What the person picked in the sheet.
    pub(super) fn versions_pick(&mut self, backend: &mut dyn Backend, pick: Pick) {
        let Some(Sheet::Versions(h)) = &mut self.sheet else { return };
        let path = h.item.path.clone();
        match pick {
            Pick::Download(None) => {
                let name = h.item.name.clone();
                backend.download(&dav::download(&self.server, &path).url, &name);
            }
            Pick::Download(Some(stamp)) => {
                let name = h.item.name.clone();
                backend.download(&dav::download(&self.server, &versions::version_path(&path, &stamp)).url, &name);
            }
            Pick::Restore(stamp) => {
                if h.restoring.is_some() || self.version_run.is_some() {
                    return;
                }
                let known = h.list.clone().unwrap_or_default();
                let when = known.iter().find(|v| v.stamp == stamp).map(|v| saved_when(v, self.clock)).unwrap_or_default();
                h.restoring = Some(stamp.clone());
                h.error = None;
                let now = time::stamp(backend.wall_clock().map_or(0.0, |c| c.unix));
                let mut keep = versions::make_folders(&self.server, &path);
                keep.push(versions::snapshot(&self.server, &path, &now));
                let steps = VecDeque::from([Step::Stat, Step::Fetch]);
                self.version_run = Some(Run { path, stamp, steps, etag: None, bytes: Vec::new(), keep, known, when });
                self.version_next(backend);
            }
        }
    }

    fn version_next(&mut self, backend: &mut dyn Backend) {
        let Some(run) = &mut self.version_run else { return };
        let Some(step) = run.steps.front().cloned() else { return };
        let req = match step {
            Step::Stat => dav::stat(&self.server, &run.path),
            Step::Fetch => dav::download(&self.server, &versions::version_path(&run.path, &run.stamp)),
            Step::Plain(req) => req,
            Step::Prune => versions::list(&self.server, &run.path),
            Step::Put => {
                let bytes = std::mem::take(&mut run.bytes);
                let etag = run.etag.clone().unwrap_or_default();
                dav::upload(&self.server, &run.path, bytes, &Write::Replace { etag }).header("Content-Type", guess_mime(&run.path))
            }
        };
        self.net.send(backend, Key::VersionStep, req);
    }

    /// The answer to a step of a restore.
    pub(super) fn on_version_step(&mut self, backend: &mut dyn Backend, result: NetResult) {
        let Some(run) = &mut self.version_run else { return };
        let Some(step) = run.steps.pop_front() else { return };
        let resp = match result {
            Ok(r) => r,
            Err(why) => return self.version_failed(&why),
        };
        match step {
            Step::Stat => {
                let etag = dav::parse_list(&resp).ok().and_then(|e| e.into_iter().find(|e| !e.is_folder)).and_then(|e| e.etag);
                let Some(etag) = etag else { return self.version_failed("couldn't read the current file on the server") };
                if !versions::already_kept(&etag, &run.known) {
                    run.steps.extend(run.keep.drain(..).map(Step::Plain));
                    run.steps.push_back(Step::Prune);
                }
                run.steps.push_back(Step::Put);
                run.etag = Some(etag);
            }
            Step::Fetch => {
                if !resp.is_success() {
                    return self.version_failed(&format!("the server answered {} for the version", resp.status));
                }
                run.bytes = resp.body;
            }
            Step::Plain(req) => {
                let ok = if req.method == Method::Mkcol {
                    resp.is_success() || dav::folder_exists_after(&resp)
                } else {
                    versions::snapshot_done(&resp).is_ok()
                };
                if !ok {
                    return self.version_failed(&format!("couldn't keep the current file first (the server answered {})", resp.status));
                }
            }
            Step::Prune => {
                let names: Vec<String> = versions::parse_list(&resp).map(|v| v.into_iter().map(|v| v.stamp).collect()).unwrap_or_default();
                for old in versions::to_prune(names) {
                    // Best effort: a version left over is harmless.
                    self.net.send(backend, Key::Ignore, versions::delete(&self.server, &run.path, &old));
                }
            }
            Step::Put => {
                let outcome = dav::parse_upload(&resp);
                let when = run.when.clone();
                self.version_run = None;
                return match outcome {
                    Ok(Uploaded::Saved { .. }) => {
                        if matches!(self.sheet, Some(Sheet::Versions(_))) {
                            self.sheet = None;
                        }
                        let text = if when.is_empty() { "Restored the version".to_owned() } else { format!("Restored the version from {when}") };
                        self.notice(&text, false, None);
                        self.reload(backend);
                    }
                    Ok(Uploaded::Conflict) => self.version_failed("it changed on the server meanwhile; nothing was overwritten. Try again"),
                    Err(e) => self.version_failed(&e.to_string()),
                };
            }
        }
        self.version_next(backend);
    }

    fn version_failed(&mut self, why: &str) {
        self.version_run = None;
        let text = format!("Couldn't restore the version: {why}.");
        if let Some(Sheet::Versions(h)) = &mut self.sheet {
            h.restoring = None;
            h.error = Some(text);
        } else {
            self.notice(&text, true, None);
        }
    }
}

/// When a version was saved: its content's own time when its name says, else when it was replaced.
fn saved_when(v: &Version, clock: Option<WallClock>) -> String {
    match v.modified_at().or_else(|| v.replaced_at()) {
        Some(at) => time::when_exact(&at, clock),
        None => v.when(),
    }
}

/// "My files/Documents/Reports": where the file is.
pub fn place(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => format!("My files/{dir}"),
        None => "My files".into(),
    }
}

const ROW: f32 = 48.0;

/// The sheet's body: what it keeps, then the current file and each kept version.
pub fn body(ui: &mut Ui, h: &History, clock: Option<WallClock>) -> Option<Pick> {
    let t = Tokens::get(ui.ctx());
    let mut pick = None;
    ui.add(
        egui::Label::new(
            RichText::new(
                "Before CraftCloud saves over a file, from a computer or from the web, it keeps the old one. The last 5 are kept. \
                 Restoring keeps the current file as a version too, so nothing is lost.",
            )
            .size(14.0)
            .color(t.muted),
        )
        .wrap(),
    );
    ui.add_space(16.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width();
    // On a phone the size goes under the time, with DOWNLOAD beside it.
    let narrow = w < 480.0;
    let size_x = (w * 0.42).max(150.0);
    let row_h = if narrow { 60.0 } else { ROW };
    let top = ui.cursor().top();
    let (header, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
    let heads: &[(f32, &str)] = if narrow { &[(14.0, "Saved")] } else { &[(14.0, "Saved"), (size_x, "Size")] };
    for (x, label) in heads {
        let g = chrome::hud_galley(ui, label, t.muted);
        ui.painter().galley(pos2(header.left() + x, header.center().y - g.size().y / 2.0), g, t.muted);
    }
    ui.painter().hline(header.x_range(), header.bottom() - 0.5, t.hairline());
    let rows: Vec<(String, u64, Option<&Version>)> = std::iter::once((time::when_exact(&h.item.modified, clock), h.item.size.unwrap_or(0), None))
        .chain(h.list.iter().flatten().map(|v| (saved_when(v, clock), v.size, Some(v))))
        .collect();
    for (i, (when, size, version)) in rows.iter().enumerate() {
        let (r, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
        if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.sunken);
        }
        if i + 1 < rows.len() || h.list.as_ref().is_some_and(|l| !l.is_empty()) {
            ui.painter().hline(r.x_range(), r.bottom() - 0.5, t.hairline());
        }
        // Children that don't take room in the column: the row already did.
        let child = |ui: &mut Ui, rect: Rect, layout: egui::Layout, side: &str| {
            ui.new_child(egui::UiBuilder::new().id_salt(("version-row", i, side)).max_rect(rect).layout(layout))
        };
        let size_text = crate::web::bytes(*size);
        let mut download = false;
        let left_end = if narrow { r.right() - 110.0 } else { r.left() + size_x - 8.0 };
        let left = Rect::from_min_max(pos2(r.left() + 14.0, r.top()), pos2(left_end, r.bottom()));
        let mut cell = child(ui, left, egui::Layout::top_down(egui::Align::Min), "when");
        cell.spacing_mut().item_spacing = vec2(8.0, 2.0);
        cell.add_space(if narrow { 9.0 } else { 0.0 });
        let line = egui::Layout::left_to_right(egui::Align::Center);
        let line_h = if narrow { 22.0 } else { row_h };
        cell.allocate_ui_with_layout(vec2(left.width(), line_h), line, |ui| {
            ui.label(RichText::new(when).size(14.0).color(t.ink_strong));
            if version.is_none() {
                chrome::badge(ui, "Current", t.good);
            }
        });
        if narrow {
            cell.allocate_ui_with_layout(vec2(left.width(), 20.0), line, |ui| {
                ui.label(RichText::new(&size_text).size(12.0).family(egui::FontFamily::Monospace).color(t.muted));
                download = super::draw::text_action(ui, "Download", false).clicked();
            });
        } else {
            ui.painter().text(pos2(r.left() + size_x, r.center().y), Align2::LEFT_CENTER, &size_text, FontId::monospace(12.0), t.ink);
        }
        let right = Rect::from_min_max(pos2(left_end + 8.0, r.top()), pos2(r.right() - 14.0, r.bottom()));
        let mut cell = child(ui, right, egui::Layout::right_to_left(egui::Align::Center), "actions");
        cell.spacing_mut().item_spacing.x = 8.0;
        if let Some(v) = version {
            let busy = h.restoring.as_deref() == Some(v.stamp.as_str());
            let label = if busy { "Restoring…" } else { "Restore" };
            if chrome::button(&mut cell, label, ButtonKind::Secondary, None, h.restoring.is_none()).clicked() {
                pick = Some(Pick::Restore(v.stamp.clone()));
            }
        }
        if !narrow {
            download = super::draw::text_action(&mut cell, "Download", false).clicked();
        }
        if download {
            pick = Some(Pick::Download(version.map(|v| v.stamp.clone())));
        }
    }
    match &h.list {
        None => {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new("Loading the versions…").size(14.0).color(t.muted));
            });
            ui.add_space(14.0);
        }
        Some(list) if list.is_empty() && h.error.is_none() => {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new("No earlier versions yet. One is kept each time the file is saved over.").size(14.0).color(t.muted));
            });
            ui.add_space(14.0);
        }
        Some(_) => {}
    }
    let frame = Rect::from_min_max(pos2(header.left(), top), pos2(header.right(), ui.cursor().top()));
    ui.painter().rect_stroke(frame, 0.0, egui::Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
    if let Some(e) = &h.error {
        ui.add_space(10.0);
        error_line(ui, e);
    }
    pick
}

#[cfg(test)]
mod tests {
    use super::{place, saved_when};
    use craftcloud_core::api::versions::Version;

    #[test]
    fn a_version_says_when_it_was_saved() {
        let clock = Some(crate::backend::WallClock { unix: 1_794_650_400.0, utc_offset_minutes: 60 });
        let v = |stamp: &str| Version { stamp: stamp.into(), size: 1, etag: None };
        // The content's own time, when the name carries it; else when it was replaced.
        assert_eq!(saved_when(&v("20261114T081500.000Z_20261113T163000Z"), clock), "Yesterday 17:30");
        assert_eq!(saved_when(&v("20261110T100300.000Z"), clock), "10 Nov 2026 11:03");
        assert_eq!(saved_when(&v("odd"), clock), "odd");
    }

    #[test]
    fn the_place_is_the_folder_under_my_files() {
        assert_eq!(place("Documents/Reports/Q3 report.docx"), "My files/Documents/Reports");
        assert_eq!(place("Ideas.md"), "My files");
    }
}
