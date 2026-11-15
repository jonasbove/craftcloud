//! The Files page against a scripted server: requests are answered by URL, widgets found by their
//! accessible labels.

use craftcloud_core::ServerUrl;
use craftcloud_core::http::{HttpResponse, Method};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::View;
use crate::backend::FakeBackend;
use crate::web::{Page, WebApp};

const ROOT: &str = "root1";
const CONFIG: &[u8] = br#"{"version": "0.1.0", "web_apps": false, "public_url": "https://cloud.example.org", "enabled_apps": []}"#;
const INITIALIZED: &[u8] = br#"{"initialized":true,"admin_count":1,"registration_allowed":false,"faces_enabled":false}"#;

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/oxicloud-0.10.0").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ok(body: impl Into<Vec<u8>>) -> Result<HttpResponse, String> {
    Ok(HttpResponse::new(200, body.into()))
}

fn json_ok(v: &Value) -> Result<HttpResponse, String> {
    ok(v.to_string().into_bytes())
}

/// A listed folder.
pub fn folder(id: &str, name: &str) -> Value {
    json!({"resource_type": "folder", "resource": {"id": id, "name": name, "path": "", "parent_id": ROOT, "modified_at": "2026-11-12T09:10:00Z", "etag": id}})
}

/// A listed file.
pub fn file(id: &str, name: &str, size: u64) -> Value {
    json!({"resource_type": "file", "resource": {"id": id, "name": name, "path": "", "size": size, "mime_type": "application/octet-stream",
        "folder_id": ROOT, "modified_at": "2026-11-13T11:42:00Z", "category": "Document", "etag": format!("e-{id}")}})
}

fn listing(items: &[Value]) -> Value {
    json!({"items": items, "next_cursor": null})
}

type App = Harness<'static, WebApp<FakeBackend>>;

/// Signs in, opens Files and answers the root listing with `items`.
fn files_page(items: &[Value]) -> App {
    let server = ServerUrl::parse("https://cloud.example.org").unwrap();
    let app = WebApp::new(&egui::Context::default(), FakeBackend::default(), server, "/files", None);
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(|ui, app: &mut WebApp<FakeBackend>| app.ui(ui), app);
    let b = &mut h.state_mut().backend;
    b.answer("/api/auth/status", ok(INITIALIZED));
    b.answer("/craftcloud/config.json", ok(CONFIG));
    b.answer("/api/auth/me", ok(fixture("me.json")));
    h.run();
    assert_eq!(h.state().page, Page::Files);
    let b = &mut h.state_mut().backend;
    b.answer("/api/folders", json_ok(&json!([{"id": ROOT, "name": "Personal", "path": "/Personal", "parent_id": null}])));
    b.answer("/craftcloud/config.json", ok(CONFIG));
    h.run();
    answer_folder(&mut h, ROOT, &[(ROOT, "Personal")], items);
    h
}

/// Answers the listing and breadcrumbs of folder `id`.
fn answer_folder(h: &mut App, id: &str, crumbs: &[(&str, &str)], items: &[Value]) {
    let b = &mut h.state_mut().backend;
    b.answer(&format!("/api/folders/{id}/resources?limit=200"), json_ok(&listing(items)));
    let ancestors: Vec<Value> = crumbs.iter().map(|(id, name)| json!({"id": id, "name": name})).collect();
    b.answer(&format!("/api/folders/{id}/ancestors"), json_ok(&json!({"ancestors": ancestors})));
    h.run();
}

fn names(h: &App) -> Vec<String> {
    h.state().files.items.iter().map(|i| i.name.clone()).collect()
}

#[test]
fn browse_lists_folders_first_and_hides_craftcloud_data() {
    let h = files_page(&[
        file("f1", "Q3 report.docx", 248_000),
        folder("d1", "Drafts"),
        folder("cc", ".craftcloud"),
        file("f2", "archive.zip", 96_300_000),
    ]);
    assert_eq!(names(&h), ["Drafts", "archive.zip", "Q3 report.docx"]);
    h.get_by_label("Q3 report.docx");
    h.get_by_label("1 FOLDER · 2 FILES · 96.5 MB");
    h.get_by_label("WordCraft");
}

#[test]
fn double_click_opens_a_folder_and_breadcrumbs_lead_back() {
    let mut h = files_page(&[folder("d1", "Reports"), file("f1", "plan.txt", 8)]);
    h.get_by_label("Reports").click();
    h.run();
    assert_eq!(h.state().files.selection, ["d1"]);
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(h.state().files.view, View::Folder("d1".into()));
    answer_folder(&mut h, "d1", &[(ROOT, "Personal"), ("d1", "Reports")], &[file("f9", "budget.xlsx", 86_000)]);
    assert_eq!(names(&h), ["budget.xlsx"]);
    assert_eq!(h.state().files.items[0].path, "Reports/budget.xlsx", "paths come from the breadcrumbs");
    h.get_by_label("REPORTS folder");
    h.get_by_label("MY FILES folder").click();
    h.run();
    assert_eq!(h.state().files.view, View::Folder(ROOT.into()));
}

#[test]
fn an_empty_folder_says_so() {
    let h = files_page(&[]);
    h.get_by_label("This folder is empty");
}

#[test]
fn sorting_by_size_puts_the_biggest_first() {
    let mut h = files_page(&[file("a", "a.txt", 5), file("b", "b.txt", 500), folder("d", "Docs")]);
    h.get_by_label("Sort by Size").click();
    h.run();
    assert_eq!(names(&h), ["Docs", "b.txt", "a.txt"]);
}

#[test]
fn several_items_download_as_one_zip() {
    let mut h = files_page(&[folder("d1", "Drafts"), file("f1", "a.txt", 5), file("f2", "b.txt", 5)]);
    h.get_by_label("Select all").click();
    h.run();
    h.get_by_label("More for a.txt").click();
    h.run();
    h.get_by_label("Download as zip").click();
    h.run();
    let (url, _) = h.state().backend.downloads.last().cloned().unwrap();
    assert_eq!(url, "https://cloud.example.org/api/batch/download?file_ids=f1,f2&folder_ids=d1");
}

