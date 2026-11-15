//! The window's place and size from one run to the next. eframe saves them (its `persistence`
//! feature) from the window as it is at the time, also while it is minimised or hidden in the
//! tray, when Windows reports it as 0×0 at (-32000, -32000): the next start opened a 64×64 window
//! at the screen's corner. So the client puts back the last good place before eframe writes it,
//! and a window that comes back smaller than it can be starts at its default size, centred.

use egui::{Rect, Vec2, ViewportCommand, ViewportInfo};

/// The window's size on a first start.
pub const DEFAULT_SIZE: Vec2 = egui::vec2(960.0, 640.0);
/// The smallest the window lays out in (its minimum inner size).
pub const MIN_SIZE: Vec2 = egui::vec2(640.0, 480.0);

/// Where eframe keeps the window's place and size in its storage (`app.ron`).
pub const KEY: &str = "window";

/// Windows' place for a minimised window; nothing real is that far off.
const OFF_SCREEN: f32 = -30_000.0;

/// Whether the window's place and size now are worth keeping: shown (not hidden in the tray), not
/// minimised, and at least as big as the window can be made. Under Wayland the window's place
/// isn't known (no `inner_rect`), but eframe saves its size, which is good then.
pub fn worth_keeping(shown: bool, viewport: &ViewportInfo) -> bool {
    shown && viewport.minimized != Some(true) && viewport.inner_rect.is_none_or(|r| fits(r.size()))
}

fn fits(size: Vec2) -> bool {
    // Rounding to pixels and back can take a fraction off.
    size.x + 1.0 >= MIN_SIZE.x && size.y + 1.0 >= MIN_SIZE.y
}

/// What puts right a window restored from a bad save: smaller than it can be, or placed where
/// Windows puts minimised windows. `None`: it is fine as it is, or its size isn't known (Wayland
/// gives no place). The window is then given its default size, centred on `monitor` (its
/// monitor's size, when known).
pub fn restore(inner: Option<Rect>, outer: Option<Rect>, monitor: Option<Vec2>) -> Option<Vec<ViewportCommand>> {
    let small = !fits(inner?.size());
    let off = outer.is_some_and(|r| r.min.x <= OFF_SCREEN || r.min.y <= OFF_SCREEN);
    if !small && !off {
        return None;
    }
    let size = if small { DEFAULT_SIZE } else { inner.map_or(DEFAULT_SIZE, |r| r.size()) };
    let mut commands = Vec::new();
    if small {
        commands.push(ViewportCommand::InnerSize(size));
    }
    if let Some(monitor) = monitor.filter(|m| m.x > 0.0 && m.y > 0.0) {
        commands.push(ViewportCommand::OuterPosition(((monitor - size) / 2.0).max(Vec2::ZERO).to_pos2()));
    }
    Some(commands)
}

/// Keeps eframe's saved place for the window to the last good one.
#[derive(Debug, Default)]
pub struct Keeper {
    /// The window's place and size were good at the last look.
    good: bool,
    /// What eframe saved the last time they were good (or what the client started with).
    kept: Option<String>,
}

impl Keeper {
    /// Starts with what eframe restored the window from.
    pub fn new(saved: Option<String>) -> Self {
        Self { good: false, kept: saved }
    }

    /// Notes whether the window's place and size are good now ([`worth_keeping`]).
    pub fn look(&mut self, good: bool) {
        self.good = good;
    }

