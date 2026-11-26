//! Two-way sync against a real server, with two local folders playing two computers.
//! Skipped unless these are set:
//!
//! ```text
//! CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8093 CRAFTCLOUD_TEST_USER=john \
//! CRAFTCLOUD_TEST_PASSWORD=… CRAFTCLOUD_HTTP_IDLE_SECS=5 \
//! cargo test -p craftcloud-native --test live -- --test-threads 1
//! ```
//!
//! That is CraftCloud's own server (OxiCloud), the suite CI's `live` job runs; on a fresh server it
//! creates the owner with those details first. With `CRAFTCLOUD_TEST_KIND=nextcloud|owncloud|opencloud|webdav`
//! and `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` the same tests run against another kind of server through
//! the generic WebDAV driver ([`kind`] says the variables; `tests/other-servers/README.md` how to
//! start one). Every pass asserts the sync mode the server is expected to get (docs/WEBDAV-STORAGE.md
//! § 5, § 16): a server that got better or worse is a failure, not a skip. The two-way tests run
//! where two-way sync is expected; the download-only suite (§ 9, the `download_only_*` tests)
//! where download only is. Use a throwaway container.
//!
//! Each test keeps to a corner of the account of its own ([`Live`]): its top-level folders are
//! named `live-<test>-<seconds>-<pid>-<name>`, its computers switch off every folder the account
//! had before it started (as the Folders tab does), and when it ends, failed or not, its folders
//! and their kept versions go to the trash and from there for good. So what is already on the
//! account never changes what a test sees, and a run leaves nothing behind. (The quota tests make
//! a user of their own each run; the server has no call to delete one, so those users stay. They
//! need OxiCloud's admin API, so they run on CraftCloud's own server only, as do the test of its
//! REST ids and stars and the one of the UI's transport.)

// A module beside this file, not a test program of its own (cargo would build `tests/kind.rs` as one).
#[path = "live/kind.rs"]
mod kind;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth::{self, Tokens};
use craftcloud_core::api::resources::{Kind as ResourceKind, Resource};
use craftcloud_core::api::versions::Version;
use craftcloud_core::api::{admin, dav, favorites, folders, trash};
use craftcloud_core::storage::Mode;
use craftcloud_core::storage::probe::{Base, StoredCheck};
use craftcloud_core::sync::{ActivityKind, Keep, LeftOut, SyncPlan, SyncProgress, SyncState};
use craftcloud_native::http::{ThreadBackend, execute};
use craftcloud_native::storage::probe::{Limits, save_check};
use craftcloud_native::storage::webdav::WebDav;
use craftcloud_native::storage::{Entry, RemoteStorage, TrashGuard, TrashItem, Trashed};
use craftcloud_native::sync::{Command, Handle};
use kind::{Dav, Kind, Server};

/// CraftCloud's own server named in the environment: (its address, user, password).
fn oxicloud_env() -> Option<(ServerUrl, String, String)> {
    let server = Server::from_env().filter(|s| s.kind == Kind::Oxicloud)?;
    let url = ServerUrl::parse(&server.url).ok()?;
    Some((url, server.user.clone(), server.account_password().to_owned()))
}

/// Signs in once per test run (the server limits sign-ins per minute).
fn sign_in(server: &ServerUrl, user: &str, password: &str) -> Tokens {
    static TOKENS: std::sync::Mutex<Option<Tokens>> = std::sync::Mutex::new(None);
    let mut kept = TOKENS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(tokens) = kept.as_ref() {
        return tokens.clone();
    }
    let status = auth::parse_status(&execute(&auth::status(server)).unwrap()).unwrap();
    if !status.initialized {
        auth::parse_setup(&execute(&auth::setup(server, user, &format!("{user}@example.org"), password)).unwrap()).unwrap();
    }
    let tokens = login(server, user, password);
    *kept = Some(tokens.clone());
    tokens
}

/// Signs `user` in, waiting out the server's limit on sign-ins per minute (a run right after
/// another one, or a new user's first sign-in, can meet it). OxiCloud answers 429 with
/// `Retry-After: 60`, and a try within that minute starts it again, so the next try waits it out.
fn login(server: &ServerUrl, user: &str, password: &str) -> Tokens {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        match auth::parse_login(&execute(&auth::login(server, user, password)).unwrap()) {
            Ok(tokens) => return tokens,
            Err(e) if e.status() == Some(429) && Instant::now() < deadline => std::thread::sleep(Duration::from_secs(65)),
            Err(e) => panic!("signing in as {user}: {e}"),
        }
    }
}

/// Seconds since 1970 and this process: what makes a test's names its own.
fn run_id() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{secs}-{}", std::process::id())
}

/// Bounds every paged listing: a server that keeps handing out cursors doesn't hang a test.
const PAGES: usize = 100;

/// Everything in the cloud folder `id` (CraftCloud's own items too), page by page.
fn children(server: &ServerUrl, tokens: &Tokens, id: &str) -> Result<Vec<Resource>, String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..PAGES {
        let resp = execute(&tokens.authorize(folders::contents(server, id, Some(200), cursor.as_deref())))?;
        let page = folders::parse_contents_all(&resp).map_err(|e| e.to_string())?;
        out.extend(page.items);
        cursor = page.next_cursor.filter(|c| !c.is_empty());
        if cursor.is_none() {
            return Ok(out);
        }
    }
    Err("the folder listing didn't end".to_owned())
}

/// The REST id of the account's top folder.
fn root_id(server: &ServerUrl, tokens: &Tokens) -> Result<String, String> {
    let roots = folders::parse_roots(&execute(&tokens.authorize(folders::roots(server)))?).map_err(|e| e.to_string())?;
    roots.into_iter().find(|f| f.parent_id.is_none()).map(|f| f.id).ok_or_else(|| "no top folder".to_owned())
}

/// Everything in the trash (CraftCloud's own items too), page by page, soonest to expire first.
fn trash_all(server: &ServerUrl, tokens: &Tokens) -> Result<Vec<trash::TrashItem>, String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..PAGES {
        let resp = execute(&tokens.authorize(trash::list(server, Some(200), cursor.as_deref())))?;
        let page = trash::parse_list_all(&resp).map_err(|e| e.to_string())?;
        out.extend(page.items);
        cursor = page.next_cursor.filter(|c| !c.is_empty());
        if cursor.is_none() {
            return Ok(out);
        }
    }
    Err("the trash listing didn't end".to_owned())
}

/// The cloud side of a test, by kind: CraftCloud's own server over REST, as these tests always
/// talked to it, or any other server through the generic WebDAV driver (the trait, so the helpers
/// keep working when a server's profile lands) and the test's own raw WebDAV client for what
/// another client or the person does to the cloud.
enum Cloud {
    Oxi { server: ServerUrl, tokens: Tokens },
    Dav { driver: Box<Mutex<WebDav>>, raw: Box<Mutex<Dav>> },
}

/// The check on connect for this run's server, run once per process: the first connect probes
/// and keeps the result; every later one finds it saved in its settings folder and reuses it,
/// the way the product reuses a saved check (`WebDav::connect`). `Err` when the server doesn't
/// get the mode expected of it, so a pass fails with that and nothing runs on a wrong mode.
fn connect(server: &Server, device: &str, dir: &Path) -> Result<(WebDav, StoredCheck), String> {
    static CHECK: Mutex<Option<StoredCheck>> = Mutex::new(None);
    let kept = CHECK.lock().unwrap_or_else(PoisonError::into_inner).clone();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    if let Some(check) = kept {
        save_check(dir, &check)?;
    }
    let (driver, check) = WebDav::connect_with(server.session(), server.profile(), device, Some(dir), Limits::default());
    if check.mode != server.mode {
        let why = check.refused_because.clone().unwrap_or_default();
        return Err(format!("{server}: the check on connect gave {} ({why}); steps: {:#?}", check.mode.words(), check.steps));
    }
    *CHECK.lock().unwrap_or_else(PoisonError::into_inner) = Some(check.clone());
    Ok((driver, check))
}

/// The kind and the mode this run is about, for [`sync`] (set by the first [`Live`]).
static RUN: OnceLock<(Kind, Mode)> = OnceLock::new();

/// One test's corner of the account: top-level folders of its own, computers that sync only
/// those, and a cleanup when the test ends, passed or failed.
///
/// Make it before the test's computers: locals drop in reverse, so the computers stop first.
struct Live {
    server: Server,
    cloud: Cloud,
    /// `live-<test>-<seconds>-<pid>`: every top-level folder of the test is named `<base>-…`.
    base: String,
    /// The top-level folders the account had before the test: switched off on its computers.
    others: Vec<String>,
    dirs: tempfile::TempDir,
}

impl Live {
    /// As the test user (made first on a fresh OxiCloud server); `None` when no test server is set.
    fn new(test: &str) -> Option<Self> {
        let server = Server::from_env()?;
        let cloud = match server.kind {
            Kind::Oxicloud => {
                let url = ServerUrl::parse(&server.url).unwrap_or_else(|e| panic!("{}: {e}", server.url));
                let tokens = sign_in(&url, &server.user, server.account_password());
                Cloud::Oxi { server: url, tokens }
            }
            _ => {
                let dir = tempfile::tempdir().unwrap();
                let (driver, _) = connect(&server, "Tester", dir.path()).unwrap_or_else(|e| panic!("{e}"));
                Cloud::Dav { driver: Box::new(Mutex::new(driver)), raw: Box::new(Mutex::new(Dav::new(&server))) }
            }
        };
        Some(Self::with(server, cloud, test))
    }

    /// Only where two-way sync is expected (the download-only suite covers the rest).
    fn two_way(test: &str) -> Option<Self> {
        let live = Self::new(test)?;
        if live.server.mode != Mode::TwoWay {
            eprintln!("skipped: {} syncs download only; the download_only_* tests cover it", live.server);
            return None;
        }
        Some(live)
    }

    /// Only where download only is expected.
    fn download_only(test: &str) -> Option<Self> {
        let live = Self::new(test)?;
        if live.server.mode != Mode::DownloadOnly {
            eprintln!("skipped: {} syncs two-way", live.server);
            return None;
        }
        Some(live)
    }

    /// As the OxiCloud account `tokens` belongs to.
    fn oxicloud_with(server: ServerUrl, tokens: Tokens, test: &str) -> Self {
        let env = Server::from_env().expect("an OxiCloud test server");
        Self::with(env, Cloud::Oxi { server, tokens }, test)
    }

    fn with(server: Server, cloud: Cloud, test: &str) -> Self {
        RUN.get_or_init(|| (server.kind, server.mode));
        let others = match &cloud {
            Cloud::Oxi { server, tokens } => {
                let root = root_id(server, tokens).unwrap();
                children(server, tokens, &root)
                    .unwrap()
                    .into_iter()
                    .filter(|r| r.kind() == ResourceKind::Folder)
                    .map(|r| r.name().to_owned())
                    .collect()
            }
            Cloud::Dav { raw, .. } => {
                let mut raw = raw.lock().unwrap_or_else(PoisonError::into_inner);
                raw.list("").unwrap_or_else(|e| panic!("{e}")).into_iter().filter(|e| e.is_folder).map(|e| e.name().to_owned()).collect()
            }
        };
        Self { base: format!("live-{test}-{}", run_id()), others, dirs: tempfile::tempdir().unwrap(), server, cloud }
    }

    fn kind(&self) -> Kind {
        self.server.kind
    }

