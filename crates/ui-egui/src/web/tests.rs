use craftcloud_core::ServerUrl;
use craftcloud_core::http::HttpResponse;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{After, Page, WebApp, bytes};
use crate::backend::FakeBackend;

pub(super) fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/oxicloud-0.10.0").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub(super) fn ok(body: &[u8]) -> Result<HttpResponse, String> {
    Ok(HttpResponse::new(200, body.to_vec()))
}

pub(super) const CONFIG: &[u8] = br#"{"version": "0.1.0", "web_apps": false, "public_url": "http://127.0.0.1:8093"}"#;
pub(super) const INITIALIZED: &[u8] = br#"{"initialized":true,"admin_count":1,"registration_allowed":false,"faces_enabled":false}"#;

pub(super) fn app(path: &str, code: Option<&str>) -> Harness<'static, WebApp<FakeBackend>> {
    let server = ServerUrl::parse("http://127.0.0.1:8093").unwrap();
    let app = WebApp::new(&egui::Context::default(), FakeBackend::default(), server, path, code.map(String::from));
    Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_ui_state(|ui, app: &mut WebApp<FakeBackend>| app.ui(ui), app)
}

/// Answers the three start-up requests.
pub(super) fn start(h: &mut Harness<'static, WebApp<FakeBackend>>, status: &[u8], signed_in: bool) {
    let b = &mut h.state_mut().backend;
    b.answer("/api/auth/status", ok(status));
    b.answer("/craftcloud/config.json", ok(CONFIG));
    let me = if signed_in { ok(&fixture("me.json")) } else { Ok(HttpResponse::new(401, Vec::new())) };
    b.answer("/api/auth/me", me);
    h.run();
}

