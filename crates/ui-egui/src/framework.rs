//! The craft-ui runtime on the desktop client, hosted from CraftCloud's own eframe app as craft-launcher hosts it
//! (its `framework.rs`; craft-libs `docs/craft-framework.md`, section 2.2): [`install`] once on the window's context
//! before its first pass, [`before_input`] from eframe's `raw_input_hook`, and [`return_window_escape`] first in
//! `DesktopApp::ui`. The web frontend links none of this (the module is not built for wasm32).
//!
//! Only the sign-in sheets draw a craft-ui component so far: their push buttons ([`button`]), in craft-ui's own design
//! (its primary and secondary variants, sizes, face and icons). CraftCloud themes craft-ui the craft-libs way: the
//! palette in use is pinned onto a craft theme ([`theme_def`]), so CraftCloud's colours and a chosen colour theme reach
//! it.

use std::sync::Arc;

use craft_behavior::IconId;
use craft_host::target::ShellInfo;
use craft_theme::{ColorToken, ContrastIntent, Mode, ModePins, ModeSeeds, Pins, RadiusSet, Rgba, SeedOverrides, Seeds, ShapeSeed, Status};
use craft_theme::{ThemeDef, ThemeId, Variant, Variants};
use craft_ui_contract::contract::{Props, Show};
use craft_ui_contract::primitives::IconSpec;
use craft_ui_contract::runtime::env::{RootEnv, Scale};
use craft_ui_contract::runtime::frame::{FrameInput, set_input, take_output};
use craft_ui_contract::runtime::keys::{Intent, with_window_intents};
use craft_ui_widgets::buttons::Button;
use craft_window::system::{NoWindows, SystemWindows, WindowSystem};

use crate::chrome::{self, Icon, Kind};
use crate::theme::Tokens;
use crate::themes::Palette;

/// The app id every OS window carries, as everywhere else (window app id, desktop entry, icons).
const APP_ID: &str = "io.github.jonasbove.craftcloud";

/// Installs the runtime on the desktop window's context, once, before its first pass. A second install, or one inside
/// a pass, is logged and skipped, and the sign-in buttons then stay CraftCloud's own ([`button`]): the window always
/// opens.
///
/// Nothing here waits on the system: `InstallSpec::new` reads the OS appearance only from what egui's integration
/// reports (no desktop watcher, so no D-Bus), and the OS window system is asked only for what a pass needs (on Linux it
/// answers nothing).
pub fn install(ctx: &egui::Context) {
    install_with(ctx, APP_ID, Arc::new(SystemWindows));
}

/// Installs the runtime on a context with no native window: the kittest harnesses.
pub fn install_headless(ctx: &egui::Context) {
    install_with(ctx, "", Arc::new(NoWindows));
}

/// Whether [`install`] put the runtime on `ctx`.
#[derive(Clone, Copy)]
struct Installed;

fn installed(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Installed>(egui::Id::NULL)).is_some()
}

fn install_with(ctx: &egui::Context, app_id: &str, windows: Arc<dyn WindowSystem>) {
    let spec = craft_ui_contract::InstallSpec::new(ShellInfo::default(), windows).app_id(app_id);
    match craft_ui_contract::install(ctx, spec) {
        Ok(()) => {
            ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, Installed));
            // The runtime turns egui's Ctrl/Cmd with + and - off at install, as an app with a zoom command of its own
            // wants. CraftCloud has none and zoomed with egui's: keep it. The scale egui's zoom sets is handed back to
            // the runtime in `before_input`, so the two never disagree.
            ctx.options_mut(|options| options.zoom_with_keyboard = true);
        }
        Err(error) => log::warn!("the UI runtime could not be installed; the sign-in buttons stay CraftCloud's own: {error}"),
    }
}

/// The root values of the coming frame, from eframe's `raw_input_hook`: the palette in use, the appearance egui draws
/// in and egui's zoom.
pub fn before_input(ctx: &egui::Context) {
    if !installed(ctx) {
        return;
    }
    let mode = if ctx.theme() == egui::Theme::Dark { Mode::Dark } else { Mode::Light };
    let root = RootEnv::new().library_theme(theme_for(ctx)).mode(mode).scale(Scale::new(ctx.zoom_factor()));
    // craft-ui-shell's `embed::before_input`, without the shell crate: the runtime's output (title-bar and window
    // requests CraftCloud doesn't make) is read and dropped.
    let _ = take_output(ctx);
    set_input(ctx, FrameInput::new(root));
}

/// Puts the window's Escape (one no craft-ui component took) back into egui's input, so CraftCloud's sheets, menus
/// and fields read it as they did before the runtime took it window-wide. First in `DesktopApp::ui`.
pub fn return_window_escape(ctx: &egui::Context) {
    if !installed(ctx) {
        return;
    }
    let mut taken = false;
    with_window_intents(ctx, false, |intents| {
        taken = intents.take(Intent::Cancel);
        taken
    });
    if taken {
        ctx.input_mut(|i| {
            let modifiers = i.modifiers;
            let key = egui::Key::Escape;
            // Only the press: the runtime takes the key-down as its Cancel intent, while the key-up reaches egui on
            // its own (in a later frame, as the keyboard sends it). Every reader here uses `key_pressed`.
            i.events.insert(0, egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers });
        });
    }
}

