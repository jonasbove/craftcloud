//! The signed-in server a command talks to: which one, its session (shared with the desktop
//! client), and turning cloud paths into the ids the REST API wants.

use std::path::PathBuf;

use craftcloud_core::ServerUrl;
use craftcloud_core::api::resources::{Folder, Resource};
use craftcloud_core::api::{dav, folders};
use craftcloud_core::http::{HttpRequest, HttpResponse};
use craftcloud_core::storage::probe::{Base, StoredCheck};
use craftcloud_core::sync::{ClientSettings, Storage};
use craftcloud_native::experimental;
use craftcloud_native::secrets::{Credential, TokenStore};
use craftcloud_native::session::{AppPasswordSession, Session};
use craftcloud_native::settings::{self, Store};
use craftcloud_native::storage::RemoteStorage;
use craftcloud_native::storage::probe::Limits;
use craftcloud_native::storage::profile::{self, ServerProfile};
use craftcloud_native::storage::webdav::WebDav;

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

    /// Whether this build may use the settings' kind of server (docs/WEBDAV-STORAGE.md § 16):
    /// another kind only with `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`, and only one with a driver.
    pub fn allowed(&self) -> Result<()> {
        experimental::allowed_here(&self.settings()).map_err(CliError::error)
    }

    /// The kind of server the commands talk to: the settings' (`--server` names another
    /// CraftCloud server, never another kind).
    pub fn storage(&self) -> Storage {
        if self.server_override.is_some() && !self.uses_clients_server() { Storage::Oxicloud } else { self.settings().storage }
    }

    /// Whether the commands go through the generic WebDAV driver (another kind of server: any
    /// other WebDAV server, or Nextcloud or OpenCloud with its profile).
    pub fn is_webdav(&self) -> bool {
        matches!(self.storage(), Storage::Webdav | Storage::Nextcloud | Storage::Opencloud)
    }

    /// What the settings' kind of server adds to the driver (docs/WEBDAV-STORAGE.md § 7.4): the
    /// Nextcloud or OpenCloud profile, or the generic one.
    pub fn profile(&self) -> Result<Box<dyn ServerProfile>> {
        let settings = self.settings();
        let base = Base::parse(&settings.dav_url).map_err(|e| CliError::error(format!("the saved WebDAV address: {e}")))?;
        profile::for_kind(settings.storage, &base)
            .ok_or_else(|| CliError::error(format!("the saved WebDAV address {} isn't one a {} server has", base.url(), settings.storage.words())))
    }

    /// The device name in trash entries and conflict copies.
    pub fn device(&self) -> String {
        Some(self.settings().device_name.trim().to_owned()).filter(|d| !d.is_empty()).unwrap_or_else(craftcloud_native::device_name)
    }

    /// The signed-in session on another WebDAV server (an app password), from the settings'
    /// `dav_url` and the keychain.
    pub fn app_password_session(&self) -> Result<AppPasswordSession> {
        let settings = self.settings();
        let base = Base::parse(&settings.dav_url).map_err(|e| CliError::error(format!("the saved WebDAV address: {e}")))?;
        let server = self.server()?;
        let credential = self.tokens(&server).load_credential().ok_or_else(|| {
            let how = match settings.storage {
                Storage::Nextcloud => format!("craftcloud login {server} --kind nextcloud"),
                Storage::Opencloud => format!("craftcloud login {server} --kind opencloud --user NAME"),
                _ => format!("craftcloud login {} --kind webdav --user NAME", base.url()),
            };
            CliError::new(Fail::SignedOut, format!("Not signed in to {server}. Run `{how}`."))
        })?;
        let Credential::AppPassword { user, secret } = credential else {
            return Err(CliError::new(Fail::SignedOut, format!("The sign-in saved for {server} isn't an app password. Sign in again.")));
        };
        AppPasswordSession::new(base, &user, &secret).map_err(CliError::error)
    }

    /// The generic WebDAV driver on the signed-in server, with the saved check on connect (or a
    /// fresh one when none is saved for this address).
    pub fn webdav(&self) -> Result<(WebDav, StoredCheck)> {
        let session = self.app_password_session()?;
        Ok(WebDav::connect_with(session, self.profile()?, &self.device(), self.dir().as_deref(), Limits::default()))
    }

    /// [`Self::webdav`] as a closure for the sync thread (the check on connect may take a while).
    pub fn webdav_connect(&self) -> Result<craftcloud_native::sync::Connect> {
        let session = self.app_password_session()?;
        let profile = self.profile()?;
        let (device, dir) = (self.device(), self.dir());
        Ok(Box::new(move || Ok(WebDav::connect_with(session, profile, &device, dir.as_deref(), Limits::default()))))
    }

    /// [`Self::webdav`] for a command that writes to the server: only where the check on connect
    /// allows two-way sync. A server that ignores a write guard gets no write from the command
    /// line either (docs/WEBDAV-STORAGE.md § 7.3): the sync doesn't write to it, and `put` or `rm`
    /// mustn't be the one way a guard can be bypassed.
    pub fn webdav_for_writes(&self) -> Result<(WebDav, StoredCheck)> {
        let (driver, check) = self.webdav()?;
        match craftcloud_core::storage::mode_for(driver.capabilities()) {
            craftcloud_core::storage::Mode::TwoWay => Ok((driver, check)),
            mode => Err(CliError::new(
                Fail::Unsupported,
                format!(
                    "writing to this server: {} (the sync mode is {}; `craftcloud server check` says why)",
                    super::output::NOT_ON_THIS_SERVER,
                    mode.words()
                ),
            )),
        }
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
