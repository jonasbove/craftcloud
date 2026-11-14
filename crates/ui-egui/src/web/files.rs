//! The web Files page: browse the cloud like OneDrive or Google Drive. A launcher-style table (or
//! grid) of the folder's files with breadcrumbs, a sidebar of views (My files, Recent, Starred,
//! Trash, search results), and the actions on them.
//!
//! Everything goes through OxiCloud's REST API by id (`craftcloud_core::api::{folders, files, …}`),
//! except file bytes, which go over WebDAV by path; the browser's session cookie signs both.
//! CraftCloud's own data (`.craftcloud/`, the file versions) never shows.

mod apps;
mod draw;
mod history;
mod model;
mod restore;
mod sheets;
mod table;
mod time;
mod upload;

use std::collections::HashMap;

use craftcloud_core::ServerUrl;
use craftcloud_core::api::resources::{Kind, Page, Resource};
use craftcloud_core::api::{auth, batch, favorites, files, folders, search, trash};
use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
use egui::{Align2, FontId, Pos2, Rect, Ui, pos2, vec2};

use self::draw::Glyph;
use self::model::{Item, Sort};
use self::sheets::Sheet;
use self::upload::{UpState, Uploads};
use crate::backend::{Backend, Net, NetResult, WallClock};
use crate::chrome::{self, Kind as ButtonKind};
use crate::theme::Tokens;

/// Items per listing page (the server's maximum).
const PAGE: u32 = 200;
/// Thumbnails asked for at once.
const THUMBS_AT_ONCE: usize = 6;
/// How long a notice stays, in seconds.
const TOAST_SECS: f64 = 6.0;

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// A folder by id; empty until the drive's root is known.
    Folder(String),
    Recent,
    Starred,
    Trash,
    /// Search results for this text.
    Search(String),
}

