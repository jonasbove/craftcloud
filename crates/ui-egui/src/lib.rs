//! CraftCloud's egui interface, shared by the desktop client and the web frontend.
//!
//! Screens follow the FigCraft design in `design/craftcloud.figcraft`.

pub mod app_grid;
pub mod backend;
pub mod chrome;
mod connect;
pub mod desktop;
pub mod kit;
pub mod theme;
pub mod themes;
pub mod web;
mod widgets;

pub use connect::{ConnectAction, ConnectScreen};
