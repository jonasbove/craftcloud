//! "Your share links" (#19) against a scripted server: five links as in the design frame (three
//! active, one expired, one whose file is gone), at 2026-11-14 13:32 UTC seen at UTC+1.

use craftcloud_core::ServerUrl;
use craftcloud_core::http::{HttpResponse, Method};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::{Filter, times};
use crate::backend::{FakeBackend, WallClock};
use crate::web::{Page, WebApp};

type App = Harness<'static, WebApp<FakeBackend>>;

const CONFIG: &[u8] = br#"{"version": "0.1.0", "web_apps": false, "public_url": "https://cloud.example.org", "enabled_apps": []}"#;
const INITIALIZED: &[u8] = br#"{"initialized":true,"admin_count":1,"registration_allowed":false,"faces_enabled":false}"#;
const LIST: &str = "/api/shares?page=1&per_page=100";

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/oxicloud-0.10.0").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ok(body: impl Into<Vec<u8>>) -> Result<HttpResponse, String> {
    Ok(HttpResponse::new(200, body.into()))
}

#[allow(clippy::too_many_arguments)]
fn link(id: &str, item: &str, kind: &str, name: &str, password: bool, expires: Option<&str>, made: &str, opened: u64) -> Value {
    json!({"id": id, "item_id": item, "item_name": name, "item_type": kind, "token": format!("tok-{id}"),
        "url": format!("https://cloud.example.org/s/tok-{id}"), "has_password": password, "expires_at": expires,
        "created_at": made, "access_count": opened})
}

/// The design frame's five links.
fn five() -> Value {
    json!({"items": [
        link("s1", "f1", "file", "Q3 report.docx", true, Some("2026-11-21T13:32:00Z"), "2026-11-14T13:32:00Z", 3),
        link("s2", "d1", "folder", "Team", false, None, "2026-11-10T09:00:00Z", 12),
        link("s3", "f3", "file", "Board pitch.pptx", true, Some("2026-12-14T08:05:00Z"), "2026-11-14T08:05:00Z", 0),
        link("s4", "f4", "file", "Poster.psd", false, None, "2026-11-03T10:00:00Z", 7),
        link("s5", "f5", "file", "Budget 2026.xlsx", false, Some("2026-11-02T12:00:00Z"), "2026-10-26T10:00:00Z", 5),
    ], "pagination": {"page": 1, "has_next": false}})
}

fn file_at(id: &str, path: &str) -> Result<HttpResponse, String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    ok(json!({"id": id, "name": name, "path": format!("/Personal/{path}"), "size": 1000}).to_string())
}

/// Signed in at `path`, the clock set.
fn signed_in(path: &str) -> App {
    let server = ServerUrl::parse("https://cloud.example.org").unwrap();
    let mut backend = FakeBackend::default();
    backend.wall = Some(WallClock { unix: 1_794_663_120.0, utc_offset_minutes: 60 });
    let app = WebApp::new(&egui::Context::default(), backend, server, path, None);
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(|ui, app: &mut WebApp<FakeBackend>| app.ui(ui), app);
    let b = &mut h.state_mut().backend;
    b.answer("/api/auth/status", ok(INITIALIZED));
    b.answer("/craftcloud/config.json", ok(CONFIG));
    b.answer("/api/auth/me", ok(fixture("me.json")));
    h.run();
    h
}

