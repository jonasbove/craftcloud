//! The ArtCraft Launcher's look as egui styles: hard edges, hairlines, quiet colours.
//!
//! The token values come from craft-launcher's `crates/ui-egui/src/theme.rs` (MIT OR Apache-2.0,
//! Copyright (c) 2026 ArtCraft Team), which copies getartcraft.com. Only colours and shapes are
//! used; no ArtCraft name or mark.

use egui::{Color32, CornerRadius, Stroke};

use crate::themes::Palette;

const fn ink_alpha(rgb: u32, alpha: u8) -> Color32 {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgba_unmultiplied_const(r, g, b, alpha)
}

const fn hex(rgb: u32) -> Color32 {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

/// One appearance's colours (`--bg`, `--ink`, … in the launcher's naming).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Color32,
    pub sunken: Color32,
    pub ink: Color32,
    pub ink_strong: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub accent: Color32,
    pub accent_ink: Color32,
    pub invert_bg: Color32,
    pub invert_fg: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
}

impl Tokens {
    pub const DARK: Self = Self {
        dark: true,
        bg: hex(0x121316),
        sunken: hex(0x060607),
        ink: hex(0xf2f1ee),
        ink_strong: hex(0xffffff),
        muted: ink_alpha(0xf2f1ee, 153),
        faint: ink_alpha(0xf2f1ee, 97),
        line: ink_alpha(0xf2f1ee, 38),
        line_strong: ink_alpha(0xf2f1ee, 102),
        // CraftCloud's green, from the mackerel logo (brightened from the PSD's #4A563D).
        accent: hex(0x5e7a48),
        accent_ink: hex(0xa9c48e),
        invert_bg: hex(0xffffff),
        invert_fg: hex(0x0b0b0c),
        good: hex(0x3fd6b8),
        warn: hex(0xffc14d),
        bad: hex(0xff6259),
    };

    pub const LIGHT: Self = Self {
        dark: false,
        bg: hex(0xf2f1ee),
        sunken: hex(0xe9e8e4),
        ink: hex(0x101014),
        ink_strong: hex(0x000000),
        muted: ink_alpha(0x101014, 158),
        faint: ink_alpha(0x101014, 107),
        line: ink_alpha(0x101014, 41),
        line_strong: ink_alpha(0x101014, 102),
        accent: hex(0x5e7a48),
        accent_ink: hex(0x3b4d2c),
        invert_bg: hex(0x101014),
        invert_fg: hex(0xf2f1ee),
        good: hex(0x0a7563),
        warn: hex(0x8a5800),
        bad: hex(0xb3261e),
    };

    /// The colours in use: the chosen theme's ([`set_palette`]) for the context's appearance,
    /// CraftCloud's own ([`Tokens::DARK`], [`Tokens::LIGHT`]) until one is chosen.
    pub fn get(ctx: &egui::Context) -> Self {
        let palette = active(ctx);
        if ctx.theme() == egui::Theme::Dark { palette.dark } else { palette.light }
    }

    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.line)
    }
}

/// The display face (headings, wordmarks): Archivo, as in the launcher.
pub fn display() -> egui::FontFamily {
    egui::FontFamily::Name("display".into())
}

