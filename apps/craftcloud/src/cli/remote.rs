//! The signed-in server a command talks to: which one, its session (shared with the desktop
//! client), and turning cloud paths into the ids the REST API wants.

use std::path::PathBuf;

use craftcloud_core::ServerUrl;
use craftcloud_core::api::resources::{Folder, Resource};
use craftcloud_core::api::{dav, folders};
use craftcloud_core::http::{HttpRequest, HttpResponse};
use craftcloud_core::sync::ClientSettings;
use craftcloud_native::secrets::TokenStore;
use craftcloud_native::session::Session;
use craftcloud_native::settings::{self, Store};

use super::output::{CliError, Fail, Result};

/// What every command gets: the options and the client's settings.
pub struct Ctx {
    pub json: bool,
    server_override: Option<String>,
    pub store: Store,
}

impl Ctx {
    pub fn new(json: bool, server_override: Option<String>) -> Self {
        Self { json, server_override, store: settings::open() }
    }

    pub fn settings(&self) -> ClientSettings {
        self.store.get().0.clone()
    }

    /// The settings folder (where the control channel and the refresh lock live).
    pub fn dir(&self) -> Option<PathBuf> {
        self.store.dir().map(Into::into)
    }

    /// `--server` / `CRAFTCLOUD_SERVER`, else the server the desktop client uses.
    pub fn server(&self) -> Result<ServerUrl> {
        let text = self.server_override.clone().unwrap_or_else(|| self.settings().server);
        if text.trim().is_empty() {
            return Err(CliError::new(Fail::SignedOut, "Not signed in. Run `craftcloud login <server>` first."));
        }
        ServerUrl::parse(&text).map_err(|e| CliError::usage(format!("{text}: {e}")))
    }

    /// Whether this command talks to the server the desktop client syncs with (no `--server`, or
    /// the same one), so the local sync folder and the running client speak for it.
    pub fn uses_clients_server(&self) -> bool {
        match &self.server_override {
            None => true,
            Some(text) => match (ServerUrl::parse(text), ServerUrl::parse(&self.settings().server)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            },
        }
    }

    pub fn tokens(&self, server: &ServerUrl) -> TokenStore {
        TokenStore::new(&server.to_string(), self.store.dir())
    }

    /// A session on the stored sign-in for the server.
    pub fn remote(&self) -> Result<Remote> {
        let server = self.server()?;
        let tokens = self
            .tokens(&server)
            .load()
            .ok_or_else(|| CliError::new(Fail::SignedOut, format!("Not signed in to {server}. Run `craftcloud login {server}`.")))?;
        let session = Session::shared(server.clone(), tokens, self.tokens(&server), self.store.dir());
        Ok(Remote { session, root: None })
    }
}

/// `/Documents//notes.md/` → `Documents/notes.md`. Refuses `.` and `..`.
pub fn clean(path: &str) -> Result<String> {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| *p == "." || *p == "..") {
        return Err(CliError::usage(format!("{path}: use full paths from the top of the cloud, without . or ..")));
    }
    Ok(parts.join("/"))
}

/// `Documents/notes.md` → (`Documents`, `notes.md`).
pub fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") }
}

pub struct Remote {
    pub session: Session,
    root: Option<Folder>,
}

impl Remote {
    pub fn server(&self) -> &ServerUrl {
        &self.session.server
    }

    /// Sends a request; transport errors become CLI errors.
    pub fn call(&mut self, req: &HttpRequest) -> Result<HttpResponse> {
        self.session.call(req).map_err(CliError::transport)
    }

    /// Sends a request and reads the answer with `parse`.
    pub fn ask<T>(
        &mut self,
        req: &HttpRequest,
        parse: impl Fn(&HttpResponse) -> std::result::Result<T, craftcloud_core::api::ApiError>,
    ) -> Result<T> {
        let resp = self.call(req)?;
        let root = self.root.as_ref().map(|r| r.name.clone()).unwrap_or_default();
        parse(&resp).map_err(|e| CliError::api(&e).plain_paths(&root))
    }

    /// The drive's root folder (what WebDAV's `/` is).
    pub fn root(&mut self) -> Result<Folder> {
        if let Some(r) = &self.root {
            return Ok(r.clone());
        }
        let roots = self.ask(&folders::roots(self.server()), folders::parse_roots)?;
        let root = roots.into_iter().next().ok_or_else(|| CliError::error("the server shows no root folder for this account"))?;
        self.root = Some(root.clone());
        Ok(root)
    }

    /// Everything in a folder, all pages.
    pub fn children(&mut self, folder_id: &str) -> Result<Vec<Resource>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self.ask(&folders::contents(self.server(), folder_id, Some(200), cursor.as_deref()), folders::parse_contents_all)?;
            out.extend(page.items);
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => return Ok(out),
            }
        }
    }

    /// The file or folder at a cloud path (`""` is the root folder).
    pub fn resolve(&mut self, path: &str) -> Result<Resource> {
        let mut current = Resource::Folder(self.root()?);
        let mut walked = String::new();
        for name in path.split('/').filter(|s| !s.is_empty()) {
            let Resource::Folder(folder) = &current else {
                return Err(CliError::not_found(format!("{walked} is a file, not a folder")));
            };
            let id = folder.id.clone();
            walked = join(&walked, name);
            current = self
                .children(&id)?
                .into_iter()
                .find(|r| r.name() == name)
                .ok_or_else(|| CliError::not_found(format!("{walked}: no such file or folder")))?;
        }
        Ok(current)
    }

    /// What WebDAV says about one path: `None` when nothing is there. The root is a folder.
    pub fn stat(&mut self, path: &str) -> Result<Option<dav::Entry>> {
        if path.is_empty() {
            return Ok(Some(dav::Entry { path: String::new(), is_folder: true, etag: None, size: 0, modified: None }));
        }
        let (parent, _) = split(path);
        let resp = self.call(&dav::list(self.server(), parent))?;
        if resp.status == 404 {
            return Ok(None);
        }
        let entries = dav::parse_list(&resp).map_err(|e| CliError::api(&e))?;
        Ok(entries.into_iter().find(|e| e.path == path))
    }
}

#[cfg(test)]
mod tests {
    use super::{clean, join, split};

    #[test]
    fn paths_are_cleaned() {
        assert_eq!(clean("/Documents//notes.md/").unwrap(), "Documents/notes.md");
        assert_eq!(clean("").unwrap(), "");
        assert_eq!(clean("/").unwrap(), "");
        assert!(clean("Documents/../etc").is_err());
        assert_eq!(split("Documents/Q3/a.txt"), ("Documents/Q3", "a.txt"));
        assert_eq!(split("a.txt"), ("", "a.txt"));
        assert_eq!(join("", "a"), "a");
        assert_eq!(join("x", "a"), "x/a");
    }
}
