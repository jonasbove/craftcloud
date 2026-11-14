//! Renders the web pages to PNGs, for looking at the design by eye (not a pixel test), as the
//! desktop's `shots`: `CRAFTCLOUD_SHOTS=/some/dir cargo test -p craftcloud-ui-egui --features shots
//! -- --ignored web_shots` (`CRAFTCLOUD_SHOTS_ONLY=home,files` for some of them). Each page comes
//! at 1440×900 dark and light, and 390×844 (a phone).

use craftcloud_core::ServerUrl;
use craftcloud_core::http::HttpResponse;
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::{AdminTab, Page, WebApp};
use crate::backend::FakeBackend;

type H = Harness<'static, WebApp<FakeBackend>>;

const UNINIT: &[u8] = br#"{"initialized":false,"admin_count":0,"registration_allowed":false}"#;
const INIT: &[u8] = br#"{"initialized":true,"admin_count":1,"registration_allowed":false}"#;
const CONFIG: &[u8] = br#"{"version":"0.1.0","web_apps":false,"public_url":"https://cloud.example.org","enabled_apps":[]}"#;
const ROOT: &str = "root1";

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/oxicloud-0.10.0").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ok(body: &[u8]) -> Result<HttpResponse, String> {
    Ok(HttpResponse::new(200, body.to_vec()))
}

fn json_ok(v: &Value) -> Result<HttpResponse, String> {
    ok(v.to_string().as_bytes())
}

/// Answers `url_end` if the page asked for it.
fn soft(h: &mut H, url_end: &str, resp: Result<HttpResponse, String>) {
    if h.state().backend.sent.iter().any(|(_, r)| r.url.ends_with(url_end)) {
        h.state_mut().backend.answer(url_end, resp);
    }
    h.run();
}

fn visit(path: &str, code: Option<&str>, size: egui::Vec2, theme: egui::Theme) -> H {
    let server = ServerUrl::parse("https://cloud.example.org").unwrap();
    let app = WebApp::new(&egui::Context::default(), FakeBackend::default(), server, path, code.map(String::from));
    Harness::builder().with_size(size).with_pixels_per_point(1.0).with_theme(theme).build_ui_state(
        |ui, app: &mut WebApp<FakeBackend>| {
            // The page's whole area, as the browser canvas gives it (the harness adds a margin).
            let full = ui.ctx().content_rect();
            ui.scope_builder(egui::UiBuilder::new().max_rect(full), |ui| app.ui(ui));
        },
        app,
    )
}

fn start(h: &mut H, status: &[u8], signed_in: bool) {
    let b = &mut h.state_mut().backend;
    b.answer("/api/auth/status", ok(status));
    b.answer("/craftcloud/config.json", ok(CONFIG));
    let me = if signed_in { ok(&fixture("me.json")) } else { Ok(HttpResponse::new(401, Vec::new())) };
    b.answer("/api/auth/me", me);
    h.run();
}

fn folder(id: &str, name: &str) -> Value {
    json!({"resource_type": "folder", "resource": {"id": id, "name": name, "path": "", "parent_id": ROOT, "modified_at": "2026-11-12T09:10:00Z", "etag": id}})
}

fn file(id: &str, name: &str, size: u64, mime: &str, at: &str) -> Value {
    json!({"resource_type": "file", "resource": {"id": id, "name": name, "path": format!("Personal/{name}"), "size": size, "mime_type": mime,
        "folder_id": ROOT, "modified_at": at, "category": "Document", "etag": format!("e-{id}")}})
}

fn listing(items: &[Value]) -> Value {
    json!({"items": items, "next_cursor": null})
}

fn sample() -> Vec<Value> {
    vec![
        folder("d1", "Design"),
        folder("d2", "Documents"),
        folder("d3", "Projects"),
        file("f1", "Budget 2027.xlsx", 91_000, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "2026-11-13T13:58:00Z"),
        file("f2", "Ideas.md", 3_000, "text/markdown", "2026-11-13T14:32:00Z"),
        file("f3", "Poster.psd", 42_100_000, "image/vnd.adobe.photoshop", "2026-11-12T16:20:00Z"),
        file("f4", "Q3 report.docx", 248_000, "application/vnd.openxmlformats-officedocument.wordprocessingml.document", "2026-11-10T09:05:00Z"),
    ]
}