/// The launcher's faces (SIL OFL 1.1, licences in `assets/fonts/`): Inter for text, Archivo for
/// display, Geist Mono for labels. egui's own faces stay as fallbacks for other scripts.
fn install_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("Inter", &include_bytes!("../assets/fonts/Inter-Variable.ttf")[..]),
        ("Archivo", &include_bytes!("../assets/fonts/Archivo-Variable.ttf")[..]),
        ("GeistMono", &include_bytes!("../assets/fonts/GeistMono-Variable.ttf")[..]),
    ] {
        fonts.font_data.insert(name.to_owned(), Arc::new(egui::FontData::from_static(bytes)));
    }
    let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter".to_owned());
    // Faces the platform added for other scripts (the desktop client's CJK font), last.
    let extra: Vec<(String, Arc<egui::FontData>)> = ctx.data(|d| d.get_temp::<ExtraFonts>(egui::Id::NULL)).map(|e| e.0).unwrap_or_default();
    for (name, data) in &extra {
        fonts.font_data.insert(name.clone(), Arc::clone(data));
    }
    let names: Vec<String> = extra.iter().map(|(n, _)| n.clone()).collect();
    proportional.extend(names.iter().cloned());
    let fallbacks = proportional.clone();
    let mono = fonts.families.entry(egui::FontFamily::Monospace).or_default();
    mono.insert(0, "GeistMono".to_owned());
    mono.extend(names);
    let mut display_stack = vec!["Archivo".to_owned()];
    display_stack.extend(fallbacks);
    fonts.families.insert(display(), display_stack);
    // On the desktop the craft-ui runtime keeps its own record of the faces (the line ratio its controls are sized by),
    // as craft-launcher's `theme::install_fonts` hands them over; the web build has no craft-ui.
    #[cfg(not(target_arch = "wasm32"))]
    craft_ui_contract::fonts::load(ctx, fonts);
    #[cfg(target_arch = "wasm32")]
    ctx.set_fonts(fonts);
}

/// Fallback faces added by the platform (kept in the context's data, so a later [`install`] keeps
/// them).
#[derive(Clone, Default)]
struct ExtraFonts(Vec<(String, std::sync::Arc<egui::FontData>)>);

/// Adds a face for scripts the bundled ones lack (CJK on the desktop), after all the others, and
/// installs the fonts again. The web build never calls this, so it carries no such face.
pub fn add_fallback_font(ctx: &egui::Context, name: &str, data: egui::FontData) {
    ctx.data_mut(|d| {
        let extra = d.get_temp_mut_or_default::<ExtraFonts>(egui::Id::NULL);
        extra.0.retain(|(n, _)| n != name);
        extra.0.push((name.to_owned(), std::sync::Arc::new(data)));
    });
    install_fonts(ctx);
}

/// The palette in use, kept in the context's data (so a later [`install`] keeps it).
#[derive(Clone, Copy)]
struct Active(Palette);

/// The palette in use: the chosen theme's, or CraftCloud's own.
pub(crate) fn active(ctx: &egui::Context) -> Palette {
    ctx.data(|d| d.get_temp::<Active>(egui::Id::NULL)).map_or(Palette::CRAFTCLOUD, |a| a.0)
}

/// Installs the launcher look on both of egui's appearances, in the colours of the palette in
/// use (CraftCloud's own until [`set_palette`] picks another).
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    apply_palette(ctx, active(ctx));
}

/// Draws with `palette` from now on (a colour theme, or [`Palette::CRAFTCLOUD`]). Only colours
/// change; the shapes, fonts and spacing stay the launcher's.
pub fn set_palette(ctx: &egui::Context, palette: Palette) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, Active(palette)));
    apply_palette(ctx, palette);
}

fn apply_palette(ctx: &egui::Context, palette: Palette) {
    ctx.style_mut_of(egui::Theme::Dark, |style| apply(style, &palette.dark));
    ctx.style_mut_of(egui::Theme::Light, |style| apply(style, &palette.light));
}

fn apply(style: &mut egui::Style, t: &Tokens) {
    let v = &mut style.visuals;
    v.dark_mode = t.dark;
    v.panel_fill = t.bg;
    v.window_fill = t.bg;
    v.extreme_bg_color = t.sunken;
    v.faint_bg_color = t.sunken;
    v.override_text_color = Some(t.ink);
    v.hyperlink_color = t.accent_ink;
    v.selection.bg_fill = t.accent;
    v.selection.stroke = Stroke::new(1.0, t.ink_strong);
    v.window_stroke = Stroke::new(1.0, t.line_strong);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.error_fg_color = t.bad;
    v.warn_fg_color = t.warn;

    let w = &mut v.widgets;
    for (state, fill, stroke, text) in [
        (&mut w.noninteractive, t.bg, t.line, t.ink),
        (&mut w.inactive, t.bg, t.line, t.ink),
        (&mut w.hovered, t.sunken, t.line_strong, t.ink_strong),
        (&mut w.active, t.sunken, t.line_strong, t.ink_strong),
        (&mut w.open, t.sunken, t.line_strong, t.ink_strong),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.0, text);
        state.corner_radius = CornerRadius::ZERO;
        state.expansion = 0.0;
    }
    style.spacing.interact_size.y = 34.0;
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
}

