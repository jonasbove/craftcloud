//! The colour themes CraftCloud offers besides its own look: seven community palettes as
//! `craft-design-theme` files, parsed and derived by craft-libs' `craft-theme` crate, and mapped
//! onto CraftCloud's [`Tokens`].
//!
//! CraftCloud's own look ([`Tokens::DARK`] and [`Tokens::LIGHT`]) stays the default and touches
//! none of this: neither the files nor `craft_theme::builtins()` are read until a theme is chosen
//! or a theme picker is shown (Settings, the web account menu).

use std::sync::LazyLock;

use craft_theme::{Appearance, ColorTokens, Contrast, Mode, Rgba, Size, TextSize, ThemeDef, Variants, format, over, resolve};
use egui::Color32;

use crate::theme::Tokens;

/// The picker's name for CraftCloud's own look, the default.
pub const CRAFTCLOUD_LOOK: &str = "CraftCloud";

/// The themes offered, in picker order: each one's id and its file, embedded for as long as
/// craft-libs doesn't ship it.
///
/// They are proposed as craft-libs built-ins (storytold/craft-libs pull requests #3701, #3704,
/// #3705, #3706, #3707, #3708 and #3709). An embedded file is used only while
/// `craft_theme::builtins()` lacks its id ([`embedded_needed`]); once the workspace's craft-libs
/// pin includes those pull requests, the built-in is used instead and the saved choice keeps
/// working. Then delete `assets/themes/`, this table's file texts and their lines in NOTICE.
const OFFERED: [(&str, &str); 7] = [
    ("craft.catppuccin", include_str!("../assets/themes/catppuccin.json")),
    ("craft.dracula", include_str!("../assets/themes/dracula.json")),
    ("craft.gruvbox", include_str!("../assets/themes/gruvbox.json")),
    ("craft.nord", include_str!("../assets/themes/nord.json")),
    ("craft.one-dark", include_str!("../assets/themes/one-dark.json")),
    ("craft.solarized-light", include_str!("../assets/themes/solarized-light.json")),
    ("craft.tokyo-night", include_str!("../assets/themes/tokyo-night.json")),
];

/// Whether CraftCloud must bring its own copy of the theme `id`: only while craft-libs' built-ins
/// (`builtin_ids`) lack it.
pub fn embedded_needed(id: &str, builtin_ids: &[&str]) -> bool {
    !builtin_ids.contains(&id)
}

/// One theme on offer.
#[derive(Clone, Debug)]
pub struct Theme {
    /// The theme's id, as saved (`craft.nord`).
    pub id: &'static str,
    /// Its name in the picker ("Nord").
    pub name: String,
    def: ThemeDef,
}

impl Theme {
    /// "dark only", "light only", or `None` for a theme with both, which follows the appearance.
    pub fn only(&self) -> Option<&'static str> {
        match self.def.variants {
            Variants::Both => None,
            Variants::DarkOnly => Some("dark only"),
            Variants::LightOnly => Some("light only"),
        }
    }

    /// The picker's label: "Nord (dark only)".
    pub fn label(&self) -> String {
        match self.only() {
            Some(only) => format!("{} ({only})", self.name),
            None => self.name.clone(),
        }
    }

    /// The colours for both of egui's appearances. A theme with one variant gives that variant
    /// for both, so it looks the same whatever the Appearance setting says.
    pub fn palette(&self) -> Palette {
        Palette { dark: tokens_for(&self.def, Mode::Dark), light: tokens_for(&self.def, Mode::Light) }
    }
}

/// The themes on offer: each of [`OFFERED`], from craft-libs' built-ins when it has the id, else
/// from the embedded file. A file that doesn't parse is left out (the tests rule that out).
fn catalog(builtins: &[ThemeDef]) -> Vec<Theme> {
    let builtin_ids: Vec<&str> = builtins.iter().map(|t| t.id.as_str()).collect();
    OFFERED
        .iter()
        .filter_map(|&(id, text)| {
            let def = if embedded_needed(id, &builtin_ids) {
                format::parse(text.as_bytes()).ok().map(|f| f.theme).filter(|t| t.id.as_str() == id)?
            } else {
                builtins.iter().find(|t| t.id.as_str() == id)?.clone()
            };
            Some(Theme { id, name: def.name.clone(), def })
        })
        .collect()
}

static CATALOG: LazyLock<Vec<Theme>> = LazyLock::new(|| catalog(craft_theme::builtins()));

/// The themes on offer, in picker order (parsed on first use).
pub fn all() -> &'static [Theme] {
    &CATALOG
}

/// The theme saved as `id`; `None` for CraftCloud's own look (an empty id) or an id this build
/// doesn't know (shown as CraftCloud's look, without changing the setting).
pub fn find(id: &str) -> Option<&'static Theme> {
    if id.is_empty() {
        return None;
    }
    all().iter().find(|t| t.id == id)
}

/// The colours for egui's two appearances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: Tokens,
    pub light: Tokens,
}

