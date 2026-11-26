use craftcloud_core::api::auth::Tokens;
use craftcloud_core::http::{HttpRequest, HttpResponse};
use craftcloud_core::sync::{Appearance, ClientSettings, ConflictCopy, Keep, SyncPlan, SyncProgress, SyncState};
use craftcloud_core::update::NewVersion;
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
    /// What the update check found.
    pub(super) new_version: Option<NewVersion>,
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
    fn new_version(&self) -> Option<NewVersion> {
        self.new_version.clone()
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
fn who_is_signed_in_is_asked_again_after_a_wait_that_grows() {
    // The main window asks who is signed in once; an offline window used to ask again on the
    // very next frame after every failure, and redraw for each answer.
    let mut h = with_state(SyncState::UpToDate);
    let asked = |h: &H| h.state().host.net.sent_to("/api/auth/me");
    assert_eq!(asked(&h), 1);
    h.state_mut().host.net.answer("/api/auth/me", Err("Can't connect to the server (refused).".into()));
    h.run();
    h.run();
    assert_eq!(asked(&h), 1, "not asked again at once");
    h.state_mut().host.net.clock = 4.9;
    h.run();
    assert_eq!(asked(&h), 1, "not asked again before 5 s");
    h.state_mut().host.net.clock = 5.0;
    h.run();
    assert_eq!(asked(&h), 2, "asked again after 5 s");

    // A refused answer counts as a failure too, and the second wait is twice as long.
    h.state_mut().host.net.answer("/api/auth/me", Ok(HttpResponse::new(502, Vec::new())));
    h.run();
    h.state_mut().host.net.clock = 10.0;
    h.run();
    assert_eq!(asked(&h), 2, "the second wait is longer than the first");
    h.state_mut().host.net.clock = 15.0;
    h.run();
    assert_eq!(asked(&h), 3, "asked again after 10 s");

    h.state_mut().host.net.answer("/api/auth/me", ok(&fixture("me.json")));
    h.run();
    assert_eq!(h.state().me.as_ref().map(|m| m.user.username.as_str()), Some("john"));
    assert!(h.state().error.is_none(), "who is signed in is a detail: no error shown");
}

/// #84: a 200 that names nobody (an empty answer from a proxy, say) is no answer: who is signed
/// in stays unknown, no error is shown, and it is asked again after the wait.
#[test]
fn an_answer_naming_nobody_is_asked_again() {
    let mut h = with_state(SyncState::UpToDate);
    let asked = |h: &H| h.state().host.net.sent_to("/api/auth/me");
    h.state_mut().host.net.answer("/api/auth/me", ok(b"{}"));
    h.run();
    assert!(h.state().me.is_none());
    assert!(h.state().error.is_none(), "who is signed in is a detail: no error shown");
    h.state_mut().host.net.clock = 5.0;
    h.run();
    assert_eq!(asked(&h), 2, "asked again after the wait");
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
    // The sign-in ends on the server too: the logout names the session by its refresh token
    // (without it OxiCloud answered 401 and the session stayed).
    let logout = h.state().host.net.last_to("/api/auth/logout").clone();
    let body: serde_json::Value = serde_json::from_slice(&logout.body).unwrap();
    assert_eq!(body["refresh_token"], "r");
    assert_eq!(logout.header_value("Authorization"), Some("Bearer a"));
    h.state_mut().host.net.answer("/api/auth/logout", Ok(HttpResponse::new(200, b"{}".to_vec())));
    h.run();
    assert!(h.state().note.is_none());
}

/// The next repaint the window asked for.
fn repaint_delay(h: &H) -> std::time::Duration {
    h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay
}

#[test]
fn an_idle_window_repaints_rarely_and_a_busy_one_often() {
    // #3: an idle window drew 4 frames a second for ever, on the CPU with a software renderer.
    let mut h = with_state(SyncState::UpToDate);
    assert!(h.state().net.is_busy(), "who is signed in is asked first");
    assert!(repaint_delay(&h) <= std::time::Duration::from_millis(250), "a request in flight is looked for often");

    h.state_mut().host.net.answer("/api/auth/me", ok(&fixture("me.json")));
    h.run();
    assert!(!h.state().net.is_busy());
    h.get_by_label("Up to date");
    let idle = repaint_delay(&h);
    assert!(idle >= std::time::Duration::from_secs(60), "idle, the window repaints once a minute at most, not every {idle:?}");
    assert!(idle < std::time::Duration::MAX, "\"Synced 2 min ago\" still moves on");
}

/// Signs out from Settings (the sign-in is over here at once, the logout still under way).
fn signed_out_from_settings() -> H {
    let mut h = with_state(SyncState::UpToDate);
    h.get_by_label("SETTINGS").click();
    h.run();
    h.get_by_label("SIGN OUT").click();
    h.run();
    assert!(h.state().host.signed_out);
    h
}

#[test]
fn a_logout_the_server_refuses_leaves_a_note() {
    let mut h = signed_out_from_settings();
    h.state_mut().host.net.answer("/api/auth/logout", Ok(HttpResponse::new(500, Vec::new())));
    h.run();
    assert!(matches!(h.state().screen, Screen::Connect(_)));
    h.get_by_label(super::LOGOUT_NOTE);

    let mut h = signed_out_from_settings();
    h.state_mut().host.net.answer("/api/auth/logout", Err("Can't connect to the server (refused).".into()));
    h.run();
    h.get_by_label(super::LOGOUT_NOTE);
    assert!(h.state().error.is_none(), "signing out worked here: no error");
}

#[test]
fn a_logout_with_a_run_out_access_token_refreshes_once() {
    let mut h = signed_out_from_settings();
    h.state_mut().host.net.answer("/api/auth/logout", Ok(HttpResponse::new(401, Vec::new())));
    h.run();
    let refresh: serde_json::Value = serde_json::from_slice(&h.state().host.net.last_to("/api/auth/refresh").body).unwrap();
    assert_eq!(refresh["refresh_token"], "r");
    h.state_mut().host.net.answer("/api/auth/refresh", Ok(HttpResponse::new(200, br#"{"access_token":"a2","refresh_token":"r2"}"#.to_vec())));
    h.run();
    // Again, with the new refresh token: the spent one names no session.
    let again: serde_json::Value = serde_json::from_slice(&h.state().host.net.last_to("/api/auth/logout").body).unwrap();
    assert_eq!(again["refresh_token"], "r2");
    h.state_mut().host.net.answer("/api/auth/logout", Ok(HttpResponse::new(200, b"{}".to_vec())));
    h.run();
    assert!(h.state().note.is_none());
    assert_eq!(h.state().host.net.sent_to("/api/auth/logout"), 2);
}

#[test]
fn a_session_the_server_ended_is_not_logged_out_again() {
    // The Activity tab's "Sign in" after the server ended the sign-in: nothing to end there.
    let mut h = with_state(SyncState::SignedOut);
    h.get_by_label("SIGN IN AGAIN").click();
    h.run();
    assert!(h.state().host.signed_out);
    assert_eq!(h.state().host.net.sent_to("/api/auth/logout"), 0);

    // Signing out to choose another folder (the folder is missing): a live sign-in, ended there.
    let mut h = with_state(SyncState::FolderMissing);
    h.get_by_label("SIGN OUT").click();
    h.run();
    assert!(h.state().host.signed_out);
    assert_eq!(h.state().host.net.sent_to("/api/auth/logout"), 1);
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
    h.get_by_label("Syncing: PAUSED").click();
    h.run();
    assert_eq!(h.state().host.paused, Some(true));
    h.get_by_label("Appearance: LIGHT").click();
    h.run();
    assert_eq!(h.state().host.prefs.as_ref().map(|s| s.appearance), Some(Appearance::Light));
    h.get_by_label("DONE").click();
    h.run();
    assert_eq!(h.state().overlay, None);
}

/// Settings › Theme lists CraftCloud's own look and the seven colour themes; picking one saves
/// it, draws with its colours at once, and picking CraftCloud's own look brings that back.
#[test]
fn settings_pick_a_colour_theme() {
    let mut h = with_state(SyncState::UpToDate);
    h.get_by_label("SETTINGS").click();
    h.run();
    assert_eq!(crate::theme::Tokens::get(&h.ctx), crate::theme::Tokens::DARK);
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "CraftCloud").click();
    h.run();
    for name in ["Catppuccin", "Dracula (dark only)", "Gruvbox", "One Dark (dark only)", "Solarized Light (light only)", "Tokyo Night"] {
        h.get_by_label(name);
    }
    h.get_by_label("Nord (dark only)").click();
    h.run();
    assert_eq!(h.state().host.prefs.as_ref().map(|s| s.theme.as_str()), Some("craft.nord"));
    let nord = crate::themes::Palette::for_id("craft.nord");
    assert_eq!(crate::theme::Tokens::get(&h.ctx), nord.dark);
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Nord (dark only)").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::MenuItem, "CraftCloud").click();
    h.run();
    assert_eq!(h.state().host.prefs.as_ref().map(|s| s.theme.as_str()), Some(""));
    assert_eq!(crate::theme::Tokens::get(&h.ctx), crate::theme::Tokens::DARK);
}

#[test]
fn checking_for_updates_is_switched_in_settings() {
    let mut h = with_state(SyncState::UpToDate);
    h.get_by_label("SETTINGS").click();
    h.run();
    h.get_by_label("Check for updates: OFF").click();
    h.run();
    assert_eq!(h.state().host.prefs.as_ref().map(|s| s.check_for_updates), Some(false));
    h.get_by_label("Check for updates: ON").click();
    h.run();
    assert_eq!(h.state().host.prefs.as_ref().map(|s| s.check_for_updates), Some(true));
}

pub(super) const RELEASE_PAGE: &str = "https://github.com/jonasbove/craftcloud/releases/tag/v9.9.0";

fn with_new_version(package_managed: bool) -> H {
    let mut h = with_state(SyncState::UpToDate);
    h.state_mut().host.new_version = Some(NewVersion { version: "9.9.0".into(), url: RELEASE_PAGE.into(), package_managed });
    h.get_by_label("SETTINGS").click();
    h.run();
    h
}

#[test]
fn about_says_when_a_new_version_is_out() {
    let mut h = with_new_version(false);
    h.get_by_label("CraftCloud 9.9.0 is out.");
    h.get_by_label("Download it from the release page.");
    h.get_by_label("WHAT'S NEW").click();
    h.run();
    assert_eq!(h.state().host.net.opened.last().map(String::as_str), Some(RELEASE_PAGE));
}

#[test]
fn a_package_managers_copy_is_told_to_update_there() {
    let h = with_new_version(true);
    h.get_by_label("CraftCloud 9.9.0 is out.");
    h.get_by_label("Update it with your package manager.");
    assert!(h.query_by_label("Download it from the release page.").is_none());
}

#[test]
fn no_new_version_is_shown_while_checking_is_off() {
    let mut h = with_new_version(false);
    h.get_by_label("Check for updates: OFF").click();
    h.run();
    assert!(h.query_by_label_contains("is out").is_none());
}

#[test]
fn no_new_version_line_without_a_find() {
    let mut h = with_state(SyncState::UpToDate);
    h.get_by_label("SETTINGS").click();
    h.run();
    assert!(h.query_by_label_contains("is out").is_none());
}

#[test]
fn a_paused_sync_is_made_active_from_settings() {
    let mut h = with_state(SyncState::Paused);
    h.get_by_label("SETTINGS").click();
    h.run();
    h.get_by_label("Syncing: ACTIVE").click();
    h.run();
    assert_eq!(h.state().host.paused, Some(false));
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

/// Starting the client as `main` does: the settings now, the keychain's answer to come.
fn starting(settings: ClientSettings) -> H {
    let app = DesktopApp::waiting_for_keychain(&egui::Context::default(), FakeDesktop::default(), settings);
    Harness::builder().with_size(egui::vec2(960.0, 640.0)).build_ui_state(|ui, app: &mut DesktopApp<FakeDesktop>| app.ui(ui), app)
}

/// What `craftcloud login` leaves: the server in the settings, the tokens in the keychain.
fn after_cli_login() -> ClientSettings {
    ClientSettings { server: "http://127.0.0.1:8093".into(), ..ClientSettings::default() }
}

fn tokens() -> Tokens {
    Tokens { access_token: "a".into(), refresh_token: "r".into() }
}

#[test]
fn a_sign_in_from_the_command_line_goes_on_to_choosing_folders() {
    let mut h = starting(after_cli_login());
    assert!(h.state().wants_keychain(), "the keychain is asked, not the Connect sheet");
    h.run();
    h.state_mut().keychain_answered(Some(tokens()));
    h.run();
    assert_eq!(h.state().host.net.last_to("/webdav/").header_value("Authorization"), Some("Bearer a"));
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(207, ROOT.as_bytes().to_vec())));
    h.run();
    h.get_by_label("Choose what to sync");
    h.get_by_label("Documents");
    h.get_by_label("START SYNCING").click();
    h.run();
    let (settings, saved) = h.state().host.saved.clone().expect("session saved");
    assert_eq!((settings.local_folder.as_str(), saved), ("/home/john/CraftCloud", tokens()));
    assert_eq!(h.state().host.started.len(), 1);
    assert!(matches!(h.state().screen, Screen::Main(Tab::Activity)));
}

#[test]
fn a_command_line_sign_in_an_hour_old_is_refreshed_once() {
    let mut h = starting(after_cli_login());
    h.run();
    h.state_mut().keychain_answered(Some(tokens()));
    h.run();
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(401, Vec::new())));
    h.run();
    let refresh: serde_json::Value = serde_json::from_slice(&h.state().host.net.last_to("/api/auth/refresh").body).unwrap();
    assert_eq!(refresh["refresh_token"], "r");
    h.state_mut().host.net.answer("/api/auth/refresh", ok(br#"{"access_token":"a2","refresh_token":"r2"}"#));
    h.run();
    let fresh = Tokens { access_token: "a2".into(), refresh_token: "r2".into() };
    assert_eq!(h.state().host.saved.clone().map(|(_, t)| t), Some(fresh), "the new refresh token is kept");
    assert_eq!(h.state().host.net.last_to("/webdav/").header_value("Authorization"), Some("Bearer a2"));
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(207, ROOT.as_bytes().to_vec())));
    h.run();
    h.get_by_label("Choose what to sync");

    // Run out altogether (revoked on the server): back to the Connect sheet, saying so.
    let mut h = starting(after_cli_login());
    h.run();
    h.state_mut().keychain_answered(Some(tokens()));
    h.run();
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(401, Vec::new())));
    h.run();
    h.state_mut().host.net.answer("/api/auth/refresh", Ok(HttpResponse::new(401, Vec::new())));
    h.run();
    assert!(matches!(h.state().screen, Screen::Connect(_)));
    h.get_by_label("The saved sign-in has run out. Connect again.");
    // The server not answering: the same sheet, with the reason.
    let mut h = starting(after_cli_login());
    h.run();
    h.state_mut().keychain_answered(Some(tokens()));
    h.run();
    h.state_mut().host.net.answer("/webdav/", Err("connection refused".into()));
    h.run();
    assert!(matches!(h.state().screen, Screen::Connect(_)), "never stuck on the keychain sheet");
}

