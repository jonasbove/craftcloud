//! Small pieces of the launcher look that egui doesn't draw by itself.

use egui::{Color32, RichText, Ui};

use crate::theme::Tokens;

const LOGO_PNG: &[u8] = include_bytes!("../assets/craftcloud-logo-128.png");

/// The CraftCloud logo (the mackerel), `size` points square. Decoded once per context.
pub fn logo(ui: &mut Ui, size: f32) {
    let id = egui::Id::unique("craftcloud-logo-texture");
    let texture = ui.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let image = match image::load_from_memory_with_format(LOGO_PNG, image::ImageFormat::Png) {
            Ok(img) => {
                let rgba = img.into_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
            }
            Err(_) => egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]),
        };
        let handle = ui.ctx().load_texture("craftcloud-logo", image, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, handle.clone()));
        handle
    });
    ui.add(egui::Image::new(&texture).fit_to_exact_size(egui::vec2(size, size)));
}

/// Muted body text.
pub fn muted(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).color(t.muted));
}

/// A line in the "bad" status colour.
pub fn error_line(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).color(t.bad));
}

/// A full-width 1px rule.
pub fn hairline(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.line);
}