impl Palette {
    /// CraftCloud's own look.
    pub const CRAFTCLOUD: Self = Self { dark: Tokens::DARK, light: Tokens::LIGHT };

    /// The palette for a saved theme id: CraftCloud's own look for an empty or unknown one.
    pub fn for_id(id: &str) -> Self {
        find(id).map_or(Self::CRAFTCLOUD, Theme::palette)
    }
}

/// `def` resolved for `mode` (or its one variant), mapped onto CraftCloud's tokens.
fn tokens_for(def: &ThemeDef, mode: Mode) -> Tokens {
    let appearance = Appearance { mode, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
    let resolved = resolve(def, appearance, None);
    map(&resolved.tokens.color, resolved.variant_used == Mode::Dark)
}

/// An opaque colour: `c` over `bg`.
fn solid(c: Rgba, bg: Rgba) -> Color32 {
    let o = over(c, bg);
    Color32::from_rgb(o.0[0], o.0[1], o.0[2])
}

/// A colour as it is, translucent or not.
fn color(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

/// craft-theme's semantic colours onto CraftCloud's tokens.
///
/// | CraftCloud | craft-theme | why |
/// |---|---|---|
/// | `bg` | `surface.panel` | the window. CraftCloud draws everything on one surface; these themes put their panels on a darker `surface.background` (which several also use for `surface.sunken` and the dividers), and text is held to its minimum on the panel |
/// | `sunken` | `surface.sunken` | fields, wells and hover fills, a step below `bg` as CraftCloud's own is |
/// | `ink` | `text.normal` | body text |
/// | `ink_strong` | `text.normal`, halfway to white (dark) or black (light) | headings and hovered text, a step past `ink` as CraftCloud's own is |
/// | `muted` | `text.muted` | secondary text |
/// | `faint` | `text.faint` | hints (craft-theme holds it to the text minimum) |
/// | `line` | `line.divider` | hairlines |
/// | `line_strong` | `line.control_edge` | the edges of controls (held to the non-text minimum) |
/// | `accent` | `accent.solid` | the theme's accent: meters, selected rows, focus bars |
/// | `accent_ink` | `accent.text` | accent text and links (held to the text minimum) |
/// | `invert_bg`, `invert_fg` | `surface.inverse`, `text.on_inverse` | primary buttons and the chosen cell |
/// | `good`, `warn`, `bad` | `status.good.fg`, `status.warn.fg`, `status.bad.fg` | status text and dots |
fn map(c: &ColorTokens, dark: bool) -> Tokens {
    let bg = c.surface.panel;
    let toward = if dark { Rgba::WHITE } else { Rgba::BLACK };
    let strong = over(c.text.normal.with_alpha(128), toward);
    Tokens {
        dark,
        bg: solid(bg, bg),
        sunken: solid(c.surface.sunken, bg),
        ink: solid(c.text.normal, bg),
        ink_strong: solid(strong, bg),
        muted: solid(c.text.muted, bg),
        faint: solid(c.text.faint, bg),
        line: color(c.line.divider),
        line_strong: color(c.line.control_edge),
        accent: solid(c.accent.solid, bg),
        accent_ink: solid(c.accent.text, bg),
        invert_bg: solid(c.surface.inverse, bg),
        invert_fg: solid(c.text.on_inverse, c.surface.inverse),
        good: solid(c.status.good.fg, bg),
        warn: solid(c.status.warn.fg, bg),
        bad: solid(c.status.bad.fg, bg),
    }
}

#[cfg(test)]
mod tests {
    use super::{OFFERED, Palette, Theme, catalog, embedded_needed, find, format, resolve, tokens_for};
    use craft_theme::{Appearance, Contrast, Mode, Rgba, Size, TextSize, Variants, contrast_ratio};
    use egui::Color32;

    fn ratio(a: Color32, b: Color32) -> f64 {
        contrast_ratio(Rgba([a.r(), a.g(), a.b(), 255]), Rgba([b.r(), b.g(), b.b(), 255]))
    }

    fn appearance(mode: Mode) -> Appearance {
        Appearance { mode, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false }
    }

    /// Every embedded file parses with craft-theme, keeps its id, and resolves in each mode with
    /// no contrast pair below its minimum.
    #[test]
    fn every_embedded_theme_parses_and_resolves() {
        for (id, text) in OFFERED {
            let file = format::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            assert_eq!(file.theme.id.as_str(), id);
            for mode in [Mode::Dark, Mode::Light] {
                let r = resolve(&file.theme, appearance(mode), None);
                assert!(r.warnings.is_empty(), "{id} {mode:?}: {:?}", r.warnings);
                assert_eq!(r.variant_used, file.theme.variants.mode_for(mode), "{id}");
            }
        }
        let ids: Vec<&str> = super::all().iter().map(|t| t.id).collect();
        assert_eq!(ids, OFFERED.map(|(id, _)| id), "every theme is offered, in order");
    }

    /// Each variant's mapping is legible: text, links, statuses and the inverted buttons meet
    /// WCAG AA (4.5:1) on their background, and the wells, hairlines and control edges stand off it.
    #[test]
    fn every_variant_maps_to_legible_tokens() {
        for theme in super::all() {
            for mode in [Mode::Dark, Mode::Light] {
                let t = tokens_for(&theme.def, mode);
                let at = format!("{} {mode:?}", theme.id);
                for (name, fg) in [
                    ("ink", t.ink),
                    ("ink_strong", t.ink_strong),
                    ("muted", t.muted),
                    ("accent_ink", t.accent_ink),
                    ("good", t.good),
                    ("warn", t.warn),
                    ("bad", t.bad),
                ] {
                    let r = ratio(fg, t.bg);
                    assert!(r >= 4.5, "{at}: {name} on bg is {r:.2}:1");
                    let r = ratio(fg, t.sunken);
                    assert!(r >= 4.5, "{at}: {name} on sunken is {r:.2}:1");
                }
                let r = ratio(t.faint, t.bg);
                assert!(r >= 3.0, "{at}: faint on bg is {r:.2}:1");
                let r = ratio(t.invert_fg, t.invert_bg);
                assert!(r >= 4.5, "{at}: inverted buttons are {r:.2}:1");
                assert!(ratio(t.ink_strong, t.bg) >= ratio(t.ink, t.bg), "{at}: ink_strong is stronger than ink");
                assert_ne!(t.sunken, t.bg, "{at}: sunken shows on bg");
                let line =
                    |c: Color32| Color32::from_rgb(blend(c.r(), t.bg.r(), c.a()), blend(c.g(), t.bg.g(), c.a()), blend(c.b(), t.bg.b(), c.a()));
                assert!(ratio(line(t.line), t.bg) >= 1.1, "{at}: line shows on bg");
                assert!(ratio(line(t.line_strong), t.bg) >= 1.2, "{at}: line_strong shows on bg");
                assert_eq!(t.dark, theme.def.variants.mode_for(mode) == Mode::Dark, "{at}");
            }
        }
    }

    /// `c`'s channel at `alpha` over the background's.
    fn blend(c: u8, bg: u8, alpha: u8) -> u8 {
        let a = f32::from(alpha) / 255.0;
        (f32::from(c) * a + f32::from(bg) * (1.0 - a)).round() as u8
    }

    /// A theme craft-libs ships is taken from its built-ins, not from the embedded copy.
    #[test]
    fn an_embedded_theme_gives_way_to_a_built_in() {
        assert!(embedded_needed("craft.nord", &["craft.pro", "craft.solarized-dark"]));
        assert!(!embedded_needed("craft.nord", &["craft.pro", "craft.nord"]));

        let mut builtin = format::parse(OFFERED[3].1.as_bytes()).map(|f| f.theme).unwrap();
        assert_eq!(builtin.id.as_str(), "craft.nord");
        builtin.name = "Nord from craft-libs".into();
        let themes = catalog(&[builtin]);
        let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Catppuccin", "Dracula", "Gruvbox", "Nord from craft-libs", "One Dark", "Solarized Light", "Tokyo Night"]);
    }

    /// At the pinned craft-libs revision none of the seven is a built-in, so all seven files are
    /// used. When this fails, the pin ships that theme: delete its file in `assets/themes/`, its
    /// entry's text in `OFFERED` and its NOTICE lines.
    #[test]
    fn the_pinned_craft_libs_lacks_the_seven() {
        let builtin_ids: Vec<&str> = craft_theme::builtins().iter().map(|t| t.id.as_str()).collect();
        for (id, _) in OFFERED {
            assert!(embedded_needed(id, &builtin_ids), "craft-libs now ships {id}: drop the embedded copy");
        }
    }

    /// CraftCloud's own look is the default and stays as it is; a theme with one variant shows it in
    /// both appearances, one with two follows the appearance.
    #[test]
    fn palettes_follow_the_variants() {
        assert_eq!(Palette::for_id(""), Palette::CRAFTCLOUD);
        assert_eq!(Palette::for_id("craft.no-such-theme"), Palette::CRAFTCLOUD);
        assert!(find("").is_none());
        let get = |id: &str| -> &Theme { find(id).unwrap_or_else(|| panic!("{id}")) };
        let nord = get("craft.nord");
        assert_eq!((nord.def.variants, nord.only(), nord.label().as_str()), (Variants::DarkOnly, Some("dark only"), "Nord (dark only)"));
        let p = nord.palette();
        assert_eq!(p.dark, p.light);
        assert!(p.light.dark);
        let solarized = get("craft.solarized-light").palette();
        assert_eq!(solarized.dark, solarized.light);
        assert!(!solarized.dark.dark);
        let gruvbox = get("craft.gruvbox");
        assert_eq!((gruvbox.only(), gruvbox.label().as_str()), (None, "Gruvbox"));
        let p = gruvbox.palette();
        assert!(p.dark.dark && !p.light.dark);
        assert_ne!(p.dark.bg, p.light.bg);
        // The theme decides the accent.
        assert_ne!(p.dark.accent, Palette::CRAFTCLOUD.dark.accent);
    }
}