    /// The test's own path for `rel`, in the cloud and in a sync folder alike: its first part
    /// becomes a top-level folder of the test (`Shared/notes.md` → `live-…-Shared/notes.md`).
    fn p(&self, rel: &str) -> String {
        format!("{}-{rel}", self.base)
    }

    /// Whether a top-level name is one of the test's folders.
    fn mine(&self, top: &str) -> bool {
        top.starts_with(&format!("{}-", self.base))
    }

    /// A folder in the test's scratch space on this computer.
    fn local(&self, name: &str) -> PathBuf {
        self.dirs.path().join(name)
    }

    /// What a computer with its sync folder at `local` syncs: every folder but the account's
    /// others and `off` (new ones too, whichever side makes them), and no files at the top.
    fn plan(&self, local: &Path, off: &[String]) -> SyncPlan {
        SyncPlan {
            server: self.server.url.clone(),
            local_folder: local.display().to_string(),
            excluded: self.others.iter().chain(off).cloned().collect(),
            folders: vec![],
            sync_root_files: false,
        }
    }

    fn computer(&self, local: &Path, name: &str) -> Handle {
        self.start(self.plan(local, &[]), name)
    }

    fn start(&self, plan: SyncPlan, name: &str) -> Handle {
        self.start_at(plan, name, None)
    }

    /// Starts a computer; `through` is another address for the server (a forwarder in front of
    /// it), with the WebDAV base's path kept.
    fn start_at(&self, mut plan: SyncPlan, name: &str, through: Option<&str>) -> Handle {
        // Setup makes the folder; the sync never does.
        std::fs::create_dir_all(&plan.local_folder).unwrap();
        match &self.cloud {
            Cloud::Oxi { tokens, .. } => {
                if let Some(url) = through {
                    plan.server = url.to_owned();
                }
                craftcloud_native::sync::start(plan, tokens.clone(), name.into(), Box::new(|_| {}), Arc::new(|| {}))
            }
            Cloud::Dav { .. } => {
                let mut server = self.server.clone();
                if let Some(url) = through {
                    server.set_dav_url(&format!("{}{}/", url.trim_end_matches('/'), self.server.base().path()));
                }
                let dir = self.dirs.path().join(format!("settings-{name}"));
                let device = name.to_owned();
                craftcloud_native::sync::start_webdav(plan, Box::new(move || connect(&server, &device, &dir)), name.into(), Arc::new(|| {}))
            }
        }
    }

    /// The test's generic driver (other kinds of server).
    fn driver(&self) -> MutexGuard<'_, WebDav> {
        match &self.cloud {
            Cloud::Dav { driver, .. } => driver.lock().unwrap_or_else(PoisonError::into_inner),
            Cloud::Oxi { .. } => panic!("OxiCloud's helpers go over REST"),
        }
    }

    /// The test's raw WebDAV client: what another client, or the person on the server, does to
    /// the cloud (other kinds of server).
    fn other(&self) -> MutexGuard<'_, Dav> {
        match &self.cloud {
            Cloud::Dav { raw, .. } => raw.lock().unwrap_or_else(PoisonError::into_inner),
            Cloud::Oxi { .. } => panic!("OxiCloud's helpers go over REST"),
        }
    }

    /// One cloud folder, one level (the listing must be whole).
    fn list(&self, folder: &str) -> Vec<Entry> {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => {
                let listed = execute(&tokens.authorize(dav::list(server, folder))).unwrap();
                dav::parse_list(&listed).unwrap().into_iter().map(Entry::from).collect()
            }
            Cloud::Dav { .. } => self.driver().list(folder).unwrap_or_else(|e| panic!("listing {folder}: {e}")),
        }
    }

    /// The newest trashed item that was at `cloud_path`.
    fn in_trash(&self, cloud_path: &str) -> Option<TrashItem> {
        self.trash_items().into_iter().rev().find(|i| i.path == cloud_path)
    }

    /// Everything in the trash, oldest first, as the cloud lists it.
    fn trash_items(&self) -> Vec<TrashItem> {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => trash_all(server, tokens)
                .unwrap()
                .into_iter()
                .filter_map(|i| {
                    let path = i.resource.cloud_path()?;
                    Some(TrashItem { id: i.resource.id().to_owned(), path, trashed_at: i.trashed_at.clone() })
                })
                .collect(),
            Cloud::Dav { .. } => self.driver().trash_list().unwrap_or_else(|e| panic!("listing the trash: {e}")),
        }
    }

    /// Whether `folder` went to the trash as one item: the folder is listed and nothing inside it
    /// is an item of its own, or (the generic driver, which lists a trashed folder holding exactly
    /// one thing as that thing, docs/WEBDAV-STORAGE.md § 7.2, "As built (pull request 4a)"; a
    /// restore puts the same tree back) exactly one item inside it and not the folder.
    fn trashed_whole(&self, folder: &str) -> bool {
        let inside = format!("{folder}/");
        let items = self.trash_items();
        let itself = items.iter().any(|i| i.path == folder);
        let within = items.iter().filter(|i| i.path.starts_with(&inside)).count();
        (itself && within == 0) || (!itself && within == 1)
    }

    /// Puts a trashed item back, as the person would.
    fn restore(&self, item: &TrashItem) {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => {
                let resp = execute(&tokens.authorize(trash::restore(server, &item.id))).unwrap();
                assert!(resp.is_success(), "restoring {}: {}", item.path, resp.status);
            }
            Cloud::Dav { .. } => self.driver().restore(item).unwrap_or_else(|e| panic!("restoring {}: {e}", item.path)),
        }
    }

    /// Deletes a cloud folder as another client would: into the trash, whole.
    fn trash_folder(&self, path: &str) {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => {
                let id = self.folder_id(path).unwrap_or_else(|| panic!("{path} isn't in the cloud"));
                assert!(execute(&tokens.authorize(folders::trash(server, &id))).unwrap().is_success());
            }
            Cloud::Dav { .. } => {
                // The guard takes the driver's version tags (on Nextcloud the ETag plus the listed
                // checksum), so the files are listed through the driver, every level.
                let mut driver = self.driver();
                let mut files: Vec<(String, String)> = Vec::new();
                let mut level = vec![path.to_owned()];
                while let Some(folder) = level.pop() {
                    for e in driver.list(&folder).unwrap_or_else(|e| panic!("listing {folder}: {e}")) {
                        if e.is_folder {
                            level.push(e.path);
                        } else {
                            files.push((e.path, e.etag.unwrap_or_default()));
                        }
                    }
                }
                let done = driver.trash(path, TrashGuard::Folder { files }).unwrap_or_else(|e| panic!("trashing {path}: {e}"));
                assert_eq!(done, Trashed::Done);
            }
        }
    }

    /// The server's id for the folder at `path`, where the server has ids that survive a move
    /// (OxiCloud's REST id; `oc:fileid` elsewhere; none on a plain WebDAV server).
    fn folder_id(&self, path: &str) -> Option<String> {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => {
                let mut id = root_id(server, tokens).unwrap();
                for part in path.split('/').filter(|p| !p.is_empty()) {
                    id =
                        children(server, tokens, &id).unwrap().iter().find(|r| r.kind() == ResourceKind::Folder && r.name() == part)?.id().to_owned();
                }
                Some(id)
            }
            Cloud::Dav { .. } => {
                let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
                self.driver().list(parent).ok()?.into_iter().find(|e| e.path == path && e.is_folder)?.file_id
            }
        }
    }

    /// Whether the check on connect confirmed the server keeps its own versions (Nextcloud with its
    /// profile): then `keep_version` is a no-op and the history follows renames by file id.
    fn server_versions(&self) -> bool {
        match &self.cloud {
            Cloud::Oxi { .. } => false,
            Cloud::Dav { .. } => self.driver().capabilities().versions == craftcloud_core::storage::Versions::Server,
        }
    }

    /// The kept versions of a cloud file, newest first.
    fn versions_of(&self, path: &str) -> Vec<Version> {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => {
                use craftcloud_core::api::versions;
                versions::parse_list(&execute(&tokens.authorize(versions::list(server, path))).unwrap()).unwrap()
            }
            Cloud::Dav { .. } => self.driver().list_versions(path).unwrap_or_else(|e| panic!("versions of {path}: {e}")),
        }
    }

    /// Keeps the current cloud bytes of `path` as a version (what an overwrite does first).
    fn snapshot(&self, path: &str) {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => {
                let mut session = craftcloud_native::session::Session::new(server.clone(), tokens.clone(), Box::new(|_| {}));
                session.snapshot(path).unwrap();
            }
            Cloud::Dav { .. } => self.driver().keep_version(path).unwrap_or_else(|e| panic!("keeping a version of {path}: {e}")),
        }
    }

    /// Whether a cloud path is the test's: in one of its folders, or in their kept versions.
    fn owns(&self, cloud_path: &str) -> bool {
        let rest = cloud_path.strip_prefix(&format!("{}/versions/", dav::CRAFTCLOUD_DIR)).unwrap_or(cloud_path);
        self.mine(rest.split('/').next().unwrap_or_default())
    }

    /// Trashes the test's folders and their kept versions, then deletes those and everything
    /// else the test trashed for good. Never panics: it runs while a failed test unwinds, where a
    /// panic would abort the whole run.
    fn clean_up(&self) -> Result<(), String> {
        match &self.cloud {
            Cloud::Oxi { server, tokens } => self.clean_up_oxicloud(server, tokens),
            Cloud::Dav { raw, .. } => self.clean_up_dav(&mut raw.lock().unwrap_or_else(PoisonError::into_inner)),
        }
    }

    fn clean_up_oxicloud(&self, server: &ServerUrl, tokens: &Tokens) -> Result<(), String> {
        let root = root_id(server, tokens)?;
        let mut trashed: Vec<String> = Vec::new();
        // Looked for again in case a stopped computer's last cycle still made a folder.
        for _ in 0..3 {
            std::thread::sleep(Duration::from_secs(1));
            let top = children(server, tokens, &root)?;
            let mut found: Vec<String> =
                top.iter().filter(|r| r.kind() == ResourceKind::Folder && self.mine(r.name())).map(|r| r.id().to_owned()).collect();
            if let Some(internal) = top.iter().find(|r| r.kind() == ResourceKind::Folder && r.name() == dav::CRAFTCLOUD_DIR)
                && let Some(versions) =
                    children(server, tokens, internal.id())?.into_iter().find(|r| r.kind() == ResourceKind::Folder && r.name() == "versions")
            {
                found.extend(children(server, tokens, versions.id())?.iter().filter(|r| self.mine(r.name())).map(|r| r.id().to_owned()));
            }
            if found.is_empty() {
                break;
            }
            for id in found {
                let resp = execute(&tokens.authorize(folders::trash(server, &id)))?;
                if !resp.is_success() {
                    return Err(format!("trashing a folder answered {}", resp.status));
                }
                trashed.push(id);
            }
        }
        // A folder just trashed can take a moment to show in the trash listing (seen once: it
        // showed only after the purge, and stayed), so the purge goes on until every folder
        // trashed above has been seen and deleted, and nothing of the test is left.
        let mut expected = trashed;
        for _ in 0..10 {
            let ours: Vec<trash::TrashItem> =
                trash_all(server, tokens)?.into_iter().filter(|i| i.resource.cloud_path().is_some_and(|p| self.owns(&p))).collect();
            if ours.is_empty() && expected.is_empty() {
                return Ok(());
            }
            for item in &ours {
                // Best effort: an item inside a folder deleted just before may be gone with it.
                let _ = execute(&tokens.authorize(trash::delete(server, item.resource.id())));
                expected.retain(|id| id != item.resource.id());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Err(format!("its trash wasn't emptied ({} trashed folders never showed in the trash)", expected.len()))
    }

    /// Another kind of server: the test's top-level folders, its kept versions
    /// (`.craftcloud/versions/<folder>`) and the client-trash entries holding only its things are
    /// deleted (on Nextcloud and ownCloud into the server's trash, and from there too).
    fn clean_up_dav(&self, raw: &mut Dav) -> Result<(), String> {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_secs(1));
            let mine: Vec<String> = raw.list("")?.into_iter().filter(|e| e.is_folder && self.mine(e.name())).map(|e| e.path).collect();
            if mine.is_empty() {
                break;
            }
            for folder in mine {
                let status = raw.delete(&folder, true);
                if !matches!(status, 200..=299 | 404) {
                    return Err(format!("deleting {folder} answered {status}"));
                }
            }
        }
        if let Ok(kept) = raw.list(&format!("{}/versions", dav::CRAFTCLOUD_DIR)) {
            for folder in kept.into_iter().filter(|e| e.is_folder && self.mine(e.name())) {
                raw.delete(&folder.path, true);
            }
        }
        if let Ok(entries) = raw.list(dav::TRASH_DIR) {
            for entry in entries.into_iter().filter(|e| e.is_folder) {
                let inside = raw.list(&entry.path)?;
                if !inside.is_empty() && inside.iter().all(|e| self.mine(e.name())) {
                    raw.delete(&entry.path, true);
                }
            }
        }
        if matches!(self.kind(), Kind::Nextcloud | Kind::Owncloud) {
            self.purge_server_trash()?;
        }
        Ok(())
    }

    /// Nextcloud and ownCloud: what the cleanup deleted sits in the server's trash as
    /// `<name>.d<seconds>`; the test's folders and the client-trash entries that held only its
    /// things go from there too, so a run leaves nothing behind.
    fn purge_server_trash(&self) -> Result<(), String> {
        let mut bin = Dav::at(&self.server, &format!("{}/remote.php/dav/trashbin/{}/trash/", self.server.url, self.server.user));
        for entry in bin.list("")? {
            let name = entry.name().rsplit_once(".d").map_or(entry.name(), |(n, _)| n);
            let ours = self.mine(name)
                || (entry.is_folder && name.contains('T') && {
                    let inside = bin.list(&entry.path).unwrap_or_default();
                    !inside.is_empty() && inside.iter().all(|e| self.mine(e.name()))
                });
            if ours {
                bin.delete(&entry.path, entry.is_folder);
            }
        }
        Ok(())
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // The computers have stopped (they drop first); without their local copies nothing a
        // last cycle still runs can bring a folder back.
        let _ = std::fs::remove_dir_all(self.dirs.path());
        if let Err(e) = self.clean_up() {
            eprintln!("cleaning up {}: {e}", self.base);
        }
    }
}

