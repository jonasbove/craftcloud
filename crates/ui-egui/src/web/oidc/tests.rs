//! Single sign-on on the sign-in page, driven through the FakeBackend with recorded answers. The
//! identity provider itself is never reached: the tests stop at the navigation to it and start
//! again at the code it sends back.

use craftcloud_core::http::HttpResponse;
use egui_kittest::kittest::Queryable;

use super::super::tests::{INITIALIZED, app, fixture, ok, start};
use crate::web::{After, Page};

#[test]
fn a_configured_provider_gets_a_button_that_navigates_to_it() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, false);
    assert_eq!(h.state().page, Page::Login(After::Home));
    h.state_mut().backend.answer("/api/auth/oidc/providers", ok(&fixture("oidc-providers-on.json")));
    h.run();
    h.get_by_label("OR");
    h.get_by_label("USERNAME"); // the password form stays
    // As wide as the form, as in the v2 frame (the sheet is 480 wide with 24 margins).
    assert_eq!(h.get_by_label("SIGN IN WITH EXAMPLE ID").rect().width(), 432.0);
    h.get_by_label("SIGN IN WITH EXAMPLE ID").click();
    h.run();
    assert_eq!(h.state().backend.opened, ["http://127.0.0.1:8093/api/auth/oidc/authorize"], "a navigation, not a fetch");
    assert_eq!(h.state().backend.sent_to("/api/auth/oidc/authorize"), 0);
    h.get_by_label("Opening Example ID.");
}

#[test]
fn no_provider_no_button() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, false);
    h.state_mut().backend.answer("/api/auth/oidc/providers", ok(&fixture("oidc-providers-off.json")));
    h.run();
    assert!(h.query_by_label_contains("SIGN IN WITH").is_none());
    h.get_by_label("USERNAME");
}

#[test]
fn an_sso_only_server_hides_the_password_form() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, false);
    let body = String::from_utf8(fixture("oidc-providers-on.json"))
        .unwrap()
        .replace("\"password_login_enabled\": true", "\"password_login_enabled\": false");
    h.state_mut().backend.answer("/api/auth/oidc/providers", ok(body.as_bytes()));
    h.run();
    assert!(h.query_by_label("USERNAME").is_none());
    h.get_by_label("This server signs people in through its identity provider only.");
    assert_eq!(h.get_by_label("SIGN IN WITH EXAMPLE ID").rect().width(), 432.0);
}

#[test]
fn auto_redirect_goes_straight_to_the_provider() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, false);
    let body =
        String::from_utf8(fixture("oidc-providers-on.json")).unwrap().replace("\"auto_redirect_to_oidc\": false", "\"auto_redirect_to_oidc\": true");
    h.state_mut().backend.answer("/api/auth/oidc/providers", ok(body.as_bytes()));
    h.run();
    assert_eq!(h.state().backend.opened.len(), 1);
}

#[test]
fn the_code_from_the_provider_becomes_a_session() {
    let mut h = app("/login", None);
    h.state_mut().oidc_return(Some("one-time-code".into()), None);
    let req = h.state().backend.last_to("/api/auth/oidc/exchange");
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&req.body).unwrap(), serde_json::json!({"code": "one-time-code"}));
    start(&mut h, INITIALIZED, false);
    h.get_by_label("Checking the answer from your identity provider.");
    h.state_mut().backend.answer("/api/auth/oidc/exchange", ok(br#"{"access_token":"a","refresh_token":"r"}"#));
    h.run();
    h.state_mut().backend.answer("/api/auth/me", ok(&fixture("me.json")));
    h.run();
    assert_eq!(h.state().page, Page::Home);
}

#[test]
fn an_expired_code_says_so_on_the_sign_in_page() {
    let mut h = app("/login", None);
    h.state_mut().oidc_return(Some("stale".into()), None);
    start(&mut h, INITIALIZED, false);
    h.state_mut().backend.answer("/api/auth/oidc/exchange", Ok(HttpResponse::new(403, fixture("oidc-exchange-invalid.json"))));
    h.run();
    h.state_mut().backend.answer("/api/auth/oidc/providers", ok(&fixture("oidc-providers-on.json")));
    h.run();
    assert_eq!(h.state().page, Page::Login(After::Home));
    h.get_by_label("That single sign-on link expired (they last a minute). Try again.");
    assert!(h.state().backend.opened.is_empty(), "no automatic second round trip");
}

#[test]
fn a_provider_error_is_shown_in_words() {
    let mut h = app("/login", None);
    h.state_mut().oidc_return(None, Some("callback_denied".into()));
    start(&mut h, INITIALIZED, false);
    h.get_by_label_contains("The identity provider's answer was refused. Try again;");
    assert_eq!(h.state().backend.sent_to("/api/auth/oidc/exchange"), 0);
}

#[test]
fn every_way_back_from_the_provider_reads_as_words() {
    // OxiCloud's callback keys, and the ones the Caddyfile gives OxiCloud's raw error pages.
    for (key, words) in [
        ("callback_denied", "The identity provider's answer was refused."),
        ("callback_failed", "Single sign-on didn't complete. Try again"),
        ("provider_denied", "The identity provider didn't sign you in"),
        ("provider_error", "The identity provider reported an error."),
        ("provider_unreachable", "This server can't reach its identity provider right now."),
        ("email_not_verified_at_idp", "hasn't confirmed your email address"),
        ("auto_link_disabled", "can't be joined to this sign-in"),
        ("email_ambiguous", "More than one account here matches your email address."),
        ("something_new", "Single sign-on didn't complete (something_new)."),
    ] {
        let mut h = app("/login", None);
        h.state_mut().oidc_return(None, Some(key.into()));
        start(&mut h, INITIALIZED, false);
        h.state_mut().backend.answer("/api/auth/oidc/providers", ok(&fixture("oidc-providers-on.json")));
        h.run();
        h.get_by_label_contains(words);
        h.get_by_label("SIGN IN WITH EXAMPLE ID"); // and the way to try again
        assert!(h.state().backend.opened.is_empty(), "{key}: no automatic round trip");
    }
}

#[test]
fn a_dropped_session_cookie_is_explained() {
    let mut h = app("/", None);
    start(&mut h, INITIALIZED, false);
    let form = &mut h.state_mut().form;
    (form.name, form.password) = ("john".into(), "correct horse".into());
    h.run();
    h.get_by_label("SIGN IN").click();
    h.run();
    h.state_mut().backend.answer("/api/auth/login", ok(br#"{"access_token":"a","refresh_token":"r"}"#));
    h.run();
    h.state_mut().backend.answer("/api/auth/me", Ok(HttpResponse::new(401, Vec::new())));
    h.run();
    h.get_by_label_contains("this browser didn't keep the session");
}