/// Signed in, on Files, the root listing answered with `items`.
fn files(size: egui::Vec2, theme: egui::Theme, items: &[Value]) -> H {
    let mut h = visit("/files", None, size, theme);
    // 2026-11-14 10:00 UTC, viewed at UTC+1.
    h.state_mut().backend.wall = Some(crate::backend::WallClock { unix: 1_794_650_400.0, utc_offset_minutes: 60 });
    start(&mut h, INIT, true);
    let b = &mut h.state_mut().backend;
    b.answer("/api/folders", json_ok(&json!([{"id": ROOT, "name": "Personal", "path": "/Personal", "parent_id": null}])));
    b.answer("/craftcloud/config.json", ok(CONFIG));
    h.run();
    let b = &mut h.state_mut().backend;
    b.answer(&format!("/api/folders/{ROOT}/resources?limit=200"), json_ok(&listing(items)));
    b.answer(&format!("/api/folders/{ROOT}/ancestors"), json_ok(&json!({"ancestors": [{"id": ROOT, "name": "Personal"}]})));
    h.run();
    h
}

fn click(h: &mut H, label: &str) {
    use egui_kittest::kittest::Queryable;
    h.get_by_label(label).click();
    h.run();
}

/// Clicks a toolbar button, labelled in caps on a wide page and in words as an icon on a phone.
fn click_tool(h: &mut H, label: &str) {
    use egui_kittest::kittest::Queryable;
    match h.query_by_label(&label.to_uppercase()) {
        Some(node) => node.click(),
        None => h.get_by_label(label).click(),
    }
    h.run();
}

/// Two items named "notes.md" from different folders, deleted on different days (T-048).
fn trash_items() -> Value {
    let item = |id: &str, name: &str, path: &str, trashed: &str, expires: &str| {
        json!({"resource_type": "file", "trashed_at": trashed, "expires_at": expires, "resource": {"id": id, "name": name, "path": path,
            "size": 3_000, "mime_type": "text/markdown", "folder_id": "gone", "modified_at": trashed, "category": "Markdown", "etag": id}})
    };
    json!({"items": [
        item("t1", "notes.md", "Personal/Projects/notes.md", "2026-11-02T08:41:00Z", "2026-12-02T08:41:00Z"),
        item("t2", "Old poster.psd", "Personal/Design/Old poster.psd", "2026-11-10T15:12:00Z", "2026-12-10T15:12:00Z"),
        item("t3", "notes.md", "Personal/Documents/notes.md", "2026-11-13T21:54:00Z", "2026-12-13T21:54:00Z"),
    ], "next_cursor": null})
}

fn local(key: u64, name: &str, size: u64) -> crate::backend::LocalFile {
    crate::backend::LocalFile { key, name: name.into(), size, mime: String::new() }
}

/// Files with "Q3 report.docx" selected.
fn files_with_selection(s: egui::Vec2, t: egui::Theme) -> H {
    let mut h = files(s, t, &sample());
    click(&mut h, "Q3 report.docx");
    h
}

/// The share sheet for "Q3 report.docx", opened from its menu.
fn share_sheet(s: egui::Vec2, t: egui::Theme) -> H {
    let mut h = files(s, t, &sample());
    click(&mut h, "More for Q3 report.docx");
    click(&mut h, "Share link…");
    h
}

/// The share sheet with a password typed (hidden: screenshots never show one).
fn share_with_password(s: egui::Vec2, t: egui::Theme) -> H {
    let mut h = share_sheet(s, t);
    if let Some(super::files::Sheet::Share(sheet)) = &mut h.state_mut().files.sheet {
        sheet.password.0 = "x".repeat(16);
    }
    h.run();
    h
}

