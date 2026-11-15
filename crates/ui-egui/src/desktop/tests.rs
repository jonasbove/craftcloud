use craftcloud_core::api::auth::Tokens;
use craftcloud_core::http::{HttpRequest, HttpResponse};
use craftcloud_core::sync::{Appearance, ClientSettings, ConflictCopy, Keep, SyncPlan, SyncProgress, SyncState};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{DesktopApp, DesktopHost, FileFacts, Overlay, Screen, Tab, WallClock};
use crate::backend::{Backend, FakeBackend, NetResult};

#[derive(Default)]
pub(super) struct FakeDesktop {
    pub(super) net: FakeBackend,
    saved: Option<(ClientSettings, Tokens)>,
    started: Vec<SyncPlan>,
    pub(super) progress: SyncProgress,
    signed_out: bool,
    paused: Option<bool>,
    changed: Option<ClientSettings>,
    prefs: Option<ClientSettings>,
    resolved: Vec<(ConflictCopy, Keep)>,
    login: bool,
    synced_now: u32,
    confirmed: u32,
    discarded: u32,
    recreated: u32,
    quit: bool,
    opened_files: Vec<String>,
    /// The system browser doesn't open.
    pub(super) no_browser: bool,
    opened_folders: u32,
}

impl Backend for FakeDesktop {
    fn send(&mut self, id: u64, request: HttpRequest) {
        self.net.send(id, request);
    }
    fn poll(&mut self) -> Vec<(u64, NetResult)> {
        self.net.poll()
    }
    fn open_url(&mut self, url: &str) {
        self.net.open_url(url);
    }
    fn now(&self) -> f64 {
        self.net.now()
    }
}