#[test]
fn a_saved_server_without_a_sign_in_asks_to_connect_without_alarm() {
    // A server typed on the Connect sheet, or left after signing out: the keychain has nothing.
    let mut h = starting(after_cli_login());
    h.run();
    h.state_mut().keychain_answered(None);
    h.run();
    assert!(matches!(h.state().screen, Screen::Connect(_)));
    assert!(h.query_by_label_contains("isn't in the keychain").is_none(), "nothing was lost");
    // Signed in here before, folder and all: the sign-in went missing, which is news.
    let mut h = starting(signed_in_settings());
    h.run();
    h.state_mut().keychain_answered(None);
    h.run();
    h.get_by_label("The saved sign-in isn't in the keychain any more. Connect again.");
}

#[test]
fn a_running_client_takes_a_sign_in_made_on_the_command_line() {
    let mut h = starting(ClientSettings::default());
    h.run();
    assert!(matches!(h.state().screen, Screen::Connect(_)));
    assert!(h.state_mut().signed_in_elsewhere(after_cli_login()));
    assert!(h.state().wants_keychain());
    h.run();
    h.state_mut().keychain_answered(Some(tokens()));
    h.run();
    h.state_mut().host.net.answer("/webdav/", Ok(HttpResponse::new(207, ROOT.as_bytes().to_vec())));
    h.run();
    h.get_by_label("Choose what to sync");
    // Choosing folders, or signed in and syncing: left as it is.
    assert!(!h.state_mut().signed_in_elsewhere(after_cli_login()));
    let mut h = signed_in();
    h.run();
    assert!(!h.state_mut().signed_in_elsewhere(after_cli_login()));
    assert!(matches!(h.state().screen, Screen::Main(_)));
    // No server in the settings: nothing to take.
    let mut h = starting(ClientSettings::default());
    assert!(!h.state_mut().signed_in_elsewhere(ClientSettings::default()));
}

