//! Preview system for rendering and capturing `WaterUI` views.
//!
//! This module provides infrastructure for the `water preview` command:
//!
//! - [`app_client`]: TCP client for communicating with preview app (ports 2106+)
//! - [`protocol`]: Message definitions for preview app communication
//! - [`watcher`]: File change detection for build caching
//! - [`launcher`]: Preview app lifecycle management

mod app_client;
mod launcher;
pub mod protocol;
pub mod watcher;

pub use app_client::PreviewAppClient;
pub use launcher::{PreviewSession, kill_preview_support_app, launch_preview_session};
pub use protocol::{PreviewPlatform, Size};
