//! Restoring from the trash without a raw server error (T-047). OxiCloud restores an item to the
//! path it had, and answers 500 when something of that name is there now. So each restore first
//! asks WebDAV whether the path is taken. When it is, a sheet says so and offers to restore the
//! item as "<name> (restored)".
//!
//! OxiCloud can neither restore under another name nor rename an item in the trash, so "Restore
//! as" runs four steps, one after the other:
//! 1. the file that's there now moves aside, to a temporary name,
//! 2. the item is restored to its old path,
//! 3. the restored item gets its new name,
//! 4. the file that was there moves back.
//!
//! If a step fails, the file that was there is moved back, if it can be.

use std::collections::VecDeque;

use craftcloud_core::api::{dav, trash};
use craftcloud_core::http::HttpRequest;

use super::model::Item;
use super::{Files, Key, Sheet};
use crate::backend::{Backend, NetResult};

/// "Plan.txt" → "Plan (restored).txt"; a folder or a name without an extension gets the words at
/// the end (the CLI's `trash restore --rename` names it the same).
pub fn restored_name(name: &str, folder: bool) -> String {
    craftcloud_core::names::restored_name(name, folder, 1)
}

/// A sibling path of `path` called `name`.
fn beside(path: &str, name: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{name}"),
        None => name.to_owned(),
    }
}

/// The steps of a "Restore as" still to send, and how to put things back if one fails.
#[derive(Default)]
pub struct Chain {
    steps: VecDeque<HttpRequest>,
    /// Moves the file that was there back, while it is aside.
    undo: Option<HttpRequest>,
    /// The same move, kept ready until step 1 has put the file aside.
    undo_later: Option<HttpRequest>,
    /// Which step is out (1 to 4).
    step: u8,
    done: String,
}

impl Files {
    /// Restores `ids` from the trash, each after checking its old place is free.
    pub(super) fn restore_checked(&mut self, backend: &mut dyn Backend, ids: &[String]) {
        for id in ids {
            let Some(item) = self.trash_item(id) else { continue };
            if item.path.is_empty() {
                self.restore(backend, std::slice::from_ref(id));
            } else {
                self.net.send(backend, Key::RestoreCheck(id.clone()), dav::stat(&self.server, &item.path));
            }
        }
        self.selection.clear();
    }

    fn trash_item(&self, id: &str) -> Option<Item> {
        self.trash_all.iter().chain(&self.items).find(|i| i.id == id).cloned()
    }

    /// The answer to "is its old place taken?".
    pub(super) fn on_restore_check(&mut self, backend: &mut dyn Backend, id: &str, result: NetResult) {
        let taken = matches!(&result, Ok(resp) if resp.is_success());
        if !taken {
            // Free (404), or the check itself failed: the restore says what it says.
            self.restore(backend, &[id.to_owned()]);
            return;
        }
        let Some(item) = self.trash_item(id) else { return };
        if self.sheet.is_none() {
            self.sheet_error = None;
            self.sheet = Some(Sheet::RestoreTaken(Box::new(item)));
        } else {
            self.notice(&format!("“{}” is already there; it stays in the trash", item.name), true, None);
        }
    }

    /// Starts "Restore as <name> (restored)" for `item`.
    pub(super) fn restore_as(&mut self, backend: &mut dyn Backend, item: &Item) {
        let folder = item.is_folder();
        let new_name = restored_name(&item.name, folder);
        let aside = beside(&item.path, &format!(".{}.craftcloud-restoring", item.name));
        let renamed = beside(&item.path, &new_name);
        let s = &self.server;
        let steps = VecDeque::from([
            dav::move_to(s, &item.path, &aside, folder),
            trash::restore(s, &item.id),
            dav::move_to(s, &item.path, &renamed, folder),
            dav::move_to(s, &aside, &item.path, folder),
        ]);
        let undo_later = Some(dav::move_to(s, &aside, &item.path, folder));
        self.chain = Chain { steps, undo: None, undo_later, step: 0, done: format!("Restored as “{new_name}”") };
        self.next_chain_step(backend);
    }

    fn next_chain_step(&mut self, backend: &mut dyn Backend) {
        if let Some(req) = self.chain.steps.pop_front() {
            self.chain.step += 1;
            self.net.send(backend, Key::Chain, req);
        } else {
            let done = std::mem::take(&mut self.chain.done);
            self.chain = Chain::default();
            self.notice(&done, false, None);
            self.reload(backend);
        }
    }

    /// The answer to a step of "Restore as".
    pub(super) fn on_chain(&mut self, backend: &mut dyn Backend, result: NetResult) {
        let ok = matches!(&result, Ok(resp) if resp.is_success());
        if ok {
            match self.chain.step {
                // The file that was there is aside from now on, until step 4 brings it back.
                1 => self.chain.undo = self.chain.undo_later.take(),
                4 => self.chain.undo = None,
                _ => {}
            }
            self.next_chain_step(backend);
            return;
        }
        let why = match result {
            Ok(resp) => format!("the server answered {}", resp.status),
            Err(e) => e,
        };
        if let Some(undo) = self.chain.undo.take() {
            self.net.send(backend, Key::Ignore, undo);
        }
        self.chain = Chain::default();
        self.notice(&format!("Couldn't restore it under another name ({why})"), true, None);
        self.reload(backend);
    }
}

#[cfg(test)]
mod tests {
    use super::{beside, restored_name};

    #[test]
    fn restored_names_keep_the_extension() {
        assert_eq!(restored_name("Plan.txt", false), "Plan (restored).txt");
        assert_eq!(restored_name("notes", false), "notes (restored)");
        assert_eq!(restored_name(".env", false), ".env (restored)");
        assert_eq!(restored_name("Photos.2026", true), "Photos.2026 (restored)");
        assert_eq!(beside("Documents/a.txt", "b.txt"), "Documents/b.txt");
        assert_eq!(beside("a.txt", "b.txt"), "b.txt");
    }
}
