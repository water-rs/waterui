//! Preview system for rendering and capturing `WaterUI` views.
//!
//! This module provides infrastructure for the `water preview` command:
//!
//! - [`app_client`]: TCP client for communicating with the preview support app
//! - [`protocol`]: Message definitions for preview support app communication
//! - [`inputs`]: Content fingerprinting for preview build caching
//! - [`launcher`]: Preview support app lifecycle management

mod app_client;
mod hydrolysis;
mod inputs;
mod launcher;
pub mod protocol;

pub use app_client::PreviewAppClient;
pub use hydrolysis::{
    HydrolysisPreviewEventKind, HydrolysisPreviewPointerButton, HydrolysisPreviewRequest,
    HydrolysisPreviewScenario, HydrolysisPreviewScenarioEvent, HydrolysisPreviewSource,
    HydrolysisPreviewTheme, render_preview_with_hydrolysis, test_preview_with_hydrolysis,
};
pub use launcher::{PreviewSession, launch_preview_session};
pub use protocol::{PreviewPlatform, Size};