/// "Link ready" for "Q3 report.docx", seven days, with a password.
fn share_ready(s: egui::Vec2, t: egui::Theme) -> H {
    let mut h = share_with_password(s, t);
    click(&mut h, "CREATE LINK");
    let made = json!({"id": "s1", "item_id": "f4", "item_name": "Q3 report.docx", "item_type": "file", "token": "aeddf06a-4200-4b05-ad1c-272cf305e350",
        "url": "https://cloud.example.org/s/aeddf06a-4200-4b05-ad1c-272cf305e350", "has_password": true, "expires_at": "2026-11-21T10:00:00Z",
        "created_at": "2026-11-14T10:00:00Z", "access_count": 0});
    soft(&mut h, "/api/shares", Ok(HttpResponse::new(201, made.to_string().into_bytes())));
    h
}

/// The device approval page, after `button` and the server's `status` answer.
fn device_answered(s: egui::Vec2, t: egui::Theme, button: &str, status: &str) -> H {
    let mut h = visit("/device", Some("7KQ-4XM"), s, t);
    start(&mut h, INIT, true);
    soft(&mut h, "/api/auth/device/verify?code=7KQ%2D4XM", ok(&fixture("device-verify-info.json")));
    click(&mut h, button);
    soft(&mut h, "/api/auth/device/verify", json_ok(&json!({ "status": status })));
    h
}

