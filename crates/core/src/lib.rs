//! CraftCloud's model and logic, shared by the desktop client (`apps/craftcloud`) and the web
//! frontend (`apps/craftcloud-web`). No UI and no platform code here: that lives in
//! `craftcloud-ui-egui` and `craftcloud-native`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod api;
pub mod catalog;
pub mod deeplink;
pub mod http;
pub mod logging;
pub mod names;
pub mod reconcile;
mod runtime_config;
pub mod server_settings;
mod server_url;
pub mod size;
pub mod storage;
pub mod sync;
pub mod update;

pub use runtime_config::{RuntimeConfig, RuntimeConfigError};
pub use server_url::{ServerUrl, ServerUrlError};
