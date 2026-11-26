use craft_theme::{Appearance, Contrast, Mode, Size, TextSize, resolve};
use craft_ui_contract::style::StyleCx;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use super::{icon_name, install_headless, installed, over, theme_def};
use crate::themes::Palette;

fn appearance(mode: Mode) -> Appearance {
    Appearance { mode, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false }
}

/// Every icon the sign-in buttons ask craft-ui for is in its icon set.
#[test]
fn the_buttons_icons_are_in_craft_uis_set() {
    use crate::chrome::Icon;
    for icon in [Icon::Refresh, Icon::External] {
        let name = icon_name(icon).unwrap_or_else(|| panic!("no craft-ui icon"));
        assert!(craft_ui_contract::primitives::has_icon(&craft_behavior::IconId::new(name)), "{name}");
    }
    assert_eq!(icon_name(Icon::Arrow), None, "craft-ui has no right arrow");
}

/// The craft theme of a palette resolves to that palette's colours: the panel, the text, the accent and the control
/// edge. For CraftCloud's own look and for colour themes, in both appearances.
#[test]
fn the_craft_theme_carries_the_palettes_button_colours() {
    for palette in [Palette::CRAFTCLOUD, Palette::for_id("craft.nord"), Palette::for_id("craft.solarized-light"), Palette::for_id("craft.gruvbox")] {
        let def = theme_def(&palette);
        for (mode, t) in [(Mode::Dark, palette.dark), (Mode::Light, palette.light)] {
            let c = resolve(&def, appearance(mode), None).tokens.color;
            let what = format!("{mode:?} {:?}", t.bg);
            assert_eq!(c.surface.panel, over(t.bg, t.bg), "panel, {what}");
            assert_eq!(c.accent.solid, over(t.bg, t.accent), "accent, {what}");
            assert_eq!(c.text.normal, over(t.bg, t.ink), "text, {what}");
            assert_eq!(c.line.control_edge, over(t.bg, t.line_strong), "control edge, {what}");
        }
    }
}

/// One pass of `ctx`, with nothing drawn.
fn pass(ctx: &egui::Context) {
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
}

/// A context with the runtime, CraftCloud's theme and `palette`, after one pass with the root values handed over.
fn context_with(palette: Palette, dark: bool) -> egui::Context {
    let ctx = egui::Context::default();
    install_headless(&ctx);
    crate::theme::install(&ctx);
    crate::theme::set_palette(&ctx, palette);
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    super::before_input(&ctx);
    pass(&ctx);
    ctx
}

/// The palette chosen in Settings reaches craft-ui: the runtime's tokens follow it, and follow a change of it.
#[test]
fn the_runtime_follows_the_palette_in_use() {
    let panel = |ctx: &egui::Context| StyleCx::with_root(ctx, |cx| cx.tokens.color.surface.panel);
    let ctx = context_with(Palette::CRAFTCLOUD, true);
    assert!(installed(&ctx));
    assert_eq!(panel(&ctx), over(Palette::CRAFTCLOUD.dark.bg, Palette::CRAFTCLOUD.dark.bg));

    let nord = Palette::for_id("craft.nord");
    assert_ne!(nord.dark.bg, Palette::CRAFTCLOUD.dark.bg, "a palette that tells the two apart");
    crate::theme::set_palette(&ctx, nord);
    super::before_input(&ctx);
    pass(&ctx);
    assert_eq!(panel(&ctx), over(nord.dark.bg, nord.dark.bg));
}

/// The runtime turns egui's keyboard zoom off at install; CraftCloud keeps it, and the zoom it sets stays (the runtime
/// is handed egui's zoom every pass).
#[test]
fn ctrl_plus_still_zooms() {
    let mut h = Harness::builder().with_context_setup(install_headless).build_ui(|ui| {
        ui.label("CraftCloud");
    });
    super::before_input(&h.ctx);
    h.run();
    assert!(h.ctx.options(|o| o.zoom_with_keyboard));
    let before = h.ctx.zoom_factor();
    let zoom_in = egui::gui_zoom::kb_shortcuts::ZOOM_IN;
    h.key_press_modifiers(zoom_in.modifiers, zoom_in.logical_key);
    h.run();
    for _ in 0..3 {
        super::before_input(&h.ctx);
        h.run();
    }
    assert!(h.ctx.zoom_factor() > before, "zoomed in: {} after {before}", h.ctx.zoom_factor());
}

/// Without the runtime (an install that failed) the sign-in buttons are CraftCloud's own, and nothing panics.
#[test]
fn without_the_runtime_the_buttons_are_craftclouds_own() {
    let mut h = Harness::builder().build_ui_state(
        |ui, clicked: &mut bool| {
            crate::theme::install(ui.ctx());
            *clicked |= super::button(ui, "Continue", crate::chrome::Kind::Primary, Some(crate::chrome::Icon::Arrow), true);
        },
        false,
    );
    super::before_input(&h.ctx);
    h.run();
    assert!(!installed(&h.ctx));
    h.get_by_label("CONTINUE").click();
    h.run();
    assert!(*h.state());
}

/// The runtime binds Backspace, Delete, Tab and the clipboard keys to commands of its own; with one of CraftCloud's
/// egui fields focused they still edit the field.
#[test]
fn keys_still_reach_craftclouds_text_fields() {
    let mut h = Harness::builder().with_context_setup(install_headless).build_ui_state(
        |ui, text: &mut String| {
            let label = ui.label("Server");
            ui.add(egui::TextEdit::singleline(text).id_salt("address")).labelled_by(label.id);
        },
        String::new(),
    );
    super::before_input(&h.ctx);
    h.run();
    h.get_by_role(egui::accesskit::Role::TextInput).focus();
    h.run();
    h.get_by_role(egui::accesskit::Role::TextInput).type_text("cloud.example.orgx");
    h.run();
    h.key_press(egui::Key::Backspace);
    h.run();
    assert_eq!(h.state(), "cloud.example.org");
    h.key_press(egui::Key::Home);
    h.key_press(egui::Key::Delete);
    h.run();
    assert_eq!(h.state(), "loud.example.org");
}
