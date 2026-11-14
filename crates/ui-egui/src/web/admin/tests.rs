//! Admin › Users and Admin › Server, driven through the FakeBackend with recorded answers.

use craftcloud_core::http::{HttpResponse, Method};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::super::tests::{INITIALIZED, app, fixture, ok, start};
use super::{Clock, Tab, gb, when};
use crate::backend::FakeBackend;
use crate::web::{Page, WebApp};

type H = Harness<'static, WebApp<FakeBackend>>;

const JANE: &str = "a15eb90c-17d8-4a34-a724-97e000789b56";
const TEST: &str = "1e9ef0fd-7510-41ea-8317-9908214ebfdd";

fn json(req: &craftcloud_core::http::HttpRequest) -> serde_json::Value {
    serde_json::from_slice(&req.body).unwrap()
}

/// Signed in as the owner, on Admin › Users with the three recorded people.
fn users_page() -> H {
    let mut h = app("/admin/users", None);
    start(&mut h, INITIALIZED, true);
    assert_eq!(h.state().page, Page::Admin(Tab::Users));
    h.state_mut().backend.answer("/api/admin/users?limit=500&offset=0", ok(&fixture("admin-users-three.json")));
    h.run();
    h
}

#[test]
fn owners_get_an_admin_entry_and_the_people_table() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, true);
    assert_eq!(h.state().page, Page::Home);
    h.get_by_label("ADMIN").click();
    h.run();
    assert_eq!(h.state().page, Page::Admin(Tab::Users));
    h.get_by_label("LOADING PEOPLE…");
    h.state_mut().backend.answer("/api/admin/users?limit=500&offset=0", ok(&fixture("admin-users-three.json")));
    h.run();
    for email in ["john@example.org", "jane@example.org", "test@example.org"] {
        h.get_by_label(email);
    }
    h.get_by_label("OWNER");
    h.get_by_label("DEACTIVATED");
    h.get_by_label("3 PEOPLE");
    // Deactivated people get ACTIVATE in place of RESET PASSWORD.
    assert_eq!(h.get_all_by_label("RESET PASSWORD").count(), 2);
    h.get_by_label("ACTIVATE");
}

#[test]
fn people_without_admin_rights_see_no_admin_entry() {
    let mut h = app("/admin/users", None);
    let me = String::from_utf8(fixture("me.json")).unwrap().replace("\"role\":\"owner\"", "\"role\":\"user\"");
    let b = &mut h.state_mut().backend;
    b.answer("/api/auth/status", ok(INITIALIZED));
    b.answer("/craftcloud/config.json", ok(super::super::tests::CONFIG));
    b.answer("/api/auth/me", ok(me.as_bytes()));
    h.run();
    assert_eq!(h.state().page, Page::Home, "/admin falls back to home");
    assert!(h.query_by_label("ADMIN").is_none());
    assert_eq!(h.state().backend.sent_to("/api/admin/users?limit=500&offset=0"), 0);
}

#[test]
fn search_narrows_the_table() {
    let mut h = users_page();
    h.state_mut().admin.search = "JAN".into();
    h.run();
    h.get_by_label("jane");
    assert!(h.query_by_label("test").is_none());
    h.state_mut().admin.search = "nobody".into();
    h.run();
    h.get_by_label("No one matches \u{201c}nobody\u{201d}.");
}

#[test]
fn a_new_user_is_created_and_the_list_reloads() {
    let mut h = users_page();
    h.get_by_label("NEW USER").click();
    h.run();
    let Some(super::Sheet::NewUser(f)) = h.state_mut().admin.sheet.as_mut() else { panic!("no new-user sheet") };
    (f.username, f.email, f.password, f.quota_gb) = ("maria".into(), "maria@example.org".into(), "secret-password".into(), "5".into());
    h.run();
    // The role cells: the header's ADMIN entry comes first, the sheet's last.
    h.get_all_by_label("ADMIN").last().unwrap().click();
    h.run();
    h.get_by_label("Admins manage people and the server, like you.");
    h.get_by_label("CREATE").click();
    h.run();
    let req = h.state().backend.last_to("/api/admin/users");
    assert_eq!(req.method, Method::Post);
    let body = json(req);
    assert_eq!(body["username"], "maria");
    assert_eq!(body["quota_bytes"], 5 * 1024 * 1024 * 1024_u64);
    assert_eq!(body["role"], "admin");
    h.state_mut().backend.answer("/api/admin/users", Ok(HttpResponse::new(201, fixture("admin-user-created.json"))));
    h.run();
    assert!(h.state().admin.sheet.is_none(), "the sheet closes");
    h.get_by_label("maria can sign in now");
    assert_eq!(h.state().backend.sent_to("/api/admin/users?limit=500&offset=0"), 2, "reloaded");
}

