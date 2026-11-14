//! Which Craft app opens a file: core's built-in copy of `apps.toml` (`default` extensions, names,
//! colours), so that the slim image, which has no `/craftcloud/apps.json`, still says "opens in
//! WordCraft". Core's tests keep it in step with `apps.toml`.

use egui::Color32;

pub use craftcloud_core::catalog::{CraftApp as App, extension, for_name};

/// The app's surface colour.
pub fn color(app: &App) -> Color32 {
    hex(app.color)
}

/// The app's text colour on a dark or light background.
pub fn ink(app: &App, dark: bool) -> Color32 {
    hex(app.ink[usize::from(!dark)])
}

fn hex(s: &str) -> Color32 {
    craftcloud_core::catalog::rgb(s).map_or(Color32::GRAY, |(r, g, b)| Color32::from_rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::{color, for_name, ink};

    #[test]
    fn apps_have_their_colours() {
        let word = for_name("Q3 report.docx").unwrap();
        assert_eq!(color(word), egui::Color32::from_rgb(0x3f, 0x5f, 0xe0));
        assert_ne!(ink(word, true), ink(word, false));
    }
}