impl DesktopHost for FakeDesktop {
    fn client_name(&self) -> String {
        "CraftCloud on Studio".into()
    }
    fn computer_name(&self) -> String {
        "Studio".into()
    }
    fn default_folder(&self) -> String {
        "/home/john/CraftCloud".into()
    }
    fn save_session(&mut self, settings: &ClientSettings, tokens: &Tokens) -> Result<(), String> {
        self.saved = Some((settings.clone(), tokens.clone()));
        Ok(())
    }
    fn start_sync(&mut self, plan: SyncPlan, _tokens: Tokens) {
        self.started.push(plan);
    }
    fn progress(&self) -> SyncProgress {
        self.progress.clone()
    }
    fn fresh_tokens(&self) -> Option<Tokens> {
        None
    }
    fn sync_now(&mut self) {
        self.synced_now += 1;
    }
    fn confirm_deletes(&mut self) {
        self.confirmed += 1;
    }
    fn discard_deletes(&mut self) {
        self.discarded += 1;
    }
    fn recreate_folder(&mut self) {
        self.recreated += 1;
    }
    fn pause(&mut self, paused: bool) {
        self.paused = Some(paused);
    }
    fn change_folders(&mut self, settings: &ClientSettings) -> Result<(), String> {
        self.changed = Some(settings.clone());
        Ok(())
    }
    fn save_prefs(&mut self, settings: &ClientSettings) -> Result<(), String> {
        self.prefs = Some(settings.clone());
        Ok(())
    }
    fn resolve(&mut self, conflict: &ConflictCopy, keep: Keep) {
        self.resolved.push((conflict.clone(), keep));
    }
    fn local_file(&self, _path: &str) -> Option<FileFacts> {
        Some(FileFacts { size: 88_000, modified: "today 13:56".into() })
    }
    fn starts_at_login(&self) -> bool {
        self.login
    }
    fn set_start_at_login(&mut self, on: bool) -> Result<(), String> {
        self.login = on;
        Ok(())
    }
    fn open_browser(&mut self, url: &str) -> bool {
        self.net.open_url(url);
        !self.no_browser
    }
    fn open_folder(&mut self, _path: &str) {
        self.opened_folders += 1;
    }
    fn open_file(&mut self, path: &str) {
        self.opened_files.push(path.to_owned());
    }
    fn clock(&self) -> WallClock {
        WallClock { unix: 1_794_570_000, date: "2026-11-13".into() }
    }
    fn sign_out(&mut self) {
        self.signed_out = true;
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/oxicloud-0.10.0").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ok(body: &[u8]) -> NetResult {
    Ok(HttpResponse::new(200, body.to_vec()))
}

const ROOT: &str = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/webdav/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/Documents/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/.craftcloud/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/readme.md</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>3</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;

type H = Harness<'static, DesktopApp<FakeDesktop>>;

fn app(settings: ClientSettings, tokens: Option<Tokens>) -> H {
    let app = DesktopApp::new(&egui::Context::default(), FakeDesktop::default(), settings, tokens);
    Harness::builder().with_size(egui::vec2(960.0, 640.0)).build_ui_state(|ui, app: &mut DesktopApp<FakeDesktop>| app.ui(ui), app)
}

fn connect(h: &mut H) {
    h.state_mut().screen = Screen::Connect(crate::connect::ConnectScreen::new("http://127.0.0.1:8093"));
    h.run();
    h.get_by_label("CONTINUE").click();
    h.run();
}

#[test]
fn the_sign_in_address_is_always_shown_and_a_failed_browser_says_so() {
    let mut h = app(ClientSettings::default(), None);
    h.state_mut().host.no_browser = true;
    connect(&mut h);
    h.state_mut().host.net.answer("/craftcloud/config.json", ok(br#"{"version":"0.1.0","web_apps":false,"public_url":"http://127.0.0.1:8093"}"#));
    h.run();
    h.state_mut().host.net.answer("/api/auth/device/authorize", ok(&fixture("device-authorize.json")));
    h.run();
    h.get_by_label("Open this address in your browser, sign in, and check it shows this code:");
    h.get_by_label("http://127.0.0.1:8093/device?code=KOQY-2239");
    assert!(h.query_by_label_contains("We opened").is_none(), "no claim that the browser opened");
    h.get_by_label("COPY");

    // The browser works the second time: the sheet says so, and the address stays.
    h.state_mut().host.no_browser = false;
    h.get_by_label("TRY THE BROWSER AGAIN").click();
    h.run();
    h.get_by_label("We opened 127.0.0.1:8093 in your browser. Sign in there and check it shows this code:");
    h.get_by_label("http://127.0.0.1:8093/device?code=KOQY-2239");
}

#[test]
fn signs_in_through_the_browser_and_starts_syncing() {
    let mut h = app(ClientSettings::default(), None);
    h.get_by_label("Not connected");
    connect(&mut h);
    h.state_mut().host.net.answer("/craftcloud/config.json", ok(br#"{"version":"0.1.0","web_apps":false,"public_url":"http://127.0.0.1:8093"}"#));
    h.run();
    h.get_by_label("FOUND");
    let body: serde_json::Value = serde_json::from_slice(&h.state().host.net.last_to("/api/auth/device/authorize").body).unwrap();
    assert_eq!(body["client_name"], "CraftCloud on Studio");

    h.state_mut().host.net.answer("/api/auth/device/authorize", ok(&fixture("device-authorize.json")));
    h.run();
    assert!(h.state().host.net.opened[0].contains("/device?code="), "the browser opens the approval page");
    h.get_by_label("Continue in your browser");
    h.get_by_label("OPEN BROWSER AGAIN").click();
    h.run();
    assert_eq!(h.state().host.net.opened.len(), 2);

    // Not before the interval…
    assert_eq!(h.state().host.net.sent_to("/api/auth/device/token"), 0);
    h.state_mut().host.net.clock = 5.0;
    h.run();
    assert_eq!(h.state().host.net.sent_to("/api/auth/device/token"), 1);
    h.state_mut().host.net.answer("/api/auth/device/token", Ok(HttpResponse::new(400, fixture("device-token-pending.json"))));
    h.run();
    // …and again after it.
    h.state_mut().host.net.clock = 10.0;
    h.run();
    assert_eq!(h.state().host.net.sent_to("/api/auth/device/token"), 2);
    h.state_mut().host.net.answer("/api/auth/device/token", ok(&fixture("device-token-ok.json")));
    h.run();

    let list = h.state().host.net.last_to("/webdav/");
    assert_eq!(list.header_value("Authorization"), Some("Bearer <redacted access_token>"));
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(207, ROOT.as_bytes().to_vec())));
    h.run();
    h.get_by_label("Choose what to sync");
    h.get_by_label("Documents");
    assert!(h.query_by_label(".craftcloud").is_none(), "CraftCloud's own folder is never offered");
    h.get_by_label("Signed in");

    h.get_by_label("START SYNCING").click();
    h.run();
    let (settings, _) = h.state().host.saved.clone().expect("session saved");
    assert_eq!(settings.local_folder, "/home/john/CraftCloud");
    assert!(settings.excluded.is_empty() && settings.folders.is_empty(), "everything syncs, folders made later too");
    assert!(settings.sync_root_files);
    assert_eq!(h.state().host.started.len(), 1);
    assert!(matches!(h.state().screen, Screen::Main(Tab::Activity)));
}

#[test]
fn the_choose_sheet_switches_folders_and_the_local_folder() {
    let mut h = app(ClientSettings::default(), None);
    h.state_mut().tokens = Some(Tokens { access_token: "a".into(), refresh_token: "r".into() });
    h.state_mut().screen = Screen::Choose {
        choice: super::Choice { folders: vec![("Documents".into(), true), ("Photos".into(), true)], root_files: true },
        local: "/home/john/CraftCloud".into(),
        changing: false,
    };
    h.run();
    h.get_by_label("Photos: OFF").click();
    h.get_by_label("Files at the top level: OFF").click();
    h.run();
    h.get_by_label("CHANGE…").click();
    h.run();
    h.get_by_label("LOCAL FOLDER").click();
    h.run();
    h.get_by_label("START SYNCING").click();
    h.run();
    let (settings, _) = h.state().host.saved.clone().expect("session saved");
    assert_eq!(settings.excluded, ["Photos"]);
    assert!(!settings.sync_root_files);
}

#[test]
fn a_server_that_is_not_craftcloud_is_refused() {
    let mut h = app(ClientSettings::default(), None);
    connect(&mut h);
    h.state_mut().host.net.answer("/craftcloud/config.json", Ok(HttpResponse::new(404, b"<html>".to_vec())));
    h.run();
    h.get_by_label("That address answers, but it isn't a CraftCloud server.");
    assert_eq!(h.state().host.net.sent_to("/api/auth/device/authorize"), 0);
}

#[test]
fn quit_on_the_connect_sheet_ends_the_client() {
    let mut h = app(ClientSettings::default(), None);
    h.get_by_label("QUIT").click();
    h.run();
    assert!(h.state().host.quit);
}

#[test]
fn a_denied_sign_in_goes_back_to_the_start() {
    let mut h = app(ClientSettings::default(), None);
    connect(&mut h);
    h.state_mut().host.net.answer("/craftcloud/config.json", ok(br#"{"version":"0.1.0","web_apps":false,"public_url":"x"}"#));
    h.run();
    h.state_mut().host.net.answer("/api/auth/device/authorize", ok(&fixture("device-authorize.json")));
    h.run();
    h.state_mut().host.net.clock = 5.0;
    h.run();
    h.state_mut().host.net.answer("/api/auth/device/token", Ok(HttpResponse::new(400, br#"{"error":"access_denied"}"#.to_vec())));
    h.run();
    h.get_by_label("Sign-in was denied in the browser.");
    assert!(matches!(h.state().screen, Screen::Connect(_)));
}

fn signed_in_settings() -> ClientSettings {
    ClientSettings {
        server: "http://127.0.0.1:8093".into(),
        local_folder: "/home/john/CraftCloud".into(),
        excluded: vec!["Photos".into()],
        sync_root_files: false,
        ..ClientSettings::default()
    }
}

fn signed_in() -> H {
    app(signed_in_settings(), Some(Tokens { access_token: "a".into(), refresh_token: "r".into() }))
}

fn with_state(state: SyncState) -> H {
    let mut h = signed_in();
    h.state_mut().host.progress = SyncProgress { state, ..SyncProgress::default() };
    h.run();
    h
}

#[test]
fn a_saved_session_goes_straight_to_syncing() {
    let mut h = with_state(SyncState::UpToDate);
    assert_eq!(h.state().host.started.len(), 1);
    h.get_by_label("Up to date");

    h.get_by_label("SETTINGS").click();
    h.run();
    assert_eq!(h.state().overlay, Some(Overlay::Settings));
    h.get_by_label("SIGN OUT").click();
    h.run();
    assert!(h.state().host.signed_out);
    assert!(matches!(h.state().screen, Screen::Connect(_)));
}

#[test]
fn the_folders_tab_changes_what_syncs() {
    let mut h = signed_in();
    h.get_by_label("FOLDERS").click();
    h.run();
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(207, ROOT_TWO.as_bytes().to_vec())));
    h.run();
    h.get_by_label("Photos: ON").click();
    h.run();
    h.get_by_label("APPLY").click();
    h.run();
    let changed = h.state().host.changed.clone().expect("folders changed");
    assert!(changed.excluded.is_empty(), "{changed:?}");
}

#[test]
fn version_0_1_folder_lists_become_excluded_folders() {
    let settings = ClientSettings { excluded: vec![], folders: vec!["Documents".into()], ..signed_in_settings() };
    let mut h = app(settings, Some(Tokens { access_token: "a".into(), refresh_token: "r".into() }));
    h.run();
    // Asked at once, without opening the Folders tab.
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(207, ROOT_TWO.as_bytes().to_vec())));
    h.run();
    let changed = h.state().host.changed.clone().expect("settings migrated");
    assert_eq!(changed.excluded, ["Photos"]);
    assert!(changed.folders.is_empty());
}

#[test]
fn the_folders_tab_says_when_the_cloud_cant_be_listed() {
    let mut h = signed_in();
    h.get_by_label("FOLDERS").click();
    h.run();
    h.state_mut().host.net.answer("/webdav/", Err("connection refused".into()));
    h.run();
    h.get_by_label("Can't list the cloud folders");
    h.get_by_label("RETRY NOW").click();
    h.run();
    assert_eq!(h.state().host.net.sent_to("/webdav/"), 2);
}

#[test]
fn conflicts_are_settled_from_the_conflict_sheet() {
    let mut h = signed_in();
    h.state_mut().host.progress = SyncProgress {
        state: SyncState::UpToDate,
        conflicts: vec![ConflictCopy { original: "Documents/notes.md".into(), copy: "Documents/notes (conflict 2026-11-13 14.32 Laptop).md".into() }],
        ..SyncProgress::default()
    };
    h.run();
    h.get_by_label("1 conflict");
    h.get_by_label("notes.md changed in two places");
    h.get_by_label("REVIEW").click();
    h.run();
    h.get_by_label("notes (conflict 2026-11-13 14.32 Laptop).md");
    h.get_by_label("KEEP MINE").click();
    h.run();
    assert_eq!(h.state().host.resolved[0].1, Keep::Mine);
    assert_eq!(h.state().overlay, None);
}

#[test]
fn settings_switch_start_at_login_pause_and_appearance() {
    let mut h = with_state(SyncState::UpToDate);
    h.get_by_label("SETTINGS").click();
    h.run();
    h.get_by_label("Start at login: ON").click();
    h.run();
    assert!(h.state().host.login);
    h.get_by_label("Pause syncing: ON").click();
    h.run();
    assert_eq!(h.state().host.paused, Some(true));
    h.get_by_label("Appearance: LIGHT").click();
    h.run();
    assert_eq!(h.state().host.prefs.as_ref().map(|s| s.appearance), Some(Appearance::Light));
    h.get_by_label("DONE").click();
    h.run();
    assert_eq!(h.state().overlay, None);
}

#[test]
fn a_paused_sync_resumes_from_its_panel() {
    let mut h = with_state(SyncState::Paused);
    h.get_by_label("Syncing is paused");
    h.get_by_label("RESUME").click();
    h.run();
    assert_eq!(h.state().host.paused, Some(false));
}

#[test]
fn a_mass_delete_waits_for_a_decision() {
    let mut h = with_state(SyncState::MassDelete { deletes: 42 });
    h.get_by_label("This would delete 42 files");
    h.get_by_label("DELETE THEM").click();
    h.run();
    assert_eq!(h.state().host.confirmed, 1);
    h.get_by_label("KEEP").click();
    h.run();
    assert_eq!(h.state().host.discarded, 1, "keeping brings the files back");
}

#[test]
fn a_missing_folder_is_never_made_again_without_asking() {
    let mut h = with_state(SyncState::FolderMissing);
    h.get_by_label("LOOK AGAIN").click();
    h.run();
    assert_eq!((h.state().host.synced_now, h.state().host.recreated), (1, 0));
    h.get_by_label("DOWNLOAD EVERYTHING AGAIN").click();
    h.run();
    assert_eq!(h.state().host.recreated, 1);
}

#[test]
fn a_missing_folder_turns_off_sync_now_and_open_folder() {
    let mut h = with_state(SyncState::FolderMissing);
    h.get_by_label("Off until the folder is back");
    h.get_by_label("SYNC NOW").click();
    h.get_by_label("OPEN FOLDER").click();
    h.run();
    assert_eq!((h.state().host.synced_now, h.state().host.opened_folders), (0, 0), "both are off while the folder is missing");
    assert_eq!(h.get_all_by_label("OPEN FOLDER").count(), 1, "the meta line has no folder link either");

    let mut h = with_state(SyncState::UpToDate);
    assert_eq!(h.get_all_by_label("OPEN FOLDER").count(), 2, "the button and the meta line's link");
    h.get_all_by_label("OPEN FOLDER").next().unwrap().click();
    h.run();
    assert_eq!(h.state().host.opened_folders, 1);
}

#[test]
fn offline_retries_and_signed_out_signs_in_again() {
    let mut h = with_state(SyncState::Offline("connection refused".into()));
    h.get_by_label("Offline");
    h.get_by_label("RETRY NOW").click();
    h.run();
    assert_eq!(h.state().host.synced_now, 1);

    let mut h = with_state(SyncState::SignedOut);
    h.get_by_label("SIGN IN AGAIN").click();
    h.run();
    assert!(h.state().host.signed_out);
    assert!(matches!(h.state().screen, Screen::Connect(_)));
}

const ROOT_TWO: &str = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/webdav/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/Documents/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/Photos/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;

#[test]
fn the_activity_list_shows_files_on_their_way_and_opens_files() {
    use craftcloud_core::sync::{ActivityEvent, ActivityKind, FileStatus};
    let mut h = signed_in();
    let mut p = SyncProgress { state: SyncState::Downloading, files_total: 2, current: "upload Documents/b.md".into(), ..SyncProgress::default() };
    p.unsynced.insert("Documents/b.md".into(), FileStatus::Syncing);
    p.record(ActivityEvent {
        at: 0,
        time: "14:32".into(),
        date: "2026-11-13".into(),
        kind: ActivityKind::Up,
        path: "Documents/a.md".into(),
        size: Some(3),
        detail: String::new(),
    });
    h.state_mut().host.progress = p;
    h.run();
    h.get_by_label("SYNCING");
    h.get_by_label("in progress");
    h.get_by_label("Open Documents/a.md").click();
    h.run();
    assert_eq!(h.state().host.opened_files, ["Documents/a.md"]);
}

/// A Windows error for a name the disk can't take, as the engine reports it.
pub(super) const LONG_DETAIL: &str = "couldn't download Weird/what?.md: can't write C:\\Users\\john\\CraftCloud\\Weird\\.what?.md.44c77418e27569db-1794748821.craftcloud-part: The filename, directory name, or volume label syntax is incorrect. (os error 123)";

#[test]
fn a_long_error_detail_stays_inside_its_activity_row() {
    use craftcloud_core::sync::{ActivityEvent, ActivityKind};
    for width in [960.0, 640.0] {
        let mut h = signed_in();
        let mut p = SyncProgress { state: SyncState::UpToDate, ..SyncProgress::default() };
        p.record(ActivityEvent {
            at: 0,
            time: "14:32".into(),
            date: "2026-11-13".into(),
            kind: ActivityKind::Error,
            path: "Weird/what?.md".into(),
            size: None,
            detail: LONG_DETAIL.into(),
        });
        h.state_mut().host.progress = p;
        h.set_size(egui::vec2(width, 640.0));
        h.run();
        let row = h.get_by_label("Open Weird/what?.md").rect();
        let path = h.get_by_label("Weird/what?.md").rect();
        let detail = h.get_by_label(LONG_DETAIL).rect();
        let badge = h.get_by_label("ERROR").rect();
        assert!(row.left() >= 0.0 && row.right() <= width, "{width}: the row {row:?} is in the window");
        assert!(detail.left() >= row.left() && detail.right() <= row.right() + 0.5, "{width}: the detail {detail:?} stays in the row {row:?}");
        assert!(detail.left() >= path.right(), "{width}: the detail {detail:?} doesn't cover the name {path:?}");
        assert!(path.left() >= badge.right(), "{width}: the name {path:?} doesn't cover the badge {badge:?}");
        assert!(path.width() > 20.0, "{width}: the name keeps some room: {path:?}");
    }
}

#[test]
fn a_failed_sync_never_says_two_full_stops() {
    let h = with_state(SyncState::Failed(
        "Another CraftCloud (the desktop client or `craftcloud daemon`) is already syncing C:\\Users\\john\\CraftCloud.".into(),
    ));
    h.get_by_label(
        "Another CraftCloud (the desktop client or `craftcloud daemon`) is already syncing C:\\Users\\john\\CraftCloud. It's tried again at the next cycle.",
    );
    let h = with_state(SyncState::Failed("the disk is full".into()));
    h.get_by_label("the disk is full. It's tried again at the next cycle.");
}

#[test]
fn the_folders_tab_counts_files_and_flags_errors() {
    use craftcloud_core::sync::FolderStats;
    let mut h = signed_in();
    h.state_mut().host.progress.folders =
        vec![FolderStats { folder: "Documents".into(), files: 1204, bytes: 2_100_000_000, errors: 2, ..FolderStats::default() }];
    h.get_by_label("FOLDERS").click();
    h.run();
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(207, ROOT_TWO.as_bytes().to_vec())));
    h.run();
    h.get_by_label("2 ERRORS");
    h.get_by_label("1,204 FILES HERE");
}

#[test]
fn a_place_left_out_of_the_sync_is_never_just_up_to_date() {
    let mut h = signed_in();
    h.state_mut().host.progress = SyncProgress { state: SyncState::UpToDate, not_synced: vec!["Private/locked".into()], ..SyncProgress::default() };
    h.run();
    h.get_by_label("Up to date · 1 not synced");
}

#[test]
fn a_file_without_room_in_the_cloud_is_shown_and_tried_again_on_request() {
    let mut h = signed_in();
    h.state_mut().host.progress = SyncProgress { state: SyncState::UpToDate, no_room: vec!["Video/big.mov".into()], ..SyncProgress::default() };
    h.run();
    h.get_by_label("1 not synced");
    h.get_by_label("big.mov doesn't fit in the cloud");
    h.get_by_label("TRY AGAIN").click();
    h.run();
    assert_eq!(h.state().host.synced_now, 1);
}

#[test]
fn an_address_without_a_scheme_tries_https_then_http_and_enter_continues() {
    let mut h = app(ClientSettings::default(), None);
    h.run();
    h.event(egui::Event::Text("cloud.example.org:8090".into()));
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(h.state().host.net.sent_to("https://cloud.example.org:8090/craftcloud/config.json"), 1, "Enter continues");
    h.state_mut().host.net.answer("/craftcloud/config.json", Err("tls handshake failed".into()));
    h.run();
    assert_eq!(h.state().host.net.sent_to("http://cloud.example.org:8090/craftcloud/config.json"), 1, "then plain http");
    h.state_mut().host.net.answer("/craftcloud/config.json", ok(br#"{"version":"0.1.0","web_apps":false,"public_url":"x"}"#));
    h.run();
    h.get_by_label("CraftCloud at cloud.example.org:8090 over http:// (not encrypted)");
    assert_eq!(h.state().settings.server, "http://cloud.example.org:8090");
}