/// Runs one more cycle (one that starts after this call) and waits for it to end, however it
/// ends: the progress then.
fn sync_ends(h: &Handle) -> SyncProgress {
    static TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = TICKET.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    h.send(Command::SyncFor(n));
    let (kind, _) = RUN.get().copied().unwrap_or((Kind::Oxicloud, Mode::TwoWay));
    let deadline = Instant::now() + kind.sync_deadline();
    loop {
        let p = h.progress.lock().unwrap().clone();
        if p.served >= n {
            return p;
        }
        assert!(Instant::now() < deadline, "no sync finished: {p:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs one more cycle and waits for it to finish up to date, in the sync mode this server is
/// expected to get (docs/WEBDAV-STORAGE.md § 5, § 16): any other mode fails the test.
fn sync(h: &Handle) {
    synced(h);
}

/// [`sync`], and the progress it ended with.
fn synced(h: &Handle) -> SyncProgress {
    let p = sync_ends(h);
    assert!(matches!(p.state, SyncState::UpToDate), "{:?}: {:?}", p.state, p.activity);
    let (_, expected) = RUN.get().copied().unwrap_or((Kind::Oxicloud, Mode::TwoWay));
    assert_eq!(p.mode, Some(expected), "the server got another sync mode than expected");
    p
}

/// Syncs these computers round after round until two rounds in a row change nothing on any of
/// them and nothing is left to try again.
///
/// For a change that needs more passes than a test can count: each computer also syncs on its
/// own (its folder watch, its timer), so two of them can be at the server at once, and a server
/// may answer "not now" to one request (Nextcloud: 500 to a `MOVE` of a folder another request
/// holds a lock in, 500 to a `PUT` into a folder made a moment before), which the client tries
/// again next pass. No sleep and no retry of a failed check: every pass must end up to date, and
/// a sync that hasn't settled after ten rounds fails.
fn settle(handles: &[&Handle]) {
    let mut quiet = 0;
    for _ in 0..10 {
        // Every computer syncs in every round, whatever the one before it did.
        let passes: Vec<SyncProgress> = handles.iter().map(|h| synced(h)).collect();
        let done = passes.iter().all(|p| p.files_done == 0 && p.unsynced.is_empty());
        quiet = if done { quiet + 1 } else { 0 };
        if quiet == 2 {
            return;
        }
    }
    let left: Vec<_> = handles.iter().map(|h| h.progress.lock().unwrap().clone()).map(|p| (p.unsynced, p.activity)).collect();
    panic!("the sync didn't settle in ten rounds: {left:#?}");
}

/// Nothing became a conflict on this computer, nothing is left unsynced, and no error stayed:
/// on CraftCloud's own server there was none at all; on another, one is fine only when the same
/// path synced later (the server answered "not now" once, see [`settle`]).
fn no_conflicts_or_lasting_errors(live: &Live, h: &Handle) {
    let p = h.progress.lock().unwrap();
    assert!(!p.activity.iter().any(|e| e.kind == ActivityKind::Conflict), "{:?}", p.activity);
    assert!(p.unsynced.is_empty(), "{:?}", p.unsynced);
    for (i, e) in p.activity.iter().enumerate().filter(|(_, e)| e.kind == ActivityKind::Error) {
        let came_right = p.activity[i + 1..].iter().any(|later| {
            later.path == e.path && matches!(later.kind, ActivityKind::Up | ActivityKind::Down | ActivityKind::Moved | ActivityKind::Deleted)
        });
        assert!(live.kind() != Kind::Oxicloud && came_right, "an error that stayed: {e:?} in {:?}", p.activity);
    }
}

/// Writes a file and waits until it counts as settled (the client waits 2 s after a write).
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
}

/// The folder's counts after a cycle: `files` synced with `bytes`, nothing pending.
fn counted(h: &Handle, folder: &str, files: u64, bytes: u64) {
    let p = h.progress.lock().unwrap();
    let stats = p.folders.iter().find(|f| f.folder == folder).cloned().unwrap_or_else(|| panic!("{folder} isn't counted: {:?}", p.folders));
    assert_eq!((stats.files, stats.bytes, stats.errors, stats.syncing), (files, bytes, 0, 0));
    assert!(p.unsynced.is_empty(), "nothing is left syncing after a cycle: {:?}", p.unsynced);
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// The kind in `CRAFTCLOUD_TEST_KIND` must be one of the five, and any but CraftCloud's own needs
/// the opt-in: a run that names what it can't do fails, never passes without running (no server
/// needed for this one).
#[test]
fn the_kind_is_checked_before_anything_runs() {
    use std::ffi::OsStr;
    assert_eq!(kind::kind_from(None, None), Ok(Kind::Oxicloud));
    assert_eq!(kind::kind_from(Some(""), None), Ok(Kind::Oxicloud));
    assert_eq!(kind::kind_from(Some("oxicloud"), None), Ok(Kind::Oxicloud));
    assert_eq!(kind::kind_from(Some(" NextCloud "), Some(OsStr::new("1"))), Ok(Kind::Nextcloud));
    for other in ["nextcloud", "owncloud", "opencloud", "webdav"] {
        let kind = Kind::parse(other).unwrap();
        assert_eq!(kind::kind_from(Some(other), Some(OsStr::new("1"))), Ok(kind));
        for off in [None, Some("0"), Some("true"), Some("yes"), Some("01")] {
            let refused = kind::kind_from(Some(other), off.map(OsStr::new));
            assert!(refused.as_ref().is_err_and(|e| e.contains(kind::NEEDS_OPT_IN)), "{other} with {off:?}: {refused:?}");
        }
    }
    for unknown in ["dropbox", "oxi cloud", "WebDAV server", "sabre"] {
        assert!(kind::kind_from(Some(unknown), Some(OsStr::new("1"))).is_err_and(|e| e.contains("unknown kind")), "{unknown}");
        assert_eq!(Kind::parse(unknown), None);
    }
    for kind in Kind::ALL {
        assert_eq!(Kind::parse(kind.as_str()), Some(kind));
    }
}

/// The server gets the sync mode expected of its kind (docs/WEBDAV-STORAGE.md § 5, § 16), measured
/// by the check on connect and shown by the sync: a server that got better or worse fails here,
/// whatever else runs.
#[test]
fn the_server_gets_the_mode_expected() {
    let Some(live) = Live::new("mode") else {
        eprintln!("skipped: CRAFTCLOUD_TEST_SERVER not set");
        return;
    };
    let root = live.local("M");
    let m = live.computer(&root, "Mode");
    sync(&m);
    let p = m.progress.lock().unwrap();
    assert_eq!(p.mode, Some(live.server.mode), "{}", live.server);
    if live.kind() != Kind::Oxicloud {
        assert_eq!(live.driver().capabilities().names, craftcloud_core::names::NameRules::oxicloud(), "the strict name rules, on every server");
    }
}

#[test]
fn two_computers_stay_in_sync() {
    let Some(live) = Live::two_way("two") else {
        eprintln!("skipped: CRAFTCLOUD_TEST_SERVER not set");
        return;
    };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    sync(&a);
    sync(&b);
    let (shared, notes) = (live.p("Shared"), live.p("Shared/notes.md"));

    // A new file on A reaches B.
    write(&a_root.join(&notes), "one");
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join(&notes)), "one");
    counted(&b, &shared, 1, 3);

    // An edit on A reaches B, and the cloud keeps the previous version.
    write(&a_root.join(&notes), "two");
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join(&notes)), "two");
    let kept = live.versions_of(&notes);
    assert_eq!(kept.len(), 1, "{kept:?}");

    // Both edit the same file: one keeps its version as a conflict copy, on both sides.
    write(&a_root.join(&notes), "from A");
    write(&b_root.join(&notes), "from B, longer");
    sync(&a);
    sync(&b);
    sync(&a);
    assert_eq!(read(&a_root.join(&notes)), "from A");
    assert_eq!(read(&b_root.join(&notes)), "from A");
    let in_a = names_in(&a_root.join(&shared));
    let in_b = names_in(&b_root.join(&shared));
    assert_eq!(in_a, in_b);
    let copies: Vec<&String> = in_a.iter().filter(|n| n.contains("(conflict ")).collect();
    assert_eq!(copies.len(), 1, "{in_a:?}");
    assert!(copies[0].contains("Beta") && copies[0].ends_with(".md"), "{copies:?}");
    assert_eq!(read(&a_root.join(&shared).join(copies[0])), "from B, longer");

    // Settling it on A: "keep mine" (the copy's text) replaces the original everywhere.
    let conflict = a.progress.lock().unwrap().conflicts.clone();
    assert_eq!(conflict.len(), 1, "{conflict:?}");
    assert_eq!(conflict[0].original, notes);
    a.send(Command::Resolve(conflict[0].clone(), Keep::Mine));
    sync(&a);
    sync(&b);
    assert_eq!(read(&a_root.join(&notes)), "from B, longer");
    assert_eq!(read(&b_root.join(&notes)), "from B, longer");
    assert!(names_in(&b_root.join(&shared)).iter().all(|n| !n.contains("(conflict ")), "{:?}", names_in(&b_root.join(&shared)));
    assert!(a.progress.lock().unwrap().conflicts.is_empty());

    // A delete on A removes it on B, and the cloud keeps it in the trash (never deleted for good).
    std::fs::remove_file(a_root.join(&notes)).unwrap();
    sync(&a);
    sync(&b);
    assert!(!b_root.join(&notes).exists());
    let trashed = live.in_trash(&notes).expect("the deleted file is in the cloud's trash");
    live.restore(&trashed);
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join(&notes)), "from B, longer", "restored from the trash, it syncs back");
    std::fs::remove_file(b_root.join(&notes)).unwrap();
    sync(&b);
    sync(&a);
    assert!(!a_root.join(&notes).exists());

    // An emptied folder goes to the trash too.
    write(&a_root.join(live.p("Shared/Old/gone.md")), "x");
    sync(&a);
    std::fs::remove_file(a_root.join(live.p("Shared/Old/gone.md"))).unwrap();
    std::fs::remove_dir(a_root.join(live.p("Shared/Old"))).unwrap();
    sync(&a);
    sync(&a);
    assert!(live.trashed_whole(&live.p("Shared/Old")), "the emptied folder is in the trash: {:?}", live.trash_items());
    assert!(live.list(&shared).iter().all(|e| e.path != live.p("Shared/Old")), "the empty folder is gone from the cloud");

    // The folder watcher: a change on A is synced without asking.
    let before = a.progress.lock().unwrap().cycles;
    std::fs::write(a_root.join(live.p("Shared/watched.md")), "seen").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while a.progress.lock().unwrap().cycles == before {
        assert!(Instant::now() < deadline, "the watcher didn't start a sync");
        std::thread::sleep(Duration::from_millis(100));
    }
    // (The first cycle may come before the 2 s settle; the next one uploads it.)
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join(live.p("Shared/watched.md"))), "seen");

    // A save in progress (CraftCloud's own temporary names, the Craft apps' .craft_tmp) is never uploaded.
    write(&a_root.join(live.p("Shared/.report.99-1.craft_tmp")), "half a save");
    sync(&a);
    sync(&b);
    assert!(!b_root.join(live.p("Shared/.report.99-1.craft_tmp")).exists());

    // Switching a folder off on B removes B's copy; the cloud (and A) keep it. What B still has
    // there is reported, not silently skipped.
    b.send(Command::Replan(live.plan(&b_root, std::slice::from_ref(&shared))));
    sync(&b);
    assert!(!b_root.join(live.p("Shared/watched.md")).exists());
    sync(&a);
    assert_eq!(read(&a_root.join(live.p("Shared/watched.md"))), "seen");
    std::fs::create_dir_all(b_root.join(&shared)).unwrap();
    write(&b_root.join(live.p("Shared/local only.md")), "here");
    sync(&b);
    let skipped = b.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Skipped && e.path == shared);
    assert!(skipped, "the switched-off folder is reported");
    sync(&a);
    assert!(!a_root.join(live.p("Shared/local only.md")).exists());
}

