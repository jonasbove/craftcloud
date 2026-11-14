//! "Connect to CraftCloud": the server address step (design frame `v2-desktop-connect`).

use craftcloud_core::{ServerUrl, ServerUrlError};
use egui::Ui;

use crate::chrome::{self, Icon, Kind};
use crate::theme::Tokens;

/// What the person asked for on this screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectAction {
    /// Continue with this server.
    Continue(ServerUrl),
}

#[derive(Default)]
pub struct ConnectScreen {
    pub address: String,
    error: Option<ServerUrlError>,
    /// Typing goes into the field from the start: it asks for focus when first shown.
    focused: bool,
}

impl ConnectScreen {
    pub fn new(address: impl Into<String>) -> Self {
        Self { address: address.into(), error: None, focused: false }
    }

    /// The address field with its hint and any error. Enter in the field submits.
    pub fn body(&mut self, ui: &mut Ui) -> Option<ConnectAction> {
        let t = Tokens::get(ui.ctx());
        let field = chrome::field(
            ui,
            "Server address",
            &mut self.address,
            false,
            Some("Include http:// or https://. Your admin has it, or the web app's home page."),
        );
        if field.changed() {
            self.error = None;
        }
        if !std::mem::replace(&mut self.focused, true) {
            field.request_focus();
        }
        let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some(error) = &self.error {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(error.to_string()).size(13.0).color(t.bad));
        }
        if submitted { self.submit() } else { None }
    }

    /// Whether there is anything to continue with.
    pub fn ready(&self) -> bool {
        !self.address.trim().is_empty()
    }

    /// Checks the address: the server to continue with, or the error shown under the field.
    pub fn submit(&mut self) -> Option<ConnectAction> {
        match ServerUrl::parse(&self.address) {
            Ok(url) => Some(ConnectAction::Continue(url)),
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// The field and a CONTINUE button, on their own.
    pub fn ui(&mut self, ui: &mut Ui) -> Option<ConnectAction> {
        let mut action = self.body(ui);
        ui.add_space(12.0);
        if chrome::button(ui, "Continue", Kind::Primary, Some(Icon::Arrow), self.ready()).clicked() {
            action = self.submit();
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectAction, ConnectScreen};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn harness(address: &str) -> Harness<'static, (ConnectScreen, Option<ConnectAction>)> {
        Harness::new_ui_state(
            |ui, (screen, action): &mut (ConnectScreen, Option<ConnectAction>)| {
                if let Some(a) = screen.ui(ui) {
                    *action = Some(a);
                }
            },
            (ConnectScreen::new(address), None),
        )
    }

    #[test]
    fn continue_with_a_valid_address() {
        let mut h = harness("cloud.example.org");
        h.get_by_label("CONTINUE").click();
        h.run();
        let (_, action) = h.state();
        let Some(ConnectAction::Continue(url)) = action else { panic!("no action: {action:?}") };
        assert_eq!(url.as_str(), "https://cloud.example.org");
    }

    #[test]
    fn typing_goes_into_the_address_field_at_once() {
        let mut h = harness("");
        h.run();
        assert!(h.ctx.memory(|m| m.focused().is_some()), "the address field has the focus");
        h.event(egui::Event::Text("cloud.example.org".into()));
        h.run();
        assert_eq!(h.state().0.address, "cloud.example.org");
    }

    #[test]
    fn an_invalid_address_shows_why() {
        let mut h = harness("ftp://cloud.example.org");
        h.get_by_label("CONTINUE").click();
        h.run();
        assert!(h.state().1.is_none());
        h.get_by_label("Use an http:// or https:// address, not ftp://.");
    }
}
