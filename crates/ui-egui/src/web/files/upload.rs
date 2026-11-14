//! Uploads from the browser: chosen with UPLOAD or dropped on the page. Each file goes to the
//! folder that was open, over WebDAV with the session cookie:
//!
//! - `PUT` with `If-None-Match: *` creates it; a 412 means the name is taken, and the person
//!   chooses **Keep both** (`name (2).ext`), **Replace** or **Skip** (optionally for all).
//! - **Replace** keeps the old bytes first, as everywhere in CraftCloud: find the file's ETag, copy
//!   it to `.craftcloud/versions/<path>/<stamp>` (making the folders), prune to the last 5, then
//!   `PUT` with `If-Match` so a change made meanwhile is never overwritten.

use std::collections::VecDeque;

use craftcloud_core::api::dav::{self, Uploaded, Write};
use craftcloud_core::api::versions;
use craftcloud_core::http::{HttpRequest, HttpResponse};

use super::model::{self, guess_mime};
use super::{Files, Key};
use crate::backend::{Backend, LocalFile, NetResult};

/// Uploads that run at once.
const AT_ONCE: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpState {
    Queued,
    Sending,
    /// The name is taken; waiting for the person's choice.
    Exists,
    /// Keeping the old version before replacing it.
    Replacing,
    Done,
    Skipped,
    Failed(String),
}

/// One step of replacing a file.
#[derive(Clone, Debug)]
pub enum Step {
    /// List the folder to learn the file's current ETag.
    FindEtag,
    /// A request whose answer must be a success, or 405 for a `MKCOL` of a folder that exists, or
    /// 404 for a snapshot of nothing.
    Plain(HttpRequest),
    /// List the versions and delete all but the newest 5.
    Prune,
    /// The upload itself, with `If-Match`.
    Put,
}

/// What to do with names that are taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Debug)]
pub struct Upload {
    pub file: LocalFile,
    /// The folder's cloud path.
    pub folder: String,
    /// The name it gets (changes for "keep both").
    pub name: String,
    pub state: UpState,
    pub sent: u64,
    pub request: Option<u64>,
    copy: u32,
    etag: Option<String>,
    steps: VecDeque<Step>,
}

impl Upload {
    pub fn path(&self) -> String {
        model::join(&self.folder, &self.name)
    }

    pub fn finished(&self) -> bool {
        matches!(self.state, UpState::Done | UpState::Skipped | UpState::Failed(_))
    }
}

#[derive(Default)]
pub struct Uploads {
    pub list: Vec<Upload>,
    /// The choice for every further clash, once the person ticked "for all".
    pub for_all: Option<Clash>,
    /// The panel is folded away.
    pub hidden: bool,
}

impl Uploads {
    pub fn active(&self) -> bool {
        self.list.iter().any(|u| !u.finished())
    }

    /// The first upload waiting for a clash choice.
    pub fn clash(&self) -> Option<usize> {
        self.list.iter().position(|u| u.state == UpState::Exists)
    }
}

impl Files {
    /// Queues files for the open folder (the drive's root outside a folder).
    pub(super) fn enqueue(&mut self, backend: &mut dyn Backend, files: Vec<LocalFile>) {
        if files.is_empty() {
            return;
        }
        let Some((_, folder)) = self.upload_target() else {
            for f in files {
                backend.forget_file(f.key);
            }
            self.notice("Open a folder first, then upload.", true, None);
            return;
        };
        self.uploads.hidden = false;
        if !self.uploads.active() {
            self.uploads.list.retain(|u| !u.finished());
            self.uploads.for_all = None;
        }
        for file in files {
            let name = file.name.clone();
            self.uploads.list.push(Upload {
                file,
                folder: folder.clone(),
                name,
                state: UpState::Queued,
                sent: 0,
                request: None,
                copy: 1,
                etag: None,
                steps: VecDeque::new(),
            });
        }
        self.pump(backend);
    }