type Scene = (&'static str, fn(egui::Vec2, egui::Theme) -> H);

fn setup_at(step: u8) -> fn(egui::Vec2, egui::Theme) -> H {
    match step {
        1 => |s, t| {
            let mut h = visit("/", None, s, t);
            start(&mut h, UNINIT, false);
            h
        },
        2 => |s, t| {
            let mut h = visit("/", None, s, t);
            start(&mut h, UNINIT, false);
            click(&mut h, "START SETUP");
            h
        },
        n => {
            let f: fn(egui::Vec2, egui::Theme) -> H = match n {
                3 => |s, t| signed_in_setup(s, t, 3),
                4 => |s, t| signed_in_setup(s, t, 4),
                _ => |s, t| signed_in_setup(s, t, 5),
            };
            f
        }
    }
}

fn signed_in_setup(size: egui::Vec2, theme: egui::Theme, step: u8) -> H {
    let mut h = visit("/", None, size, theme);
    start(&mut h, INIT, true);
    if step == 4 {
        h.state_mut().open_web_apps_step();
        h.run();
        soft(&mut h, "/craftcloud/apps.json", ok(CATALOG.as_bytes()));
        soft(&mut h, "/craftcloud/api/settings", ok(SETTINGS));
    } else {
        h.state_mut().page = Page::Setup(step);
    }
    h.run();
    h
}

const CATALOG: &str = r##"{"apps":[
 {"id":"wordcraft","name":"WordCraft","tagline":"Writing and document design","color":"#3f5fe0","ink":["#8ea2ff","#2a44b8"],"version":"0.18.1","bridge":"native","default":["docx","docm","dotx","dotm"]},
 {"id":"gridcraft","name":"GridCraft","tagline":"Spreadsheets and data","color":"#1f9d55","ink":["#4fd68a","#12703a"],"version":"0.19.1","bridge":"native","default":["xlsx","xlsm","xls","xlsb"]},
 {"id":"deckcraft","name":"DeckCraft","tagline":"Presentations and slide shows","color":"#f07a1e","ink":["#ffa060","#a84f0a"],"version":"0.18.1","bridge":"native","default":["deckcraft","pptx","potx","ppsx"]},
 {"id":"pdfcraft","name":"PdfCraft","tagline":"Reading, organizing and protecting PDFs","color":"#e8204f","ink":["#f23a62","#b70228"],"version":"0.21.1","bridge":"native","default":["pdf"]},
 {"id":"figcraft","name":"FigCraft","tagline":"Interface design and prototyping","color":"#02da96","ink":["#4fe8b6","#047a56"],"version":"0.18.1","bridge":"native","default":["figcraft","fig","jam","penpot"]},
 {"id":"cadcraft","name":"CADCraft","tagline":"Computer-aided design and drafting","color":"#3ea6e8","ink":["#7cc8ff","#1a6fa8"],"version":"0.20.1","bridge":"native","default":["dxf","dwg"]},
 {"id":"filmcraft","name":"FilmCraft","tagline":"Video editing, color and sound","color":"#8b5cf6","ink":["#b69cff","#6a3fd6"],"version":"0.20.1","bridge":"native","default":["fcproj","mp4","mov","m4v"]},
 {"id":"lightcraft","name":"LightCraft","tagline":"Photo library and raw development","color":"#f2a516","ink":["#ffc14d","#8a5800"],"version":"0.21.1","bridge":"lightcraft","default":["dng","cr2","cr3","nef"]},
 {"id":"vectorcraft","name":"VectorCraft","tagline":"Vector illustration","color":"#e8573f","ink":["#ff8a70","#b83a24"],"version":"0.24.1","bridge":"shim","default":["vectorcraft","drawcraft","svg","svgz"]},
 {"id":"effectcraft","name":"EffectCraft","tagline":"Motion graphics and visual effects","color":"#e0368f","ink":["#ff7ab8","#b0206c"],"version":"0.24.1","bridge":"shim","default":["ecproj","aep","aepx"]},
 {"id":"photocraft","name":"PhotoCraft","tagline":"Image editing","color":"#2f7bf5","ink":["#7fb0ff","#1a5bc4"],"version":"0.23.1","bridge":"shim","default":["pcraft","psd","psb","ora"]},
 {"id":"designcraft","name":"DesignCraft","tagline":"Page layout and publishing","color":"#7bb51c","ink":["#a6e04a","#457008"],"version":"0.20.1","bridge":"shim","default":["designcraft","idml","indd","indt"]},
 {"id":"soundcraft","name":"SoundCraft","tagline":"Recording, editing and mixing audio","color":"#1fa5c9","ink":["#5fd0ee","#0e7290"],"version":"0.20.1","bridge":"shim","default":["scraft","scraftz","wav","flac"]},
 {"id":"texturecraft","name":"TextureCraft","tagline":"3D texture painting","color":"#c2672b","ink":["#e58a4e","#9c4a15"],"version":"0.6.1","bridge":"shim","default":["tcraft","glb","gltf","fbx"]},
 {"id":"materialcraft","name":"MaterialCraft","tagline":"Procedural material design","color":"#178a9c","ink":["#63c5d3","#106573"],"version":"0.4.1","bridge":"shim","default":["mcraft","sbs"]}]}"##;

const SETTINGS: &[u8] = br#"{"web_apps":true,"disabled_apps":["lightcraft","vectorcraft","designcraft","cadcraft","filmcraft","effectcraft","soundcraft","texturecraft","materialcraft"],"bundled_apps":["wordcraft","gridcraft","deckcraft","pdfcraft","figcraft","cadcraft","filmcraft","lightcraft","vectorcraft","effectcraft","photocraft","designcraft","soundcraft","texturecraft","materialcraft"]}"#;