impl View {
    fn title(&self) -> String {
        match self {
            Self::Folder(_) => "My files".into(),
            Self::Recent => "Recent".into(),
            Self::Starred => "Starred".into(),
            Self::Trash => "Trash".into(),
            Self::Search(q) => format!("Results for “{q}”"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Roots,
    Me,
    Config,
    /// A page of the current listing (`gen` tells stale answers apart).
    List(u64),
    Crumbs(u64),
    Thumb(String),
    /// One request of change number `n` (see [`Change`]).
    Change(u64),
    /// The `PUT` of the upload of local file `key`.
    Upload(u64),
    /// A step of replacing a file with the upload of local file `key`.
    UpStep(u64),
    /// The request of the open sheet (new folder, rename).
    Sheet,
    /// The move picker's listing.
    Picker(u64),
    /// Whether a trashed item's old place is taken, before restoring it (by item id).
    RestoreCheck(String),
    /// A step of "Restore as" (see [`restore::Chain`]).
    Chain,
    /// A step of the kept versions' housekeeping (see [`history`]); `Some(path)` asks whether
    /// `path` is free.
    History(Option<String>),
    /// The trash's ids, for leaving trashed files out of search results (by generation).
    TrashIds(u64),
    /// Best effort; the answer doesn't matter.
    Ignore,
}

enum Thumb {
    Loading,
    None,
    /// Decoded, not yet on the GPU (answers arrive outside drawing).
    Image(egui::ColorImage),
    Ready(egui::TextureHandle),
}

/// The open "…" menu: which item, and where.
struct Menu {
    id: String,
    at: Pos2,
}

/// Something the person changed, sent as one or more requests; when every answer is in, the list
/// reloads and a notice says how it went.
struct Change {
    left: usize,
    failed: Vec<String>,
    done: String,
    undo: Option<Undo>,
    /// Kept-version steps for when every request worked.
    after: Vec<history::Step>,
}

/// How to take a change back from its notice.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Undo {
    /// Restore these ids from the trash.
    Restore(Vec<String>),
}

struct Toast {
    text: String,
    bad: bool,
    undo: Option<Undo>,
    until: f64,
}

pub struct Files {
    server: ServerUrl,
    net: Net<Key>,
    started: bool,
    /// The drive's root folder ("Personal", shown as "My files").
    root: Option<String>,
    pub(crate) view: View,
    /// Bumped on every navigation; answers for older listings are ignored.
    generation: u64,
    pub(crate) items: Vec<Item>,
    loading: bool,
    error: Option<String>,
    /// `(id, name)` from the root to the current folder.
    crumbs: Vec<(String, String)>,
    crumbs_for: u64,
    /// The current folder's cloud path (`Documents/Reports`).
    folder_path: String,
    pub(crate) selection: Vec<String>,
    anchor: Option<usize>,
    sort: Sort,
    grid: bool,
    /// The search box.
    query: String,
    /// `(used, quota)` bytes.
    storage: Option<(u64, u64)>,
    /// Craft apps the server offers in the browser (`/craftcloud/config.json`'s `enabled_apps`).
    web_apps: Vec<String>,
    clock: Option<WallClock>,
    now: f64,
    thumbs: HashMap<String, Thumb>,
    menu: Option<Menu>,
    pub(crate) sheet: Option<Sheet>,
    changes: HashMap<u64, Change>,
    next_change: u64,
    toast: Option<Toast>,
    pub(crate) uploads: Uploads,
    sheet_error: Option<String>,
    /// The notice when the sheet's request works.
    sheet_done: String,
    /// What the open rename moves, `(from, to)`: its kept versions follow when it worked.
    sheet_moves: Vec<(String, String)>,
    /// The kept versions' housekeeping, one request at a time.
    history: history::Queue,
    picker_generation: u64,
    /// Everything in the trash; `items` is what the trash filter lets through.
    trash_all: Vec<Item>,
    /// Bumped by every change and every search: OxiCloud caches search answers per exact query,
    /// so each search asks with a page size it hasn't used lately (see [`search_limit`]) and gets
    /// a fresh answer (T-043).
    search_bust: u32,
    /// The trash's ids while searching (T-043); `None` until the trash has answered.
    search_trashed: Option<std::collections::HashSet<String>>,
    chain: restore::Chain,
}

impl Files {
    pub fn new(server: ServerUrl) -> Self {
        Self {
            server,
            net: Net::default(),
            started: false,
            root: None,
            view: View::Folder(String::new()),
            generation: 0,
            items: Vec::new(),
            loading: true,
            error: None,
            crumbs: Vec::new(),
            crumbs_for: 0,
            folder_path: String::new(),
            selection: Vec::new(),
            anchor: None,
            sort: Sort::default(),
            grid: false,
            query: String::new(),
            storage: None,
            web_apps: Vec::new(),
            clock: None,
            now: 0.0,
            thumbs: HashMap::new(),
            menu: None,
            sheet: None,
            changes: HashMap::new(),
            next_change: 1,
            toast: None,
            uploads: Uploads::default(),
            sheet_error: None,
            sheet_done: String::new(),
            sheet_moves: Vec::new(),
            history: history::Queue::default(),
            picker_generation: 0,
            trash_all: Vec::new(),
            search_bust: 0,
            search_trashed: None,
            chain: restore::Chain::default(),
        }
    }

    /// Called when the page is shown: the first time it loads the drive, later it refreshes.
    pub fn open(&mut self, backend: &mut dyn Backend, me: Option<&auth::Me>) {
        if let Some(me) = me {
            self.storage = Some((me.storage_used_bytes, me.storage_quota_bytes));
        }
        if !self.started {
            self.started = true;
            self.net.send(backend, Key::Roots, folders::roots(&self.server));
            self.net.send(backend, Key::Config, HttpRequest::new(Method::Get, self.server.join("craftcloud/config.json")));
        } else {
            self.reload(backend);
        }
    }

    /// Takes this page's answers out of `answers`.
    pub fn receive(&mut self, backend: &mut dyn Backend, answers: &mut Vec<(u64, NetResult)>) {
        for (key, result) in self.net.receive(answers) {
            self.on_answer(backend, key, result);
        }
    }

    fn on_answer(&mut self, backend: &mut dyn Backend, key: Key, result: NetResult) {
        let resp = match result {
            Ok(resp) => resp,
            Err(why) => {
                match key {
                    Key::List(g) if g == self.generation => {
                        self.loading = false;
                        self.error = Some(why);
                    }
                    Key::Thumb(id) => {
                        self.thumbs.insert(id, Thumb::None);
                    }
                    Key::Change(n) => self.change_answered(backend, n, Some(why)),
                    Key::Upload(k) => self.on_uploaded(backend, k, Err(why)),
                    Key::UpStep(k) => self.on_step(backend, k, Err(why)),
                    Key::Sheet => self.on_sheet_answer(backend, Err(why)),
                    Key::Picker(g) => self.on_picker_answer(g, Err(why)),
                    Key::RestoreCheck(id) => self.on_restore_check(backend, &id, Err(why)),
                    Key::Chain => self.on_chain(backend, Err(why)),
                    Key::History(path) => self.on_history(backend, path, &Err(why)),
                    _ => {}
                }
                return;
            }
        };
        match key {
            Key::Roots => match folders::parse_roots(&resp) {
                Ok(roots) => {
                    if let Some(root) = roots.into_iter().next() {
                        self.root = Some(root.id.clone());
                        if self.view == View::Folder(String::new()) {
                            self.navigate(backend, View::Folder(root.id));
                        }
                    }
                }
                Err(e) => {
                    self.loading = false;
                    self.error = Some(e.to_string());
                }
            },
            Key::Me => {
                if let Ok(Some(me)) = auth::parse_me(&resp) {
                    self.storage = Some((me.storage_used_bytes, me.storage_quota_bytes));
                }
            }
            Key::Config => {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                    let on = v["web_apps"].as_bool().unwrap_or(false);
                    self.web_apps = if on {
                        v["enabled_apps"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                }
            }
            Key::List(g) if g == self.generation => self.on_page(backend, g, &resp),
            Key::Crumbs(g) if g == self.generation => {
                if let Ok(list) = folders::parse_ancestors(&resp) {
                    self.crumbs = list.into_iter().map(|a| (a.id, a.name)).collect();
                    self.crumbs_for = g;
                    self.folder_path = self.crumbs.iter().skip(1).map(|(_, n)| n.as_str()).collect::<Vec<_>>().join("/");
                    let folder = self.folder_path.clone();
                    for item in &mut self.items {
                        item.path = model::join(&folder, &item.name);
                    }
                }
            }
            Key::List(_) | Key::Crumbs(_) => {}
            Key::Thumb(id) => {
                let thumb = files::parse_thumbnail(&resp).ok().flatten().and_then(|bytes| decode(&bytes)).map_or(Thumb::None, Thumb::Image);
                self.thumbs.insert(id, thumb);
            }
            Key::Change(n) => {
                let problem = change_problem(&resp);
                self.change_answered(backend, n, problem);
            }
            Key::Upload(k) => self.on_uploaded(backend, k, Ok(resp)),
            Key::UpStep(k) => self.on_step(backend, k, Ok(resp)),
            Key::Sheet => self.on_sheet_answer(backend, Ok(resp)),
            Key::Picker(g) => self.on_picker_answer(g, Ok(resp)),
            Key::RestoreCheck(id) => self.on_restore_check(backend, &id, Ok(resp)),
            Key::Chain => self.on_chain(backend, Ok(resp)),
            Key::TrashIds(g) => self.on_trash_ids(g, &resp),
            Key::History(path) => self.on_history(backend, path, &Ok(resp)),
            Key::Ignore => {}
        }
    }

    /// Reads a page of the current listing; asks for the next page when there is one.
    fn on_page(&mut self, backend: &mut dyn Backend, generation: u64, resp: &HttpResponse) {
        let folder = if self.crumbs_for == generation { self.folder_path.clone() } else { String::new() };
        let place = |r: Resource| Item::from_resource(r, |name| model::join(&folder, name));
        let parsed: Result<(Vec<Item>, Option<String>), String> = match &self.view {
            View::Folder(_) => folders::parse_contents(resp).map(|p| keep(p, |r| r, place)).map_err(|e| e.to_string()),
            View::Recent => favorites::parse_recent(resp)
                .map(|p| {
                    let next = p.next_cursor.clone();
                    let items = p
                        .items
                        .into_iter()
                        .filter(|r| !r.resource.is_internal())
                        .map(|r| {
                            let mut item = place(r.resource);
                            item.opened = Some(r.accessed_at);
                            item
                        })
                        .collect();
                    (items, next)
                })
                .map_err(|e| e.to_string()),
            View::Starred => favorites::parse_list(resp).map(|p| keep(p, |f| f.resource, place)).map_err(|e| e.to_string()),
            View::Trash => trash::parse_list(resp)
                .map(|p| {
                    let next = p.next_cursor.clone();
                    let items = p
                        .items
                        .into_iter()
                        .filter(|t| !t.resource.is_internal())
                        .map(|t| {
                            let mut item = Item::from_resource(t.resource, str::to_owned);
                            item.expires = Some(t.expires_at);
                            item.trashed = Some(t.trashed_at);
                            item
                        })
                        .collect();
                    (items, next)
                })
                .map_err(|e| e.to_string()),
            View::Search(_) => search::parse_search(resp)
                .map(|r| (r.items.into_iter().map(|h| h.resource).filter(|r| !r.is_internal()).map(place).collect(), r.next_cursor))
                .map_err(|e| e.to_string()),
        };
        match parsed {
            Ok((items, next)) => {
                if self.view == View::Trash {
                    self.trash_all.extend(items);
                    self.filter_trash();
                } else if let (View::Search(_), Some(trashed)) = (&self.view, &self.search_trashed) {
                    self.items.extend(items.into_iter().filter(|i| !trashed.contains(&i.id)));
                } else {
                    self.items.extend(items);
                }
                match next {
                    Some(cursor) => {
                        if let Some(req) = self.listing_request(Some(&cursor)) {
                            self.net.send(backend, Key::List(generation), req);
                        }
                    }
                    None => {
                        self.loading = false;
                        // Recent and Trash come in the server's order (last opened, soonest
                        // deleted), which their headers show.
                        if !matches!(self.view, View::Recent | View::Trash) {
                            model::sort_items(&mut self.items, self.sort);
                        }
                        self.selection.retain(|id| self.items.iter().any(|i| i.id == *id));
                    }
                }
            }
            Err(e) => {
                self.loading = false;
                self.error = Some(e);
            }
        }
    }

    /// Shows `view` from scratch.
    fn navigate(&mut self, backend: &mut dyn Backend, view: View) {
        // Recent comes in the order things were opened, and its header says so; leaving it goes
        // back to the usual name order.
        if view == View::Recent {
            self.sort = Sort::RECENT;
        } else if view == View::Trash {
            self.sort = Sort::TRASH;
        } else if matches!(self.sort.key, model::SortKey::Opened | model::SortKey::Expires | model::SortKey::Trashed) {
            self.sort = Sort::default();
        }
        self.view = view;
        self.selection.clear();
        self.anchor = None;
        self.menu = None;
        if !matches!(self.view, View::Search(_)) {
            self.query.clear();
        }
        self.request_listing(backend);
    }

    /// Lists the current view again, keeping the selection where it still applies.
    fn reload(&mut self, backend: &mut dyn Backend) {
        self.request_listing(backend);
        self.net.send(backend, Key::Me, auth::me(&self.server));
    }

    fn request_listing(&mut self, backend: &mut dyn Backend) {
        self.generation += 1;
        self.items.clear();
        self.trash_all.clear();
        self.error = None;
        self.loading = true;
        let g = self.generation;
        if matches!(self.view, View::Search(_)) {
            // A new key for every search, in every session (T-043), and the trash alongside, so
            // a file deleted elsewhere leaves the results even if the server's answer lags.
            self.search_bust += 1;
            self.search_trashed = None;
            self.net.send(backend, Key::TrashIds(g), trash::list(&self.server, Some(PAGE), None));
        }
        let Some(req) = self.listing_request(None) else { return };
        self.net.send(backend, Key::List(g), req);
        if let View::Folder(id) = &self.view {
            self.net.send(backend, Key::Crumbs(g), folders::ancestors(&self.server, id));
        }
    }

    /// The trash's ids arrived: trashed files leave the search results.
    fn on_trash_ids(&mut self, generation: u64, resp: &HttpResponse) {
        if generation != self.generation || !matches!(self.view, View::Search(_)) {
            return;
        }
        let Ok(page) = trash::parse_list(resp) else { return };
        let ids: std::collections::HashSet<String> = page.items.into_iter().map(|t| t.resource.id().to_owned()).collect();
        self.items.retain(|i| !ids.contains(&i.id));
        self.selection.retain(|id| !ids.contains(id));
        self.search_trashed = Some(ids);
    }

    fn listing_request(&self, cursor: Option<&str>) -> Option<HttpRequest> {
        Some(match &self.view {
            View::Folder(id) if id.is_empty() => return None,
            View::Folder(id) => folders::contents(&self.server, id, Some(PAGE), cursor),
            // Recent is short by nature: one page.
            View::Recent if cursor.is_some() => return None,
            View::Recent => favorites::recent(&self.server, Some(100), None),
            View::Starred => favorites::list(&self.server, Some(PAGE), cursor),
            View::Trash => trash::list(&self.server, Some(PAGE), cursor),
            View::Search(text) => search::search(
                &self.server,
                &search::Query {
                    text: text.clone(),
                    limit: Some(search_limit(self.clock.map(|c| c.unix), self.search_bust)),
                    cursor: cursor.map(str::to_owned),
                    ..Default::default()
                },
            ),
        })
    }

    fn go_home(&mut self, backend: &mut dyn Backend) {
        if let Some(root) = self.root.clone() {
            self.navigate(backend, View::Folder(root));
        }
    }

    fn open_item(&mut self, backend: &mut dyn Backend, index: usize) {
        let Some(item) = self.items.get(index).cloned() else { return };
        if self.view == View::Trash {
            return;
        }
        if item.is_folder() {
            self.navigate(backend, View::Folder(item.id));
        } else {
            self.download(backend, &[index]);
        }
    }

    /// Opens the folder an item is in (from Recent, Starred or search).
    fn show_in_folder(&mut self, backend: &mut dyn Backend, index: usize) {
        if let Some(parent) = self.items.get(index).and_then(|i| i.parent.clone()) {
            let id = self.items[index].id.clone();
            self.navigate(backend, View::Folder(parent));
            self.selection = vec![id];
        }
    }

    /// Downloads one file as itself, or several items (or a folder) as a zip.
    fn download(&mut self, backend: &mut dyn Backend, indices: &[usize]) {
        let picked: Vec<&Item> = indices.iter().filter_map(|i| self.items.get(*i)).collect();
        match picked.as_slice() {
            [] => {}
            [one] if !one.is_folder() => backend.download(&files::download(&self.server, &one.id).url, &one.name),
            [one] => backend.download(&folders::download_zip(&self.server, &one.id).url, &format!("{}.zip", one.name)),
            many => {
                let ids = |k: Kind| many.iter().filter(|i| i.kind == k).map(|i| i.id.clone()).collect::<Vec<_>>();
                backend.download(&batch::download(&self.server, &ids(Kind::File), &ids(Kind::Folder)).url, "CraftCloud files.zip");
            }
        }
    }

    fn selected_items(&self) -> Vec<Item> {
        self.items.iter().filter(|i| self.selection.contains(&i.id)).cloned().collect()
    }

    fn selected_indices(&self) -> Vec<usize> {
        self.items.iter().enumerate().filter(|(_, i)| self.selection.contains(&i.id)).map(|(n, _)| n).collect()
    }

    fn select_click(&mut self, index: usize, modifiers: egui::Modifiers) {
        let Some(id) = self.items.get(index).map(|i| i.id.clone()) else { return };
        if modifiers.shift
            && let Some(anchor) = self.anchor
        {
            let (a, b) = if anchor <= index { (anchor, index) } else { (index, anchor) };
            self.selection = self.items[a..=b].iter().map(|i| i.id.clone()).collect();
            return;
        }
        if modifiers.command {
            self.toggle(index);
        } else {
            self.selection = vec![id];
        }
        self.anchor = Some(index);
    }

    fn toggle(&mut self, index: usize) {
        let Some(id) = self.items.get(index).map(|i| i.id.clone()) else { return };
        if let Some(pos) = self.selection.iter().position(|s| *s == id) {
            self.selection.remove(pos);
        } else {
            self.selection.push(id);
        }
        self.anchor = Some(index);
    }

    // ─── Changes ────────────────────────────────────────────────────────────────────────────────

    /// Sends the requests of one change; `done` is the notice when all of them worked.
    fn change(&mut self, backend: &mut dyn Backend, requests: Vec<HttpRequest>, done: String, undo: Option<Undo>) {
        self.change_then(backend, requests, done, undo, Vec::new());
    }

    /// As [`Files::change`], then the kept-version steps `after` when every request worked.
    fn change_then(&mut self, backend: &mut dyn Backend, requests: Vec<HttpRequest>, done: String, undo: Option<Undo>, after: Vec<history::Step>) {
        if requests.is_empty() {
            return;
        }
        let n = self.next_change;
        self.next_change += 1;
        self.changes.insert(n, Change { left: requests.len(), failed: Vec::new(), done, undo, after });
        for req in requests {
            self.net.send(backend, Key::Change(n), req);
        }
    }

    fn change_answered(&mut self, backend: &mut dyn Backend, n: u64, problem: Option<String>) {
        let Some(change) = self.changes.get_mut(&n) else { return };
        change.left = change.left.saturating_sub(1);
        if let Some(p) = problem {
            change.failed.push(p);
        }
        if change.left > 0 {
            return;
        }
        let Some(change) = self.changes.remove(&n) else { return };
        self.search_bust += 1;
        if change.failed.is_empty() {
            self.notice(&change.done, false, change.undo);
            self.history_then(backend, change.after);
        } else {
            let mut why = change.failed;
            why.dedup();
            self.notice(&why.join(" · "), true, None);
        }
        self.reload(backend);
    }

    fn notice(&mut self, text: &str, bad: bool, undo: Option<Undo>) {
        self.toast = Some(Toast { text: text.to_owned(), bad, undo, until: self.now + TOAST_SECS });
    }

    /// Moves the selection to the trash, with UNDO in the notice.
    fn trash_selection(&mut self, backend: &mut dyn Backend) {
        let picked = self.selected_items();
        if picked.is_empty() {
            return;
        }
        let ids = |k: Kind| picked.iter().filter(|i| i.kind == k).map(|i| i.id.clone()).collect::<Vec<_>>();
        let req = if let [one] = picked.as_slice() {
            if one.is_folder() { folders::trash(&self.server, &one.id) } else { files::trash(&self.server, &one.id) }
        } else {
            batch::trash(&self.server, &ids(Kind::File), &ids(Kind::Folder))
        };
        let what = if let [one] = picked.as_slice() { format!("“{}”", one.name) } else { count(picked.len(), "item") };
        let undo = Undo::Restore(picked.iter().map(|i| i.id.clone()).collect());
        self.selection.clear();
        // Gone from this list at once: a search answer can lag behind (T-043).
        self.items.retain(|i| !picked.iter().any(|p| p.id == i.id));
        self.change(backend, vec![req], format!("Moved {what} to the trash"), Some(undo));
    }

    fn restore(&mut self, backend: &mut dyn Backend, ids: &[String]) {
        let reqs = ids.iter().map(|id| trash::restore(&self.server, id)).collect();
        let what = if ids.len() == 1 { "1 item".to_owned() } else { count(ids.len(), "item") };
        self.selection.clear();
        self.change(backend, reqs, format!("Restored {what}"), None);
    }

    fn delete_forever(&mut self, backend: &mut dyn Backend, ids: &[String]) {
        let reqs = ids.iter().map(|id| trash::delete(&self.server, id)).collect();
        // Their kept versions go too, unless something is at their path again (T-049).
        let paths: Vec<String> =
            ids.iter().filter_map(|id| self.trash_all.iter().chain(&self.items).find(|i| &i.id == id)).map(|i| i.path.clone()).collect();
        self.selection.clear();
        self.change_then(backend, reqs, format!("Deleted {} for good", count(ids.len(), "item")), None, Self::history_gone(paths));
    }

    fn set_starred(&mut self, backend: &mut dyn Backend, picked: &[Item], on: bool) {
        let reqs = picked
            .iter()
            .map(|i| if on { favorites::add(&self.server, i.kind, &i.id) } else { favorites::remove(&self.server, i.kind, &i.id) })
            .collect();
        let what = if let [one] = picked { format!("“{}”", one.name) } else { count(picked.len(), "item") };
        self.change(backend, reqs, if on { format!("Starred {what}") } else { format!("Removed the star from {what}") }, None);
    }

    /// The trash list through the search box's text (the trash is filtered here, not searched on
    /// the server, which only searches what isn't in the trash).
    fn filter_trash(&mut self) {
        let q = self.query.trim().to_lowercase();
        self.items = self.trash_all.iter().filter(|i| q.is_empty() || i.name.to_lowercase().contains(&q)).cloned().collect();
        if self.sort != Sort::TRASH {
            model::sort_items(&mut self.items, self.sort);
        }
        self.selection.retain(|id| self.items.iter().any(|i| i.id == *id));
    }

    fn run_search(&mut self, backend: &mut dyn Backend) {
        let text = self.query.trim().to_owned();
        if text.is_empty() {
            self.go_home(backend);
        } else {
            self.navigate(backend, View::Search(text));
        }
    }

    // ─── Drawing ────────────────────────────────────────────────────────────────────────────────

    pub fn ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend) {
        self.clock = backend.wall_clock();
        self.now = backend.now();
        let progress = backend.poll_progress();
        self.on_progress(&progress);
        let picked = backend.take_picked();
        self.enqueue(backend, picked);
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            let files = backend.accept_dropped(&dropped);
            self.enqueue(backend, files);
        }
        if self.sheet.is_none() && self.uploads.clash().is_some() {
            self.sheet = Some(Sheet::Clash { all: false });
        }
        if self.net.is_busy() || self.toast.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let wide = area.width() >= 900.0;
        let sidebar_w = if wide { 232.0 } else { 0.0 };
        if wide {
            let side = Rect::from_min_max(area.min, pos2(area.left() + sidebar_w, area.bottom()));
            self.sidebar_ui(ui, side, backend);
        }
        let main = Rect::from_min_max(pos2(area.left() + sidebar_w, area.top()), area.max);
        let small = main.width() < 600.0;
        let pad = if small { 16.0 } else { chrome::PAD };
        let inner = main.shrink2(vec2(pad, 0.0));
        let mut y = main.top() + if small { 16.0 } else { 24.0 };

        if !wide {
            // The views as cells, above everything else.
            let row = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), chrome::CONTROL_H));
            self.view_cells_ui(ui, row, backend);
            y = row.bottom() + 12.0;
        }

        // Toolbar: where we are on the left, actions on the right.
        // On a phone the search and actions take the whole row, so the breadcrumbs get one of their
        // own under it (at the top of My files the view cells already say where this is).
        let bar = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), chrome::CONTROL_H));
        let crumbs_apart = small;
        self.toolbar_ui(ui, bar, backend, !crumbs_apart);
        y = bar.bottom() + 14.0;
        if crumbs_apart && (self.crumbs.len() > 1 || matches!(self.view, View::Search(_))) {
            let row = Rect::from_min_size(pos2(inner.left(), y - 4.0), vec2(inner.width(), chrome::CONTROL_H));
            ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                self.crumbs_ui(ui, row.width(), backend);
            });
            y = row.bottom() + 10.0;
        }

        // Meta line (counts), or what to do with the selection.
        let meta = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 28.0));
        self.meta_ui(ui, meta, backend);
        y = meta.bottom() + 8.0;

        let list = Rect::from_min_max(pos2(inner.left(), y), pos2(inner.right(), main.bottom()));
        self.list_ui(ui, list, backend);
        if ui.input(|i| !i.raw.hovered_files.is_empty()) {
            self.drop_overlay(ui, list);
        }
        self.uploads_ui(ui, main);

        if self.menu.is_some() {
            self.menu_ui(ui, backend);
        }
        self.toast_ui(ui, main, backend);
        if self.sheet.is_some() {
            self.sheet_ui(ui, area, backend);
        }
    }

    fn sidebar_ui(&mut self, ui: &mut Ui, rect: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        ui.painter().vline(rect.right() - 0.5, rect.y_range(), t.hairline());
        let inner = Rect::from_min_max(rect.min + vec2(12.0, 24.0), rect.max - vec2(13.0, 16.0));
        let mut go: Option<View> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (glyph, label, view) in self.view_choices() {
                let on = std::mem::discriminant(&self.view) == std::mem::discriminant(&view);
                if draw::side_item(ui, glyph, label, on, None).clicked() {
                    go = Some(view);
                }
            }
        });
        if let Some(view) = go {
            self.navigate(backend, view);
        }
        // Storage, at the bottom.
        if let Some((used, quota)) = self.storage {
            let panel = Rect::from_min_max(pos2(inner.left() + 8.0, inner.bottom() - 64.0), inner.max - vec2(8.0, 0.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(panel).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                chrome::hud(ui, "Storage", t.muted);
                ui.add_space(6.0);
                storage_line(ui, used, quota);
                ui.add_space(6.0);
                chrome::meter(ui, if quota > 0 { used as f32 / quota as f32 } else { 0.0 });
            });
        }
    }

    /// The views the sidebar (or, on a phone, the cells row) offers.
    fn view_choices(&self) -> Vec<(Glyph, &'static str, View)> {
        let root = self.root.clone().unwrap_or_default();
        vec![
            (Glyph::Folder, "My files", View::Folder(root)),
            (Glyph::Clock, "Recent", View::Recent),
            (Glyph::Star, "Starred", View::Starred),
            (Glyph::Trash, "Trash", View::Trash),
        ]
    }

    fn view_cells_ui(&mut self, ui: &mut Ui, row: Rect, backend: &mut dyn Backend) {
        let choices = self.view_choices();
        let labels: Vec<&str> = choices.iter().map(|(_, l, _)| *l).collect();
        let current = choices.iter().position(|(_, _, v)| std::mem::discriminant(v) == std::mem::discriminant(&self.view));
        let mut picked = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            picked = draw::cells(ui, &labels, current);
        });
        if let Some(i) = picked {
            let view = choices[i].2.clone();
            self.navigate(backend, view);
        }
    }

    /// Where this is: the breadcrumbs of a folder, or the view's name, in `width`.
    fn crumbs_ui(&mut self, ui: &mut Ui, width: f32, backend: &mut dyn Backend) {
        match &self.view {
            View::Folder(_) => {
                let names: Vec<String> =
                    self.crumbs.iter().enumerate().map(|(i, (_, n))| if i == 0 { "My files".to_owned() } else { n.clone() }).collect();
                if names.is_empty() {
                    chrome::hud(ui, "My files", Tokens::get(ui.ctx()).muted);
                } else if let Some(i) = draw::crumbs(ui, &names, width) {
                    let id = self.crumbs[i].0.clone();
                    self.navigate(backend, View::Folder(id));
                }
            }
            other => {
                draw::crumbs(ui, &[other.title()], width);
            }
        }
    }

    fn toolbar_ui(&mut self, ui: &mut Ui, bar: Rect, backend: &mut dyn Backend, with_crumbs: bool) {
        let narrow = bar.width() < 640.0;
        let mut search_now = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let View::Folder(_) = self.view {
                let ready = self.upload_target().is_some();
                if narrow {
                    if draw::icon_button(ui, Glyph::Up, "Upload", true, ready).clicked() {
                        backend.pick_files();
                    }
                    if draw::icon_button(ui, Glyph::Plus, "New folder", false, ready).clicked() {
                        self.sheet = Some(Sheet::NewFolder { name: String::new() });
                    }
                } else {
                    if draw::icon_label_button(ui, Glyph::Up, "Upload", ButtonKind::Primary, ready).clicked() {
                        backend.pick_files();
                    }
                    if draw::icon_label_button(ui, Glyph::Plus, "New folder", ButtonKind::Secondary, ready).clicked() {
                        self.sheet_error = None;
                        self.sheet = Some(Sheet::NewFolder { name: String::new() });
                    }
                }
            }
            if self.view == View::Trash {
                let any = !self.items.is_empty();
                if chrome::button(ui, "Empty trash", ButtonKind::Danger, None, any).clicked() {
                    self.sheet = Some(Sheet::EmptyTrash);
                }
            }
            let (glyph, label) = if self.grid { (Glyph::List, "Show as list") } else { (Glyph::Grid, "Show as grid") };
            if draw::icon_button(ui, glyph, label, false, true).clicked() {
                self.grid = !self.grid;
            }
            if draw::icon_button(ui, Glyph::Refresh, "Refresh", false, !self.loading).clicked() {
                self.reload(backend);
            }
            let search_w = if !with_crumbs {
                ui.available_width().clamp(120.0, 320.0)
            } else if narrow {
                (bar.width() * 0.42).clamp(120.0, 220.0)
            } else {
                240.0
            };
            let in_trash = self.view == View::Trash;
            let field = draw::search_box(ui, &mut self.query, search_w, if in_trash { "Filter the trash" } else { "Search files" });
            if in_trash {
                if field.changed() {
                    self.filter_trash();
                }
            } else if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                search_now = true;
            }
            if with_crumbs {
                let used = bar.right() - ui.cursor().right();
                let left = (bar.width() - used - 16.0).max(80.0);
                let width = if narrow { left } else { left.min(720.0) };
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| self.crumbs_ui(ui, width, backend));
            }
        });
        if search_now {
            self.run_search(backend);
        }
    }

    fn meta_ui(&mut self, ui: &mut Ui, rect: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        if self.loading {
            return;
        }
        let picked = self.selected_items();
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if picked.is_empty() {
                let files = self.items.iter().filter(|i| !i.is_folder()).count();
                let folders = self.items.len() - files;
                let bytes: u64 = self.items.iter().filter_map(|i| i.size).sum();
                let mut parts = vec![count(folders, "folder"), count(files, "file")];
                if files > 0 {
                    parts.push(super::bytes(bytes));
                }
                if self.view == View::Trash {
                    // The trash counts items (filtered: "2 of 9 items"), soonest deleted first.
                    let all = self.trash_all.len();
                    let shown = if self.items.len() == all { count(all, "item") } else { format!("{} of {}", self.items.len(), count(all, "item")) };
                    parts = vec![shown];
                    if files > 0 {
                        parts.push(super::bytes(bytes));
                    }
                    // A phone has room for the short form on the one line.
                    parts.push(if rect.width() < 520.0 { "kept 30 days" } else { "deleted for good after 30 days" }.into());
                }
                chrome::hud(ui, &parts.join(" · "), t.muted);
                return;
            }
            chrome::hud(ui, &format!("{} selected", picked.len()), t.ink_strong);
            ui.add_space(10.0);
            let ids: Vec<String> = picked.iter().map(|i| i.id.clone()).collect();
            if self.view == View::Trash {
                if draw::text_action(ui, "Restore", false).clicked() {
                    self.restore_checked(backend, &ids);
                }
                if draw::text_action(ui, "Delete forever", true).clicked() {
                    self.sheet = Some(Sheet::DeleteForever(ids));
                }
            } else {
                if draw::text_action(ui, "Download", false).clicked() {
                    let at = self.selected_indices();
                    self.download(backend, &at);
                }
                let all_starred = picked.iter().all(|i| i.starred);
                if draw::text_action(ui, if all_starred { "Unstar" } else { "Star" }, false).clicked() {
                    self.set_starred(backend, &picked, !all_starred);
                }
                if draw::text_action(ui, "Move", false).clicked() {
                    self.start_move(backend);
                }
                if draw::text_action(ui, "Delete", true).clicked() {
                    self.trash_selection(backend);
                }
            }
            if draw::text_action(ui, "Clear", false).clicked() {
                self.selection.clear();
            }
        });
    }

    fn list_ui(&mut self, ui: &mut Ui, rect: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        if let Some(e) = self.error.clone() {
            let mut retry = false;
            centered_note(ui, rect, "Couldn't load this list", &e, |ui| {
                retry = chrome::button(ui, "Try again", ButtonKind::Secondary, None, true).clicked();
            });
            if retry {
                self.reload(backend);
            }
            return;
        }
        if self.loading && self.items.is_empty() {
            let g = chrome::hud_galley(ui, "Loading…", t.muted);
            ui.painter().galley(rect.center_top() + vec2(-g.size().x / 2.0, 80.0), g, t.muted);
            return;
        }
        if self.items.is_empty() {
            let (title, note) = match &self.view {
                View::Folder(_) => ("This folder is empty", "Files you add here sync to every computer you connect."),
                View::Recent => ("Nothing opened yet", "Files you open show up here."),
                View::Starred => ("No starred files", "Star files and folders to find them here quickly."),
                View::Trash => ("The trash is empty", "Deleted files stay here for 30 days."),
                View::Search(_) => ("No matches", "Try another name, or part of one."),
            };
            // An empty folder offers the way to fill it, as the toolbar does.
            let mut upload = false;
            let can_upload = matches!(self.view, View::Folder(_)) && self.upload_target().is_some();
            centered_note(ui, rect, title, note, |ui| {
                if can_upload {
                    upload = draw::icon_label_button(ui, Glyph::Up, "Upload files", ButtonKind::Secondary, true).clicked();
                }
            });
            if upload {
                backend.pick_files();
            }
            return;
        }
        if self.grid {
            self.grid_ui(ui, rect, backend);
        } else {
            self.table_ui(ui, rect, backend);
        }
    }

    fn menu_ui(&mut self, ui: &mut Ui, backend: &mut dyn Backend) {
        let Some(menu) = &self.menu else { return };
        let Some(index) = self.items.iter().position(|i| i.id == menu.id) else {
            self.menu = None;
            return;
        };
        let item = self.items[index].clone();
        let picked = self.selected_items();
        let many = picked.len() > 1;
        let all_starred = picked.iter().all(|i| i.starred);
        let in_trash = self.view == View::Trash;
        let in_folder = matches!(self.view, View::Folder(_));
        let entries: Vec<(MenuAction, draw::MenuEntry<'_>)> = if in_trash {
            vec![(MenuAction::Restore, Some(("Restore", false, true))), (MenuAction::DeleteForever, Some(("Delete forever", true, true)))]
        } else {
            vec![
                (MenuAction::Open, (!many).then_some((if item.is_folder() { "Open" } else { "Download" }, false, true))),
                (MenuAction::Download, (many || item.is_folder()).then_some(("Download as zip", false, true))),
                (MenuAction::ShowInFolder, (!many && !in_folder && item.parent.is_some()).then_some(("Show in folder", false, true))),
                (MenuAction::Rule, None),
                (MenuAction::Rename, (!many).then_some(("Rename…", false, true))),
                (MenuAction::Move, Some(("Move to…", false, true))),
                (MenuAction::Star, Some((if all_starred { "Remove star" } else { "Star" }, false, true))),
                (MenuAction::Rule, None),
                (MenuAction::Trash, Some(("Delete", true, true))),
            ]
        };
        let shown: Vec<(MenuAction, draw::MenuEntry<'_>)> = entries.into_iter().filter(|(a, e)| e.is_some() || *a == MenuAction::Rule).collect();
        let labels: Vec<draw::MenuEntry<'_>> = shown.iter().map(|(_, e)| *e).collect();
        let (chosen, close) = draw::menu(ui, menu.at, &labels);
        if close {
            self.menu = None;
        }
        let Some(chosen) = chosen.map(|i| shown[i].0) else { return };
        let ids: Vec<String> = picked.iter().map(|i| i.id.clone()).collect();
        match chosen {
            MenuAction::Open => self.open_item(backend, index),
            MenuAction::Download => {
                let at = self.selected_indices();
                self.download(backend, &at);
            }
            MenuAction::ShowInFolder => self.show_in_folder(backend, index),
            MenuAction::Rename => self.start_rename(index),
            MenuAction::Move => self.start_move(backend),
            MenuAction::Star => self.set_starred(backend, &picked, !all_starred),
            MenuAction::Trash => self.trash_selection(backend),
            MenuAction::Restore => self.restore_checked(backend, &ids),
            MenuAction::DeleteForever => self.sheet = Some(Sheet::DeleteForever(ids)),
            MenuAction::Rule => {}
        }
    }

    fn start_rename(&mut self, index: usize) {
        if let Some(item) = self.items.get(index).cloned() {
            self.sheet_error = None;
            let name = item.name.clone();
            self.sheet = Some(Sheet::Rename { item: Box::new(item), name });
        }
    }

    /// While files are dragged over the page: where they will go.
    fn drop_overlay(&self, ui: &Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        let place = match self.upload_target() {
            Some((_, path)) if path.is_empty() => "My files".to_owned(),
            Some((_, path)) => path.rsplit('/').next().unwrap_or_default().to_owned(),
            None => return,
        };
        let r = rect.shrink(2.0);
        ui.painter().rect(r, 0.0, chrome::tint(t.accent, 0.14), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            format!("DROP TO UPLOAD TO {}", place.to_uppercase()),
            FontId::monospace(13.0),
            t.ink_strong,
        );
    }

    /// The uploads, bottom right: each file with its progress.
    fn uploads_ui(&mut self, ui: &mut Ui, area: Rect) {
        let t = Tokens::get(ui.ctx());
        if self.uploads.list.is_empty() || self.uploads.hidden {
            return;
        }
        let total = self.uploads.list.len();
        let done = self.uploads.list.iter().filter(|u| u.state == UpState::Done).count();
        let active = self.uploads.active();
        let rows = self.uploads.list.len().min(5);
        let w = 360.0_f32.min(area.width() - 32.0);
        let h = 44.0 + rows as f32 * 46.0;
        // On the page margin (24, a phone 16), like the list it sits over.
        let m = if area.width() < 600.0 { 16.0 } else { 24.0 };
        let rect = Rect::from_min_size(pos2(area.right() - w - m, area.bottom() - h - m), vec2(w, h));
        egui::Area::new(ui.scope_id().with("uploads")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(t.bg)
                .stroke(egui::Stroke::new(1.0, t.line_strong))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(if t.dark { 140 } else { 36 }),
                })
                .show(ui, |ui| {
                    ui.set_width(w);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let (head, head_resp) = ui.allocate_exact_size(vec2(w, 44.0), egui::Sense::hover());
                    ui.painter().hline(head.x_range(), head.bottom() - 0.5, t.hairline());
                    let title =
                        if active { format!("Uploading {} of {total}", (done + 1).min(total)) } else { format!("{done} of {total} uploaded") };
                    ui.painter().text(
                        pos2(head.left() + 14.0, head.center().y),
                        Align2::LEFT_CENTER,
                        title.to_uppercase(),
                        FontId::monospace(11.0),
                        t.ink_strong,
                    );
                    head_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, title.to_uppercase()));
                    let close = Rect::from_center_size(pos2(head.right() - 22.0, head.center().y), vec2(28.0, 28.0));
                    let resp = ui.interact(close, ui.scope_id().with("uploads-close"), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(close, 0.0, t.sunken);
                    }
                    draw::paint_glyph(ui, Glyph::Close, close.center(), t.ink);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close uploads"));
                    if resp.clicked() {
                        if active {
                            self.uploads.hidden = true;
                        } else {
                            self.uploads.list.clear();
                        }
                    }
                    egui::ScrollArea::vertical().max_height(rows as f32 * 46.0).show(ui, |ui| {
                        for up in &self.uploads.list {
                            let (r, row) = ui.allocate_exact_size(vec2(w, 46.0), egui::Sense::hover());
                            let name = draw::fit_text(ui, &up.name, FontId::proportional(13.0), t.ink_strong, w - 120.0);
                            ui.painter().galley(pos2(r.left() + 14.0, r.top() + 9.0), name, t.ink_strong);
                            let (status, tone) = match &up.state {
                                UpState::Queued => ("Waiting".to_owned(), t.muted),
                                UpState::Sending | UpState::Replacing => {
                                    (format!("{} of {}", super::bytes(up.sent), super::bytes(up.file.size)), t.muted)
                                }
                                UpState::Exists => ("Name taken".to_owned(), t.warn),
                                UpState::Done => ("Done".to_owned(), t.good),
                                UpState::Skipped => ("Skipped".to_owned(), t.muted),
                                UpState::Failed(_) => ("Failed".to_owned(), t.bad),
                            };
                            ui.painter().text(
                                pos2(r.right() - 14.0, r.top() + 16.0),
                                Align2::RIGHT_CENTER,
                                status.to_uppercase(),
                                FontId::monospace(10.0),
                                tone,
                            );
                            let track = Rect::from_min_size(pos2(r.left() + 14.0, r.bottom() - 12.0), vec2(w - 28.0, 3.0));
                            ui.painter().rect_filled(track, 0.0, t.line);
                            let fraction = match up.state {
                                UpState::Done => 1.0,
                                _ if up.file.size > 0 => up.sent as f32 / up.file.size as f32,
                                _ => 0.0,
                            };
                            let fill = Rect::from_min_size(track.min, vec2(track.width() * fraction.clamp(0.0, 1.0), 3.0));
                            ui.painter().rect_filled(fill, 0.0, if matches!(up.state, UpState::Failed(_)) { t.bad } else { t.accent });
                            let text = format!("{} {}", up.name, status);
                            let row = if let UpState::Failed(why) = &up.state { row.on_hover_text(why) } else { row };
                            row.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &text));
                        }
                    });
                });
        });
    }

    fn toast_ui(&mut self, ui: &mut Ui, area: Rect, backend: &mut dyn Backend) {
        let t = Tokens::get(ui.ctx());
        let Some(toast) = &self.toast else { return };
        if self.now > toast.until {
            self.toast = None;
            return;
        }
        let galley = ui.painter().layout_no_wrap(toast.text.clone(), FontId::proportional(13.5), if toast.bad { t.bad } else { t.invert_fg });
        let undo_w = if toast.undo.is_some() { 64.0 } else { 0.0 };
        let w = (galley.size().x + 40.0 + undo_w).min(area.width() - 32.0);
        let rect = Rect::from_center_size(pos2(area.center().x, area.bottom() - 36.0), vec2(w, 40.0));
        let fill = if toast.bad { t.bg } else { t.invert_bg };
        ui.painter().add(
            egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(if t.dark { 140 } else { 36 }) }
                .as_shape(rect, 0.0),
        );
        ui.painter().rect(rect, 0.0, fill, egui::Stroke::new(1.0, if toast.bad { t.bad } else { fill }), egui::StrokeKind::Inside);
        let text_rect = rect.shrink2(vec2(20.0, 0.0));
        ui.painter().with_clip_rect(text_rect).galley(pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0), galley, t.invert_fg);
        let label = ui.interact(rect, ui.scope_id().with("toast"), egui::Sense::hover());
        let text = toast.text.clone();
        label.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Status, true, &text));
        if let Some(undo) = toast.undo.clone() {
            let button = Rect::from_min_max(pos2(rect.right() - undo_w, rect.top()), rect.max);
            let resp = ui.interact(button, ui.scope_id().with("toast-undo"), egui::Sense::click());
            let color = if t.dark { t.accent } else { egui::Color32::from_rgb(0xa9, 0xc4, 0x8e) };
            ui.painter().text(button.center(), Align2::CENTER_CENTER, "UNDO", FontId::monospace(11.0), color);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "UNDO"));
            if resp.clicked() {
                self.toast = None;
                match undo {
                    Undo::Restore(ids) => self.restore(backend, &ids),
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    Open,
    Rename,
    Move,
    Download,
    ShowInFolder,
    Star,
    Trash,
    Restore,
    DeleteForever,
    Rule,
}

/// The items of a listing page that aren't CraftCloud's own, and the next cursor.
fn keep<T>(page: Page<T>, resource: impl Fn(T) -> Resource, place: impl Fn(Resource) -> Item) -> (Vec<Item>, Option<String>) {
    let items = page.items.into_iter().map(resource).filter(|r| !r.is_internal()).map(place).collect();
    (items, page.next_cursor)
}

/// What went wrong with one request of a change, if anything (a batch can fail in part).
fn change_problem(resp: &HttpResponse) -> Option<String> {
    if let Ok(result) = batch::parse(resp)
        && !result.all_done()
    {
        return Some(result.failed.into_iter().map(|f| f.error).collect::<Vec<_>>().join(" · "));
    }
    craftcloud_core::api::expect_ok(resp).err().map(|e| e.to_string())
}

/// "1 folder", "3 files".
fn count(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// "20.1 KB of 107 GB".
fn storage_line(ui: &mut Ui, used: u64, quota: u64) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(egui::RichText::new(super::bytes(used)).size(15.0).color(t.ink_strong));
        if quota > 0 {
            ui.label(egui::RichText::new(format!("of {}", super::bytes(quota))).size(13.0).color(t.muted));
        }
    });
}