#[test]
fn folders_made_anywhere_join_the_sync() {
    let Some(live) = Live::two_way("join") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    sync(&a);
    sync(&b);
    let ete = live.p("Été");

    // Folders made on one computer join the sync (only folders switched off stay out), however
    // many files they hold and whatever their names.
    write(&a_root.join(&ete).join("fichier.txt"), "x");
    for i in 0..300 {
        std::fs::write(a_root.join(&ete).join(format!("{i:03}.txt")), i.to_string()).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join(&ete).join("fichier.txt")), "x");
    assert_eq!(names_in(&b_root.join(&ete)).len(), 301);
    let ups = a.progress.lock().unwrap().activity.iter().filter(|e| e.path.starts_with(&format!("{ete}/"))).count();
    // Each upload is in the activity list (which keeps the newest 200).
    assert_eq!(ups, craftcloud_core::sync::ACTIVITY_KEPT);

    // Moving a file into a new folder moves it everywhere (it is never lost on the way).
    std::fs::create_dir_all(b_root.join(live.p("sub/deep"))).unwrap();
    std::fs::rename(b_root.join(&ete).join("000.txt"), b_root.join(live.p("sub/deep/000.txt"))).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&b);
    sync(&a);
    assert_eq!(read(&a_root.join(live.p("sub/deep/000.txt"))), "0");
    assert!(!a_root.join(&ete).join("000.txt").exists());

    // Moving one into a folder switched off on that computer keeps it in the cloud, and says so.
    let private = live.p("Private");
    b.send(Command::Replan(live.plan(&b_root, std::slice::from_ref(&private))));
    std::fs::create_dir_all(b_root.join(&private)).unwrap();
    std::fs::rename(b_root.join(&ete).join("001.txt"), b_root.join(&private).join("001.txt")).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&b);
    sync(&a);
    assert_eq!(read(&a_root.join(&ete).join("001.txt")), "1", "the cloud keeps it");
    let moved_out = format!("{ete}/001.txt");
    let said = b.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Skipped && e.path == moved_out);
    assert!(said, "the move out of the synced folders is reported");
}

#[test]
fn renames_and_moves_are_moves_on_both_sides() {
    let Some(live) = Live::two_way("moves") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    let m = |rel: &str| live.p(&format!("Moves/{rel}"));
    write(&a_root.join(m("c1.txt")), "rename me");
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join(m("c1.txt"))), "rename me");
    let moved = |h: &Handle, path: &str| h.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Moved && e.path == path);

    // A rename here is one MOVE in the cloud (no upload, nothing in the trash) and a rename there.
    std::fs::rename(a_root.join(m("c1.txt")), a_root.join(m("c1-renamed.txt"))).unwrap();
    sync(&a);
    assert!(moved(&a, &m("c1-renamed.txt")), "{:?}", a.progress.lock().unwrap().activity.last());
    assert!(live.in_trash(&m("c1.txt")).is_none(), "nothing went to the trash");
    sync(&b);
    assert!(moved(&b, &m("c1-renamed.txt")), "{:?}", b.progress.lock().unwrap().activity.last());
    assert_eq!(read(&b_root.join(m("c1-renamed.txt"))), "rename me");
    assert!(!b_root.join(m("c1.txt")).exists());

    // A move into new folders, the same way.
    std::fs::create_dir_all(b_root.join(m("sub/deep"))).unwrap();
    std::fs::rename(b_root.join(m("c1-renamed.txt")), b_root.join(m("sub/deep/c1-renamed.txt"))).unwrap();
    sync(&b);
    assert!(moved(&b, &m("sub/deep/c1-renamed.txt")), "{:#?}", b.progress.lock().unwrap().activity.iter().rev().take(4).collect::<Vec<_>>());
    sync(&a);
    assert!(moved(&a, &m("sub/deep/c1-renamed.txt")));
    assert_eq!(read(&a_root.join(m("sub/deep/c1-renamed.txt"))), "rename me");
    assert!(!a_root.join(m("c1-renamed.txt")).exists());
}

#[test]
fn a_renamed_folder_is_one_move_and_keeps_its_id() {
    let Some(live) = Live::two_way("proj") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    let (proj, proj2, proj3) = (live.p("Proj"), live.p("Proj2"), live.p("Proj3"));
    write(&a_root.join(&proj).join("a.txt"), "one");
    write(&a_root.join(&proj).join("Sub/b.txt"), "two");
    sync(&a);
    sync(&b);
    // The folder's id, where the server has ids that survive a move (none on a plain WebDAV server).
    let has_ids = live.kind() == Kind::Oxicloud || live.server.has_file_ids();
    let id = live.folder_id(&proj);
    assert_eq!(id.is_some(), has_ids, "{}: {id:?}", live.server);
    // A star, on CraftCloud's own server (its REST API).
    let oxi = match &live.cloud {
        Cloud::Oxi { server, tokens } => Some((server.clone(), tokens.clone())),
        Cloud::Dav { .. } => None,
    };
    if let Some((server, tokens)) = &oxi {
        execute(&tokens.authorize(favorites::add(server, ResourceKind::Folder, id.as_deref().unwrap()))).unwrap();
    }
    let moved = |h: &Handle, path: &str| h.progress.lock().unwrap().activity.iter().any(|e| e.kind == ActivityKind::Moved && e.path == path);

    // Renamed on A: one MOVE in the cloud. The folder keeps its id and its star; nothing is trashed.
    std::fs::rename(a_root.join(&proj), a_root.join(&proj2)).unwrap();
    sync(&a);
    assert!(moved(&a, &proj2), "{:#?}", a.progress.lock().unwrap().activity.iter().rev().take(4).collect::<Vec<_>>());
    if has_ids {
        assert_eq!(live.folder_id(&proj2), id, "same folder, new name");
    }
    assert!(live.list("").iter().all(|e| e.path != proj), "the old name is gone");
    if let Some((server, tokens)) = &oxi {
        let stars = favorites::parse_list_all(&execute(&tokens.authorize(favorites::list(server, Some(200), None))).unwrap()).unwrap();
        assert!(stars.items.iter().any(|f| Some(f.resource.id()) == id.as_deref()), "still starred");
    }
    assert!(live.in_trash(&format!("{proj}/a.txt")).is_none() && live.in_trash(&proj).is_none(), "nothing went to the trash");

    // B renames its local folder to match.
    sync(&b);
    assert!(moved(&b, &proj2));
    assert_eq!(read(&b_root.join(&proj2).join("Sub/b.txt")), "two");
    assert!(!b_root.join(&proj).exists());

    // B renames it while offline, A edits a file in it meanwhile: one folder in the end, holding the edit.
    std::fs::rename(b_root.join(&proj2), b_root.join(&proj3)).unwrap();
    write(&a_root.join(&proj2).join("a.txt"), "edited on A");
    // Both are at the server at once here (B's own pass starts with its rename), so which gets
    // there first, and how many passes that takes, isn't the test's to say: see `settle`.
    settle(&[&a, &b]);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join(&proj3).join("a.txt")), "edited on A", "{}", root.display());
        assert_eq!(read(&root.join(&proj3).join("Sub/b.txt")), "two");
        assert!(!root.join(&proj2).exists(), "{:?}", names_in(root));
        assert_eq!(names_in(&root.join(&proj3)), ["Sub", "a.txt"], "no conflict copies");
    }
    // One folder in the cloud too: never the new one built beside the old one.
    assert!(live.list("").iter().all(|e| e.path != proj2), "the old name is gone from the cloud");
    if has_ids {
        assert_eq!(live.folder_id(&proj3), id);
    }
}