    /// Called as eframe saves, after it wrote the window's place under [`KEY`]: kept when good,
    /// put back to the last good one otherwise.
    pub fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.good {
            self.kept = storage.get_string(KEY);
        } else if let Some(kept) = &self.kept {
            storage.set_string(KEY, kept.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use egui::{Rect, ViewportCommand, ViewportInfo, pos2, vec2};

    use super::{DEFAULT_SIZE, KEY, Keeper, restore, worth_keeping};

    #[derive(Default)]
    struct Storage(HashMap<String, String>);

    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    fn at(x: f32, y: f32, w: f32, h: f32) -> Option<Rect> {
        Some(Rect::from_min_size(pos2(x, y), vec2(w, h)))
    }

    const FULL_HD: Option<egui::Vec2> = Some(vec2(1920.0, 1080.0));

    #[test]
    fn a_window_saved_minimised_comes_back_at_its_default_size_centred() {
        // What 0.1.1 saved on Windows: minimised, 0×0 at (-32000, -32000), which eframe makes 64×64 at (0, 0).
        let fix = restore(at(0.0, 0.0, 64.0, 64.0), at(0.0, 0.0, 64.0, 64.0), FULL_HD).unwrap();
        assert_eq!(fix, [ViewportCommand::InnerSize(DEFAULT_SIZE), ViewportCommand::OuterPosition(pos2(480.0, 220.0))]);
        // Where Windows parks minimised windows, at a good size: centred.
        let fix = restore(at(-32000.0, -32000.0, 1000.0, 700.0), at(-32000.0, -32000.0, 1000.0, 700.0), FULL_HD).unwrap();
        assert_eq!(fix, [ViewportCommand::OuterPosition(pos2(460.0, 190.0))]);
        // No monitor size: the size at least.
        assert_eq!(restore(at(0.0, 0.0, 64.0, 64.0), None, None).unwrap(), [ViewportCommand::InnerSize(DEFAULT_SIZE)]);
        // Nothing known: left as it is.
        assert_eq!(restore(None, None, FULL_HD), None);
        // A monitor smaller than the window: the corner, never off it.
        let fix = restore(at(0.0, 0.0, 64.0, 64.0), None, Some(vec2(800.0, 600.0))).unwrap();
        assert_eq!(fix[1], ViewportCommand::OuterPosition(pos2(0.0, 0.0)));
    }

    #[test]
    fn a_good_place_is_left_alone() {
        assert_eq!(restore(at(100.0, 80.0, 1000.0, 700.0), at(92.0, 49.0, 1016.0, 739.0), FULL_HD), None);
        assert_eq!(restore(at(100.0, 80.0, 639.6, 479.6), None, FULL_HD), None, "the minimum, rounded");
        // Wayland gives no position.
        assert_eq!(restore(at(0.0, 0.0, 960.0, 640.0), None, None), None);
        // A second monitor left of the first.
        assert_eq!(restore(at(-1800.0, 100.0, 960.0, 640.0), at(-1808.0, 69.0, 976.0, 679.0), FULL_HD), None);
    }

    #[test]
    fn a_minimised_or_hidden_window_is_not_kept() {
        let shown = ViewportInfo { minimized: Some(false), inner_rect: at(100.0, 80.0, 1000.0, 700.0), ..ViewportInfo::default() };
        assert!(worth_keeping(true, &shown));
        assert!(!worth_keeping(false, &shown), "hidden in the tray");
        assert!(!worth_keeping(true, &ViewportInfo { minimized: Some(true), ..shown.clone() }));
        assert!(!worth_keeping(true, &ViewportInfo { inner_rect: at(0.0, 0.0, 0.0, 0.0), ..shown.clone() }));
        // Wayland gives no place: the size eframe saves is kept, as before.
        assert!(worth_keeping(true, &ViewportInfo { inner_rect: None, ..shown.clone() }));
        assert!(!worth_keeping(true, &ViewportInfo { inner_rect: None, minimized: Some(true), ..shown }));
    }

    #[test]
    fn the_last_good_place_is_saved_while_minimised() {
        let mut storage = Storage::default();
        let mut keeper = Keeper::new(Some("started".into()));
        // eframe saves, then asks the app: what it wrote while the window was shown is kept.
        storage.0.insert(KEY.into(), "good".into());
        keeper.look(true);
        keeper.save(&mut storage);
        assert_eq!(storage.0[KEY], "good");
        // Minimised: eframe writes 0×0 at (-32000, -32000); the good place goes back.
        storage.0.insert(KEY.into(), "minimised".into());
        keeper.look(false);
        keeper.save(&mut storage);
        assert_eq!(storage.0[KEY], "good");
        // Hidden from the start (a start at login): what it started with.
        let mut keeper = Keeper::new(Some("started".into()));
        storage.0.insert(KEY.into(), "hidden".into());
        keeper.save(&mut storage);
        assert_eq!(storage.0[KEY], "started");
    }
}
