//! The Files page's sheets (launcher dialogs over the dimmed page): new folder, rename, move (a
//! folder picker), a taken name on upload, and confirmations for what can't be undone.

use craftcloud_core::api::resources::{Kind, Resource};
use craftcloud_core::api::{batch, files, folders, trash};
use egui::{Rect, RichText, Ui};

use super::draw::{self, Glyph};
use super::model::{Item, name_problem};
use super::upload::Clash;
use super::{Files, Key, count};
use crate::backend::Backend;
use crate::chrome::{self, Icon, Kind as ButtonKind};
use crate::theme::Tokens;
use crate::widgets::error_line;

/// The open sheet.
#[derive(Debug)]
pub enum Sheet {
    NewFolder {
        name: String,
    },
    Rename {
        item: Box<Item>,
        name: String,
    },
    /// Pick the folder to move `items` into.
    Move {
        items: Vec<Item>,
        /// The folder being looked at, root first: `(id, name)`.
        trail: Vec<(String, String)>,
        /// Its subfolders; `None` while loading.
        folders: Option<Vec<(String, String)>>,
    },
    /// An upload's name is taken.
    Clash {
        all: bool,
    },
    /// Delete these trashed items for good?
    DeleteForever(Vec<String>),
    /// Empty the whole trash?
    EmptyTrash,
    /// A trashed item's old place is taken: restore it under another name?
    RestoreTaken(Box<Item>),
    /// Make a share link (drawn by [`super::share`]).
    Share(Box<super::share::ShareSheet>),
}

enum Choice {
    None,
    Cancel,
    Confirm,
    Other(usize),
}

