//! Kept versions follow their files, as in the desktop client and the CLI:
//! - a rename or move takes the file's (or folder's) `.craftcloud/versions/` history along
//!   (T-050, `versions::follow_move`);
//! - a new file starts with no history: one still kept at its path was another file's (T-050);
//! - what is deleted for good (a trash item, the emptied trash) loses its history too, unless a
//!   file is at that path again (T-049).
//!
//! The requests go one after the other, after the change itself worked, and are best effort: a
//! failed step doesn't undo the change or stop the next one.

use std::collections::VecDeque;

use craftcloud_core::api::{dav, versions};
use craftcloud_core::http::HttpRequest;

use super::{Files, Key};
use crate::backend::{Backend, NetResult};

pub(super) enum Step {
    Send(HttpRequest),
    /// Drop the history of this path when nothing is there now (a `PROPFIND` says 404).
    DropIfGone(String),
}

#[derive(Default)]
pub(super) struct Queue {
    steps: VecDeque<Step>,
    busy: bool,
}

impl Files {
    /// The steps that take the histories of `moves` (`(from, to)` cloud paths) along.
    pub(super) fn history_moves(&self, moves: &[(String, String)]) -> Vec<Step> {
        moves
            .iter()
            .filter(|(from, to)| !from.is_empty() && !to.is_empty() && from != to)
            .flat_map(|(from, to)| versions::follow_move(&self.server, from, to))
            .map(Step::Send)
            .collect()
    }

    /// The steps that drop the histories of `paths`, deleted for good, where nothing is now.
    pub(super) fn history_gone(paths: impl IntoIterator<Item = String>) -> Vec<Step> {
        paths.into_iter().filter(|p| !p.is_empty()).map(Step::DropIfGone).collect()
    }

    /// Drops the history at `path`, where a new file was just made.
    pub(super) fn history_fresh(&mut self, backend: &mut dyn Backend, path: &str) {
        let req = versions::drop_history(&self.server, path);
        self.history_then(backend, vec![Step::Send(req)]);
    }

    /// Queues `steps` and sends the first when nothing is out.
    pub(super) fn history_then(&mut self, backend: &mut dyn Backend, steps: Vec<Step>) {
        self.history.steps.extend(steps);
        self.history_pump(backend);
    }

    fn history_pump(&mut self, backend: &mut dyn Backend) {
        if self.history.busy {
            return;
        }
        let Some(step) = self.history.steps.pop_front() else { return };
        self.history.busy = true;
        match step {
            Step::Send(req) => self.net.send(backend, Key::History(None), req),
            Step::DropIfGone(path) => {
                let req = dav::stat(&self.server, &path);
                self.net.send(backend, Key::History(Some(path)), req);
            }
        }
    }

    /// An answer to a history step; `gone_check` is the path a `PROPFIND` asked about.
    pub(super) fn on_history(&mut self, backend: &mut dyn Backend, gone_check: Option<String>, result: &NetResult) {
        self.history.busy = false;
        if let Some(path) = gone_check
            && matches!(result, Ok(resp) if resp.status == 404)
        {
            self.history.steps.push_front(Step::Send(versions::drop_history(&self.server, &path)));
        }
        self.history_pump(backend);
    }
}