/// Under download only (docs/WEBDAV-STORAGE.md § 11) the header, the tray's words and the
/// headline say so, so nobody takes it for a two-way sync; a two-way sync reads as before.
#[test]
fn download_only_is_said_beside_the_state() {
    use craftcloud_core::storage::Mode;
    let mut h = signed_in();
    h.state_mut().host.progress = SyncProgress { state: SyncState::UpToDate, mode: Some(Mode::DownloadOnly), ..SyncProgress::default() };
    h.run();
    h.get_by_label("Up to date · Download only");
    h.get_by_label_contains("downloaded");
    assert_eq!(super::status_text(&h.state().host.progress), "Up to date · Download only");
    let two_way = SyncProgress { state: SyncState::UpToDate, mode: Some(Mode::TwoWay), ..SyncProgress::default() };
    assert_eq!(super::status_text(&two_way), "Up to date");
    let unknown = SyncProgress { state: SyncState::UpToDate, ..SyncProgress::default() };
    assert_eq!(super::status_text(&unknown), "Up to date");
    let failed = SyncProgress { state: SyncState::Failed("x".into()), mode: Some(Mode::DownloadOnly), ..SyncProgress::default() };
    assert_eq!(super::status_text(&failed), "Error", "a stopped sync says what stopped it");
}