/// A file with a full server path, as search, recent, starred and the trash list them.
fn placed(id: &str, path: &str) -> Value {
    let name = path.rsplit('/').next().unwrap();
    json!({"resource_type": "file", "resource": {"id": id, "name": name, "path": path, "size": 8, "mime_type": "text/plain",
        "folder_id": "d1", "modified_at": "2026-11-13T11:42:00Z", "category": "Text"}})
}

#[test]
fn deleting_moves_to_the_trash_and_undo_restores() {
    let mut h = files_page(&[file("f1", "a.txt", 5), file("f2", "b.txt", 5)]);
    h.get_by_label("a.txt").click();
    h.run();
    h.get_by_label("DELETE").click();
    h.run();
    let req = h.state().backend.last_to("/api/files/f1");
    assert_eq!(req.method, Method::Delete);
    h.state_mut().backend.answer("/api/files/f1", Ok(HttpResponse::new(204, Vec::new())));
    h.run();
    h.get_by_label("Moved “a.txt” to the trash");
    assert!(h.state().backend.waiting(&format!("/api/folders/{ROOT}/resources?limit=200")), "the folder reloads");
    h.get_by_label("UNDO").click();
    h.run();
    assert_eq!(h.state().backend.last_to("/api/trash/f1/restore").method, Method::Post);
}

#[test]
fn several_items_go_to_the_trash_in_one_batch() {
    let mut h = files_page(&[folder("d1", "Drafts"), file("f1", "a.txt", 5)]);
    h.get_by_label("Select all").click();
    h.run();
    h.get_by_label("DELETE").click();
    h.run();
    let body: Value = serde_json::from_slice(&h.state().backend.last_to("/api/batch/trash").body).unwrap();
    assert_eq!(body, json!({"file_ids": ["f1"], "folder_ids": ["d1"]}));
    h.state_mut().backend.answer("/api/batch/trash", Ok(HttpResponse::new(206, fixture("batch-failed.json"))));
    h.run();
    assert!(h.state().files.toast.as_ref().is_some_and(|t| t.bad), "a partial failure is reported");
}