/// A push button of the sign-in sheets: craft-ui's `Button` with its primary or secondary variant and, where its icon
/// set has one, its icon; or CraftCloud's own (`chrome::button`) where the runtime isn't installed. Whether it was
/// clicked.
pub fn button(ui: &mut egui::Ui, text: &str, kind: Kind, icon: Option<Icon>, enabled: bool) -> bool {
    if !installed(ui.ctx()) {
        return chrome::button(ui, text, kind, icon, enabled).clicked();
    }
    let variant = match kind {
        Kind::Primary => Variant::PRIMARY,
        Kind::Secondary => Variant::SECONDARY,
        Kind::Danger => Variant::DANGER,
    };
    let mut button = Button::new(text.to_owned()).variant(variant).enabled(enabled);
    if let Some(name) = icon.and_then(icon_name) {
        button = button.icon(IconSpec::svg(IconId::new(name)));
    }
    button.show(ui).clicked()
}

/// craft-ui's icon for one of CraftCloud's, by its name in craft-ui's icon set; `None` where the set has none that
/// means the same (it has no right arrow: Continue shows no icon).
fn icon_name(icon: Icon) -> Option<&'static str> {
    match icon {
        Icon::Refresh => Some("refresh"),
        Icon::External => Some("open"),
        _ => None,
    }
}

/// The craft theme of the palette in use, made once per palette and kept in the context's data, so the runtime's
/// token cache sees one `Arc` for as long as the palette stays.
#[derive(Clone)]
struct Made(Palette, Arc<ThemeDef>);

fn theme_for(ctx: &egui::Context) -> Arc<ThemeDef> {
    let palette = crate::theme::active(ctx);
    if let Some(Made(made, def)) = ctx.data(|d| d.get_temp::<Made>(egui::Id::NULL))
        && made == palette
    {
        return def;
    }
    let def = Arc::new(theme_def(&palette));
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, Made(palette, Arc::clone(&def))));
    def
}

/// `color` as it shows over `on`: theme colours are opaque.
fn over(on: egui::Color32, color: egui::Color32) -> Rgba {
    let [r, g, b, _] = on.blend(color).to_array();
    Rgba::rgb(r, g, b)
}

fn pins(t: &Tokens) -> Pins {
    let on_bg = |color| over(t.bg, color);
    [
        (ColorToken::SurfaceBackground, on_bg(t.bg)),
        (ColorToken::SurfacePanel, on_bg(t.bg)),
        (ColorToken::SurfaceChrome, on_bg(t.bg)),
        (ColorToken::SurfaceRaised, on_bg(t.bg)),
        (ColorToken::SurfaceSunken, on_bg(t.sunken)),
        (ColorToken::TextNormal, on_bg(t.ink)),
        (ColorToken::TextMuted, on_bg(t.muted)),
        (ColorToken::TextFaint, on_bg(t.faint)),
        (ColorToken::LineDivider, on_bg(t.line)),
        (ColorToken::LineBorder, on_bg(t.line)),
        (ColorToken::LineControlEdge, on_bg(t.line_strong)),
        (ColorToken::AccentSolid, on_bg(t.accent)),
        (ColorToken::AccentText, on_bg(t.accent_ink)),
    ]
    .into_iter()
    .collect()
}

fn seeds(t: &Tokens) -> SeedOverrides {
    let solid = |color| Some(over(t.bg, color));
    SeedOverrides {
        neutral: solid(t.bg),
        accent: solid(t.accent),
        status: Status { good: solid(t.good), warn: solid(t.warn), bad: solid(t.bad), info: solid(t.accent) },
    }
}

/// `palette` as a craft theme with CraftCloud's hard corners: its colours pinned onto craft-theme's tokens, so a
/// craft-ui component reads the colours CraftCloud's own widgets do.
pub(crate) fn theme_def(palette: &Palette) -> ThemeDef {
    let (dark, light) = (palette.dark, palette.light);
    let solid = |color| over(dark.bg, color);
    ThemeDef {
        id: ThemeId::parse("craftcloud").unwrap_or_else(|_| ThemeId::suite_default()),
        name: "CraftCloud".into(),
        version: 1,
        seeds: Seeds {
            neutral: solid(dark.bg),
            accent: solid(dark.accent),
            status: Status { good: solid(dark.good), warn: solid(dark.warn), bad: solid(dark.bad), info: solid(dark.accent) },
        },
        mode_seeds: ModeSeeds { light: seeds(&light), dark: seeds(&dark) },
        shape: Some(ShapeSeed {
            radius: Some(RadiusSet::default()),
            radius_offset: None,
            button_radius: None,
            edge: None,
            knob: None,
            wells: None,
            grip: None,
            locator: None,
        }),
        variants: Variants::Both,
        pins: ModePins { light: pins(&light), dark: pins(&dark) },
        high_contrast: ModePins::default(),
        contrast_intent: ContrastIntent::Standard,
        icon_pack: None,
        apps: Default::default(),
    }
}

#[cfg(test)]
mod tests;