/// Settings naming a kind of server this build won't sync with: the host refuses, the window
/// opens on the Connect sheet with the message, and no sign-in is started or taken.
#[test]
fn a_refused_kind_of_server_opens_the_window_with_the_message() {
    let mut h = app(signed_in_settings(), None);
    h.state_mut().screen = Screen::Keychain;
    h.state_mut().refuse("This build syncs only with a CraftCloud server; set CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 to try other servers".into());
    h.run();
    assert!(matches!(h.state().screen, Screen::Connect(_)));
    h.get_by_label_contains("CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1");
    assert!(!h.state().wants_keychain(), "a keychain answer that arrives late changes nothing");
    h.state_mut().keychain_answered(Some(Tokens { access_token: "a".into(), refresh_token: "r".into() }));
    h.run();
    assert!(matches!(h.state().screen, Screen::Connect(_)) && h.state().host.started.is_empty(), "no sync was started");
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
fn a_folder_move_that_keeps_failing_warns_from_the_third_try_and_is_tried_again_on_request() {
    use craftcloud_core::sync::WaitingMove;
    let waiting = |attempts| SyncProgress {
        state: SyncState::UpToDate,
        waiting_moves: vec![WaitingMove { from: "Work/Proj".into(), to: "Work/Proj2".into(), attempts }],
        ..SyncProgress::default()
    };
    // Two tries: no warning yet (the activity list has said it).
    let mut h = signed_in();
    h.state_mut().host.progress = waiting(2);
    h.run();
    h.get_by_label("Up to date");
    assert!(h.query_by_label("Proj can't be moved in the cloud yet").is_none());
    // From the third on: the status line and a panel say so, and "Try again" asks at once.
    h.state_mut().host.progress = waiting(3);
    h.run();
    h.get_by_label("A folder move is waiting");
    h.get_by_label("Proj can't be moved in the cloud yet");
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