#[test]
fn the_trash_restores_and_deletes_for_good() {
    let mut h = files_page(&[]);
    h.get_by_label("TRASH").click();
    h.run();
    assert_eq!(h.state().files.view, View::Trash);
    h.state_mut().backend.answer("/api/trash/resources?limit=200", ok(fixture("trash.json")));
    h.run();
    h.get_by_label("b.txt").click();
    h.run();
    h.get_by_label("RESTORE").click();
    h.run();
    // First: is its old place free?
    assert_eq!(h.state().backend.last_to("/webdav/Documents/b.txt").method, Method::Propfind);
    h.state_mut().backend.answer("/webdav/Documents/b.txt", Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    let last = &h.state().backend.sent.last().unwrap().1;
    assert_eq!((last.method, last.url.ends_with("/restore")), (Method::Post, true));

    h.get_by_label("EMPTY TRASH").click();
    h.run();
    h.get_by_label("Empty the trash?");
    h.get_all_by_label("EMPTY TRASH").last().unwrap().click();
    h.run();
    assert_eq!(h.state().backend.last_to("/api/trash/empty").method, Method::Delete);
}

#[test]
fn restoring_onto_a_taken_name_offers_another_name() {
    let mut h = files_page(&[]);
    h.get_by_label("TRASH").click();
    h.run();
    h.state_mut().backend.answer("/api/trash/resources?limit=200", ok(fixture("trash.json")));
    h.run();
    h.get_by_label("b.txt").click();
    h.run();
    h.get_by_label("RESTORE").click();
    h.run();
    h.state_mut().backend.answer("/webdav/Documents/b.txt", Ok(HttpResponse::new(207, fixture("propfind-documents.xml"))));
    h.run();
    h.get_by_label("“b.txt” is already there");
    h.get_by_label("Documents/b.txt");
    assert_eq!(h.state().backend.sent_to("/restore"), 0, "no restore onto the taken path");

    h.get_by_label("RESTORE WITH THE NEW NAME").click();
    h.run();
    // 1. The file that's there moves aside, 2. restore, 3. rename the restored one, 4. move back.
    let aside = "/webdav/Documents/.b.txt.craftcloud-restoring";
    let step = |h: &App| {
        h.state().backend.sent.last().map(|(_, r)| (r.method, r.url.clone(), r.header_value("Destination").unwrap_or_default().to_owned())).unwrap()
    };
    let (m, url, to) = step(&h);
    assert_eq!((m, url.ends_with("/webdav/Documents/b.txt"), to.ends_with(aside)), (Method::Move, true, true));
    h.state_mut().backend.answer("/webdav/Documents/b.txt", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    assert!(step(&h).1.ends_with("/restore"));
    h.state_mut().backend.answer("/restore", ok(b"{}".to_vec()));
    h.run();
    let (_, url, to) = step(&h);
    assert!(url.ends_with("/webdav/Documents/b.txt") && to.ends_with("/webdav/Documents/b%20(restored).txt"), "{url} → {to}");
    h.state_mut().backend.answer("/webdav/Documents/b.txt", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    let (_, url, to) = step(&h);
    assert!(url.ends_with(aside) && to.ends_with("/webdav/Documents/b.txt"), "{url} → {to}");
    h.state_mut().backend.answer(aside, Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    h.get_by_label("Restored as “b (restored).txt”");
}

#[test]
fn the_trash_filters_itself_and_shows_its_order() {
    let mut h = files_page(&[]);
    h.get_by_label("TRASH").click();
    h.run();
    h.state_mut().backend.answer("/api/trash/resources?limit=200", ok(fixture("trash.json")));
    h.run();
    assert_eq!(names(&h), ["b.txt", "a.txt"], "soonest deleted first, as the server sends it");
    assert_eq!(h.state().files.sort, super::model::Sort::TRASH);
    h.get_by_label_contains("2 ITEMS");
    h.state_mut().files.query = "a.t".into();
    h.state_mut().files.filter_trash();
    h.run();
    assert_eq!(h.state().files.view, View::Trash, "filtering doesn't leave the trash");
    assert_eq!(names(&h), ["a.txt"]);
    h.get_by_label_contains("1 OF 2 ITEMS");
}

#[test]
fn a_search_after_a_change_is_asked_afresh() {
    let mut h = files_page(&[]);
    h.state_mut().files.query = "plan".into();
    h.run();
    h.get_by_label("Search files").click();
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let found = listing(&[placed("f1", "/Personal/Reports/plan.txt"), placed("f2", "/Personal/plan b.txt")]);
    let first = search_url(&h);
    h.state_mut().backend.answer(&first, json_ok(&found));
    h.run();
    h.get_by_label("plan.txt").click();
    h.run();
    h.get_by_label("DELETE").click();
    h.run();
    assert_eq!(names(&h), ["plan b.txt"], "the trashed file leaves the results at once");
    h.state_mut().backend.answer("/api/files/f1", Ok(HttpResponse::new(204, Vec::new())));
    h.run();
    // OxiCloud caches a search per exact query string: the reload asks a slightly different one.
    let again = search_url(&h);
    assert_ne!(again, first);
    assert!(h.state().backend.waiting(&again));
}

/// The last search request's URL from `/api/search` on.
fn search_url(h: &App) -> String {
    let url = &h.state().backend.sent.iter().rev().find(|(_, r)| r.url.contains("/api/search?")).expect("a search").1.url;
    url[url.find("/api/search").unwrap()..].to_owned()
}

/// A fresh session at `unix` seconds that searches for "plan".
fn fresh_search(unix: f64) -> App {
    let mut h = files_page(&[]);
    h.state_mut().backend.wall = Some(crate::backend::WallClock { unix, utc_offset_minutes: 0 });
    h.run();
    h.state_mut().files.query = "plan".into();
    h.run();
    h.get_by_label("Search files").click();
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h
}

#[test]
fn a_fresh_session_never_gets_a_stale_search() {
    // T-043: a file deleted from another device stayed in a new session's results, because the
    // new session asked the exact query the server had cached.
    let a = fresh_search(1_794_650_400.0);
    let b = fresh_search(1_794_650_430.0);
    assert_ne!(search_url(&a), search_url(&b), "30 s later, a new session asks a new query");
    // And a result that is in the trash by now is left out, whichever answer comes first.
    let mut h = b;
    let url = search_url(&h);
    let found = listing(&[placed("f1", "/Personal/Reports/plan.txt"), placed("f2", "/Personal/plan b.txt")]);
    h.state_mut().backend.answer(&url, json_ok(&found));
    h.run();
    let trashed = json!({"items": [{"resource_type": "file", "trashed_at": "2026-11-14T09:59:00Z", "expires_at": "2026-12-14T09:59:00Z",
        "resource": {"id": "f1", "name": "plan.txt", "path": "Personal/Reports/plan.txt", "size": 8, "mime_type": "text/plain",
        "folder_id": "r", "modified_at": "2026-11-14T09:00:00Z", "category": "Text", "etag": "e"}}], "next_cursor": null});
    h.state_mut().backend.answer("/api/trash/resources?limit=200", json_ok(&trashed));
    h.run();
    assert_eq!(names(&h), ["plan b.txt"]);
}

#[test]
fn search_sizes_stay_full_and_turn_slowly() {
    use super::search_limit;
    for t in [0.0, 1_794_650_400.0, 1_794_650_401.0] {
        for bust in 0..900 {
            assert!((100..500).contains(&search_limit(Some(t), bust)));
        }
    }
    // A key comes back only after about 13 minutes.
    let at = |t: f64| search_limit(Some(t), 0);
    assert!((1..400).all(|step| at(1_794_650_400.0 + 2.0 * f64::from(step)) != at(1_794_650_400.0)));
}

#[test]
fn search_finds_files_but_never_craftcloud_data() {
    let mut h = files_page(&[]);
    h.state_mut().files.query = "plan".into();
    h.run();
    h.get_by_label("Search files").click();
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(h.state().files.view, View::Search("plan".into()));
    let found = listing(&[
        placed("f1", "/Personal/Reports/plan.txt"),
        placed("v1", "/Personal/.craftcloud/versions/Reports/plan.txt/20261113T151013.123Z"),
        json!({"resource_type": "folder", "resource": {"id": "v2", "name": "plan.txt", "path": "/Personal/.craftcloud/versions/Reports/plan.txt"}}),
    ]);
    let url = search_url(&h);
    h.state_mut().backend.answer(&url, json_ok(&found));
    h.run();
    assert_eq!(names(&h), ["plan.txt"]);
    assert_eq!(h.state().files.items[0].path, "Reports/plan.txt");
}

#[test]
fn starred_and_recent_list_what_the_server_says() {
    let mut h = files_page(&[]);
    h.get_by_label("STARRED").click();
    h.run();
    h.state_mut().backend.answer("/api/favorites/resources?limit=200", ok(fixture("favorites.json")));
    h.run();
    assert!(!h.state().files.items.is_empty());
    h.get_by_label("RECENT").click();
    h.run();
    let recent = listing(&[placed("f1", "Personal/.craftcloud/x"), placed("f2", "Personal/notes.md")]);
    h.state_mut().backend.answer("/api/recent/resources?limit=100", json_ok(&recent));
    h.run();
    assert_eq!(names(&h), ["notes.md"]);
}

#[test]
fn recent_shows_the_order_it_is_in() {
    let mut h = files_page(&[]);
    h.get_by_label("RECENT").click();
    h.run();
    let recent = json!({"items": [
        {"resource_type": "file", "accessed_at": "2026-11-13T14:00:00Z", "resource": placed("f2", "Personal/b.md")["resource"]},
        {"resource_type": "file", "accessed_at": "2026-11-13T12:00:00Z", "resource": placed("f1", "Personal/a.md")["resource"]},
    ], "next_cursor": null});
    h.state_mut().backend.answer("/api/recent/resources?limit=100", json_ok(&recent));
    h.run();
    assert_eq!(names(&h), ["b.md", "a.md"], "last opened first, not by name");
    assert_eq!(h.state().files.sort, super::model::Sort::RECENT, "the header marks Opened, not Name");
    h.get_by_label("Sort by Opened");
    // Back in a folder, the usual name order.
    h.get_by_label("MY FILES").click();
    h.run();
    assert_eq!(h.state().files.sort, super::model::Sort::default());
}

#[test]
fn starring_a_file() {
    let mut h = files_page(&[file("f1", "a.txt", 5)]);
    h.get_by_label("More for a.txt").click();
    h.run();
    h.get_by_label("Star").click();
    h.run();
    assert_eq!(h.state().backend.last_to("/api/favorites/file/f1").method, Method::Post);
}

fn local(key: u64, name: &str, size: u64) -> crate::backend::LocalFile {
    crate::backend::LocalFile { key, name: name.into(), size, mime: String::new() }
}

/// Opens folder "Reports" (d1) under the root.
fn in_reports(items: &[Value]) -> App {
    let mut h = files_page(&[folder("d1", "Reports")]);
    h.get_by_label("Reports").click();
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    answer_folder(&mut h, "d1", &[(ROOT, "Personal"), ("d1", "Reports")], items);
    h
}

#[test]
fn upload_puts_the_chosen_files_into_the_open_folder() {
    let mut h = in_reports(&[]);
    h.state_mut().backend.to_pick = vec![local(7, "Q3 report.docx", 248_000), local(8, "notes.md", 11)];
    h.get_by_label("UPLOAD").click();
    h.run();
    assert_eq!(h.state().backend.picks, 1);
    h.run();
    let put = h.state().backend.last_to("/webdav/Reports/Q3%20report.docx");
    assert_eq!((put.method, put.header_value("If-None-Match")), (Method::Put, Some("*")));
    assert_eq!(put.header_value("Content-Type"), Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"));
    assert!(h.state().backend.uploads.iter().any(|(_, key)| *key == 7), "the bytes come from the local file");
    // Progress shows while it runs.
    let id = h.state().backend.sent.iter().find(|(_, r)| r.url.ends_with("Q3%20report.docx")).unwrap().0;
    h.state_mut().backend.progress.push((id, 124_000));
    h.run();
    h.get_by_label("Q3 report.docx 124 KB of 248 KB");
    h.state_mut().backend.answer("/webdav/Reports/Q3%20report.docx", Ok(HttpResponse::new(201, Vec::new())));
    h.state_mut().backend.answer("/webdav/Reports/notes.md", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    h.get_by_label("2 OF 2 UPLOADED");
    assert!(h.state().backend.waiting("/api/folders/d1/resources?limit=200"), "the folder reloads");
}

#[test]
fn a_taken_name_can_be_kept_twice_or_replaced_after_keeping_a_version() {
    let mut h = in_reports(&[]);
    h.state_mut().backend.wall = Some(crate::backend::WallClock { unix: 1_794_608_247.123, utc_offset_minutes: 0 });
    h.state_mut().backend.to_pick = vec![local(1, "plan.txt", 8), local(2, "budget.xlsx", 9)];
    h.get_by_label("UPLOAD").click();
    h.run();
    h.run();
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(412, Vec::new())));
    h.run();
    h.get_by_label("This name is taken");
    h.get_by_label("KEEP BOTH").click();
    h.run();
    assert_eq!(h.state().backend.last_to("/webdav/Reports/plan%20(2).txt").method, Method::Put);

    h.state_mut().backend.answer("/webdav/Reports/budget.xlsx", Ok(HttpResponse::new(412, Vec::new())));
    h.run();
    h.get_by_label("REPLACE").click();
    h.run();
    // The current ETag, the versions folders, the snapshot, the pruning, then the guarded PUT.
    let listing = concat!(
        r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/webdav/Reports/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype>"#,
        r#"</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/webdav/Reports/budget.xlsx</D:href><D:propstat><D:prop>"#,
        r#"<D:resourcetype/><D:getetag>&quot;e-1&quot;</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#
    );
    h.state_mut().backend.answer("/webdav/Reports/", Ok(HttpResponse::new(207, listing.as_bytes().to_vec())));
    h.run();
    for folder in [".craftcloud/", ".craftcloud/versions/", ".craftcloud/versions/Reports/", ".craftcloud/versions/Reports/budget.xlsx/"] {
        let url = format!("/webdav/{folder}");
        assert_eq!(h.state().backend.last_to(&url).method, Method::Mkcol);
        h.state_mut().backend.answer(&url, Ok(HttpResponse::new(405, Vec::new())));
        h.run();
    }
    let copy = h.state().backend.last_to("/webdav/Reports/budget.xlsx");
    assert_eq!(copy.method, Method::Copy);
    assert!(copy.header_value("Destination").unwrap().ends_with("/webdav/.craftcloud/versions/Reports/budget.xlsx/20261113T221727.123Z"));
    h.state_mut().backend.answer("/webdav/Reports/budget.xlsx", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    h.state_mut().backend.answer("/webdav/.craftcloud/versions/Reports/budget.xlsx/", Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    let put = h.state().backend.last_to("/webdav/Reports/budget.xlsx");
    assert_eq!((put.method, put.header_value("If-Match")), (Method::Put, Some("\"e-1\"")));
}

#[test]
fn new_folder_and_rename() {
    let mut h = in_reports(&[file("f1", "plan.txt", 8)]);
    h.get_by_label("NEW FOLDER").click();
    h.run();
    h.state_mut().files.sheet = Some(super::Sheet::NewFolder { name: "Q4".into() });
    h.run();
    h.get_by_label("CREATE").click();
    h.run();
    let body: Value = serde_json::from_slice(&h.state().backend.last_to("/api/folders").body).unwrap();
    assert_eq!(body, json!({"name": "Q4", "parent_id": "d1"}));
    h.state_mut().backend.answer("/api/folders", Ok(HttpResponse::new(409, fixture("folder-exists.json"))));
    h.run();
    h.get_by_label("Folder already exists: Documents already exists in parent");
    h.get_by_label("CANCEL").click();
    h.run();

    h.get_by_label("plan.txt").click();
    h.run();
    h.key_press(egui::Key::F2);
    h.run();
    h.get_by_label("Rename file");
    h.state_mut().files.sheet = Some(super::Sheet::Rename { item: Box::new(h.state().files.items[0].clone()), name: "plan v2.txt".into() });
    h.run();
    h.get_by_label("RENAME").click();
    h.run();
    let body: Value = serde_json::from_slice(&h.state().backend.last_to("/api/files/f1/rename").body).unwrap();
    assert_eq!(body, json!({"name": "plan v2.txt"}));
    h.state_mut().backend.answer("/api/files/f1/rename", ok(fixture("file-rename.json")));
    h.run();
    assert!(h.state().files.sheet.is_none());
    assert_eq!(h.state().files.toast.as_ref().map(|t| t.text.as_str()), Some("Renamed to “plan v2.txt”"));
}

#[test]
fn move_picks_a_folder_and_moves_files_and_folders() {
    let mut h = files_page(&[folder("d1", "Reports"), folder("d2", "Drafts"), file("f1", "plan.txt", 8)]);
    h.get_by_label("Select all").click();
    h.run();
    h.get_by_label("MOVE").click();
    h.run();
    h.state_mut()
        .backend
        .answer(&format!("/api/folders/{ROOT}/resources?limit=200"), json_ok(&listing(&[folder("d1", "Reports"), folder("d9", "Archive")])));
    h.run();
    assert!(egui_kittest::kittest::NodeT::accesskit_node(&h.get_by_label("Open Reports")).is_disabled(), "a folder can't go into itself");
    h.get_by_label("Open Archive").click();
    h.run();
    h.state_mut().backend.answer("/api/folders/d9/resources?limit=200", json_ok(&listing(&[])));
    h.run();
    h.get_by_label("MOVE HERE").click();
    h.run();
    let files: Value = serde_json::from_slice(&h.state().backend.last_to("/api/batch/files/move").body).unwrap();
    assert_eq!(files, json!({"file_ids": ["f1"], "target_folder_id": "d9"}));
    let folders: Value = serde_json::from_slice(&h.state().backend.last_to("/api/batch/folders/move").body).unwrap();
    assert_eq!(folders["folder_ids"], json!(["d2", "d1"]), "in the list order: Drafts, Reports");
}

#[test]
fn the_trash_says_when_each_item_went() {
    // T-048: two items of the same name told apart by when they were deleted, at any width.
    let mut h = files_page(&[]);
    h.get_by_label("TRASH").click();
    h.run();
    h.state_mut().backend.answer("/api/trash/resources?limit=200", ok(fixture("trash.json")));
    h.run();
    let files = &h.state().files;
    assert_eq!(files.items[0].trashed.as_deref(), Some("2026-11-13T21:54:13.276335Z"));
    let has = |w: f32, col: super::table::Col| files.columns(w).iter().any(|(c, _)| *c == col);
    for w in [1400.0, 700.0, 350.0] {
        assert!(has(w, super::table::Col::Trashed), "DELETED at {w}");
    }
    assert!(has(1400.0, super::table::Col::Deletes), "the purge date keeps its own column");
    assert!(!has(700.0, super::table::Col::Deletes), "narrower, it is the row's tooltip");
}

/// Answers every MKCOL still out with 201, one after the other (the kept-version steps go singly).
fn answer_mkcols(h: &mut App) {
    loop {
        let b = &h.state().backend;
        let next = b.sent.iter().map(|(_, r)| r).find(|r| r.method == Method::Mkcol && b.waiting(&r.url)).map(|r| r.url.clone());
        let Some(url) = next else { break };
        h.state_mut().backend.answer(&url, Ok(HttpResponse::new(201, Vec::new())));
        h.run();
    }
}

#[test]
fn a_rename_and_a_move_take_the_kept_versions_along() {
    // T-050: the history at .craftcloud/versions/<path>/ follows the file.
    let mut h = in_reports(&[file("f1", "plan.txt", 8)]);
    h.get_by_label("plan.txt").click();
    h.run();
    h.key_press(egui::Key::F2);
    h.run();
    h.state_mut().files.sheet = Some(super::Sheet::Rename { item: Box::new(h.state().files.items[0].clone()), name: "plan v2.txt".into() });
    h.run();
    h.get_by_label("RENAME").click();
    h.run();
    // (The details panel lists the kept versions: a PROPFIND, which moves nothing.)
    assert_eq!(
        h.state().backend.sent.iter().filter(|(_, r)| r.url.contains("/.craftcloud/") && r.method != Method::Propfind).count(),
        0,
        "nothing moves before the rename worked"
    );
    h.state_mut().backend.answer("/api/files/f1/rename", ok(fixture("file-rename.json")));
    h.run();
    answer_mkcols(&mut h);
    let mv = h.state().backend.last_to("/webdav/.craftcloud/versions/Reports/plan.txt/");
    assert_eq!(mv.method, Method::Move);
    assert!(mv.header_value("Destination").unwrap().ends_with("/webdav/.craftcloud/versions/Reports/plan%20v2.txt/"));

    let mut h = files_page(&[folder("d1", "Reports"), file("f1", "plan.txt", 8)]);
    h.get_by_label("Select plan.txt").click();
    h.run();
    h.get_by_label("MOVE").click();
    h.run();
    h.state_mut().backend.answer(&format!("/api/folders/{ROOT}/resources?limit=200"), json_ok(&listing(&[folder("d1", "Reports")])));
    h.run();
    h.get_by_label("Open Reports").click();
    h.run();
    h.state_mut().backend.answer("/api/folders/d1/resources?limit=200", json_ok(&listing(&[])));
    h.run();
    h.get_by_label("MOVE HERE").click();
    h.run();
    h.state_mut().backend.answer("/api/batch/files/move", ok(b"{}".to_vec()));
    h.run();
    answer_mkcols(&mut h);
    let mv = h.state().backend.last_to("/webdav/.craftcloud/versions/plan.txt/");
    assert_eq!(mv.method, Method::Move);
    assert!(mv.header_value("Destination").unwrap().ends_with("/webdav/.craftcloud/versions/Reports/plan.txt/"));
}

#[test]
fn a_new_upload_starts_without_history() {
    // T-050: versions left at the path belong to the file that was there before.
    let mut h = in_reports(&[]);
    h.state_mut().backend.to_pick = vec![local(7, "notes.md", 11)];
    h.get_by_label("UPLOAD").click();
    h.run();
    h.run();
    h.state_mut().backend.answer("/webdav/Reports/notes.md", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    assert_eq!(h.state().backend.last_to("/webdav/.craftcloud/versions/Reports/notes.md/").method, Method::Delete);
}

#[test]
fn what_goes_for_good_loses_its_kept_versions() {
    // T-049: unless a file is at that path again.
    let mut h = files_page(&[]);
    h.get_by_label("TRASH").click();
    h.run();
    h.state_mut().backend.answer("/api/trash/resources?limit=200", ok(fixture("trash.json")));
    h.run();
    h.get_by_label("EMPTY TRASH").click();
    h.run();
    h.get_all_by_label("EMPTY TRASH").last().unwrap().click();
    h.run();
    h.state_mut().backend.answer("/api/trash/empty", Ok(HttpResponse::new(200, Vec::new())));
    h.run();
    // b.txt's place is free: its history goes. a.txt is there again: its history stays.
    assert_eq!(h.state().backend.last_to("/webdav/Documents/b.txt").method, Method::Propfind);
    h.state_mut().backend.answer("/webdav/Documents/b.txt", Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    assert_eq!(h.state().backend.last_to("/webdav/.craftcloud/versions/Documents/b.txt/").method, Method::Delete);
    h.state_mut().backend.answer("/webdav/.craftcloud/versions/Documents/b.txt/", Ok(HttpResponse::new(204, Vec::new())));
    h.run();
    assert_eq!(h.state().backend.last_to("/webdav/Documents/Q4/a.txt").method, Method::Propfind);
    h.state_mut().backend.answer("/webdav/Documents/Q4/a.txt", Ok(HttpResponse::new(207, Vec::new())));
    h.run();
    assert_eq!(h.state().backend.sent_to("/webdav/.craftcloud/versions/Documents/Q4/a.txt/"), 0);
}

/// A WebDAV listing of `folder` (a path under `/webdav/`) with `files` as `(name, etag, size)`.
fn multistatus(folder: &str, files: &[(&str, &str, u64)]) -> Vec<u8> {
    let mut xml = format!(
        r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/webdav/{folder}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#
    );
    for (name, etag, size) in files {
        xml.push_str(&format!(
            r#"<D:response><D:href>/webdav/{folder}{name}</D:href><D:propstat><D:prop><D:resourcetype/><D:getetag>&quot;{etag}&quot;</D:getetag><D:getcontentlength>{size}</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#
        ));
    }
    xml.push_str("</D:multistatus>");
    xml.into_bytes()
}

const HISTORY: &str = ".craftcloud/versions/Reports/plan.txt/";
const NEWEST: &str = "20261113T101500.000Z_20261113T090000Z";

/// Opens "Versions…" of Reports/plan.txt from its "…" menu, with two kept versions.
fn plan_versions() -> App {
    let mut h = in_reports(&[file("f1", "plan.txt", 8)]);
    h.state_mut().backend.wall = Some(crate::backend::WallClock { unix: 1_794_608_247.123, utc_offset_minutes: 0 });
    h.get_by_label("More for plan.txt").click();
    h.run();
    h.get_by_label("Move to trash");
    h.get_by_label("Versions…").click();
    h.run();
    assert_eq!(h.state().backend.last_to(&format!("/webdav/{HISTORY}")).method, Method::Propfind);
    h.get_by_label("Versions of plan.txt");
    h.get_by_label("My files/Reports");
    let list = multistatus(HISTORY, &[("20261112T091200.000Z", "h1-1", 6), (NEWEST, "h2-2", 7)]);
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(207, list)));
    h.run();
    h
}

#[test]
fn versions_list_the_current_file_and_the_kept_ones() {
    let mut h = plan_versions();
    h.get_by_label("CURRENT");
    h.get_by_label("Today 09:00");
    h.get_by_label("Yesterday 09:12");
    assert_eq!(h.get_all_by_label("RESTORE").count(), 2, "the current file has nothing to restore");
    // DOWNLOAD of a kept version fetches it from the history, under the file's own name.
    h.get_all_by_label("DOWNLOAD").last().unwrap().click();
    h.run();
    let (url, name) = h.state().backend.downloads.last().cloned().unwrap();
    assert_eq!(
        (url.as_str(), name.as_str()),
        ("https://cloud.example.org/webdav/.craftcloud/versions/Reports/plan.txt/20261112T091200.000Z", "plan.txt")
    );
    h.get_by_label("CLOSE").click();
    h.run();
    assert!(h.state().files.sheet.is_none());
}

#[test]
fn a_late_answer_never_fills_another_files_versions() {
    let mut h = in_reports(&[file("f1", "plan.txt", 8), file("f2", "notes.md", 3)]);
    for name in ["plan.txt", "notes.md"] {
        h.get_by_label(&format!("More for {name}")).click();
        h.run();
        h.get_by_label("Versions…").click();
        h.run();
        h.key_press(egui::Key::Escape);
        h.run();
    }
    h.get_by_label("More for notes.md").click();
    h.run();
    h.get_by_label("Versions…").click();
    h.run();
    let list = multistatus(HISTORY, &[(NEWEST, "h2-2", 7)]);
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(207, list)));
    h.run();
    h.get_by_label("Versions of notes.md");
    assert_eq!(h.query_all_by_label("RESTORE").count(), 0, "plan.txt's versions stay out of notes.md's sheet");
}

#[test]
fn a_file_without_history_says_so() {
    let mut h = in_reports(&[file("f1", "plan.txt", 8)]);
    h.get_by_label("More for plan.txt").click();
    h.run();
    h.get_by_label("Versions…").click();
    h.run();
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    h.get_by_label("No earlier versions yet. One is kept each time the file is saved over.");
}

/// Clicks RESTORE on the newest version and answers the ETag and the version's bytes.
fn restore_newest(h: &mut App) {
    h.get_all_by_label("RESTORE").next().unwrap().click();
    h.run();
    let stat = h.state().backend.last_to("/webdav/Reports/plan.txt");
    assert_eq!((stat.method, stat.header_value("Depth")), (Method::Propfind, Some("0")));
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(207, multistatus("Reports/", &[("plan.txt", "cur-9", 8)]))));
    h.run();
    // The version's bytes come before anything is pruned.
    let version = format!("/webdav/{HISTORY}{NEWEST}");
    assert_eq!(h.state().backend.last_to(&version).method, Method::Get);
    h.state_mut().backend.answer(&version, ok(b"old plan".to_vec()));
    h.run();
    answer_mkcols(h);
}

#[test]
fn restoring_a_version_keeps_the_current_file_and_never_overwrites_a_change() {
    let mut h = plan_versions();
    restore_newest(&mut h);
    // The current file is kept first, never over anything.
    let copy = h.state().backend.last_to("/webdav/Reports/plan.txt");
    assert_eq!((copy.method, copy.header_value("Overwrite")), (Method::Copy, Some("F")));
    assert!(copy.header_value("Destination").unwrap().ends_with(&format!("/webdav/{HISTORY}20261113T221727.123Z")));
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    // Pruned to 5, then the guarded write.
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    let put = h.state().backend.last_to("/webdav/Reports/plan.txt");
    assert_eq!((put.method, put.header_value("If-Match")), (Method::Put, Some("\"cur-9\"")));
    assert_eq!(put.body, b"old plan");
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(412, Vec::new())));
    h.run();
    h.get_by_label("Couldn't restore the version: it changed on the server meanwhile; nothing was overwritten. Try again.");
}