/// Answers the list and every lookup: Poster.psd is in the trash (404).
fn answer_five(h: &mut App) {
    h.state_mut().backend.answer(LIST, ok(five().to_string()));
    h.run();
    let b = &mut h.state_mut().backend;
    b.answer("/api/files/f1?metadata=true", file_at("f1", "Documents/Reports/Q3 report.docx"));
    b.answer("/api/folders/d1", ok(json!({"id": "d1", "name": "Team", "path": "/Personal/Projects/Team"}).to_string()));
    b.answer("/api/files/f3?metadata=true", file_at("f3", "Clients/North/Board pitch.pptx"));
    b.answer("/api/files/f4?metadata=true", Ok(HttpResponse::new(404, br#"{"error":"Not found"}"#.to_vec())));
    b.answer("/api/files/f5?metadata=true", file_at("f5", "Finance/Budget 2026.xlsx"));
    h.run();
}

fn links_page() -> App {
    let mut h = signed_in("/links");
    assert_eq!(h.state().page, Page::Links);
    answer_five(&mut h);
    h
}

fn names(h: &App) -> Vec<String> {
    let links = &h.state().links;
    links.rows().iter().map(|s| links.name(s)).collect()
}

fn disabled(h: &App, label: &str, nth: usize) -> bool {
    h.query_all_by_label(label).nth(nth).unwrap_or_else(|| panic!("no {label} #{nth}")).accesskit_node().is_disabled()
}

#[test]
fn the_list_shows_where_each_item_is_now_and_what_each_link_does() {
    let h = links_page();
    assert_eq!(names(&h), ["Q3 report.docx", "Board pitch.pptx", "Team", "Poster.psd", "Budget 2026.xlsx"], "newest first");
    h.get_by_label("ALL 5");
    h.get_by_label("ACTIVE 3");
    h.get_by_label("EXPIRED 1");
    h.get_by_label("NO LONGER THERE 1");
    h.get_by_label("5 LINKS");
    h.get_by_label("2 WITH A PASSWORD");
    h.get_by_label("OPENED 27 TIMES IN ALL");
    // Where each one is now, by its id; the expiry with what is left; the password; the opens.
    h.get_by_label("My files / Documents / Reports");
    h.get_by_label("My files / Projects");
    h.get_by_label("21 Nov 2026");
    h.get_by_label("in 7 days");
    h.get_by_label("EXPIRED");
    h.get_by_label("2 Nov 2026");
    assert_eq!(h.query_all_by_label("Yes").count(), 2);
    h.get_by_label("12 times");
    h.get_by_label("Not yet");
    h.get_by_label("Today 14:32");
    // COPY works for the active links only (the rows are in the order above).
    assert_eq!(h.query_all_by_label("COPY").count(), 5);
    assert!(!disabled(&h, "COPY", 0) && !disabled(&h, "COPY", 1) && !disabled(&h, "COPY", 2));
    assert!(disabled(&h, "COPY", 3), "its item is gone");
    assert!(disabled(&h, "COPY", 4), "expired");
    assert_eq!(h.query_all_by_label("REVOKE").filter(|n| !n.accesskit_node().is_disabled()).count(), 5);
}

#[test]
fn a_gone_item_shows_the_name_it_was_shared_with() {
    let h = links_page();
    h.get_by_label("Poster.psd");
    h.get_by_label("NO LONGER THERE");
    h.get_by_label("Shared as Poster.psd · moved to the trash or deleted");
}

#[test]
fn a_failed_lookup_is_never_called_gone() {
    let mut h = signed_in("/links");
    h.state_mut().backend.answer(LIST, ok(five().to_string()));
    h.run();
    h.state_mut().backend.answer("/api/files/f4?metadata=true", Ok(HttpResponse::new(500, b"{}".to_vec())));
    h.run();
    h.get_by_label("Where it is now couldn't be looked up");
    assert!(h.query_by_label("NO LONGER THERE").is_none());
    assert!(h.query_by_label("NO LONGER THERE 0").is_none(), "the cell shows only when something is gone");
    h.get_by_label("ACTIVE 4");
}

#[test]
fn each_filter_shows_its_links() {
    let mut h = links_page();
    for (cell, filter, want) in [
        ("ACTIVE 3", Filter::Active, vec!["Q3 report.docx", "Board pitch.pptx", "Team"]),
        ("EXPIRED 1", Filter::Expired, vec!["Budget 2026.xlsx"]),
        ("NO LONGER THERE 1", Filter::Gone, vec!["Poster.psd"]),
        ("ALL 5", Filter::All, vec!["Q3 report.docx", "Board pitch.pptx", "Team", "Poster.psd", "Budget 2026.xlsx"]),
    ] {
        h.get_by_label(cell).click();
        h.run();
        assert_eq!(h.state().links.filter, filter);
        assert_eq!(names(&h), want, "{cell}");
        assert_eq!(h.query_all_by_label("REVOKE").count(), want.len(), "{cell}: one row each");
    }
}

#[test]
fn search_narrows_by_name_and_place() {
    let mut h = links_page();
    h.state_mut().links.query = "budget".into();
    h.run();
    assert_eq!(names(&h), ["Budget 2026.xlsx"]);
    h.state_mut().links.query = "clients".into();
    h.run();
    assert_eq!(names(&h), ["Board pitch.pptx"], "where it is now counts too");
    h.state_mut().links.query = "nothing like it".into();
    h.run();
    h.get_by_label("0 LINKS");
    h.get_by_label("No link matches.");
    h.get_by_label("ALL 5");
}

#[test]
fn sorting_by_name_and_by_opens() {
    let mut h = links_page();
    h.get_by_label("SHARED ITEM").click();
    h.run();
    assert_eq!(names(&h), ["Board pitch.pptx", "Budget 2026.xlsx", "Poster.psd", "Q3 report.docx", "Team"]);
    h.get_by_label("OPENED").click();
    h.run();
    assert_eq!(names(&h), ["Team", "Poster.psd", "Budget 2026.xlsx", "Q3 report.docx", "Board pitch.pptx"]);
    h.get_by_label("MADE").click();
    h.run();
    assert_eq!(names(&h)[0], "Q3 report.docx", "newest again");
    h.get_by_label("MADE").click();
    h.run();
    assert_eq!(names(&h)[0], "Budget 2026.xlsx", "then oldest");
}

#[test]
fn copy_puts_the_link_on_the_clipboard() {
    let mut h = links_page();
    h.query_all_by_label("COPY").next().unwrap().click();
    h.step();
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "https://cloud.example.org/s/tok-s1"));
    assert!(copied);
    h.run();
    h.get_by_label("Link copied");
}

#[test]
fn revoke_asks_first_and_cancel_keeps_the_link() {
    let mut h = links_page();
    h.query_all_by_label("REVOKE").next().unwrap().click();
    h.run();
    h.get_by_label("Revoke this link?");
    h.get_by_label("MADE TODAY 14:32");
    h.get_by_label("My files / Documents / Reports · opened 3 times · password on");
    h.get_by_label("Anyone who opens it afterwards sees \"This link doesn't work\". The file stays where it is, and you can make a new link later; it gets a new address.");
    h.get_by_label("CANCEL").click();
    h.run();
    assert!(h.query_by_label("Revoke this link?").is_none());
    assert_eq!(h.state().backend.sent.iter().filter(|(_, r)| r.method == Method::Delete).count(), 0, "nothing was revoked");
    assert_eq!(names(&h).len(), 5);
}

#[test]
fn revoke_removes_the_link_after_the_confirmation() {
    let mut h = links_page();
    h.query_all_by_label("REVOKE").nth(2).unwrap().click();
    h.run();
    h.get_by_label("REVOKE LINK").click();
    h.run();
    let req = h.state().backend.last_to("/api/shares/s2");
    assert_eq!(req.method, Method::Delete, "the link to Team, only the link");
    assert_eq!(h.state().backend.sent.iter().filter(|(_, r)| r.method == Method::Delete).count(), 1);
    // Refused first: the sheet stays and says why.
    h.state_mut().backend.answer("/api/shares/s2", Ok(HttpResponse::new(500, br#"{"error":"Internal error"}"#.to_vec())));
    h.run();
    h.get_by_label("NOT REVOKED");
    assert_eq!(names(&h).len(), 5);
    h.get_by_label("TRY AGAIN").click();
    h.run();
    h.state_mut().backend.answer("/api/shares/s2", Ok(HttpResponse::new(204, Vec::new())));
    h.run();
    assert!(h.query_by_label("Revoke this link?").is_none());
    assert_eq!(names(&h), ["Q3 report.docx", "Board pitch.pptx", "Poster.psd", "Budget 2026.xlsx"]);
    h.get_by_label("Link revoked");
    h.get_by_label("ALL 4");
}

#[test]
fn no_links_yet_says_how_to_make_one() {
    let mut h = signed_in("/links");
    h.state_mut().backend.answer(LIST, ok(json!({"items": [], "pagination": {"page": 1, "has_next": false}}).to_string()));
    h.run();
    h.get_by_label("YOUR SHARE LINKS");
    h.get_by_label("ALL 0");
    h.get_by_label("EXPIRED 0");
    assert!(h.query_by_label("NO LONGER THERE 0").is_none());
    h.get_by_label("LINKS WORK WITHOUT SIGNING IN");
    h.get_by_label("GO TO FILES").click();
    h.run();
    assert_eq!(h.state().page, Page::Files);
}

#[test]
fn every_page_of_links_is_read() {
    let mut h = signed_in("/links");
    let page1 =
        json!({"items": [link("s1", "f1", "file", "a.txt", false, None, "2026-11-01T10:00:00Z", 0)], "pagination": {"page": 1, "has_next": true}});
    h.state_mut().backend.answer(LIST, ok(page1.to_string()));
    h.run();
    let page2 =
        json!({"items": [link("s2", "f2", "file", "b.txt", false, None, "2026-11-02T10:00:00Z", 0)], "pagination": {"page": 2, "has_next": false}});
    h.state_mut().backend.answer("/api/shares?page=2&per_page=100", ok(page2.to_string()));
    h.run();
    h.get_by_label("ALL 2");
}

#[test]
fn the_account_menu_leads_to_the_links() {
    let mut h = signed_in("/");
    assert_eq!(h.state().page, Page::Home);
    h.get_by_label("Account: john").click();
    h.run();
    h.get_by_label("JOHN · JOHN@EXAMPLE.ORG");
    h.state_mut().backend.answer(LIST, ok(five().to_string()));
    h.run();
    assert_eq!(h.state().backend.sent_to("?metadata=true"), 0, "the menu only counts");
    h.get_by_label("Your share links, 5").click();
    h.run();
    assert_eq!(h.state().page, Page::Links);
    assert!(!h.state().account_open);
    answer_five(&mut h);
    h.get_by_label("NO LONGER THERE 1");
}

/// The account menu offers CraftCloud's own look and the seven colour themes; a pick is drawn at
/// once, kept by the browser, and used again on the next visit.
#[test]
fn the_account_menu_picks_a_colour_theme_the_browser_keeps() {
    use crate::theme::Tokens;
    use crate::themes::Palette;
    let mut h = signed_in("/");
    assert_eq!(Tokens::get(&h.ctx), Tokens::DARK);
    h.get_by_label("Account: john").click();
    h.run();
    assert_eq!(h.get_by_label("Theme: CraftCloud").accesskit_node().data().toggled(), Some(egui::accesskit::Toggled::True));
    for name in ["Catppuccin", "Dracula (dark only)", "Gruvbox", "Nord (dark only)", "One Dark (dark only)", "Tokyo Night"] {
        h.get_by_label(&format!("Theme: {name}"));
    }
    h.get_by_label("Theme: Solarized Light (light only)").click();
    h.run();
    assert_eq!(h.state().backend.prefs.get(crate::web::THEME_PREF).map(String::as_str), Some("craft.solarized-light"));
    let solarized = Palette::for_id("craft.solarized-light");
    assert_eq!(Tokens::get(&h.ctx), solarized.dark, "light only: shown in a dark browser too");
    assert!(h.state().account_open, "the menu stays open");
    assert_eq!(h.get_by_label("Theme: Solarized Light (light only)").accesskit_node().data().toggled(), Some(egui::accesskit::Toggled::True));

    // The next visit starts with it.
    let server = ServerUrl::parse("https://cloud.example.org").unwrap();
    let mut backend = FakeBackend::default();
    backend.prefs = h.state().backend.prefs.clone();
    let app = WebApp::new(&egui::Context::default(), backend, server, "/", None);
    let mut again = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(|ui, app: &mut WebApp<FakeBackend>| app.ui(ui), app);
    again.run();
    assert_eq!(Tokens::get(&again.ctx), solarized.dark);

    // An id this build doesn't know shows CraftCloud's own look.
    let server = ServerUrl::parse("https://cloud.example.org").unwrap();
    let mut backend = FakeBackend::default();
    backend.prefs.insert(crate::web::THEME_PREF.into(), "craft.gone".into());
    let app = WebApp::new(&egui::Context::default(), backend, server, "/", None);
    let mut gone = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(|ui, app: &mut WebApp<FakeBackend>| app.ui(ui), app);
    gone.run();
    assert_eq!(Tokens::get(&gone.ctx), Tokens::DARK);
}

#[test]
fn the_account_menu_closes_on_escape() {
    let mut h = signed_in("/");
    h.get_by_label("Account: john").click();
    h.run();
    assert!(h.state().account_open);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!h.state().account_open);
    assert!(h.query_by_label("JOHN · JOHN@EXAMPLE.ORG").is_none());
}

#[test]
fn the_share_sheet_leads_to_every_link() {
    let mut h = signed_in("/files");
    let b = &mut h.state_mut().backend;
    b.answer("/api/folders", ok(json!([{"id": "root1", "name": "Personal", "path": "/Personal", "parent_id": null}]).to_string()));
    b.answer("/craftcloud/config.json", ok(CONFIG));
    h.run();
    let file = json!({"resource_type": "file", "resource": {"id": "f1", "name": "Q3 report.docx", "path": "", "size": 248_000,
        "mime_type": "application/octet-stream", "folder_id": "root1", "modified_at": "2026-11-13T11:42:00Z", "category": "Document", "etag": "e-f1"}});
    let b = &mut h.state_mut().backend;
    b.answer("/api/folders/root1/resources?limit=200", ok(json!({"items": [file], "next_cursor": null}).to_string()));
    b.answer("/api/folders/root1/ancestors", ok(json!({"ancestors": [{"id": "root1", "name": "Personal"}]}).to_string()));
    h.run();
    h.get_by_label("More for Q3 report.docx").click();
    h.run();
    h.get_by_label("Share link…").click();
    h.run();
    h.get_by_label("CREATE LINK").click();
    h.run();
    h.state_mut().backend.answer("/api/shares", Ok(HttpResponse::new(201, fixture("share-create.json"))));
    h.run();
    h.get_by_label("See all your share links").click();
    h.run();
    assert_eq!(h.state().page, Page::Links);
    assert!(h.state().files.sheet.is_none(), "the sheet closed");
    assert!(h.state().backend.waiting(LIST), "the page loads the links");
}

#[test]
fn opened_counts_read_as_words() {
    assert_eq!(times(0, false), "Not yet");
    assert_eq!(times(1, false), "1 time");
    assert_eq!(times(12, false), "12 times");
    assert_eq!(times(27, true), "27 times in all");
}