impl Files {
    pub(super) fn sheet_ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend) {
        if matches!(self.sheet, Some(Sheet::Share(_))) {
            self.share_ui(ui, area, backend);
            return;
        }
        let t = Tokens::get(ui.ctx());
        let busy = self.net.is_waiting(&Key::Sheet);
        let error = self.sheet_error.clone();
        let items_len = self.items.len();
        let clash = self.clash_name();
        let place = self.place_name();
        let waiting = self.uploads.list.iter().filter(|u| u.state == super::upload::UpState::Exists).count();
        let more = waiting > 1 || self.uploads.list.iter().any(|u| u.state == super::upload::UpState::Queued);
        let Some(sheet) = &mut self.sheet else { return };
        let mut choice = Choice::None;
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !matches!(sheet, Sheet::Clash { .. }) {
            choice = Choice::Cancel;
        }
        let (title, caption, size): (String, String, egui::Vec2) = match sheet {
            Sheet::NewFolder { .. } => ("New folder".into(), place, egui::vec2(480.0, 300.0)),
            Sheet::Rename { item, .. } => {
                (format!("Rename {}", if item.is_folder() { "folder" } else { "file" }), item.name.clone(), egui::vec2(480.0, 300.0))
            }
            Sheet::Move { items, .. } => {
                let what = if let [one] = items.as_slice() { format!("“{}”", one.name) } else { count(items.len(), "item") };
                (format!("Move {what}"), "Choose a folder".into(), egui::vec2(560.0, 560.0))
            }
            Sheet::Clash { .. } => ("This name is taken".into(), "Upload".into(), egui::vec2(560.0, 320.0)),
            Sheet::DeleteForever(ids) => (format!("Delete {} for good?", count(ids.len(), "item")), "Trash".into(), egui::vec2(520.0, 280.0)),
            Sheet::EmptyTrash => ("Empty the trash?".into(), "Trash".into(), egui::vec2(520.0, 280.0)),
            Sheet::RestoreTaken(item) => (format!("“{}” is already there", item.name), item.path.clone(), egui::vec2(560.0, 320.0)),
            Sheet::Share(_) => return,
        };
        let mut open_folder: Option<usize> = None;
        let mut up_to: Option<usize> = None;
        chrome::sheet(
            ui,
            area,
            size,
            &chrome::Hero {
                title: &title,
                caption: &caption,
                keep_case: matches!(sheet, Sheet::NewFolder { .. } | Sheet::Rename { .. } | Sheet::RestoreTaken(_)),
            },
            |ui, part| match (&mut *sheet, part) {
                (Sheet::NewFolder { name } | Sheet::Rename { name, .. }, chrome::Part::Body) => {
                    let field = chrome::field(ui, "Name", name, false, None);
                    if !field.has_focus() && !field.lost_focus() && ui.ctx().memory(|m| m.focused().is_none()) {
                        field.request_focus();
                    }
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        choice = Choice::Confirm;
                    }
                    let problem = if name.is_empty() { None } else { name_problem(name) };
                    if let Some(e) = problem.map(str::to_owned).or(error.clone()) {
                        ui.add_space(8.0);
                        error_line(ui, &e);
                    }
                }
                (Sheet::Move { trail, folders, items }, chrome::Part::Body) => {
                    let names: Vec<String> = trail.iter().enumerate().map(|(i, (_, n))| if i == 0 { "My files".into() } else { n.clone() }).collect();
                    ui.horizontal(|ui| {
                        up_to = draw::crumbs(ui, &names, ui.available_width());
                    });
                    ui.add_space(10.0);
                    match folders {
                        None => {
                            ui.label(RichText::new("Loading…").size(14.0).color(t.muted));
                        }
                        Some(list) if list.is_empty() => {
                            ui.label(RichText::new("No folders in here.").size(14.0).color(t.muted));
                        }
                        Some(list) => {
                            for (i, (id, name)) in list.iter().enumerate() {
                                // A folder can't go into itself.
                                let moving = items.iter().any(|m| m.id == *id);
                                let resp = folder_row(ui, name, !moving);
                                if resp.clicked() && !moving {
                                    open_folder = Some(i);
                                }
                            }
                        }
                    }
                    if let Some(e) = &error {
                        ui.add_space(8.0);
                        error_line(ui, e);
                    }
                }
                (Sheet::Clash { all }, chrome::Part::Body) => {
                    if let Some((_, name, folder)) = &clash {
                        ui.label(RichText::new(format!("“{name}” is already in {folder}.")).size(15.0).color(t.ink_strong));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Replace keeps the current file as an earlier version, so nothing is lost. Keep both adds a number to the new one.",
                            )
                            .size(13.5)
                            .color(t.muted),
                        );
                        ui.add_space(12.0);
                        if more {
                            ui.checkbox(all, "Do this for every name that is taken");
                        }
                    }
                }
                (Sheet::DeleteForever(_), chrome::Part::Body) => {
                    ui.label(
                        RichText::new("They can't be restored afterwards, and computers that sync them lose them too.").size(14.0).color(t.muted),
                    );
                }
                (Sheet::RestoreTaken(item), chrome::Part::Body) => {
                    let new_name = super::restore::restored_name(&item.name, item.is_folder());
                    ui.label(
                        RichText::new(format!(
                            "Something called “{}” is in its old place now. Restore it next to that as “{new_name}”, or leave it in the trash.",
                            item.name
                        ))
                        .size(14.0)
                        .color(t.muted),
                    );
                }
                (Sheet::EmptyTrash, chrome::Part::Body) => {
                    ui.label(
                        RichText::new(format!("{} in the trash will be deleted for good. This can't be undone.", count(items_len, "item")))
                            .size(14.0)
                            .color(t.muted),
                    );
                }
                (Sheet::Share(_), chrome::Part::Body) => {}
                (Sheet::Clash { .. }, chrome::Part::ActionsLeft) => {
                    if chrome::button(ui, "Skip", ButtonKind::Secondary, None, true).clicked() {
                        choice = Choice::Other(2);
                    }
                }
                (_, chrome::Part::ActionsLeft) => {
                    if chrome::button(ui, "Cancel", ButtonKind::Secondary, None, true).clicked() {
                        choice = Choice::Cancel;
                    }
                }
                (sheet, chrome::Part::ActionsRight) => match sheet {
                    Sheet::NewFolder { name } => {
                        let ok = name_problem(name).is_none() && !busy;
                        if chrome::button(ui, if busy { "Creating…" } else { "Create" }, ButtonKind::Primary, Some(Icon::Check), ok).clicked() {
                            choice = Choice::Confirm;
                        }
                    }
                    Sheet::Rename { name, item } => {
                        let ok = name_problem(name).is_none() && name.trim() != item.name && !busy;
                        if chrome::button(ui, if busy { "Renaming…" } else { "Rename" }, ButtonKind::Primary, Some(Icon::Check), ok).clicked() {
                            choice = Choice::Confirm;
                        }
                    }
                    Sheet::Move { trail, items, .. } => {
                        let here = trail.last().map(|(id, _)| id.clone()).unwrap_or_default();
                        let pointless = items.iter().all(|i| i.parent.as_deref() == Some(here.as_str())) || items.iter().any(|i| i.id == here);
                        if chrome::button(ui, "Move here", ButtonKind::Primary, Some(Icon::Arrow), !pointless && !here.is_empty()).clicked() {
                            choice = Choice::Confirm;
                        }
                    }
                    Sheet::Clash { .. } => {
                        if chrome::button(ui, "Replace", ButtonKind::Primary, None, true).clicked() {
                            choice = Choice::Other(1);
                        }
                        if chrome::button(ui, "Keep both", ButtonKind::Secondary, None, true).clicked() {
                            choice = Choice::Other(0);
                        }
                    }
                    Sheet::DeleteForever(_) => {
                        if chrome::button(ui, "Delete forever", ButtonKind::Danger, None, true).clicked() {
                            choice = Choice::Confirm;
                        }
                    }
                    Sheet::EmptyTrash => {
                        if chrome::button(ui, "Empty trash", ButtonKind::Danger, None, true).clicked() {
                            choice = Choice::Confirm;
                        }
                    }
                    Sheet::RestoreTaken(_) => {
                        if chrome::button(ui, "Restore with the new name", ButtonKind::Primary, None, true).clicked() {
                            choice = Choice::Confirm;
                        }
                    }
                    Sheet::Share(_) => {}
                },
                (_, chrome::Part::HeroRight) => {}
            },
        );
        // The move picker walks the folders.
        if let Some(Sheet::Move { trail, folders, .. }) = &mut self.sheet {
            let target = if let Some(i) = up_to {
                trail.truncate(i + 1);
                trail.last().cloned()
            } else if let Some(i) = open_folder {
                let next = folders.as_ref().and_then(|f| f.get(i)).cloned();
                if let Some(next) = &next {
                    trail.push(next.clone());
                }
                next
            } else {
                None
            };
            if let Some((id, _)) = target {
                *folders = None;
                self.picker_generation += 1;
                let g = self.picker_generation;
                self.net.send(backend, Key::Picker(g), folders::contents(&self.server, &id, Some(super::PAGE), None));
            }
        }
        match choice {
            Choice::None => {}
            Choice::Cancel => {
                self.sheet = None;
                self.sheet_error = None;
            }
            Choice::Confirm => self.confirm_sheet(backend),
            Choice::Other(i) => {
                let all = matches!(self.sheet, Some(Sheet::Clash { all: true }));
                let pick = [Clash::KeepBoth, Clash::Replace, Clash::Skip][i];
                if all {
                    self.uploads.for_all = Some(pick);
                }
                if let Some((n, _, _)) = self.clash_name() {
                    self.resolve(backend, n, pick);
                }
                if all {
                    while let Some((n, _, _)) = self.clash_name() {
                        self.resolve(backend, n, pick);
                    }
                }
                if self.clash_name().is_none() {
                    self.sheet = None;
                }
            }
        }
    }

    /// "Documents", or "My files" at the root.
    fn place_name(&self) -> String {
        self.crumbs.last().filter(|_| self.crumbs.len() > 1).map_or_else(|| "My files".into(), |(_, n)| n.clone())
    }

    fn confirm_sheet(&mut self, backend: &mut dyn Backend) {
        self.sheet_error = None;
        match self.sheet.take() {
            Some(Sheet::NewFolder { name }) => {
                let Some((parent, _)) = self.upload_target() else { return };
                let req = folders::create(&self.server, &parent, name.trim());
                self.sheet_done = format!("Created the folder “{}”", name.trim());
                self.sheet_moves.clear();
                self.net.send(backend, Key::Sheet, req);
                self.sheet = Some(Sheet::NewFolder { name });
            }
            Some(Sheet::Rename { item, name }) => {
                let req = match item.kind {
                    Kind::Folder => folders::rename(&self.server, &item.id, name.trim()),
                    Kind::File => files::rename(&self.server, &item.id, name.trim()),
                };
                self.sheet_done = format!("Renamed to “{}”", name.trim());
                // Its kept versions follow once the rename worked (T-050).
                let to = match item.path.rsplit_once('/') {
                    Some((dir, _)) => format!("{dir}/{}", name.trim()),
                    None => name.trim().to_owned(),
                };
                self.sheet_moves = vec![(item.path.clone(), to)];
                self.net.send(backend, Key::Sheet, req);
                self.sheet = Some(Sheet::Rename { item, name });
            }
            Some(Sheet::Move { items, trail, .. }) => {
                let Some((target, target_name)) = trail.last().cloned() else { return };
                let target_name = if trail.len() == 1 { "My files".to_owned() } else { target_name };
                let ids = |k: Kind| items.iter().filter(|i| i.kind == k).map(|i| i.id.clone()).collect::<Vec<_>>();
                let (file_ids, folder_ids) = (ids(Kind::File), ids(Kind::Folder));
                let mut reqs = Vec::new();
                if !file_ids.is_empty() {
                    reqs.push(batch::move_files(&self.server, &file_ids, &target));
                }
                if !folder_ids.is_empty() {
                    reqs.push(batch::move_folders(&self.server, &folder_ids, &target));
                }
                let what = if let [one] = items.as_slice() { format!("“{}”", one.name) } else { count(items.len(), "item") };
                // The kept versions follow (T-050): the target's path is the trail below the root.
                let target_path = trail.iter().skip(1).map(|(_, n)| n.as_str()).collect::<Vec<_>>().join("/");
                let moves: Vec<(String, String)> = items.iter().map(|i| (i.path.clone(), super::model::join(&target_path, &i.name))).collect();
                let after = self.history_moves(&moves);
                self.selection.clear();
                self.change_then(backend, reqs, format!("Moved {what} to {target_name}"), None, after);
            }
            Some(Sheet::DeleteForever(ids)) => self.delete_forever(backend, &ids),
            Some(Sheet::RestoreTaken(item)) => self.restore_as(backend, &item),
            Some(Sheet::EmptyTrash) => {
                let req = trash::empty(&self.server);
                // Their kept versions go too, unless something is at their path again (T-049).
                let after = Self::history_gone(self.trash_all.iter().map(|i| i.path.clone()));
                self.change_then(backend, vec![req], "Emptied the trash".into(), None, after);
            }
            Some(other @ (Sheet::Clash { .. } | Sheet::Share(_))) => self.sheet = Some(other),
            None => {}
        }
    }

    /// The answer to a sheet's request (new folder, rename): close it, or say why not.
    pub(super) fn on_sheet_answer(&mut self, backend: &mut dyn Backend, result: crate::backend::NetResult) {
        let problem = match result {
            Ok(resp) => craftcloud_core::api::expect_ok(&resp).err().map(|e| e.to_string()),
            Err(why) => Some(why),
        };
        match problem {
            Some(why) => self.sheet_error = Some(why),
            None => {
                self.sheet = None;
                let done = std::mem::take(&mut self.sheet_done);
                self.notice(&done, false, None);
                let moves = std::mem::take(&mut self.sheet_moves);
                let after = self.history_moves(&moves);
                self.history_then(backend, after);
                self.reload(backend);
            }
        }
    }

    /// The folders of the picker's current folder.
    pub(super) fn on_picker_answer(&mut self, generation: u64, result: crate::backend::NetResult) {
        if generation != self.picker_generation {
            return;
        }
        let Some(Sheet::Move { folders: list, .. }) = &mut self.sheet else { return };
        match result.and_then(|resp| folders::parse_contents(&resp).map_err(|e| e.to_string())) {
            Ok(page) => {
                *list = Some(
                    page.items
                        .into_iter()
                        .filter_map(|r| match r {
                            Resource::Folder(f) => Some((f.id, f.name)),
                            Resource::File(_) => None,
                        })
                        .collect(),
                );
            }
            Err(why) => {
                *list = Some(Vec::new());
                self.sheet_error = Some(why);
            }
        }
    }

    /// Opens the move picker for the selection, at the drive's root.
    pub(super) fn start_move(&mut self, backend: &mut dyn Backend) {
        let items = self.selected_items();
        let Some(root) = self.root.clone() else { return };
        if items.is_empty() {
            return;
        }
        // Start where the items are, when that's the open folder.
        let trail = if matches!(self.view, super::View::Folder(_)) && !self.crumbs.is_empty() {
            self.crumbs.clone()
        } else {
            vec![(root, "Personal".into())]
        };
        let at = trail.last().map(|(id, _)| id.clone()).unwrap_or_default();
        self.sheet_error = None;
        self.sheet = Some(Sheet::Move { items, trail, folders: None });
        self.picker_generation += 1;
        let g = self.picker_generation;
        self.net.send(backend, Key::Picker(g), folders::contents(&self.server, &at, Some(super::PAGE), None));
    }
}

/// A folder in the picker: glyph and name, 36 tall.
fn folder_row(ui: &mut Ui, name: &str, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, 0.0, t.sunken);
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, t.hairline());
    let color = if enabled { t.ink_strong } else { t.muted };
    draw::paint_glyph(ui, Glyph::Folder, egui::pos2(rect.left() + 16.0, rect.center().y), color);
    ui.painter().text(egui::pos2(rect.left() + 36.0, rect.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(14.0), color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, format!("Open {name}")));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}