#[test]
fn a_restore_stops_when_the_current_file_cant_be_kept() {
    let mut h = plan_versions();
    restore_newest(&mut h);
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(507, Vec::new())));
    h.run();
    assert_eq!(h.state().backend.sent.iter().filter(|(_, r)| r.method == Method::Put).count(), 0, "nothing is written");
    h.get_by_label("Couldn't restore the version: couldn't keep the current file first (the server answered 507).");
    assert_eq!(h.get_all_by_label("RESTORE").count(), 2, "it can be tried again");
}

#[test]
fn a_restored_version_closes_the_sheet_and_says_so() {
    let mut h = plan_versions();
    restore_newest(&mut h);
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(204, Vec::new())));
    h.run();
    assert!(h.state().files.sheet.is_none());
    h.get_by_label("Restored the version from Today 09:00");
}

// ─── The details panel ──────────────────────────────────────────────────────────────────────────

const REPORT_HISTORY: &str = ".craftcloud/versions/Reports/report.docx/";

/// Reports with a folder, `report.docx` and `plan.txt`, and `report.docx` clicked.
fn report_details() -> App {
    let mut h = in_reports(&[folder("d2", "Drafts"), file("f1", "report.docx", 248_000), file("f2", "plan.txt", 8)]);
    h.state_mut().backend.wall = Some(crate::backend::WallClock { unix: 1_794_608_247.123, utc_offset_minutes: 0 });
    h.get_by_label("report.docx").click();
    h.run();
    h
}

