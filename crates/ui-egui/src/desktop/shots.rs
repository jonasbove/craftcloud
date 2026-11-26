//! Renders every desktop screen to a PNG, for looking at the design by eye (not a pixel test):
//! `CRAFTCLOUD_SHOTS=/some/dir cargo test -p craftcloud-ui-egui --features shots -- --ignored shots`
//! (`CRAFTCLOUD_SHOTS_ONLY=activity,folders` for some of them).

use craftcloud_core::api::auth::{DeviceStart, Me, Tokens, User};
use craftcloud_core::sync::{ActivityEvent, ActivityKind, ClientSettings, ConflictCopy, FolderStats, SyncProgress, SyncState};
use egui_kittest::Harness;

use super::tests::FakeDesktop;
use super::{Choice, DesktopApp, Overlay, Screen, Tab};

type App = DesktopApp<FakeDesktop>;

fn settings() -> ClientSettings {
    ClientSettings {
        server: "https://cloud.example.org".into(),
        local_folder: "/home/john/CraftCloud".into(),
        excluded: vec!["Photos".into(), "Music".into()],
        sync_root_files: true,
        ..ClientSettings::default()
    }
}

fn signed_in() -> App {
    let mut app = DesktopApp::new(
        &egui::Context::default(),
        FakeDesktop::default(),
        settings(),
        Some(Tokens { access_token: "a".into(), refresh_token: "r".into() }),
    );
    app.me = Some(Me {
        user: User { id: "1".into(), username: "john".into(), email: "john@example.org".into(), role: "owner".into() },
        storage_used_bytes: 0,
        storage_quota_bytes: 0,
    });
    app
}

fn signed_out() -> App {
    DesktopApp::new(&egui::Context::default(), FakeDesktop::default(), ClientSettings::default(), None)
}

fn event(time: &str, kind: ActivityKind, path: &str, size: Option<u64>, detail: &str) -> ActivityEvent {
    ActivityEvent { at: 0, time: time.into(), date: "2026-11-13".into(), kind, path: path.into(), size, detail: detail.into() }
}

fn folder(name: &str, files: u64, bytes: u64) -> FolderStats {
    FolderStats { folder: name.into(), files, bytes, ..FolderStats::default() }
}

fn up_to_date() -> SyncProgress {
    let mut p = SyncProgress { state: SyncState::UpToDate, cycles: 3, last_synced: Some(1_794_569_880), ..SyncProgress::default() };
    for e in [
        event("11:47", ActivityKind::Up, "Projects/Studio/Floorplan.dwg", Some(3_400_000), ""),
        event("12:05", ActivityKind::Down, "Photos/2026/IMG_2041.CR3", Some(28_600_000), ""),
        event("13:41", ActivityKind::Deleted, "Finance/Old budget.xlsx", None, "to the cloud trash"),
        event("13:58", ActivityKind::Conflict, "Finance/Budget 2027.xlsx", Some(91_000), "kept both"),
        event("14:20", ActivityKind::Down, "Design/Poster.psd", Some(42_100_000), ""),
        event("14:32", ActivityKind::Up, "Notes/Ideas.md", Some(3_000), ""),
        event("14:32", ActivityKind::Up, "Documents/Reports/Q3 report.docx", Some(248_000), "previous version kept"),
    ] {
        p.record(e);
    }
    p.folders = vec![
        folder("Documents", 1_204, 2_100_000_000),
        folder("Design", 212, 840_000_000),
        folder("Projects", 96, 1_200_000_000),
        folder("Shared with me", 58, 340_000_000),
        folder("", 4, 2_000_000),
    ];
    p
}

fn conflict() -> ConflictCopy {
    ConflictCopy { original: "Finance/Budget 2027.xlsx".into(), copy: "Finance/Budget 2027 (conflict 2026-11-13 13.58 Studio).xlsx".into() }
}