#[test]
fn a_deleted_folder_goes_to_the_trash_whole() {
    let Some(live) = Live::two_way("gone") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    let gone = live.p("Gone");
    // (Few enough files not to pause for a mass delete.)
    for i in 0..2 {
        std::fs::create_dir_all(a_root.join(&gone).join("Sub")).unwrap();
        std::fs::write(a_root.join(&gone).join(format!("{i}.txt")), i.to_string()).unwrap();
        std::fs::write(a_root.join(&gone).join(format!("Sub/{i}.txt")), i.to_string()).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    sync(&b);
    assert_eq!(read(&b_root.join(&gone).join("Sub/1.txt")), "1");

    // Deleted on A: one item in the trash (the folder), not one per file and folder.
    std::fs::remove_dir_all(a_root.join(&gone)).unwrap();
    sync(&a);
    let trashed = live.in_trash(&gone).expect("the folder is in the trash");
    assert!(live.in_trash(&format!("{gone}/0.txt")).is_none() && live.in_trash(&format!("{gone}/Sub")).is_none(), "only the folder");
    sync(&b);
    assert!(!b_root.join(&gone).exists());

    // Restored, it comes back whole.
    live.restore(&trashed);
    sync(&a);
    sync(&b);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join(&gone).join("Sub/1.txt")), "1");
        assert_eq!(read(&root.join(&gone).join("0.txt")), "0");
    }
}

#[test]
fn a_file_and_a_folder_of_one_name_both_survive() {
    let Some(live) = Live::two_way("clash") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    let t5 = live.p("t5");
    write(&a_root.join(&t5).join("keep.txt"), "k");
    sync(&a);
    sync(&b);

    // A makes a file `t5/clash`, B (not synced meanwhile) a folder of that name.
    write(&a_root.join(&t5).join("clash"), "a file");
    write(&b_root.join(&t5).join("clash/inner.txt"), "in a folder");
    sync(&a);
    sync(&b);
    sync(&b);
    sync(&a);
    let copy = names_in(&b_root.join(&t5)).into_iter().find(|n| n.starts_with("clash (conflict ")).expect("B's folder is set aside");
    assert!(copy.contains("Beta"), "{copy}");
    for root in [&a_root, &b_root] {
        assert_eq!(read(&root.join(&t5).join("clash")), "a file", "the cloud's file is kept");
        assert_eq!(read(&root.join(&t5).join(&copy).join("inner.txt")), "in a folder");
    }
    let clash = format!("{t5}/clash");
    let p = b.progress.lock().unwrap();
    assert_eq!(p.activity.iter().filter(|e| e.kind == ActivityKind::Conflict && e.path == clash).count(), 1, "said once");
    assert!(!p.activity.iter().any(|e| e.kind == ActivityKind::Error), "{:?}", p.activity);
    assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
}

/// T-052: a synced folder replaced by a file of its name on one computer (and a synced file by a
/// folder) is a replacement, not a conflict: the cloud and the other computer follow.
#[test]
fn a_folder_replaced_by_a_file_replaces_it_everywhere() {
    let Some(live) = Live::two_way("t17") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Laptop");
    let b = live.computer(&b_root, "Studio");
    let t17 = live.p("t17");
    let at = |root: &Path, rel: &str| root.join(&t17).join(rel);
    write(&at(&a_root, "sw/in.txt"), "in");
    write(&at(&a_root, "sf"), "a file");
    sync(&a);
    sync(&b);
    assert_eq!(read(&at(&b_root, "sw/in.txt")), "in");

    // On A: `rm -r t17/sw; echo x > t17/sw`, and the file `t17/sf` becomes a folder.
    std::fs::remove_dir_all(at(&a_root, "sw")).unwrap();
    std::fs::remove_file(at(&a_root, "sf")).unwrap();
    std::fs::create_dir_all(at(&a_root, "sf")).unwrap();
    std::fs::write(at(&a_root, "sf/a.txt"), "inside").unwrap();
    write(&at(&a_root, "sw"), "x\n");
    sync(&a);
    assert!(live.trashed_whole(&format!("{t17}/sw")), "the folder is one trash item: {:?}", live.trash_items());
    settle(&[&a, &b]);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&at(root, "sw")), "x\n", "{}", root.display());
        assert_eq!(read(&at(root, "sf/a.txt")), "inside", "{}", root.display());
        assert_eq!(names_in(&root.join(&t17)), ["sf", "sw"], "no conflict copies");
    }
    for h in [&a, &b] {
        no_conflicts_or_lasting_errors(&live, h);
    }

    // And back, the other way round on B: the file becomes a folder again, the folder a file.
    std::fs::remove_file(at(&b_root, "sw")).unwrap();
    std::fs::create_dir_all(at(&b_root, "sw")).unwrap();
    std::fs::write(at(&b_root, "sw/again.txt"), "again").unwrap();
    std::fs::remove_dir_all(at(&b_root, "sf")).unwrap();
    write(&at(&b_root, "sf"), "a file again");
    settle(&[&b, &a]);
    for root in [&a_root, &b_root] {
        assert_eq!(read(&at(root, "sw/again.txt")), "again", "{}", root.display());
        assert_eq!(read(&at(root, "sf")), "a file again", "{}", root.display());
        assert_eq!(names_in(&root.join(&t17)), ["sf", "sw"], "no conflict copies");
    }
    for h in [&a, &b] {
        no_conflicts_or_lasting_errors(&live, h);
    }
}

#[test]
fn a_vanished_sync_folder_is_never_made_again_on_its_own() {
    let Some(live) = Live::two_way("vanish") else { return };
    let root = live.local("A");
    let a = live.computer(&root, "Alpha");
    let file = live.p("Vanish/a.txt");
    write(&root.join(&file), "still here");
    sync(&a);

    // Moved away (or its drive unplugged): held, not made again, nothing deleted in the cloud.
    std::fs::rename(&root, live.local("Elsewhere")).unwrap();
    a.send(Command::SyncNow);
    let deadline = Instant::now() + Duration::from_secs(30);
    while a.progress.lock().unwrap().state != SyncState::FolderMissing {
        assert!(Instant::now() < deadline, "{:?}", a.progress.lock().unwrap().state);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!root.exists(), "the folder is not made again");
    assert!(live.list(&live.p("Vanish")).iter().any(|e| e.path == file), "nothing deleted in the cloud");

    // Put back, it syncs again.
    std::fs::rename(live.local("Elsewhere"), &root).unwrap();
    sync(&a);
    assert_eq!(read(&root.join(&file)), "still here");
}

#[test]
fn versions_follow_renames_and_are_kept_once() {
    let Some(live) = Live::two_way("hist") else { return };
    let root = live.local("A");
    let a = live.computer(&root, "Alpha");
    let (doc, renamed) = (live.p("Hist/doc.txt"), live.p("Hist/renamed.txt"));
    let renamed2 = live.p("Hist2/renamed.txt");
    write(&root.join(&doc), "one");
    sync(&a);
    write(&root.join(&doc), "two");
    sync(&a);
    let kept = live.versions_of(&doc);
    assert_eq!(kept.len(), 1);
    // T-019: the version says when its content was written, before it was replaced (OxiCloud's
    // listing carries the content's time in the version's tag; a plain listing has only the stamp).
    if live.kind() == Kind::Oxicloud {
        let (modified, replaced) = (kept[0].modified_at().expect("the content's time is kept"), kept[0].replaced_at().unwrap());
        assert!(modified < replaced, "{modified} < {replaced}");
    }

    // A rename takes the history along.
    std::fs::rename(root.join(&doc), root.join(&renamed)).unwrap();
    sync(&a);
    assert_eq!(live.versions_of(&renamed).len(), 1, "the history moved");
    assert!(live.versions_of(&doc).is_empty(), "nothing left behind");

    // So does a folder rename.
    std::fs::rename(root.join(live.p("Hist")), root.join(live.p("Hist2"))).unwrap();
    sync(&a);
    assert_eq!(live.versions_of(&renamed2).len(), 1);
    assert!(live.versions_of(&renamed).is_empty());

    // Keeping the same version twice (an overwrite tried again) adds nothing. (A server whose
    // version tags carry no content hash can't tell the same bytes from new ones: one more
    // version there, pruned with the rest; docs/WEBDAV-STORAGE.md § 7.5, "already_kept".)
    live.snapshot(&renamed2);
    live.snapshot(&renamed2);
    let twice = live.versions_of(&renamed2).len();
    if live.kind() == Kind::Oxicloud {
        assert_eq!(twice, 2, "the current bytes once, not twice");
    } else if live.server_versions() {
        assert_eq!(twice, 1, "the server keeps the versions itself: keeping one is a no-op");
    } else {
        assert!((2..=3).contains(&twice), "{twice}");
    }
}

/// A new user with `quota` bytes, named `<prefix><seconds><pid>` so a test can run again on the
/// same server: its id and its tokens. Quota is the whole account's, so these tests need one of
/// their own rather than a folder of their own.
fn new_user(server: &ServerUrl, owner: &Tokens, prefix: &str, password: &str, quota: u64) -> (String, Tokens) {
    let name = format!("{prefix}{}", run_id().replace('-', ""));
    let created =
        execute(&owner.authorize(admin::create(server, &name, &format!("{name}@example.org"), password, Some(quota), admin::Role::User))).unwrap();
    let person = admin::parse_create(&created).unwrap();
    let tokens = login(server, &name, password);
    (person.id, tokens)
}

#[test]
fn a_file_too_big_for_the_quota_waits_without_holding_up_the_rest() {
    let Some((server, user, password)) = oxicloud_env() else { return };
    let owner = sign_in(&server, &user, &password);
    let (tiny, tokens) = new_user(&server, &owner, "tiny", "correct-horse-tiny", 100_000);
    let live = Live::oxicloud_with(server.clone(), tokens, "quota");
    let root = live.local("T");
    let t = live.computer(&root, "Tiny");
    sync(&t);
    let (big, small) = (live.p("Big/big.bin"), live.p("Big/small.txt"));

    std::fs::create_dir_all(root.join(live.p("Big"))).unwrap();
    std::fs::write(root.join(&big), vec![b'x'; 200_000]).unwrap();
    write(&root.join(&small), "fits");
    sync(&t);
    {
        let p = t.progress.lock().unwrap();
        assert_eq!(p.no_room, std::slice::from_ref(&big), "{:?}", p.activity.last());
        assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Skipped && e.path == big));
        assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Up && e.path == small), "the rest syncs");
    }
    // Not tried again while there is no room: the next cycles have nothing to do.
    sync(&t);
    assert_eq!(t.progress.lock().unwrap().files_total, 0);

    // Room again: it goes up without being asked.
    execute(&owner.authorize(admin::set_quota(&server, &tiny, 1_000_000))).unwrap();
    sync(&t);
    let p = t.progress.lock().unwrap();
    assert!(p.no_room.is_empty(), "{:?}", p.activity.last());
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Up && e.path == big));
}

#[test]
fn room_freed_by_emptying_the_trash_is_used_once_counted() {
    let Some((server, user, password)) = oxicloud_env() else { return };
    let owner = sign_in(&server, &user, &password);
    let (_, tokens) = new_user(&server, &owner, "roomy", "correct-horse-roomy", 100_000);
    let live = Live::oxicloud_with(server.clone(), tokens.clone(), "room");
    let root = live.local("R");
    let r = live.computer(&root, "Roomy");
    let (first, second) = (live.p("Box/first.bin"), live.p("Box/second.bin"));
    std::fs::create_dir_all(root.join(live.p("Box"))).unwrap();
    std::fs::write(root.join(&first), vec![b'1'; 70_000]).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&r);

    // Deleted, the trash emptied: the room is free, but the cloud still counts it.
    std::fs::remove_file(root.join(&first)).unwrap();
    sync(&r);
    execute(&tokens.authorize(trash::empty(&server))).unwrap();
    std::fs::write(root.join(&second), vec![b'2'; 70_000]).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&r);
    {
        let p = r.progress.lock().unwrap();
        let said = p.activity.iter().find(|e| e.path == second).map(|e| e.detail.clone()).unwrap_or_default();
        if p.no_room.is_empty() {
            eprintln!("this server counted the freed room at once");
            return;
        }
        assert!(said.contains("within a few minutes after emptying the trash"), "{said}");
    }
    // Counted again (an admin's client asks for it; here the owner), it goes up.
    let recount = execute(&owner.authorize(admin::trigger_job(&server, admin::USAGE_RECONCILE))).unwrap();
    assert!(recount.is_success(), "{}", recount.status);
    sync(&r);
    let p = r.progress.lock().unwrap();
    assert!(p.no_room.is_empty(), "{:?}", p.activity.last());
    assert!(p.activity.iter().any(|e| e.kind == ActivityKind::Up && e.path == second));
}