fn details_shown(h: &App) -> bool {
    h.query_by_label("Close details").is_some()
}

#[test]
fn a_selected_file_shows_its_details() {
    let mut h = report_details();
    assert!(details_shown(&h));
    assert_eq!(h.get_all_by_label("report.docx").count(), 2, "the row and the panel's name");
    h.get_by_label("DOCX document · 248 KB · opens in WordCraft");
    h.get_by_label("Modified: Today 11:42");
    h.get_by_label("Location: My files/Reports");
    h.get_by_label("Owner: john");
    h.get_by_label("Shared: Not shared");
    // No "Open in WordCraft": the page that frames a web app isn't built.
    assert!(h.query_by_label_contains("OPEN IN").is_none());
    // Its kept versions: the newest two under the current file.
    assert_eq!(h.state().backend.last_to(&format!("/webdav/{REPORT_HISTORY}")).method, Method::Propfind);
    h.get_by_label("Loading…");
    let list = multistatus(REPORT_HISTORY, &[("20261111T080000.000Z", "h0-0", 5), ("20261112T091200.000Z", "h1-1", 6), (NEWEST, "h2-2", 7)]);
    h.state_mut().backend.answer(&format!("/webdav/{REPORT_HISTORY}"), Ok(HttpResponse::new(207, list)));
    h.run();
    h.get_by_label("CURRENT");
    h.get_by_label("Today 09:00");
    h.get_by_label("Yesterday 09:12");
    assert_eq!(h.get_all_by_label("RESTORE").count(), 2, "the oldest is in ALL VERSIONS…");
    // DOWNLOAD fetches the file (the meta line has one too); ALL VERSIONS… opens the history.
    assert_eq!(h.get_all_by_label("DOWNLOAD").count(), 2);
    h.get_all_by_label("DOWNLOAD").last().unwrap().click();
    h.run();
    assert_eq!(h.state().backend.downloads.last().map(|(_, n)| n.as_str()), Some("report.docx"));
    h.get_by_label("ALL VERSIONS…").click();
    h.run();
    h.get_by_label("Versions of report.docx");
}