/// A centred title and note in `rect` (empty folders, errors), with optional actions under them.
fn centered_note(ui: &mut Ui, rect: Rect, title: &str, note: &str, actions: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    let r = Rect::from_center_size(pos2(rect.center().x, rect.top() + 130.0), vec2(rect.width().min(440.0), 180.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
        ui.label(egui::RichText::new(title).size(22.0).family(crate::theme::display()).color(t.ink_strong));
        ui.add_space(6.0);
        ui.add(egui::Label::new(egui::RichText::new(note).size(14.0).color(t.muted)).wrap().halign(egui::Align::Center));
        ui.add_space(14.0);
        actions(ui);
    });
}

fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

/// The page size a search asks with (T-043). OxiCloud caches search answers for a few minutes per
/// exact query string, across sessions, so a fixed query could return a file deleted meanwhile
/// (from another device too). The size runs through 100 to 499, moved on by the wall clock (one
/// step per 2 s, a full turn in about 13 minutes) and by every search in this session, so a key
/// comes back only long after its answer has expired. Every size is at least 100, so the first
/// page holds the same results either way.
fn search_limit(now_unix: Option<f64>, bust: u32) -> u32 {
    let seed = now_unix.map_or(0, |t| (t.max(0.0) / 2.0) as u64);
    100 + ((seed + u64::from(bust)) % 400) as u32
}

#[cfg(test)]
mod tests;