#[cfg(test)]
mod tests {
    use super::Tokens;
    use egui::Color32;

    /// WCAG 2 relative luminance of an opaque colour.
    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (hi, lo) = if luminance(a) > luminance(b) { (luminance(a), luminance(b)) } else { (luminance(b), luminance(a)) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// A fallback face added by the platform survives the theme being installed again, and
    /// draws what the bundled faces can't (checked with a CJK font where the system has one).
    #[test]
    fn fallback_fonts_stay_and_draw_cjk() {
        let ctx = egui::Context::default();
        super::install(&ctx);
        let font = ["/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"]
            .iter()
            .find_map(|p| std::fs::read(p).ok());
        let Some(bytes) = font else {
            eprintln!("skipped: no CJK font on this system");
            return;
        };
        let frame = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
        };
        frame(&ctx);
        let id = egui::FontId::proportional(13.0);
        assert!(!ctx.fonts_mut(|f| f.has_glyphs(&id, "漢字")), "the bundled faces have no CJK");
        super::add_fallback_font(&ctx, "system-cjk", egui::FontData::from_owned(bytes));
        super::install(&ctx);
        frame(&ctx);
        assert!(ctx.fonts_mut(|f| f.has_glyphs(&id, "漢字かな한글")));
        assert!(ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::monospace(11.0), "漢字")));
    }

    /// Until a theme is chosen the colours are CraftCloud's own; a chosen palette survives the
    /// theme being installed again, and going back gives CraftCloud's own look again.
    #[test]
    fn the_palette_in_use_is_kept_across_installs() {
        use crate::themes::Palette;
        let ctx = egui::Context::default();
        super::install(&ctx);
        ctx.set_theme(egui::Theme::Dark);
        assert_eq!(Tokens::get(&ctx), Tokens::DARK);
        ctx.set_theme(egui::Theme::Light);
        assert_eq!(Tokens::get(&ctx), Tokens::LIGHT);
        let nord = Palette::for_id("craft.nord");
        super::set_palette(&ctx, nord);
        super::install(&ctx);
        assert_eq!(Tokens::get(&ctx), nord.light);
        assert_eq!(ctx.style_of(egui::Theme::Light).visuals.panel_fill, nord.light.bg);
        assert!(ctx.style_of(egui::Theme::Light).visuals.dark_mode, "Nord is dark only");
        super::set_palette(&ctx, Palette::CRAFTCLOUD);
        assert_eq!(Tokens::get(&ctx), Tokens::LIGHT);
        assert_eq!(ctx.style_of(egui::Theme::Light).visuals.panel_fill, Tokens::LIGHT.bg);
    }

    /// Text colours over the background must meet WCAG AA (4.5:1) on both appearances.
    #[test]
    fn body_text_and_status_colours_meet_aa() {
        for t in [Tokens::DARK, Tokens::LIGHT] {
            for (name, fg) in [("ink", t.ink), ("accent_ink", t.accent_ink), ("good", t.good), ("warn", t.warn), ("bad", t.bad)] {
                let ratio = contrast(fg, t.bg);
                assert!(ratio >= 4.5, "{name} on bg is {ratio:.2}:1 (dark: {})", t.dark);
            }
            assert!(contrast(t.invert_fg, t.invert_bg) >= 4.5, "inverted buttons (dark: {})", t.dark);
        }
    }
}