#[test]
fn details_are_for_one_file_outside_the_trash() {
    let mut h = report_details();
    // A folder has none.
    h.get_by_label("Drafts").click();
    h.run();
    assert!(!details_shown(&h));
    // Nor have two files.
    h.get_by_label("Select report.docx").click();
    h.run();
    h.get_by_label("Select plan.txt").click();
    h.run();
    assert_eq!(h.state().files.selection.len(), 3);
    assert!(!details_shown(&h));
    h.get_by_label("Select Drafts").click();
    h.run();
    h.get_by_label("Select report.docx").click();
    h.run();
    assert!(details_shown(&h), "one file again");
    // Closed, they stay closed until a file is clicked again.
    h.get_by_label("Close details").click();
    h.run();
    assert!(!details_shown(&h));
    h.get_by_label("plan.txt").click();
    h.run();
    assert!(details_shown(&h));
    h.get_by_label("Text · 8 B · opens in WordCraft");
}

#[test]
fn the_menu_leaves_hidden_details_hidden() {
    let mut h = in_reports(&[file("f1", "report.docx", 248_000)]);
    h.get_by_label("More for report.docx").click();
    h.run();
    assert!(!details_shown(&h), "the page doesn't shift under the menu");
    h.get_by_label("Versions…");
}

#[test]
fn the_details_more_button_opens_the_files_menu() {
    let mut h = report_details();
    h.get_by_label("More actions").click();
    h.run();
    h.get_by_label("Rename…");
    h.get_by_label("Move to trash");
}