#[test]
fn an_interrupted_download_resumes_where_it_stopped() {
    let Some(live) = Live::two_way("resume") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let (big, movie) = (live.p("Big"), live.p("Big/movie.bin"));
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::create_dir_all(a_root.join(&big)).unwrap();
    std::fs::write(a_root.join(&movie), &bytes).unwrap();
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);

    // B got the first half before the connection dropped: its part file, named after the version.
    let etag = live.list(&big).iter().find(|e| e.path == movie).and_then(|e| e.etag.clone()).unwrap();
    let version: String = etag.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    std::fs::create_dir_all(b_root.join(&big)).unwrap();
    std::fs::write(b_root.join(&big).join(format!(".movie.bin.{version}.craftcloud-part")), &bytes[..120_000]).unwrap();
    let b = live.computer(&b_root, "Beta");
    sync(&b);
    assert_eq!(std::fs::read(b_root.join(&movie)).unwrap(), bytes, "whole and right");
    let p = b.progress.lock().unwrap();
    let down = p.activity.iter().find(|e| e.path == movie).unwrap_or_else(|| panic!("{:#?}", p.activity));
    assert!(down.detail.starts_with("resumed at"), "{down:?}");
    assert!(names_in(&b_root.join(&big)).iter().all(|n| !n.ends_with(".craftcloud-part")), "no part file left");
}
/// A TCP forwarder to the test server that can go silent: connections stay open, but no byte
/// passes (a dropped route after a Wi-Fi change or a suspend), until it is let go again.
struct Forwarder {
    url: String,
    silent: Arc<std::sync::atomic::AtomicBool>,
}

impl Forwarder {
    /// In front of `server` (`http://host:port`, any path ignored).
    fn to(server: &str) -> Self {
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;
        let target = host_port(server);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let silent = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let quiet = Arc::clone(&silent);
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(upstream) = std::net::TcpStream::connect(&target) else { continue };
                for (mut from, mut to) in [(client.try_clone().unwrap(), upstream.try_clone().unwrap()), (upstream, client)] {
                    let quiet = Arc::clone(&quiet);
                    std::thread::spawn(move || {
                        let mut buf = [0u8; 16 * 1024];
                        loop {
                            if quiet.load(Ordering::SeqCst) {
                                std::thread::sleep(Duration::from_millis(50));
                                continue;
                            }
                            let _ = from.set_read_timeout(Some(Duration::from_millis(100)));
                            match from.read(&mut buf) {
                                Ok(0) => return,
                                Ok(n) => {
                                    if to.write_all(&buf[..n]).is_err() {
                                        return;
                                    }
                                }
                                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                                Err(_) => return,
                            }
                        }
                    });
                }
            }
        });
        Self { url, silent }
    }

    fn silent(&self, on: bool) {
        self.silent.store(on, std::sync::atomic::Ordering::SeqCst);
    }
}

/// `host:port` of an `http://` address (port 80 when it names none).
fn host_port(url: &str) -> String {
    let rest = url.trim_start_matches("http://");
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.contains(':') { authority.to_owned() } else { format!("{authority}:80") }
}