    /// Where an upload goes: the open folder when its path is known, else the drive's root.
    pub(super) fn upload_target(&self) -> Option<(String, String)> {
        match &self.view {
            super::View::Folder(id) if !id.is_empty() && (self.root.as_deref() == Some(id.as_str()) || self.crumbs_for == self.generation) => {
                let path = if self.root.as_deref() == Some(id.as_str()) { String::new() } else { self.folder_path.clone() };
                Some((id.clone(), path))
            }
            super::View::Folder(_) => None,
            _ => self.root.clone().map(|r| (r, String::new())),
        }
    }

    /// Starts queued uploads while fewer than [`AT_ONCE`] run.
    pub(super) fn pump(&mut self, backend: &mut dyn Backend) {
        loop {
            let running = self.uploads.list.iter().filter(|u| matches!(u.state, UpState::Sending | UpState::Replacing)).count();
            if running >= AT_ONCE {
                return;
            }
            let Some(n) = self.uploads.list.iter().position(|u| u.state == UpState::Queued) else { return };
            self.put(backend, n, Write::Create);
        }
    }

    fn put(&mut self, backend: &mut dyn Backend, n: usize, write: Write) {
        let up = &mut self.uploads.list[n];
        let mime = if up.file.mime.is_empty() { guess_mime(&up.name).to_owned() } else { up.file.mime.clone() };
        let req = dav::upload(&self.server, &up.path(), Vec::new(), &write).header("Content-Type", mime);
        up.state = if matches!(write, Write::Create) { UpState::Sending } else { UpState::Replacing };
        up.sent = 0;
        let key = Key::Upload(up.file.key);
        let file = up.file.key;
        let id = self.net.send_file(backend, key, req, file);
        self.uploads.list[n].request = Some(id);
    }

    pub(super) fn on_progress(&mut self, progress: &[(u64, u64)]) {
        for (id, sent) in progress {
            if let Some(up) = self.uploads.list.iter_mut().find(|u| u.request == Some(*id)) {
                up.sent = *sent;
            }
        }
    }

    /// The answer to an upload's `PUT`.
    pub(super) fn on_uploaded(&mut self, backend: &mut dyn Backend, key: u64, result: NetResult) {
        let Some(n) = self.uploads.list.iter().position(|u| u.file.key == key && !u.finished()) else { return };
        let outcome = result.and_then(|resp| dav::parse_upload(&resp).map_err(|e| upload_error(&e, &resp)));
        let replacing = self.uploads.list[n].state == UpState::Replacing;
        match outcome {
            Ok(Uploaded::Saved { .. }) => {
                let up = &mut self.uploads.list[n];
                up.state = UpState::Done;
                up.sent = up.file.size;
                let path = up.path();
                backend.forget_file(key);
                if !replacing {
                    // A new file starts with no history: versions kept at this path were another
                    // file's, and restoring one would put that file's bytes here (T-050).
                    self.history_fresh(backend, &path);
                }
            }
            Ok(Uploaded::Conflict) if replacing => {
                self.fail(backend, n, "It changed on the server meanwhile; nothing was overwritten. Try again.".into());
            }
            Ok(Uploaded::Conflict) => match self.uploads.for_all {
                Some(choice) => self.resolve(backend, n, choice),
                None => self.uploads.list[n].state = UpState::Exists,
            },
            Err(why) => self.fail(backend, n, why),
        }
        self.pump(backend);
        self.after_upload(backend);
    }

    fn fail(&mut self, backend: &mut dyn Backend, n: usize, why: String) {
        let up = &mut self.uploads.list[n];
        up.state = UpState::Failed(why);
        backend.forget_file(up.file.key);
    }

    /// Reloads the folder once the last upload is in.
    fn after_upload(&mut self, backend: &mut dyn Backend) {
        if self.uploads.active() {
            return;
        }
        let done = self.uploads.list.iter().filter(|u| u.state == UpState::Done).count();
        if done > 0 {
            self.reload(backend);
        }
    }