#[test]
fn the_details_star_stars_the_file() {
    let mut h = report_details();
    h.get_by_label("Star").click();
    h.run();
    assert_eq!(h.state().backend.last_to("/api/favorites/file/f1").method, Method::Post);
}

#[test]
fn a_narrow_page_has_no_details() {
    let mut h = report_details();
    h.set_size(egui::vec2(1100.0, 900.0));
    h.run();
    assert!(!details_shown(&h));
    h.set_size(egui::vec2(1440.0, 900.0));
    h.run();
    assert!(details_shown(&h));
}

#[test]
fn a_late_versions_answer_never_fills_another_files_details() {
    let mut h = report_details();
    h.get_by_label("plan.txt").click();
    h.run();
    // report.docx's answer comes after plan.txt was picked.
    let list = multistatus(REPORT_HISTORY, &[(NEWEST, "h2-2", 7)]);
    h.state_mut().backend.answer(&format!("/webdav/{REPORT_HISTORY}"), Ok(HttpResponse::new(207, list)));
    h.run();
    assert_eq!(h.query_all_by_label("RESTORE").count(), 0, "report.docx's versions stay out of plan.txt's details");
    h.get_by_label("Loading…");
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    h.get_by_label("No earlier versions yet.");
}

#[test]
fn restore_in_the_details_keeps_the_current_file_and_says_so() {
    let mut h = in_reports(&[file("f1", "plan.txt", 8)]);
    h.state_mut().backend.wall = Some(crate::backend::WallClock { unix: 1_794_608_247.123, utc_offset_minutes: 0 });
    h.get_by_label("plan.txt").click();
    h.run();
    let list = multistatus(HISTORY, &[("20261112T091200.000Z", "h1-1", 6), (NEWEST, "h2-2", 7)]);
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(207, list)));
    h.run();
    restore_newest(&mut h);
    h.get_by_label("RESTORING…");
    let copy = h.state().backend.last_to("/webdav/Reports/plan.txt");
    assert_eq!((copy.method, copy.header_value("Overwrite")), (Method::Copy, Some("F")));
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(201, Vec::new())));
    h.run();
    h.state_mut().backend.answer(&format!("/webdav/{HISTORY}"), Ok(HttpResponse::new(404, Vec::new())));
    h.run();
    let put = h.state().backend.last_to("/webdav/Reports/plan.txt");
    assert_eq!((put.method, put.header_value("If-Match")), (Method::Put, Some("\"cur-9\"")));
    h.state_mut().backend.answer("/webdav/Reports/plan.txt", Ok(HttpResponse::new(204, Vec::new())));
    h.run();
    h.get_by_label("Restored the version from Today 09:00");
}

#[test]
fn a_hostile_name_and_history_stay_inside_the_details() {
    let long = format!("{}.docx", "x".repeat(5000));
    let mut h = in_reports(&[file("f1", &long, u64::MAX)]);
    h.get_by_label(&long).click();
    h.run();
    assert!(details_shown(&h));
    let history = format!(".craftcloud/versions/Reports/{long}/");
    let odd = multistatus(&history, &[("not-a-stamp", "e", u64::MAX), ("../../etc", "e", 0)]);
    h.state_mut().backend.answer(&format!("/webdav/{history}"), Ok(HttpResponse::new(207, odd)));
    h.run();
    h.get_by_label("CURRENT");
}