/// Waits (up to `limit`) until the sync's state is what `want` says.
fn until(h: &Handle, limit: Duration, want: impl Fn(&SyncState) -> bool) {
    let deadline = Instant::now() + limit;
    loop {
        let state = h.progress.lock().unwrap().state.clone();
        if want(&state) {
            return;
        }
        assert!(Instant::now() < deadline, "still {state:?} after {limit:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Needs a short idle limit: `CRAFTCLOUD_HTTP_IDLE_SECS=5` (skipped otherwise).
#[test]
fn a_connection_gone_silent_goes_offline_and_recovers_by_itself() {
    if Server::from_env().is_none() {
        return;
    }
    let idle = craftcloud_native::http::idle();
    if idle > Duration::from_secs(10) {
        eprintln!("skipped: set CRAFTCLOUD_HTTP_IDLE_SECS=5");
        return;
    }
    let Some(live) = Live::two_way("quiet") else { return };
    let forwarder = Forwarder::to(&live.server.url);
    let root = live.local("S");
    let s = live.start_at(live.plan(&root, &[]), "Silent", Some(&forwarder.url));
    write(&root.join(live.p("Quiet/before.txt")), "before");
    sync(&s);

    // The route drops while the client is idle: the kept-alive connections go silent.
    forwarder.silent(true);
    write(&root.join(live.p("Quiet/during.txt")), "during");
    s.send(Command::SyncNow);
    until(&s, idle * 3 + Duration::from_secs(10), |st| matches!(st, SyncState::Offline(_)));
    let SyncState::Offline(said) = s.progress.lock().unwrap().state.clone() else { unreachable!() };
    assert_eq!(said, craftcloud_native::sync::OFFLINE, "plain words, not the raw error");

    // Pause takes effect while offline, and resume.
    s.send(Command::Pause(true));
    until(&s, idle * 2 + Duration::from_secs(5), |st| *st == SyncState::Paused);
    s.send(Command::Pause(false));

    // The network is back: it syncs again by itself (no restart, no command).
    forwarder.silent(false);
    until(&s, craftcloud_native::sync::INTERVAL + idle * 3 + Duration::from_secs(10), |st| *st == SyncState::UpToDate);
    let during = live.p("Quiet/during.txt");
    assert!(live.list(&live.p("Quiet")).iter().any(|e| e.path == during), "the change made offline went up");
}

#[test]
fn names_the_cloud_cant_take_are_said_once_and_never_sent() {
    let Some(live) = Live::two_way("names") else { return };
    let root = live.local("N");
    let n = live.computer(&root, "Names");
    let (slash, colon) = (live.p("Names/back\\slash.txt"), live.p("Names/a:b.txt"));
    write(&root.join(&slash), "never sent");
    write(&root.join(&colon), "fine here, not on Windows");
    write(&root.join(live.p("Names/ok.txt")), "ok");
    sync(&n);
    sync(&n);
    sync(&n);
    let listed = live.list(&live.p("Names"));
    let names: Vec<&str> = listed.iter().map(|e| e.path.rsplit('/').next().unwrap_or_default()).collect();
    assert!(names.contains(&"ok.txt") && names.contains(&"a:b.txt"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains('\\')), "{names:?}");
    let p = n.progress.lock().unwrap();
    let about = |path: &str, kind: ActivityKind| p.activity.iter().filter(|e| e.path == path && e.kind == kind).count();
    assert_eq!(about(&slash, ActivityKind::Skipped), 1, "said once: {:?}", p.activity);
    assert_eq!(about(&colon, ActivityKind::Warning), 1);
    assert_eq!(p.not_synced, std::slice::from_ref(&slash), "never just \"up to date\"");
    assert_eq!(p.left_out.get(&slash), Some(&LeftOut::Name), "left out for good, not a hold");
    assert!(!p.activity.iter().any(|e| e.kind == ActivityKind::Error), "{:?}", p.activity);
}

#[test]
fn a_deleted_nested_or_renamed_folder_is_one_trash_item() {
    let Some(live) = Live::two_way("nested") else { return };
    let root = live.local("A");
    let a = live.computer(&root, "Alpha");
    for dir in ["t11/top", "t11/top/a", "t11/top/a/x", "t11/top/b", "t9/fold"] {
        std::fs::create_dir_all(root.join(live.p(dir))).unwrap();
        for f in ["f1", "f2", "f3"] {
            std::fs::write(root.join(live.p(dir)).join(f), format!("{dir}/{f}")).unwrap();
        }
    }
    std::thread::sleep(Duration::from_millis(2_200));
    sync(&a);
    std::fs::rename(root.join(live.p("t9/fold")), root.join(live.p("t9/fold-ren"))).unwrap();
    sync(&a);

    // Nested: 12 files in four folders, deleted at once (confirmed, it's a mass delete).
    std::fs::remove_dir_all(root.join(live.p("t11/top"))).unwrap();
    a.send(Command::ConfirmDeletes);
    sync(&a);
    assert!(live.in_trash(&live.p("t11/top")).is_some(), "the folder is in the trash");
    for inner in ["t11/top/f1", "t11/top/a", "t11/top/a/x", "t11/top/a/x/f1", "t11/top/b"] {
        assert!(live.in_trash(&live.p(inner)).is_none(), "{inner} is not a trash item of its own");
    }

    // A folder renamed earlier: the same.
    std::fs::remove_dir_all(root.join(live.p("t9/fold-ren"))).unwrap();
    sync(&a);
    assert!(live.in_trash(&live.p("t9/fold-ren")).is_some());
    assert!(live.in_trash(&live.p("t9/fold-ren/f1")).is_none());
}

/// Q13, the other direction (docs/WEBDAV-STORAGE.md § 16), on a real Nextcloud: another client
/// overwrites a file within the same second as this client's upload of it, so the file keeps the
/// ETag this client saved. The next sync must still bring that write down (the version tag is the
/// ETag plus the listed checksum, which the write cleared), and nothing is lost on the way.
#[test]
fn a_write_under_an_unchanged_etag_still_comes_down() {
    let Some(live) = Live::two_way("sametag") else { return };
    if live.kind() != Kind::Nextcloud {
        eprintln!("skipped: only Nextcloud's profile verifies writes by checksum so far ({})", live.server);
        return;
    }
    let root = live.local("A");
    let a = live.computer(&root, "Alpha");
    let dir = live.p("Tag");
    write(&root.join(&dir).join("seed.txt"), "seed");
    sync(&a);
    for n in 0..10 {
        let path = format!("{dir}/f{n}.txt");
        write(&root.join(&path), "ours");
        a.send(Command::SyncNow);
        // The moment the upload is listed (with our checksum), the other client writes.
        let deadline = Instant::now() + Duration::from_secs(120);
        let ours = loop {
            let listed = live.other().list(&dir).ok().and_then(|l| l.into_iter().find(|e| e.path == path && !e.checksums.is_empty()));
            if let Some(e) = listed {
                break e;
            }
            assert!(Instant::now() < deadline, "{path} was never uploaded");
            std::thread::sleep(Duration::from_millis(5));
        };
        // The same size as ours, so nothing but the version tag tells the two writes apart.
        assert!(matches!(live.other().put(&path, b"THEM"), 200..=299));
        let theirs = live.other().list(&dir).unwrap().into_iter().find(|e| e.path == path).unwrap();
        sync(&a);
        sync(&a);
        assert_eq!(read(&root.join(&path)), "THEM", "their write came down ({})", if theirs.etag == ours.etag { "same ETag" } else { "new ETag" });
        assert_eq!(live.other().get(&path).as_deref(), Some(b"THEM".as_slice()), "and stays in the cloud");
        if theirs.etag == ours.etag {
            assert!(theirs.checksums.is_empty(), "the write cleared the listed checksum: {theirs:?}");
            eprintln!("staged at try {n}: two writes within one second, one ETag");
            return;
        }
        // A later second: an ordinary change (a new ETag). Try again.
    }
    panic!("ten tries and the two writes never shared a second");
}

#[test]
fn the_same_bytes_written_again_are_not_uploaded_again() {
    let Some(live) = Live::two_way("same") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    let up = live.p("Same/up.txt");
    write(&a_root.join(&up), "base\n");
    sync(&a);
    sync(&b);
    let ups = |h: &Handle, path: &str| h.progress.lock().unwrap().activity.iter().filter(|e| e.kind == ActivityKind::Up && e.path == path).count();

    // Uploaded from A, then written again with the same bytes: no upload, no version.
    write(&a_root.join(&up), "base\n");
    sync(&a);
    assert_eq!(ups(&a, &up), 1);
    assert!(live.versions_of(&up).is_empty());
    // Downloaded on B, the same.
    write(&b_root.join(&up), "base\n");
    sync(&b);
    assert_eq!(ups(&b, &up), 0);
    // Other bytes still go up.
    write(&b_root.join(&up), "BASE\n");
    sync(&b);
    assert_eq!(ups(&b, &up), 1);
}

#[test]
fn an_edit_made_offline_follows_a_rename_made_elsewhere() {
    let Some(live) = Live::two_way("follow") else { return };
    let (a_root, b_root) = (live.local("A"), live.local("B"));
    let a = live.computer(&a_root, "Alpha");
    let b = live.computer(&b_root, "Beta");
    let follow = live.p("Follow");
    let at = |root: &Path, name: &str| root.join(&follow).join(name);
    write(&at(&a_root, "f.txt"), "first");
    write(&at(&a_root, "g.txt"), "second");
    sync(&a);
    sync(&b);

    // B is offline (paused) while it changes things: otherwise its folder watcher syncs its change
    // at once, before the other computer's, and the order this test is about depends on timing.
    let offline = |on: bool| {
        b.send(Command::Pause(on));
        if on {
            until(&b, Duration::from_secs(60), |st| *st == SyncState::Paused);
        }
    };

    // A renames f while B (not syncing) edits it: one file in the end, renamed, with the edit.
    offline(true);
    std::fs::rename(at(&a_root, "f.txt"), at(&a_root, "renamed-A.txt")).unwrap();
    sync(&a);
    write(&at(&b_root, "f.txt"), "edited on B");
    offline(false);
    sync(&b);
    sync(&b);
    sync(&a);
    // And the other way round: B (not syncing) renames g while A edits it.
    offline(true);
    std::fs::rename(at(&b_root, "g.txt"), at(&b_root, "renamed-B.txt")).unwrap();
    write(&at(&a_root, "g.txt"), "edited on A");
    sync(&a);
    offline(false);
    sync(&b);
    sync(&b);
    sync(&a);
    for root in [&a_root, &b_root] {
        assert_eq!(names_in(&root.join(&follow)), ["renamed-A.txt", "renamed-B.txt"], "{}", root.display());
        assert_eq!(read(&at(root, "renamed-A.txt")), "edited on B");
        assert_eq!(read(&at(root, "renamed-B.txt")), "edited on A");
    }
}

/// T-038: a mass delete made in the cloud and discarded here comes back from the cloud's trash
/// (the same folder, its id kept), not uploaded again as new files.
#[test]
fn discarded_cloud_deletes_come_back_from_the_trash() {
    let Some(live) = Live::two_way("t38") else { return };
    let root = live.local("A");
    let t38 = live.p("t38");
    std::fs::create_dir_all(root.join(&t38).join("data")).unwrap();
    std::fs::write(root.join(&t38).join("keep.txt"), "keep").unwrap();
    for i in 0..12 {
        std::fs::write(root.join(&t38).join(format!("data/f{i}.txt")), format!("file {i}")).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    let a = live.computer(&root, "Alpha");
    sync(&a);
    let data = format!("{t38}/data");
    let has_ids = live.kind() == Kind::Oxicloud || live.server.has_file_ids();
    let id = live.folder_id(&data);
    assert_eq!(id.is_some(), has_ids, "{}: {id:?}", live.server);

    // Deleted in the cloud (by another client): 12 of 13 files would go here too, so the sync holds them.
    live.trash_folder(&data);
    a.send(Command::SyncNow);
    until(&a, Duration::from_secs(60), |s| matches!(s, SyncState::MassDelete { .. }));
    a.send(Command::DiscardDeletes);
    sync(&a);

    if has_ids {
        assert_eq!(live.folder_id(&data), id, "the same folder is back");
    }
    assert!(live.list(&t38).iter().any(|e| e.path == data && e.is_folder), "the folder is back");
    assert!(live.in_trash(&data).is_none(), "nothing left in the trash");
    let p = a.progress.lock().unwrap();
    let kept = p.activity.iter().position(|e| e.detail.contains("12 put back from the cloud's trash")).expect("said what came back");
    assert!(!p.activity[kept..].iter().any(|e| e.kind == ActivityKind::Up), "nothing uploaded as new: {:?}", &p.activity[kept..]);
    assert_eq!(names_in(&root.join(&t38).join("data")).len(), 12, "nothing deleted here");
}

/// T-042: a folder deleted here and held as a mass delete isn't made again (empty) by the passes
/// while it waits, and once confirmed it goes to the trash as one item.
#[test]
fn a_held_folder_delete_is_one_trash_item_when_confirmed() {
    let Some(live) = Live::two_way("t42") else { return };
    let root = live.local("A");
    let (t42, many) = (live.p("t42"), live.p("t42/many"));
    std::fs::create_dir_all(root.join(&many)).unwrap();
    std::fs::write(root.join(&t42).join("keep.txt"), "keep").unwrap();
    for i in 0..20 {
        std::fs::write(root.join(&many).join(format!("f{i}.txt")), format!("file {i}")).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    let a = live.computer(&root, "Alpha");
    sync(&a);

    std::fs::remove_dir_all(root.join(&many)).unwrap();
    for _ in 0..2 {
        // Held, and held again: the pass in between must not bring the folder back here.
        let cycles = a.progress.lock().unwrap().cycles;
        a.send(Command::SyncNow);
        until(&a, Duration::from_secs(60), |s| matches!(s, SyncState::MassDelete { .. }));
        while a.progress.lock().unwrap().cycles == cycles {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!root.join(&many).exists(), "not made again while held");
    }
    a.send(Command::ConfirmDeletes);
    sync(&a);
    assert!(!root.join(&many).exists());
    assert!(live.in_trash(&many).is_some(), "the folder is one trash item");
    assert!(live.in_trash(&format!("{many}/f0.txt")).is_none(), "its files are not trash items of their own");
    let p = a.progress.lock().unwrap();
    let deleted: Vec<_> = p.activity.iter().filter(|e| e.kind == ActivityKind::Deleted).collect();
    assert_eq!(deleted.len(), 1, "one row for the folder: {deleted:?}");
    assert_eq!(deleted[0].path, many);
}

#[test]
fn the_ui_transport_answers_on_its_thread() {
    let Some((server, _, _)) = oxicloud_env() else { return };
    let mut backend = ThreadBackend::new(Arc::new(|| {}));
    backend.send(7, auth::status(&server));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some((id, result)) = backend.poll().pop() {
            assert_eq!(id, 7);
            assert!(auth::parse_status(&result.unwrap()).is_ok());
            return;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------------------------
// Download only (docs/WEBDAV-STORAGE.md § 9): a server that can't guard writes (OpenCloud, Q1;
// `rclone serve webdav`, Q2). Nothing is ever written to it; nothing is ever lost here.

/// Every file and folder under `folder` in the cloud as (path, is_folder, ETag): what a cycle
/// must leave exactly as it was.
fn cloud_snapshot(live: &Live, folder: &str) -> Vec<(String, bool, Option<String>)> {
    live.other().list_all(folder).into_iter().map(|e| (e.path, e.is_folder, e.etag)).collect()
}

/// Files in the local folder's own trash (`.craftcloud/trash/<entry>/<path>`): (cloud path, bytes).
fn local_trash(root: &Path) -> Vec<(String, String)> {
    let trash = root.join(dav::CRAFTCLOUD_DIR).join("trash");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&trash) else { return out };
    for entry in entries.flatten() {
        let mut stack = vec![entry.path()];
        while let Some(dir) = stack.pop() {
            for item in std::fs::read_dir(&dir).unwrap().flatten() {
                if item.path().is_dir() {
                    stack.push(item.path());
                } else {
                    let rel = item.path().strip_prefix(entry.path()).unwrap().to_string_lossy().replace('\\', "/");
                    out.push((rel, read(&item.path())));
                }
            }
        }
    }
    out.sort();
    out
}

/// How often the activity list says `path` wasn't uploaded.
fn said_not_uploaded(h: &Handle, path: &str) -> usize {
    h.progress.lock().unwrap().activity.iter().filter(|e| e.path == path && e.detail.contains("not uploaded")).count()
}

#[test]
fn download_only_writes_nothing_to_the_server_and_keeps_local_changes_here() {
    let Some(live) = Live::download_only("ro") else { return };
    let ro = live.p("Ro");
    {
        let mut other = live.other();
        other.mkdirs(&format!("{ro}/sub"));
        assert!(matches!(other.put(&format!("{ro}/a.txt"), b"cloud a"), 201 | 204));
        assert!(matches!(other.put(&format!("{ro}/sub/b.txt"), b"cloud b"), 201 | 204));
    }
    let root = live.local("D");
    let d = live.computer(&root, "Down");
    sync(&d);
    assert_eq!(read(&root.join(&ro).join("a.txt")), "cloud a");
    assert_eq!(read(&root.join(&ro).join("sub/b.txt")), "cloud b");
    let before = cloud_snapshot(&live, &ro);
    assert_eq!(before.len(), 3, "{before:?}");

    // A change here, a new file here, a delete here: nothing of it reaches the server.
    write(&root.join(&ro).join("a.txt"), "changed here");
    write(&root.join(&ro).join("new here.txt"), "new here");
    std::fs::remove_file(root.join(&ro).join("sub/b.txt")).unwrap();
    for _ in 0..3 {
        sync(&d);
    }
    assert_eq!(cloud_snapshot(&live, &ro), before, "the server is exactly as it was");
    // The changed file stays as it is here, said once; the new one too; the deleted one comes back.
    assert_eq!(read(&root.join(&ro).join("a.txt")), "changed here");
    assert_eq!(read(&root.join(&ro).join("new here.txt")), "new here");
    assert_eq!(read(&root.join(&ro).join("sub/b.txt")), "cloud b", "a file deleted here comes back from the cloud");
    let (a, new) = (format!("{ro}/a.txt"), format!("{ro}/new here.txt"));
    assert_eq!(said_not_uploaded(&d, &a), 1, "{:?}", d.progress.lock().unwrap().activity);
    assert_eq!(said_not_uploaded(&d, &new), 1);
    let p = d.progress.lock().unwrap();
    assert_eq!(p.left_out.get(&a), Some(&LeftOut::NotUploaded));
    assert_eq!(p.left_out.get(&new), Some(&LeftOut::NotUploaded));
    assert!(p.not_synced.contains(&a) && p.not_synced.contains(&new), "{:?}", p.not_synced);
    assert!(!p.activity.iter().any(|e| matches!(e.kind, ActivityKind::Up | ActivityKind::Error)), "{:?}", p.activity);
    drop(p);
    // Changed again here: said once more for the new version, not again for the old.
    write(&root.join(&ro).join("a.txt"), "changed here again");
    sync(&d);
    sync(&d);
    assert_eq!(said_not_uploaded(&d, &a), 2);
    assert_eq!(cloud_snapshot(&live, &ro), before);
}

#[test]
fn download_only_a_cloud_change_over_a_local_change_makes_a_local_conflict_copy() {
    let Some(live) = Live::download_only("roc") else { return };
    let roc = live.p("Roc");
    {
        let mut other = live.other();
        other.mkdirs(&roc);
        assert!(matches!(other.put(&format!("{roc}/doc.txt"), b"v1"), 201 | 204));
    }
    let root = live.local("D");
    let d = live.computer(&root, "Down");
    sync(&d);
    assert_eq!(read(&root.join(&roc).join("doc.txt")), "v1");

    // Changed here, then changed in the cloud too: the cloud version comes down, this computer's
    // becomes a conflict copy here, and nothing goes up.
    write(&root.join(&roc).join("doc.txt"), "edited here");
    sync(&d);
    let before = cloud_snapshot(&live, &roc);
    assert!(matches!(live.other().put(&format!("{roc}/doc.txt"), b"v2 from the cloud"), 200 | 201 | 204));
    sync(&d);
    sync(&d);
    assert_eq!(read(&root.join(&roc).join("doc.txt")), "v2 from the cloud");
    let names = names_in(&root.join(&roc));
    let copy = names.iter().find(|n| n.contains("(conflict ")).unwrap_or_else(|| panic!("{names:?}"));
    assert!(copy.contains("Down") && copy.ends_with(".txt"), "{copy}");
    assert_eq!(read(&root.join(&roc).join(copy)), "edited here");
    let after = cloud_snapshot(&live, &roc);
    assert_eq!(after.len(), 1, "the conflict copy stayed here: {after:?}");
    assert_eq!(after[0].0, before[0].0);
    let p = d.progress.lock().unwrap();
    assert!(!p.activity.iter().any(|e| e.kind == ActivityKind::Up), "{:?}", p.activity);
    assert_eq!(p.activity.iter().filter(|e| e.kind == ActivityKind::Conflict).count(), 1, "{:?}", p.activity);
}

/// Both ways too, a file deleted in the cloud is never removed for good here: it leaves its
/// place and is kept in the local folder's own trash, whatever the cloud's trash does.
#[test]
fn a_file_deleted_in_the_cloud_lands_in_the_local_trash_both_ways_too() {
    let Some(live) = Live::two_way("dlt") else { return };
    let root = live.local("A");
    let dlt = live.p("Dlt");
    std::fs::create_dir_all(root.join(&dlt).join("sub")).unwrap();
    std::fs::write(root.join(&dlt).join("sub/one.txt"), "one").unwrap();
    for n in ["keep1.txt", "keep2.txt", "keep3.txt"] {
        std::fs::write(root.join(&dlt).join(n), n).unwrap();
    }
    std::thread::sleep(Duration::from_millis(2_200));
    let a = live.computer(&root, "Alpha");
    sync(&a);
    live.trash_folder(&format!("{dlt}/sub"));
    sync(&a);
    assert!(!root.join(&dlt).join("sub/one.txt").exists(), "gone from its place");
    let trashed = local_trash(&root);
    assert_eq!(trashed, [(format!("{dlt}/sub/one.txt"), "one".to_owned())], "{trashed:?}");
    assert_eq!(names_in(&root.join(&dlt)).len(), 3, "the rest stays");
}

#[test]
fn download_only_a_file_deleted_in_the_cloud_lands_in_the_local_trash() {
    let Some(live) = Live::download_only("rod") else { return };
    let rod = live.p("Rod");
    {
        let mut other = live.other();
        other.mkdirs(&format!("{rod}/many"));
        assert!(matches!(other.put(&format!("{rod}/one.txt"), b"one"), 201 | 204));
        assert!(matches!(other.put(&format!("{rod}/keep.txt"), b"keep"), 201 | 204));
        for i in 0..12 {
            assert!(matches!(other.put(&format!("{rod}/many/f{i}.txt"), format!("file {i}").as_bytes()), 201 | 204));
        }
    }
    let root = live.local("D");
    let d = live.computer(&root, "Down");
    sync(&d);
    assert_eq!(names_in(&root.join(&rod).join("many")).len(), 12);

    // One file deleted in the cloud: gone from its place here, kept in the local folder's own trash.
    assert!(matches!(live.other().delete(&format!("{rod}/one.txt"), false), 200 | 204));
    sync(&d);
    assert!(!root.join(&rod).join("one.txt").exists());
    let trashed = local_trash(&root);
    assert_eq!(trashed, [(format!("{rod}/one.txt"), "one".to_owned())], "{trashed:?}");
    let said = d
        .progress
        .lock()
        .unwrap()
        .activity
        .iter()
        .any(|e| e.kind == ActivityKind::Deleted && e.path == format!("{rod}/one.txt") && e.detail.contains("kept here"));
    assert!(said, "{:?}", d.progress.lock().unwrap().activity);

    // A whole folder deleted in the cloud (12 of 13 files): held as a mass delete first, and once
    // confirmed every file lands in the local trash, none is gone.
    assert!(matches!(live.other().delete(&format!("{rod}/many"), true), 200 | 204));
    d.send(Command::SyncNow);
    until(&d, Duration::from_secs(60), |s| matches!(s, SyncState::MassDelete { .. }));
    assert_eq!(names_in(&root.join(&rod).join("many")).len(), 12, "held: nothing moved yet");
    d.send(Command::ConfirmDeletes);
    sync(&d);
    assert!(!root.join(&rod).join("many").exists(), "{:?}", names_in(&root.join(&rod)));
    let trashed = local_trash(&root);
    assert_eq!(trashed.len(), 13, "{trashed:?}");
    for i in 0..12 {
        assert!(trashed.contains(&(format!("{rod}/many/f{i}.txt"), format!("file {i}"))), "{trashed:?}");
    }
    assert_eq!(read(&root.join(&rod).join("keep.txt")), "keep");
    // A file changed here since the last sync stays where it is when the cloud deletes it.
    write(&root.join(&rod).join("keep.txt"), "changed here");
    assert!(matches!(live.other().delete(&format!("{rod}/keep.txt"), false), 200 | 204));
    sync(&d);
    assert_eq!(read(&root.join(&rod).join("keep.txt")), "changed here", "a change made here is never moved away");
    assert_eq!(local_trash(&root).len(), 13);
}

/// A small HTTP proxy in front of the server that answers 500 to every `PROPFIND` under one
/// cloud path and passes everything else through: a listing that fails, on any server.
struct FailingProxy {
    url: String,
    /// The percent-encoded URL path prefix whose listings fail, or none.
    failing: Arc<Mutex<Option<String>>>,
}

impl FailingProxy {
    fn to(server: &str) -> Self {
        use std::io::{BufRead, BufReader, Read, Write};
        let target = host_port(server);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let failing: Arc<Mutex<Option<String>>> = Arc::default();
        let rule = Arc::clone(&failing);
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let rule = Arc::clone(&rule);
                let target = target.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(client.try_clone().unwrap());
                    let mut client = client;
                    // One request per connection: the answer is relayed and both sides closed.
                    {
                        // The request head, line by line.
                        let mut head = Vec::new();
                        let mut line = String::new();
                        let mut method_path = (String::new(), String::new());
                        let mut length = 0usize;
                        let mut first = true;
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                                return;
                            }
                            if first {
                                let mut parts = line.split_whitespace();
                                method_path = (parts.next().unwrap_or_default().to_owned(), parts.next().unwrap_or_default().to_owned());
                                first = false;
                            } else if let Some(v) = line.strip_prefix("Content-Length:").or_else(|| line.strip_prefix("content-length:")) {
                                length = v.trim().parse().unwrap_or(0);
                            } else if line.to_ascii_lowercase().starts_with("connection:") {
                                continue;
                            }
                            if line == "\r\n" {
                                break;
                            }
                            head.extend_from_slice(line.as_bytes());
                        }
                        let mut body = vec![0u8; length];
                        reader.read_exact(&mut body).unwrap();
                        let blocked = rule.lock().unwrap().as_ref().is_some_and(|p| method_path.0 == "PROPFIND" && method_path.1.starts_with(p));
                        if blocked {
                            let _ = client.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            return;
                        }
                        let Ok(mut upstream) = std::net::TcpStream::connect(&target) else { return };
                        let mut request = head;
                        request.extend_from_slice(b"Connection: close\r\n\r\n");
                        request.extend_from_slice(&body);
                        if upstream.write_all(&request).is_err() {
                            return;
                        }
                        let mut answer = Vec::new();
                        if upstream.read_to_end(&mut answer).is_err() {
                            return;
                        }
                        // The server closed after the answer (asked to); so does this side.
                        let _ = client.write_all(&answer);
                    }
                });
            }
        });
        Self { url, failing }
    }

    /// Listings of `cloud_path` (a folder, every level under it) fail from now on, or none do.
    fn fail_listings_under(&self, base: &Base, cloud_path: Option<&str>) {
        *self.failing.lock().unwrap() = cloud_path.map(|p| {
            let full = dav::url_under(base.url(), p, true);
            full.find("://").and_then(|i| full[i + 3..].find('/').map(|j| full[i + 3 + j..].to_owned())).unwrap_or(full)
        });
    }
}