fn cloud_folders() -> Vec<(String, bool)> {
    ["Documents", "Design", "Photos", "Projects", "Music", "Shared with me"]
        .iter()
        .map(|n| ((*n).to_owned(), !matches!(*n, "Photos" | "Music")))
        .collect()
}

fn with(state: SyncState) -> App {
    let mut app = signed_in();
    app.host.progress = SyncProgress { state, ..up_to_date() };
    app
}

type Scene = (&'static str, fn() -> App);

fn scenes() -> Vec<Scene> {
    vec![
        ("connect", || {
            let mut app = signed_out();
            app.screen = Screen::Connect(crate::connect::ConnectScreen::new("https://cloud.example.org"));
            app
        }),
        ("connect-error", || {
            let mut app = signed_out();
            app.screen = Screen::Connect(crate::connect::ConnectScreen::new("cloud.example.org"));
            app.error = Some("That address answers, but it isn't a CraftCloud server.".into());
            app
        }),
        ("browser", || {
            let mut app = signed_out();
            app.server = craftcloud_core::ServerUrl::parse("https://cloud.example.org").ok();
            let start = DeviceStart {
                device_code: "d".into(),
                user_code: "7KQ-4XM".into(),
                verification_uri: "https://cloud.example.org/device".into(),
                verification_uri_complete: None,
                expires_in: 900,
                interval: 5,
            };
            app.screen = Screen::Browser { start, interval: 5.0, next_poll: 1e9, until: 1e9, opened: true };
            app
        }),
        ("browser-failed", || {
            let mut app = signed_out();
            app.server = craftcloud_core::ServerUrl::parse("https://cloud.example.org").ok();
            let start = DeviceStart {
                device_code: "d".into(),
                user_code: "7KQ-4XM".into(),
                verification_uri: "https://cloud.example.org/device".into(),
                verification_uri_complete: Some("https://cloud.example.org/device?code=7KQ-4XM".into()),
                expires_in: 900,
                interval: 5,
            };
            app.screen = Screen::Browser { start, interval: 5.0, next_poll: 1e9, until: 1e9, opened: false };
            app
        }),
        ("choose", || {
            let mut app = signed_out();
            app.server = craftcloud_core::ServerUrl::parse("https://cloud.example.org").ok();
            app.screen = Screen::Choose {
                choice: Choice { folders: cloud_folders(), root_files: true },
                local: "/home/john/CraftCloud".into(),
                changing: false,
            };
            app
        }),
        ("activity", || with(SyncState::UpToDate)),
        // Colour themes: a dark-only one, a light-only one, and Tokyo Night (both variants).
        ("activity-theme-nord", || themed(with(SyncState::UpToDate), "craft.nord")),
        ("activity-theme-solarized-light", || themed(with(SyncState::UpToDate), "craft.solarized-light")),
        ("activity-theme-tokyo-night", || themed(with(SyncState::UpToDate), "craft.tokyo-night")),
        ("settings-theme-catppuccin", || {
            let mut app = themed(with(SyncState::UpToDate), "craft.catppuccin");
            app.overlay = Some(Overlay::Settings);
            app
        }),
        ("activity-syncing", || {
            let mut app = signed_in();
            app.host.progress = SyncProgress {
                state: SyncState::Downloading,
                files_done: 2,
                files_total: 12,
                current: "download Design/Poster.psd".into(),
                ..up_to_date()
            };
            app
        }),
        ("conflict", || {
            let mut app = signed_in();
            app.host.progress = SyncProgress { conflicts: vec![conflict()], ..up_to_date() };
            app
        }),
        ("conflict-sheet", || {
            let mut app = signed_in();
            app.host.progress = SyncProgress { conflicts: vec![conflict()], ..up_to_date() };
            app.overlay = Some(Overlay::Conflict(conflict().copy));
            app
        }),
        ("folders", || {
            let mut app = with(SyncState::UpToDate);
            app.screen = Screen::Main(Tab::Folders);
            app.editing = Some(Choice { folders: cloud_folders(), root_files: true });
            app
        }),
        ("folders-loading", || {
            let mut app = with(SyncState::UpToDate);
            app.screen = Screen::Main(Tab::Folders);
            app
        }),
        ("settings", || {
            let mut app = with(SyncState::UpToDate);
            app.overlay = Some(Overlay::Settings);
            app
        }),
        ("settings-new-version", || {
            let mut app = with(SyncState::UpToDate);
            app.host.new_version =
                Some(craftcloud_core::update::NewVersion { version: "0.3.0".into(), url: super::tests::RELEASE_PAGE.into(), package_managed: true });
            app.overlay = Some(Overlay::Settings);
            app
        }),
        ("activity-long-error", || {
            let mut app = signed_in();
            let mut p = up_to_date();
            p.record(event("14:40", ActivityKind::Error, "Weird/what?.md", None, super::tests::LONG_DETAIL));
            app.host.progress = p;
            app
        }),
        ("failed", || {
            with(SyncState::Failed(
                "Another CraftCloud (the desktop client or `craftcloud daemon`) is already syncing C:\\Users\\john\\CraftCloud.".into(),
            ))
        }),
        ("paused", || with(SyncState::Paused)),
        ("offline", || with(SyncState::Offline("connection refused".into()))),
        ("signed-out", || with(SyncState::SignedOut)),
        ("mass-delete", || with(SyncState::MassDelete { deletes: 42 })),
        ("folder-missing", || with(SyncState::FolderMissing)),
        ("empty", || {
            let mut app = signed_in();
            app.host.progress = SyncProgress { state: SyncState::UpToDate, ..SyncProgress::default() };
            app
        }),
        ("keychain", || {
            let mut app = signed_in();
            app.screen = Screen::Keychain;
            app
        }),
    ]
}

/// `app` with the colour theme `id` chosen.
fn themed(mut app: App, id: &str) -> App {
    app.settings.theme = id.to_owned();
    app
}

#[test]
#[ignore = "renders PNGs for review; needs a GPU"]
fn shots() {
    let Some(dir) = std::env::var_os("CRAFTCLOUD_SHOTS") else { panic!("set CRAFTCLOUD_SHOTS to a folder") };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let only = std::env::var("CRAFTCLOUD_SHOTS_ONLY").ok();
    for (name, make) in scenes() {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n == name)) {
            continue;
        }
        for (suffix, size, theme) in [
            ("", egui::vec2(960.0, 640.0), egui::Theme::Dark),
            ("-light", egui::vec2(960.0, 640.0), egui::Theme::Light),
            ("-narrow", egui::vec2(640.0, 480.0), egui::Theme::Dark),
        ] {
            let mut app = make();
            app.settings.appearance = match theme {
                egui::Theme::Dark => craftcloud_core::sync::Appearance::Dark,
                egui::Theme::Light => craftcloud_core::sync::Appearance::Light,
            };
            // The window's whole area, as eframe gives it (the harness adds a margin).
            // The craft-ui runtime as `main` installs it, before the first pass.
            let mut h = Harness::builder()
                .with_context_setup(crate::framework::install_headless)
                .with_size(size)
                .with_pixels_per_point(1.0)
                .with_theme(theme)
                .build_ui_state(
                    |ui, app: &mut App| {
                        let full = ui.ctx().content_rect();
                        ui.scope_builder(egui::UiBuilder::new().max_rect(full), |ui| app.ui(ui));
                    },
                    app,
                );
            crate::framework::before_input(&h.ctx);
            // Again once the app has set its appearance and palette, as eframe's `raw_input_hook` does every pass.
            h.run_steps(1);
            crate::framework::before_input(&h.ctx);
            h.run_steps(4);
            let image = h.render().unwrap_or_else(|e| panic!("render {name}: {e}"));
            let path = dir.join(format!("{name}{suffix}.png"));
            image.save(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }
}
