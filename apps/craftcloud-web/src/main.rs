//! CraftCloud's web frontend, served by the CraftCloud image in place of OxiCloud's pages.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
mod fetch;

#[cfg(target_arch = "wasm32")]
mod web {
    use craftcloud_core::ServerUrl;
    use craftcloud_ui_egui::web::WebApp;
    use wasm_bindgen::JsCast;

    use crate::fetch::FetchBackend;

    struct App {
        web: WebApp<FetchBackend>,
    }

    impl eframe::App for App {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            self.web.ui(ui);
        }
    }

    pub fn start() {
        eframe::WebLogger::init(log::LevelFilter::Warn).ok();
        wasm_bindgen_futures::spawn_local(async {
            let Some(window) = web_sys::window() else { return };
            let Some(document) = window.document() else { return };
            let Some(canvas) = document.get_element_by_id("craftcloud_canvas").and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
                log::error!("no #craftcloud_canvas");
                return;
            };
            // This page is served by the server itself, so its own origin is the server address.
            let location = window.location();
            let origin = location.origin().unwrap_or_default();
            let path = location.pathname().unwrap_or_else(|_| "/".into());
            let search = location.search().unwrap_or_default();
            let code = query_value(&search, "code");
            // Back from the identity provider (single sign-on): `/login?oidc_code=…` or
            // `?login_error=…`. The code works once, so it leaves the address bar at once.
            let oidc_code = query_value(&search, "oidc_code");
            let login_error = query_value(&search, "login_error");
            if (oidc_code.is_some() || login_error.is_some())
                && let Ok(history) = window.history()
            {
                let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/"));
            }
            let Ok(server) = ServerUrl::parse(&origin) else {
                log::error!("this page's origin is not a server address: {origin}");
                return;
            };
            let result = eframe::WebRunner::new()
                .start(
                    canvas,
                    eframe::WebOptions::default(),
                    Box::new(move |cc| {
                        let backend = FetchBackend::new(cc.egui_ctx.clone());
                        let mut web = WebApp::new(&cc.egui_ctx, backend, server, &path, code);
                        // The browser's clock and time zone, for "Today 18:02" in the admin pages.
                        let now = js_sys::Date::now() / 1000.0;
                        let offset = -js_sys::Date::new_0().get_timezone_offset();
                        web.set_clock(now as i64, offset as i32);
                        web.oidc_return(oidc_code, login_error);
                        Ok(Box::new(App { web }))
                    }),
                )
                .await;
            if let Some(loading) = document.get_element_by_id("craftcloud_loading") {
                loading.remove();
            }
            if let Err(e) = result {
                log::error!("couldn't start: {e:?}");
            }
        });
    }

    /// `?code=AB-12&x=1` → `Some("AB-12")` for `code`.
    fn query_value(search: &str, key: &str) -> Option<String> {
        search
            .trim_start_matches('?')
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| js_sys::decode_uri_component(v).ok())
            .map(String::from)
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("craftcloud-web is the browser build: run `trunk serve` in apps/craftcloud-web.");
}