#[test]
fn download_only_a_failed_listing_holds_everything_under_it() {
    let Some(live) = Live::download_only("rol") else { return };
    let (held, other_folder) = (live.p("Held"), live.p("Other"));
    {
        let mut other = live.other();
        other.mkdirs(&format!("{held}/sub"));
        other.mkdirs(&other_folder);
        assert!(matches!(other.put(&format!("{held}/a.txt"), b"a"), 201 | 204));
        assert!(matches!(other.put(&format!("{held}/sub/b.txt"), b"b"), 201 | 204));
        assert!(matches!(other.put(&format!("{other_folder}/c.txt"), b"c"), 201 | 204));
    }
    let proxy = FailingProxy::to(&live.server.url);
    let root = live.local("D");
    let d = live.start_at(live.plan(&root, &[]), "Down", Some(&proxy.url));
    sync(&d);
    assert_eq!(read(&root.join(&held).join("sub/b.txt")), "b");

    // The folder's listing fails while a file in it is deleted in the cloud: the pass ends without
    // a result, nothing under the folder is taken as deleted, nothing is moved to the trash.
    proxy.fail_listings_under(&live.server.base(), Some(&held));
    assert!(matches!(live.other().delete(&format!("{held}/a.txt"), false), 200 | 204));
    assert!(matches!(live.other().put(&format!("{other_folder}/c.txt"), b"c2"), 200 | 201 | 204));
    let p = sync_ends(&d);
    assert!(matches!(p.state, SyncState::Failed(_)), "a pass with a failed listing doesn't end up to date: {:?}", p.state);
    assert_eq!(read(&root.join(&held).join("a.txt")), "a", "held, not deleted");
    assert_eq!(read(&root.join(&held).join("sub/b.txt")), "b");
    assert!(local_trash(&root).is_empty(), "{:?}", local_trash(&root));
    assert!(!p.activity.iter().any(|e| e.kind == ActivityKind::Deleted), "{:?}", p.activity);

    // Listed again, the delete is seen and the file goes to the local trash, never away.
    proxy.fail_listings_under(&live.server.base(), None);
    sync(&d);
    assert!(!root.join(&held).join("a.txt").exists());
    assert_eq!(local_trash(&root), [(format!("{held}/a.txt"), "a".to_owned())]);
    assert_eq!(read(&root.join(&other_folder).join("c.txt")), "c2");
}
