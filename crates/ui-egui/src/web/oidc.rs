//! Single sign-on on the sign-in page (OxiCloud's OpenID Connect support).
//!
//! The sign-in sheet asks `/api/auth/oidc/providers` whether a provider is set up and, if so,
//! offers "Sign in with <name>": a navigation to `/api/auth/oidc/authorize`, which OxiCloud answers
//! with a redirect to the provider. The provider sends the browser back through OxiCloud's
//! callback to `/login?oidc_code=…` (or `?login_error=…`), which is this frontend again: the web app
//! gets the code from its address ([`WebApp::oidc_return`]) and swaps it for a session.
//! Caddy passes both `/api/auth/oidc/*` (the `/api/*` allow-list) and `/login` (the frontend).

use craftcloud_core::api::{ApiError, auth, oidc};
use egui::{Rect, RichText, Ui};

use super::{Page, WebApp};
use crate::backend::{Backend, Net, NetResult};
use crate::chrome::{self, Icon, Kind};
use crate::theme::Tokens;
use crate::widgets::error_line;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Providers,
    Exchange,
}

#[derive(Default)]
pub(super) struct Sso {
    pub(super) net: Net<Key>,
    pub(super) providers: Option<oidc::Providers>,
    /// The one-time code from `/login?oidc_code=…`, until it is exchanged.
    code: Option<String>,
    pub(super) error: Option<String>,
    /// The browser is on its way to the provider.
    leaving: bool,
}

impl Sso {
    /// A provider is set up and switched on.
    pub(super) fn offered(&self) -> Option<&oidc::Providers> {
        self.providers.as_ref().filter(|p| p.enabled)
    }

    /// Whether the password form shows (it does until the server says otherwise).
    pub(super) fn password_login(&self) -> bool {
        self.providers.as_ref().is_none_or(|p| p.password_login_enabled || !p.enabled)
    }

    pub(super) fn exchanging(&self) -> bool {
        self.code.is_some() || self.net.is_waiting(&Key::Exchange)
    }
}

impl<B: Backend> WebApp<B> {
    /// What the address carried back from the identity provider: `oidc_code` and `login_error`
    /// of `/login?…`. Call once, right after [`WebApp::new`].
    pub fn oidc_return(&mut self, code: Option<String>, error: Option<String>) {
        if let Some(e) = error.filter(|e| !e.is_empty()) {
            self.sso.error = Some(oidc::login_error_text(&e));
        }
        if let Some(code) = code.filter(|c| !c.is_empty()) {
            let req = oidc::exchange(&self.server, &code);
            self.sso.code = Some(code);
            self.sso.net.send(&mut self.backend, Key::Exchange, req);
        }
    }

    /// Asks whether the server offers single sign-on (once, when the sign-in page shows).
    pub(super) fn sso_check(&mut self) {
        if self.sso.providers.is_none() && !self.sso.net.is_waiting(&Key::Providers) {
            let req = oidc::providers(&self.server);
            self.sso.net.send(&mut self.backend, Key::Providers, req);
        }
    }

    pub(super) fn on_sso_answer(&mut self, key: Key, result: NetResult) {
        let resp = match result {
            Ok(resp) => resp,
            Err(why) => {
                if key == Key::Exchange {
                    self.sso.code = None;
                    self.sso.error = Some(why);
                }
                // Without an answer to the providers question, the page keeps the password form.
                return;
            }
        };
        match key {
            Key::Providers => {
                // A server without OIDC support answers 404/503: no button.
                let providers = oidc::parse_providers(&resp).unwrap_or(oidc::Providers {
                    enabled: false,
                    provider_name: String::new(),
                    authorize_endpoint: String::new(),
                    password_login_enabled: true,
                    auto_redirect_to_oidc: false,
                });
                let go = providers.enabled
                    && providers.auto_redirect_to_oidc
                    && self.sso.error.is_none()
                    && self.sso.code.is_none()
                    && self.me.is_none()
                    && matches!(self.page, Page::Login(_));
                self.sso.providers = Some(providers);
                if go {
                    self.sso_go();
                }
            }
            Key::Exchange => {
                self.sso.code = None;
                match craftcloud_core::api::expect_ok(&resp) {
                    // The cookies are set: ask who this is, which moves on like a password sign-in.
                    Ok(()) => self.send(super::Key::Me, auth::me(&self.server)),
                    Err(ApiError::Status { status: 401 | 403, .. }) => {
                        self.sso.error = Some("That single sign-on link expired (they last a minute). Try again.".into());
                    }
                    Err(e) => self.sso.error = Some(e.to_string()),
                }
            }
        }
    }

    /// Off to the identity provider.
    fn sso_go(&mut self) {
        if let Some(p) = self.sso.offered() {
            let url = oidc::authorize_url(&self.server, p);
            self.sso.leaving = true;
            self.backend.open_url(&url);
        }
    }

    /// While the one-time code is swapped for a session: a small sheet instead of the form.
    /// Returns `true` when it drew the page.
    pub(super) fn sso_busy_ui(&mut self, ui: &mut Ui, area: Rect) -> bool {
        if !self.sso.exchanging() && !self.sso.leaving {
            return false;
        }
        let name = self.sso.offered().map_or("single sign-on", |p| p.display_name()).to_owned();
        let caption = if self.sso.leaving { format!("To {name}") } else { "Single sign-on".to_owned() };
        chrome::sheet(
            ui,
            area,
            egui::vec2(480.0, 260.0),
            &chrome::Hero { title: "Signing you in…", caption: &caption, keep_case: false },
            |ui, part| {
                if part == chrome::Part::Body {
                    let t = Tokens::get(ui.ctx());
                    let text =
                        if self.sso.leaving { format!("Opening {name}.") } else { "Checking the answer from your identity provider.".to_owned() };
                    ui.label(RichText::new(text).size(14.0).color(t.muted));
                }
            },
        );
        true
    }

    /// The single sign-on part of the sign-in sheet: the button, and the "or" rule above the
    /// password form when both are offered.
    pub(super) fn sso_sign_in_ui(&mut self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        if let Some(e) = self.sso.error.clone() {
            error_line(ui, &e);
            ui.add_space(10.0);
        }
        let Some(p) = self.sso.offered() else { return };
        let label = format!("Sign in with {}", p.display_name());
        let password = self.sso.password_login();
        let busy = self.net.is_busy();
        let full = ui.available_width();
        let clicked = ui
            .scope(|ui| {
                ui.set_width(full);
                chrome::button(ui, &label, if password { Kind::Secondary } else { Kind::Primary }, Some(Icon::Arrow), !busy).clicked()
            })
            .inner;
        if clicked {
            self.sso.error = None;
            self.sso_go();
        }
        if password {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 40.0) / 2.0;
                let (l, _) = ui.allocate_exact_size(egui::vec2(w, 12.0), egui::Sense::hover());
                ui.painter().hline(l.x_range(), l.center().y, t.hairline());
                chrome::hud(ui, "or", t.muted);
                let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 12.0), egui::Sense::hover());
                ui.painter().hline(r.x_range(), r.center().y, t.hairline());
            });
            ui.add_space(10.0);
        } else {
            ui.add_space(10.0);
            ui.label(RichText::new("This server signs people in through its identity provider only.").size(12.5).color(t.muted));
        }
    }
}

#[cfg(test)]
mod tests;
