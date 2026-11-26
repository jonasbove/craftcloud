//! CraftCloud's egui interface, shared by the desktop client and the web frontend.
//!
//! Screens follow the FigCraft design in `design/craftcloud.figcraft`.

pub mod app_grid;
pub mod backend;
pub mod chrome;
mod connect;
pub mod desktop;
/// The craft-ui runtime and its components, on the desktop client only.
#[cfg(not(target_arch = "wasm32"))]
pub mod framework;
pub mod kit;
pub mod theme;
pub mod themes;
pub mod web;
mod widgets;

pub use connect::{ConnectAction, ConnectScreen};