fn scenes() -> Vec<Scene> {
    vec![
        ("setup-1", setup_at(1)),
        ("setup-2", setup_at(2)),
        ("setup-3", setup_at(3)),
        ("setup-4", setup_at(4)),
        ("setup-5", setup_at(5)),
        ("login", |s, t| {
            let mut h = visit("/", None, s, t);
            start(&mut h, INIT, false);
            h
        }),
        ("login-device", |s, t| {
            let mut h = visit("/device", Some("7KQ-4XM"), s, t);
            start(&mut h, INIT, false);
            h
        }),
        ("device", |s, t| {
            let mut h = visit("/device", Some("7KQ-4XM"), s, t);
            start(&mut h, INIT, true);
            soft(&mut h, "/api/auth/device/verify?code=7KQ%2D4XM", ok(&fixture("device-verify-info.json")));
            h
        }),
        ("device-code", |s, t| {
            let mut h = visit("/device", None, s, t);
            start(&mut h, INIT, true);
            h
        }),
        ("device-connected", |s, t| device_answered(s, t, "APPROVE", "approved")),
        ("device-denied", |s, t| device_answered(s, t, "DENY", "denied")),
        ("files-upload", |s, t| {
            let mut h = files(s, t, &sample());
            h.state_mut().backend.to_pick = vec![local(7, "Q4 plan.docx", 248_000), local(8, "Moodboard.png", 6_400_000), local(9, "notes.md", 11)];
            click_tool(&mut h, "Upload");
            h.run();
            let sent: Vec<(u64, String)> = h.state().backend.sent.iter().map(|(id, r)| (*id, r.url.clone())).collect();
            for (id, url) in sent {
                if url.ends_with("Q4%20plan.docx") {
                    h.state_mut().backend.progress.push((id, 124_000));
                }
            }
            soft(&mut h, "/webdav/notes.md", Ok(HttpResponse::new(201, Vec::new())));
            h
        }),
        ("files-upload-taken", |s, t| {
            let mut h = files(s, t, &sample());
            h.state_mut().backend.to_pick = vec![local(1, "Ideas.md", 3_100)];
            click_tool(&mut h, "Upload");
            h.run();
            soft(&mut h, "/webdav/Ideas.md", Ok(HttpResponse::new(412, Vec::new())));
            h
        }),
        ("files-new-folder", |s, t| {
            let mut h = files(s, t, &sample());
            click_tool(&mut h, "New folder");
            h
        }),
        ("files-rename", |s, t| {
            let mut h = files_with_selection(s, t);
            h.key_press(egui::Key::F2);
            h.run();
            h
        }),
        ("files-move", |s, t| {
            let mut h = files_with_selection(s, t);
            click_tool(&mut h, "Move");
            soft(&mut h, &format!("/api/folders/{ROOT}/resources?limit=200"), json_ok(&listing(&sample()[..3])));
            h
        }),
        ("files-menu", |s, t| {
            let mut h = files(s, t, &sample());
            click(&mut h, "More for Q3 report.docx");
            h
        }),
        ("files-share-create", share_with_password),
        ("files-share-create-date", |s, t| {
            let mut h = share_sheet(s, t);
            click(&mut h, "ON A DATE…");
            click(&mut h, "28 Nov 2026");
            click(&mut h, "Pick a day");
            h
        }),
        ("files-share-create-error", |s, t| {
            let mut h = share_with_password(s, t);
            click(&mut h, "CREATE LINK");
            soft(&mut h, "/api/shares", Ok(HttpResponse::new(403, br#"{"error":"Access denied"}"#.to_vec())));
            h
        }),
        ("files-share-created", share_ready),
        ("files-share-created-copied", |s, t| {
            let mut h = share_ready(s, t);
            click(&mut h, "COPY");
            h
        }),
        ("home", |s, t| {
            let mut h = visit("/", None, s, t);
            start(&mut h, INIT, true);
            soft(&mut h, "/api/auth/device/devices", ok(&fixture("device-devices.json")));
            h
        }),
        ("home-loading", |s, t| {
            let mut h = visit("/", None, s, t);
            start(&mut h, INIT, true);
            h
        }),
        ("files", |s, t| files(s, t, &sample())),
        ("files-empty", |s, t| files(s, t, &[])),
        ("files-recent", |s, t| {
            let mut h = files(s, t, &sample());
            click(&mut h, "RECENT");
            let items: Vec<Value> = sample()
                .into_iter()
                .filter(|v| v["resource_type"] == "file")
                .enumerate()
                .map(|(i, v)| json!({"resource_type": "file", "resource": v["resource"], "accessed_at": format!("2026-11-13T1{}:00:00Z", 4 - i)}))
                .collect();
            soft(&mut h, "/api/recent/resources?limit=100", json_ok(&listing(&items)));
            h
        }),
        ("files-starred", |s, t| {
            let mut h = files(s, t, &sample());
            click(&mut h, "STARRED");
            soft(&mut h, "/api/favorites/resources?limit=200", ok(&fixture("favorites.json")));
            h
        }),
        ("files-trash", |s, t| {
            let mut h = files(s, t, &sample());
            click(&mut h, "TRASH");
            soft(&mut h, "/api/trash/resources?limit=200", json_ok(&trash_items()));
            h
        }),
        ("files-loading", |s, t| {
            let mut h = visit("/files", None, s, t);
            start(&mut h, INIT, true);
            h
        }),
        ("share-password", |s, t| {
            let mut h = visit("/s/tok1", None, s, t);
            start(&mut h, INIT, false);
            soft(&mut h, "/api/s/tok1", Ok(HttpResponse::new(401, br#"{"error":"Password required","requiresPassword":true}"#.to_vec())));
            h
        }),
        ("share-expired", |s, t| {
            let mut h = visit("/s/old", None, s, t);
            start(&mut h, INIT, false);
            soft(&mut h, "/api/s/old", Ok(HttpResponse::new(410, Vec::new())));
            h
        }),
        ("share-file", |s, t| {
            let mut h = visit("/s/tok1", None, s, t);
            start(&mut h, INIT, false);
            soft(&mut h, "/api/s/tok1", ok(&fixture("share-create.json")));
            let item = "6d728410%2D1791%2D454c%2Db27f%2Ddf91ebe60264";
            soft(&mut h, &format!("/api/files/{item}?metadata=true"), ok(&fixture("file-upload.json")));
            h
        }),
        ("share-subfolder", |s, t| {
            let mut h = visit("/s/tokF", None, s, t);
            start(&mut h, INIT, false);
            let share = br#"{"id":"s1","item_id":"top","item_name":"Team","item_type":"folder","token":"tokF","url":"https://cloud.example.org/s/tokF","has_password":false,"created_at":"2026-11-13T09:00:00Z"}"#;
            soft(&mut h, "/api/s/tokF", ok(share));
            soft(&mut h, "/api/folders/top/resources?limit=200", json_ok(&listing(&[folder("sub", "Photos"), sample()[3].clone()])));
            click(&mut h, "Open Photos");
            soft(&mut h, "/api/folders/sub/resources?limit=200", json_ok(&listing(&sample()[4..])));
            h
        }),
        ("admin-users", |s, t| {
            let mut h = visit("/admin/users", None, s, t);
            start(&mut h, INIT, true);
            soft(&mut h, "/api/admin/users?limit=500&offset=0", ok(&fixture("admin-users-three.json")));
            h
        }),
        ("admin-server", |s, t| {
            let mut h = visit("/admin/server", None, s, t);
            start(&mut h, INIT, true);
            assert_eq!(h.state().page, Page::Admin(AdminTab::Server));
            soft(&mut h, "/craftcloud/apps.json", ok(CATALOG.as_bytes()));
            soft(&mut h, "/craftcloud/api/settings", ok(SETTINGS));
            h
        }),
    ]
}

#[test]
#[ignore = "renders PNGs for review; needs a GPU"]
fn web_shots() {
    let Some(dir) = std::env::var_os("CRAFTCLOUD_SHOTS") else { panic!("set CRAFTCLOUD_SHOTS to a folder") };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let only = std::env::var("CRAFTCLOUD_SHOTS_ONLY").ok();
    for (name, make) in scenes() {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n == name)) {
            continue;
        }
        for (suffix, size, theme) in [
            ("", egui::vec2(1440.0, 900.0), egui::Theme::Dark),
            ("-light", egui::vec2(1440.0, 900.0), egui::Theme::Light),
            ("-phone", egui::vec2(390.0, 844.0), egui::Theme::Dark),
        ] {
            let mut h = make(size, theme);
            h.run_steps(4);
            let image = h.render().unwrap_or_else(|e| panic!("render {name}: {e}"));
            let path = dir.join(format!("web-{name}{suffix}.png"));
            image.save(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }
}