#[test]
fn new_user_mistakes_are_caught_before_sending() {
    let mut h = users_page();
    h.get_by_label("NEW USER").click();
    h.run();
    let Some(super::Sheet::NewUser(f)) = h.state_mut().admin.sheet.as_mut() else { panic!() };
    (f.username, f.email, f.password) = ("maria".into(), "maria@example.org".into(), "short".into());
    h.run();
    h.get_by_label("CREATE").click();
    h.run();
    h.get_by_label("The password needs at least 8 characters.");
    assert_eq!(h.state().backend.sent.iter().filter(|(_, r)| r.method == Method::Post).count(), 0);
}

#[test]
fn reset_password_shows_a_toast() {
    let mut h = users_page();
    h.state_mut().row_action(super::RowAction::Reset(JANE.into()));
    h.run();
    let Some(super::Sheet::Reset { password, .. }) = h.state_mut().admin.sheet.as_mut() else { panic!("no reset sheet") };
    *password = "a-new-password".into();
    h.run();
    h.get_by_label("SAVE").click();
    h.run();
    let req = h.state().backend.last_to(&format!("/api/admin/users/{JANE}/password"));
    assert_eq!(json(req)["new_password"], "a-new-password");
    h.state_mut().backend.answer("/password", ok(br#"{"message":"Password updated"}"#));
    h.run();
    h.get_by_label("Password changed for jane");
}

#[test]
fn the_menu_changes_role_and_deactivates() {
    let mut h = users_page();
    h.get_by_label("More for jane").click();
    h.run();
    // jane is an admin in the recording.
    h.get_by_label("Make user").click();
    h.run();
    assert_eq!(json(h.state().backend.last_to(&format!("/api/admin/users/{JANE}/role"))), serde_json::json!({"role": "user"}));
    h.state_mut().backend.answer("/role", ok(&fixture("admin-user-role.json")));
    h.run();
    h.get_by_label("jane is a user now");

    h.get_by_label("More for jane").click();
    h.run();
    h.get_by_label("Deactivate").click();
    h.run();
    assert_eq!(json(h.state().backend.last_to(&format!("/api/admin/users/{JANE}/active"))), serde_json::json!({"active": false}));
    h.state_mut().backend.answer("/active", ok(&fixture("admin-user-deactivated.json")));
    h.run();
    h.get_by_label("jane is deactivated");
}

#[test]
fn the_owner_cannot_be_deactivated_from_the_menu() {
    let mut h = users_page();
    h.get_by_label("More for john").click();
    h.run();
    h.get_by_label("Details");
    assert!(h.query_by_label("Deactivate").is_none());
    assert!(h.query_by_label("Make user").is_none() && h.query_by_label("Make admin").is_none());
}

#[test]
fn activate_brings_a_deactivated_person_back() {
    let mut h = users_page();
    h.get_by_label("ACTIVATE").click();
    h.run();
    assert_eq!(json(h.state().backend.last_to(&format!("/api/admin/users/{TEST}/active"))), serde_json::json!({"active": true}));
}

#[test]
fn details_show_sign_ins_and_change_the_quota() {
    let mut h = users_page();
    h.get_by_label("More for jane").click();
    h.run();
    h.get_by_label("Details").click();
    h.run();
    h.state_mut().backend.answer(&format!("/api/admin/sessions?user_id={JANE}"), ok(&fixture("admin-sessions.json")));
    h.state_mut().backend.answer(&format!("/api/admin/users/{JANE}"), ok(&fixture("admin-user.json")));
    h.run();
    h.get_by_label("Web · other client"); // curl, in the recording
    h.get_by_label("None connected.");
    // jane's recorded quota (5 GB decimal) is not a preset: it is listed as it is.
    h.state_mut().row_action(super::RowAction::Quota(JANE.into(), 10 * super::GIB));
    h.run();
    let req = h.state().backend.last_to(&format!("/api/admin/users/{JANE}/quota"));
    assert_eq!(json(req), serde_json::json!({"quota_bytes": 10 * super::GIB}));
}

fn server_page(catalog: Result<HttpResponse, String>) -> H {
    let mut h = app("/admin/server", None);
    start(&mut h, INITIALIZED, true);
    assert_eq!(h.state().page, Page::Admin(Tab::Server));
    let b = &mut h.state_mut().backend;
    b.answer("/craftcloud/apps.json", catalog);
    b.answer("/craftcloud/api/settings", ok(br#"{"web_apps":false,"disabled_apps":[],"bundled_apps":["wordcraft","effectcraft"]}"#));
    b.answer("/api/admin/settings/oidc", ok(&fixture("oidc-settings.json")));
    // Tall enough to show the single sign-on panel under the app grid.
    h.set_size(egui::vec2(1280.0, 2000.0));
    h.run();
    h
}

const CATALOG: &str = r##"{"apps":[
 {"id":"wordcraft","name":"WordCraft","tagline":"Writing","color":"#3f5fe0","ink":["#8ea2ff","#2a44b8"],"version":"0.18.1","bridge":"native","default":["docx"]},
 {"id":"effectcraft","name":"EffectCraft","tagline":"Effects","color":"#e0368f","ink":["#ff7ab8","#b0206c"],"version":"0.24.1","bridge":"shim","default":["ecproj"]}]}"##;

#[test]
fn server_web_apps_save_without_leaving_admin() {
    let mut h = server_page(ok(CATALOG.as_bytes()));
    h.get_by_label("OXICLOUD: 0.10.0 · unmodified");
    h.get_by_label("CRAFTCLOUD: 0.1.0 · -apps image");
    h.get_all_by_label("ON").next().unwrap().click();
    h.run();
    h.get_by_label("EffectCraft").click();
    h.run();
    h.get_by_label("1 of 2 on");
    h.get_by_label("SAVE").click();
    h.run();
    let req = h.state().backend.last_to("/craftcloud/api/settings");
    assert_eq!(req.method, Method::Put);
    assert_eq!(json(req), serde_json::json!({"web_apps": true, "disabled_apps": ["effectcraft"]}));
    h.state_mut().backend.answer("/craftcloud/api/settings", ok(br#"{"web_apps":true,"disabled_apps":["effectcraft"],"bundled_apps":[]}"#));
    h.run();
    assert_eq!(h.state().page, Page::Admin(Tab::Server), "saving here doesn't jump into setup");
    h.get_by_label("Web apps saved");
}

#[test]
fn the_slim_image_shows_the_badge_and_cannot_switch_on() {
    let h = server_page(Ok(HttpResponse::new(404, Vec::new())));
    h.get_by_label("NEEDS THE -APPS IMAGE");
    h.get_by_label("CRAFTCLOUD: 0.1.0 · slim image");
    assert!(!h.state().web_apps.on);
}

#[test]
fn single_sign_on_settings_round_trip() {
    let mut h = server_page(ok(CATALOG.as_bytes()));
    assert!(!h.state().admin.sso.enabled);
    h.get_by_label("http://127.0.0.1:8097/api/auth/oidc/callback"); // the redirect URI to register
    let f = &mut h.state_mut().admin.sso;
    (f.provider_name, f.issuer_url, f.client_id, f.client_secret, f.admin_groups) =
        ("Example ID".into(), "https://id.example.org".into(), "craftcloud".into(), "s3cret".into(), "craftcloud-admins".into());
    f.enabled = true;
    h.run();
    h.get_by_label("SAVE SINGLE SIGN-ON").click();
    h.run();
    let req = h.state().backend.last_to("/api/admin/settings/oidc");
    assert_eq!(req.method, Method::Put);
    let body = json(req);
    assert_eq!((body["enabled"].as_bool(), body["issuer_url"].as_str()), (Some(true), Some("https://id.example.org")));
    assert_eq!((body["client_secret"].as_str(), body["provider_name"].as_str()), (Some("s3cret"), Some("Example ID")));
    assert!(h.state().admin.sso.client_secret.is_empty(), "the typed secret isn't kept");
    h.state_mut().backend.answer("/api/admin/settings/oidc", ok(br#"{"message":"OIDC settings saved and applied successfully"}"#));
    h.run();
    h.state_mut().backend.answer("/api/admin/settings/oidc", ok(&fixture("oidc-settings-saved.json")));
    h.run();
    h.get_by_label("ON · EXAMPLE ID");
    h.get_by_label("Saved. Single sign-on uses the new settings now.");
}

#[test]
fn single_sign_on_needs_the_basics_before_it_is_switched_on() {
    let mut h = server_page(ok(CATALOG.as_bytes()));
    h.state_mut().admin.sso.enabled = true;
    h.run();
    h.get_by_label("SAVE SINGLE SIGN-ON").click();
    h.run();
    h.get_by_label("To switch it on, fill in the issuer URL, the client ID and the client secret.");
    assert_eq!(h.state().backend.sent.iter().filter(|(_, r)| r.method == Method::Put).count(), 0);
}

#[test]
fn environment_settings_are_read_only() {
    let mut h = app("/admin/server", None);
    start(&mut h, INITIALIZED, true);
    let settings =
        String::from_utf8(fixture("oidc-settings-saved.json")).unwrap().replace("\"env_overrides\": []", "\"env_overrides\": [\"issuer_url\"]");
    let b = &mut h.state_mut().backend;
    b.answer("/craftcloud/apps.json", ok(CATALOG.as_bytes()));
    b.answer("/craftcloud/api/settings", ok(br#"{"web_apps":false,"disabled_apps":[]}"#));
    b.answer("/api/admin/settings/oidc", ok(settings.as_bytes()));
    h.run();
    h.get_by_label("Set by OXICLOUD_OIDC_ISSUER_URL.");
}

#[test]
fn password_sign_in_goes_off_only_with_the_provider_on() {
    let mut h = server_page(ok(CATALOG.as_bytes()));
    h.state_mut().admin.sso.password_login = false;
    h.run();
    h.get_by_label("SAVE SINGLE SIGN-ON").click();
    h.run();
    h.get_by_label("Password sign-in can only be switched off while sign-in with the provider is on.");
    assert_eq!(h.state().backend.sent.iter().filter(|(_, r)| r.method == Method::Put).count(), 0);
    let f = &mut h.state_mut().admin.sso;
    (f.issuer_url, f.client_id, f.client_secret) = ("https://id.example.org".into(), "craftcloud".into(), "s3cret".into());
    f.enabled = true;
    h.run();
    h.get_by_label("SAVE SINGLE SIGN-ON").click();
    h.run();
    let body = json(h.state().backend.last_to("/api/admin/settings/oidc"));
    assert_eq!(body["disable_password_login"].as_bool(), Some(true));
}

#[test]
fn test_connection_shows_what_the_server_saw() {
    let mut h = server_page(ok(CATALOG.as_bytes()));
    h.state_mut().admin.sso.issuer_url = "https://id.example.org/realms/test".into();
    h.run();
    h.get_by_label("TEST CONNECTION").click();
    h.run();
    let req = h.state().backend.last_to("/api/admin/settings/oidc/test");
    assert_eq!((req.method, json(req)), (Method::Post, serde_json::json!({"issuer_url": "https://id.example.org/realms/test"})));
    h.state_mut().backend.answer("/api/admin/settings/oidc/test", ok(&fixture("oidc-test-unreachable.json")));
    h.run();
    h.get_by_label_contains("The server can't use this issuer URL: Failed to fetch OIDC discovery");
    h.get_by_label("TEST CONNECTION").click();
    h.run();
    h.state_mut().backend.answer("/api/admin/settings/oidc/test", ok(&fixture("oidc-test-ok.json")));
    h.run();
    h.get_by_label("The server reached the provider (https://id.example.org/realms/test).");
}

#[test]
fn dates_read_like_the_design() {
    assert_eq!(when("2026-11-13T21:54:22.130637Z", None, false), "13 Nov 2026");
    assert_eq!(when("2026-11-13T21:54:22Z", None, true), "13 Nov 2026, 21:54 UTC");
    // 2026-11-13 22:30 UTC, one hour east.
    let clock = Some(Clock { now_unix: 1_794_609_000, utc_offset_minutes: 60 });
    assert_eq!(when("2026-11-13T21:54:22Z", clock, false), "Today 22:54");
    assert_eq!(when("2026-11-12T20:00:00+00:00", clock, false), "Yesterday 21:00");
    assert_eq!(when("2026-11-02T10:00:00Z", clock, false), "2 Nov 2026");
    assert_eq!(when("2026-11-02T10:00:00Z", clock, true), "2 Nov 2026, 11:00");
    assert_eq!(when("not a date", None, false), "not a date");
    assert_eq!(gb(0), "0.0");
    assert_eq!(gb(1024 * 1024 * 1024), "1");
    assert_eq!(gb(13_300_000_000), "12.4");
}
