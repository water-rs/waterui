//! TCP protocol definitions for preview daemon communication.
//!
//! Defines the request/response types used between the CLI and preview daemon app.

use serde::{Deserialize, Serialize};

/// Frame size for rendering.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Size {
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
}

impl Size {
    /// Create a new size.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// How to load the dylib.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DylibSource {
    /// Send new dylib bytes (first request or after code change).
    Bytes(Vec<u8>),
    /// Reuse the cached dylib (no code change since last request).
    Cached,
}

/// Request from CLI to preview daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreviewRequest {
    /// Render a view from a dylib.
    Render {
        /// Dylib source - either new bytes or reuse cached.
        dylib: DylibSource,
        /// Symbol name (e.g., "waterui_preview_dashboard_admin_card").
        symbol: String,
        /// Frame size for rendering.
        frame: Size,
    },
    /// Shutdown the daemon.
    Shutdown,
}

/// Successful render output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewOutput {
    /// PNG image bytes.
    pub png_data: Vec<u8>,
}

/// Errors that can occur during preview rendering.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum PreviewError {
    /// No cached dylib available - client should resend with `DylibSource::Bytes`.
    #[error("No cached dylib available")]
    NoCachedDylib,
    /// Failed to load dylib.
    #[error("Failed to load dylib: {0}")]
    DylibLoad(String),
    /// Symbol not found in dylib.
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),
    /// Rendering failed.
    #[error("Render failed: {0}")]
    RenderFailed(String),
}

/// Response from preview daemon (Result-based).
pub type PreviewResponse = Result<PreviewOutput, PreviewError>;
