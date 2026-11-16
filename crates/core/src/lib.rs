//! CraftCloud's model and logic, shared by the desktop client (`apps/craftcloud`) and the web
//! frontend (`apps/craftcloud-web`). No UI and no platform code here: that lives in
//! `craftcloud-ui-egui` and `craftcloud-native`.

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
pub mod sync;

pub use runtime_config::{RuntimeConfig, RuntimeConfigError};
pub use server_url::{ServerUrl, ServerUrlError};