#[test]
fn a_new_server_runs_setup_and_signs_the_owner_in() {
    let mut h = app("/", None);
    start(&mut h, &fixture("auth-status-uninit.json"), false);
    assert_eq!(h.state().page, Page::Setup(1));
    h.get_by_label("START SETUP").click();
    h.run();

    let form = &mut h.state_mut().form;
    (form.name, form.email, form.password, form.password2) =
        ("john".into(), "john@example.org".into(), "correct horse".into(), "correct horse".into());
    h.run();
    h.get_by_label("CREATE ACCOUNT").click();
    h.run();
    let body: serde_json::Value = serde_json::from_slice(&h.state().backend.last_to("/api/setup").body).unwrap();
    assert_eq!((body["username"].as_str(), body["email"].as_str()), (Some("john"), Some("john@example.org")));

    h.state_mut().backend.answer("/api/setup", Ok(HttpResponse::new(201, fixture("setup.json"))));
    h.run();
    h.state_mut().backend.answer("/api/auth/login", ok(br#"{"access_token":"a","refresh_token":"r"}"#));
    h.run();
    h.state_mut().backend.answer("/api/auth/me", ok(&fixture("me.json")));
    h.run();
    assert_eq!(h.state().page, Page::Setup(3));
    assert!(h.state().form.password.is_empty(), "the password is not kept after signing in");
    h.get_by_label("Matches the address you're using now.");
}

#[test]
fn setup_can_go_back_from_the_owner_step() {
    let mut h = app("/", None);
    start(&mut h, &fixture("auth-status-uninit.json"), false);
    h.get_by_label("START SETUP").click();
    h.run();
    assert_eq!(h.state().page, Page::Setup(2));
    h.get_by_label("BACK").click();
    h.run();
    assert_eq!(h.state().page, Page::Setup(1));
}

#[test]
fn setup_shows_the_servers_reason() {
    let mut h = app("/", None);
    start(&mut h, &fixture("auth-status-uninit.json"), false);
    h.state_mut().page = Page::Setup(2);
    let form = &mut h.state_mut().form;
    (form.name, form.email, form.password, form.password2) = ("john".into(), "john@example.org".into(), "short".into(), "short".into());
    h.run();
    h.get_by_label("CREATE ACCOUNT").click();
    h.run();
    h.state_mut().backend.answer("/api/setup", Ok(HttpResponse::new(400, fixture("setup-rejected.json"))));
    h.run();
    h.get_by_label("Password must be at least 8 characters long");
}

#[test]
fn mismatched_passwords_never_reach_the_server() {
    let mut h = app("/", None);
    start(&mut h, &fixture("auth-status-uninit.json"), false);
    h.state_mut().page = Page::Setup(2);
    let form = &mut h.state_mut().form;
    (form.name, form.email, form.password, form.password2) = ("john".into(), "j@example.org".into(), "one-password".into(), "another-one".into());
    h.run();
    h.get_by_label("CREATE ACCOUNT").click();
    h.run();
    assert_eq!(h.state().backend.sent_to("/api/setup"), 0);
    h.get_by_label("The passwords don't match.");
}

#[test]
fn signed_out_visitors_get_the_sign_in_page() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, false);
    assert_eq!(h.state().page, Page::Login(After::Home));
    let form = &mut h.state_mut().form;
    (form.name, form.password) = ("john".into(), "wrong".into());
    h.run();
    h.get_by_label("SIGN IN").click();
    h.run();
    h.state_mut().backend.answer("/api/auth/login", Ok(HttpResponse::new(401, br#"{"message":"Invalid credentials"}"#.to_vec())));
    h.run();
    h.get_by_label("Wrong username or password.");
}

#[test]
fn signed_in_visitors_see_home() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, true);
    assert_eq!(h.state().page, Page::Home);
    h.state_mut().backend.answer("/api/auth/device/devices", ok(&fixture("device-devices.json")));
    h.run();
    h.get_by_label("1 DEVICE");
    h.get_by_label("CraftCloud on Laptop");
}

#[test]
fn the_device_page_signs_in_first_then_asks_to_connect() {
    let mut h = app("/device", Some("KOQY-2239"));
    start(&mut h, INITIALIZED, false);
    assert_eq!(h.state().page, Page::Login(After::Device));
    let form = &mut h.state_mut().form;
    (form.name, form.password) = ("john".into(), "correct horse".into());
    h.run();
    h.get_by_label("SIGN IN").click();
    h.run();
    h.state_mut().backend.answer("/api/auth/login", ok(br#"{"access_token":"a","refresh_token":"r"}"#));
    h.run();
    h.state_mut().backend.answer("/api/auth/me", ok(&fixture("me.json")));
    h.run();
    assert_eq!(h.state().page, Page::Device);
    h.state_mut().backend.answer("verify?code=KOQY%2D2239", ok(&fixture("device-verify-info.json")));
    h.run();
    h.get_by_label("CraftCloud desktop (test)");

    h.get_by_label("APPROVE").click();
    h.run();
    let body: serde_json::Value = serde_json::from_slice(&h.state().backend.last_to("/api/auth/device/verify").body).unwrap();
    assert_eq!((body["user_code"].as_str(), body["action"].as_str()), (Some("KOQY-2239"), Some("approve")));
    h.state_mut().backend.answer("/api/auth/device/verify", ok(br#"{"status":"approved"}"#));
    h.run();
    h.get_by_label("You can close this tab: the client takes it from here and starts syncing.");
    h.get_by_label("GO TO HOME").click();
    h.run();
    assert_eq!(h.state().page, Page::Home);
}

#[test]
fn byte_sizes_read_like_the_design() {
    assert_eq!(bytes(0), "0 B");
    assert_eq!(bytes(3_000), "3.0 KB");
    assert_eq!(bytes(340_000_000), "340 MB");
    assert_eq!(bytes(12_400_000_000), "12.4 GB");
    assert_eq!(bytes(107_374_182_400), "107 GB");
}

const CATALOG: &str = r##"{"apps":[
 {"id":"wordcraft","name":"WordCraft","tagline":"Writing and document design","color":"#3f5fe0","ink":["#8ea2ff","#2a44b8"],"version":"0.18.1","bridge":"native","default":["docx","md"]},
 {"id":"effectcraft","name":"EffectCraft","tagline":"Motion graphics and visual effects","color":"#e0368f","ink":["#ff7ab8","#b0206c"],"version":"0.24.1","bridge":"shim","default":["ecproj"]}]}"##;

fn at_web_apps_step(catalog: Result<HttpResponse, String>) -> Harness<'static, WebApp<FakeBackend>> {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, true);
    h.state_mut().backend.answer("/api/auth/device/devices", ok(b"[]"));
    h.run();
    h.state_mut().open_web_apps_step();
    h.run();
    h.state_mut().backend.answer("/craftcloud/apps.json", catalog);
    h.state_mut()
        .backend
        .answer("/craftcloud/api/settings", ok(br#"{"web_apps":false,"disabled_apps":[],"bundled_apps":["wordcraft","effectcraft"]}"#));
    h.run();
    h
}

#[test]
fn the_slim_image_cannot_switch_web_apps_on() {
    let mut h = at_web_apps_step(Ok(HttpResponse::new(404, Vec::new())));
    h.get_by_label("NEEDS THE -APPS IMAGE");
    h.get_by_label("CONTINUE").click();
    h.run();
    assert_eq!(h.state().page, Page::Setup(5));
    assert_eq!(h.state().backend.sent_to("/craftcloud/api/settings"), 1, "only the read, no write");
}

#[test]
fn web_apps_switch_on_and_one_app_is_left_out() {
    let mut h = at_web_apps_step(ok(CATALOG.as_bytes()));
    assert!(h.query_by_label("WordCraft").is_none(), "no app grid while off");
    h.get_by_label("ON").click();
    h.run();
    h.get_by_label("WordCraft");
    h.get_by_label("EffectCraft").click();
    h.run();
    h.get_by_label("1 of 2 on · click a card to switch it");
    h.get_by_label("SAVE AND CONTINUE").click();
    h.run();
    let req = h.state().backend.last_to("/craftcloud/api/settings");
    assert_eq!(req.method, craftcloud_core::http::Method::Put);
    let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body, serde_json::json!({"web_apps": true, "disabled_apps": ["effectcraft"]}));
    h.state_mut().backend.answer("/craftcloud/api/settings", ok(br#"{"web_apps":true,"disabled_apps":["effectcraft"],"bundled_apps":[]}"#));
    h.run();
    assert_eq!(h.state().page, Page::Setup(5));
}

#[test]
fn clicking_cards_with_the_pointer_switches_exactly_those_apps() {
    let mut h = at_web_apps_step(ok(CATALOG3.as_bytes()));
    h.get_by_label("ON").click();
    h.run();
    for name in ["GridCraft", "DeckCraft"] {
        let center = h.get_by_label(name).rect().center();
        let pos = egui::pos2(center.x, center.y);
        h.event(egui::Event::PointerMoved(pos));
        h.run();
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run();
    }
    let mut off = h.state().web_apps.disabled.clone();
    off.sort();
    assert_eq!(off, ["deckcraft", "gridcraft"]);
}

const CATALOG3: &str = r##"{"apps":[
 {"id":"wordcraft","name":"WordCraft","tagline":"Writing","color":"#3f5fe0","ink":["#8ea2ff","#2a44b8"],"version":"0.18.1","bridge":"native","default":["docx"]},
 {"id":"gridcraft","name":"GridCraft","tagline":"Spreadsheets","color":"#1f9d55","ink":["#4fd68a","#12703a"],"version":"0.19.1","bridge":"native","default":["xlsx"]},
 {"id":"deckcraft","name":"DeckCraft","tagline":"Slides","color":"#f07a1e","ink":["#ffa060","#a84f0a"],"version":"0.18.1","bridge":"native","default":["pptx"]}]}"##;

#[test]
fn too_many_sign_ins_say_how_long_to_wait() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, false);
    let form = &mut h.state_mut().form;
    (form.name, form.password) = ("john".into(), "correct horse".into());
    h.run();
    h.get_by_label("SIGN IN").click();
    h.run();
    // OxiCloud's limiter: a Retry-After header and the same number in the body.
    let limited = HttpResponse::new(429, br#"{"error":"Too many requests","retry_after_secs":60}"#.to_vec()).with_header("Retry-After", "42");
    h.state_mut().backend.answer("/api/auth/login", Ok(limited));
    h.run();
    h.get_by_label("Too many sign-in attempts. Try again in 42 s.");

    // Without the header (a proxy dropped it), the body still says how long.
    h.get_by_label("SIGN IN").click();
    h.run();
    h.state_mut().backend.answer("/api/auth/login", Ok(HttpResponse::new(429, br#"{"error":"Too many requests","retry_after_secs":60}"#.to_vec())));
    h.run();
    h.get_by_label("Too many sign-in attempts. Try again in 60 s.");
}

/// The web frontend never installs the craft-ui runtime (it is the desktop client's, and the wasm build doesn't link
/// it): after the frontend's passes, an install on its context still succeeds. Native only: the craft-ui crates it
/// calls are desktop-target dependencies, so a wasm test runner would not have them.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_web_frontend_does_not_install_the_craft_ui_runtime() {
    let mut h = app("/", None);
    h.run();
    let spec = craft_ui_contract::InstallSpec::new(craft_host::target::ShellInfo::default(), std::sync::Arc::new(craft_window::system::NoWindows));
    assert!(craft_ui_contract::install(&h.ctx, spec).is_ok(), "the web frontend had installed the runtime");
}
