use craftcloud_core::http::HttpResponse;
use craftcloud_core::{RuntimeConfig, ServerUrl};
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::{app_url, offered};
use crate::backend::FakeBackend;
use crate::web::WebApp;
use crate::web::tests::{CONFIG, INITIALIZED, app, fixture, ok};

type H = Harness<'static, WebApp<FakeBackend>>;

fn config(web_apps: bool, enabled: &[&str]) -> RuntimeConfig {
    RuntimeConfig {
        version: "0.3.0".into(),
        web_apps,
        public_url: "https://cloud.example.org".into(),
        enabled_apps: enabled.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn ids(config: Option<&RuntimeConfig>, signed_in: bool) -> Vec<&'static str> {
    offered(config, signed_in).iter().map(|a| a.id).collect()
}

#[test]
fn the_menu_offers_the_apps_switched_on_to_people_signed_in() {
    let on = config(true, &["wordcraft", "gridcraft", "photocraft"]);
    assert_eq!(ids(Some(&on), true), ["wordcraft", "gridcraft", "photocraft"], "in the server's order");
    assert!(ids(Some(&on), false).is_empty(), "signed out: no menu");
    assert!(ids(None, true).is_empty(), "no config.json (or not a CraftCloud one): no menu");
}

#[test]
fn the_menu_is_gone_when_the_apps_are_off_or_none_is_on() {
    assert!(ids(Some(&config(false, &["wordcraft"])), true).is_empty(), "web apps off");
    assert!(ids(Some(&config(true, &[])), true).is_empty(), "web apps on, every app switched off");
}

#[test]
fn the_menu_skips_apps_it_does_not_know_and_lists_each_once() {
    let odd = config(true, &["nonesuch", "WordCraft", "wordcraft", " gridcraft "]);
    assert_eq!(ids(Some(&odd), true), ["wordcraft", "gridcraft"]);
}

#[test]
fn an_app_opens_at_its_folder_on_this_server() {
    let server = ServerUrl::parse("https://cloud.example.org").unwrap();
    assert_eq!(app_url(&server, "wordcraft"), "https://cloud.example.org/apps/wordcraft/");
}

const APPS_ON: &[u8] = br#"{"version": "0.3.0", "web_apps": true, "web_apps_bundled": true, "public_url": "http://127.0.0.1:8093", "enabled_apps": ["wordcraft", "gridcraft", "deckcraft", "pdfcraft", "figcraft", "photocraft"]}"#;

/// Home, signed in (or not), with this config.json.
fn home(config: &[u8], signed_in: bool) -> H {
    let mut h = app("/", None);
    let b = &mut h.state_mut().backend;
    b.answer("/api/auth/status", ok(INITIALIZED));
    b.answer("/craftcloud/config.json", ok(config));
    let me = if signed_in { ok(&fixture("me.json")) } else { Ok(HttpResponse::new(401, Vec::new())) };
    b.answer("/api/auth/me", me);
    h.run();
    if signed_in {
        h.state_mut().backend.answer("/api/auth/device/devices", ok(b"[]"));
        h.run();
    }
    h
}

fn expanded(h: &H) -> Option<bool> {
    h.get_by_label("APPS").accesskit_node().data().is_expanded()
}

#[test]
fn apps_shows_in_the_header_only_when_the_server_has_apps_on() {
    let h = home(APPS_ON, true);
    let apps = h.get_by_label("APPS");
    assert_eq!(apps.accesskit_node().role(), egui::accesskit::Role::Button, "a button, not a page tab");
    assert_eq!(expanded(&h), Some(false));

    assert!(home(CONFIG, true).query_by_label("APPS").is_none(), "web apps off");
    let none_on = br#"{"version": "0.3.0", "web_apps": true, "public_url": "http://127.0.0.1:8093", "enabled_apps": []}"#;
    assert!(home(none_on, true).query_by_label("APPS").is_none(), "no app switched on");
    assert!(home(APPS_ON, false).query_by_label("APPS").is_none(), "signed out");
}

#[test]
fn a_tile_opens_its_app_in_a_new_tab() {
    let mut h = home(APPS_ON, true);
    h.get_by_label("APPS").click();
    h.run();
    assert_eq!(expanded(&h), Some(true));
    for name in ["WordCraft", "GridCraft", "DeckCraft", "PdfCraft", "FigCraft", "PhotoCraft"] {
        let tile = h.get_by_label(name);
        assert_eq!(tile.accesskit_node().role(), egui::accesskit::Role::Link, "{name}");
        assert_eq!(tile.accesskit_node().description().as_deref(), Some("Opens in a new tab"), "{name}");
    }
    h.get_by_label("GridCraft").click();
    h.run();
    assert_eq!(h.state().backend.tabs, ["http://127.0.0.1:8093/apps/gridcraft/"]);
    assert!(h.query_by_label("GridCraft").is_none(), "the menu closes");
    assert_eq!(expanded(&h), Some(false));
}

#[test]
fn the_keyboard_opens_moves_through_and_closes_the_menu() {
    let mut h = home(APPS_ON, true);
    h.get_by_label("APPS").focus();
    h.run();
    assert!(h.get_by_label("APPS").is_focused());
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(expanded(&h), Some(true));
    assert!(h.get_by_label("WordCraft").is_focused(), "opening puts the focus on the first tile");

    h.key_press(Key::ArrowRight);
    h.run();
    assert!(h.get_by_label("GridCraft").is_focused());
    h.key_press(Key::ArrowDown);
    h.run();
    assert!(h.get_by_label("PhotoCraft").is_focused(), "down a row: four tiles to a row");
    h.key_press(Key::ArrowDown);
    h.run();
    assert!(h.get_by_label("PhotoCraft").is_focused(), "the focus stays in the menu at the last row");
    h.key_press(Key::ArrowUp);
    h.run();
    assert!(h.get_by_label("GridCraft").is_focused());
    h.key_press(Key::End);
    h.run();
    assert!(h.get_by_label("PhotoCraft").is_focused());
    h.key_press(Key::Home);
    h.run();
    assert!(h.get_by_label("WordCraft").is_focused());

    h.key_press(Key::Escape);
    h.run();
    assert!(h.query_by_label("WordCraft").is_none(), "Esc closes the menu");
    assert_eq!(expanded(&h), Some(false));
    assert!(h.get_by_label("APPS").is_focused(), "and puts the focus back on APPS");
    assert!(h.state().backend.tabs.is_empty());

    h.key_press(Key::Enter);
    h.run();
    h.key_press(Key::ArrowRight);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().backend.tabs, ["http://127.0.0.1:8093/apps/gridcraft/"], "Enter opens the focused app");
    assert!(h.query_by_label("GridCraft").is_none());
}

#[test]
fn the_apps_menu_and_the_account_menu_take_turns() {
    let mut h = home(APPS_ON, true);
    h.get_by_label("APPS").click();
    h.run();
    h.get_by_label("Account: john").click();
    h.run();
    assert!(h.query_by_label("WordCraft").is_none(), "the account menu closes the apps menu");
    assert!(h.query_by_label("Your share links").is_some());
    h.get_by_label("APPS").click();
    h.run();
    assert!(h.query_by_label("Your share links").is_none(), "and the other way round");
    assert!(h.query_by_label("WordCraft").is_some());
}