    /// The person's answer to a taken name (and for the rest too, when `all`).
    pub(super) fn resolve(&mut self, backend: &mut dyn Backend, n: usize, choice: Clash) {
        match choice {
            Clash::Skip => {
                let up = &mut self.uploads.list[n];
                up.state = UpState::Skipped;
                backend.forget_file(up.file.key);
            }
            Clash::KeepBoth => {
                let up = &mut self.uploads.list[n];
                up.copy += 1;
                up.name = model::numbered(&up.file.name, up.copy);
                up.state = UpState::Queued;
            }
            Clash::Replace => {
                let stamp = super::time::stamp(backend.wall_clock().map_or(0.0, |c| c.unix));
                let path = self.uploads.list[n].path();
                let mut steps: VecDeque<Step> = VecDeque::from([Step::FindEtag]);
                steps.extend(versions::make_folders(&self.server, &path).into_iter().map(Step::Plain));
                steps.push_back(Step::Plain(versions::snapshot(&self.server, &path, &stamp)));
                steps.push_back(Step::Prune);
                steps.push_back(Step::Put);
                let up = &mut self.uploads.list[n];
                up.steps = steps;
                up.state = UpState::Replacing;
                self.next_step(backend, n);
            }
        }
        self.pump(backend);
        self.after_upload(backend);
    }

    fn next_step(&mut self, backend: &mut dyn Backend, n: usize) {
        let key = self.uploads.list[n].file.key;
        let Some(step) = self.uploads.list[n].steps.front().cloned() else { return };
        let path = self.uploads.list[n].path();
        match step {
            Step::FindEtag => {
                let req = dav::list(&self.server, &self.uploads.list[n].folder);
                self.net.send(backend, Key::UpStep(key), req);
            }
            Step::Plain(req) => self.net.send(backend, Key::UpStep(key), req),
            Step::Prune => self.net.send(backend, Key::UpStep(key), versions::list(&self.server, &path)),
            Step::Put => {
                self.uploads.list[n].steps.pop_front();
                let etag = self.uploads.list[n].etag.clone().unwrap_or_default();
                self.put(backend, n, Write::Replace { etag });
            }
        }
    }

    /// The answer to one step of a replace.
    pub(super) fn on_step(&mut self, backend: &mut dyn Backend, key: u64, result: NetResult) {
        let Some(n) = self.uploads.list.iter().position(|u| u.file.key == key && u.state == UpState::Replacing) else { return };
        let resp = match result {
            Ok(r) => r,
            Err(why) => return self.fail(backend, n, why),
        };
        let Some(step) = self.uploads.list[n].steps.pop_front() else { return };
        match step {
            Step::FindEtag => {
                let path = self.uploads.list[n].path();
                let etag = dav::parse_list(&resp).ok().and_then(|entries| entries.into_iter().find(|e| e.path == path).and_then(|e| e.etag));
                match etag {
                    Some(etag) => self.uploads.list[n].etag = Some(etag),
                    None => return self.fail(backend, n, "Couldn't read the file on the server.".into()),
                }
            }
            Step::Plain(req) => {
                let ok = resp.is_success()
                    || (req.method == craftcloud_core::http::Method::Mkcol && dav::folder_exists_after(&resp))
                    || versions::snapshot_done(&resp).is_ok();
                if !ok {
                    let why = craftcloud_core::api::expect_ok(&resp).err().map_or_else(|| "the server refused".into(), |e| e.to_string());
                    return self.fail(backend, n, format!("Couldn't keep the old version: {why}"));
                }
            }
            Step::Prune => {
                let path = self.uploads.list[n].path();
                let names: Vec<String> = versions::parse_list(&resp).map(|v| v.into_iter().map(|v| v.stamp).collect()).unwrap_or_default();
                for old in versions::to_prune(names) {
                    // Best effort: a version left over is harmless.
                    self.net.send(backend, Key::Ignore, versions::delete(&self.server, &path, &old));
                }
            }
            Step::Put => {}
        }
        self.next_step(backend, n);
    }

    /// The first clash, for the sheet.
    pub(super) fn clash_name(&self) -> Option<(usize, String, String)> {
        let n = self.uploads.clash()?;
        let up = &self.uploads.list[n];
        let folder = if up.folder.is_empty() { "My files".to_owned() } else { up.folder.rsplit('/').next().unwrap_or_default().to_owned() };
        Some((n, up.name.clone(), folder))
    }
}

fn upload_error(e: &craftcloud_core::api::ApiError, resp: &HttpResponse) -> String {
    match resp.status {
        507 => "Not enough space left on the server.".into(),
        413 => "The file is too large for the server.".into(),
        _ => e.to_string(),
    }
}
