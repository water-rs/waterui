//! Protocol definitions for preview daemon communication.
//!
//! This module defines the messages exchanged between:
//! - CLI → Daemon (preview requests via Unix socket)
//! - Daemon → Preview App (render commands via TCP)
//! - Preview App → Daemon (PNG data via TCP)

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ============================================================================
// CLI → Daemon protocol (Unix socket)
// ============================================================================

/// Request from CLI to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonRequest {
    /// Request to preview a specific view function.
    Preview(PreviewRequest),
    /// Shutdown the daemon gracefully.
    Shutdown,
    /// Ping to check if daemon is alive.
    Ping,
}

/// A preview request containing all info needed to render a view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewRequest {
    /// Absolute path to the project directory.
    pub project_path: PathBuf,
    /// Function path like `dashboard::admin::card`.
    pub function_path: String,
    /// Target platform.
    pub platform: PreviewPlatform,
    /// Frame size (width, height) in points.
    pub frame: (f32, f32),
}

/// Response from daemon to CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    /// Preview completed successfully with PNG data.
    Png(Vec<u8>),
    /// Preview failed with error message.
    Error(String),
    /// Build progress update.
    Progress(String),
    /// Pong response to ping.
    Pong,
}

/// Target platform for preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PreviewPlatform {
    /// Physical iOS device.
    Ios,
    /// iOS Simulator.
    IosSimulator,
    /// macOS.
    Macos,
    /// Android device or emulator.
    Android,
}

impl std::str::FromStr for PreviewPlatform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ios" => Ok(Self::Ios),
            "ios-simulator" | "iossimulator" => Ok(Self::IosSimulator),
            "macos" => Ok(Self::Macos),
            "android" => Ok(Self::Android),
            _ => Err(format!("Unknown platform: {s}")),
        }
    }
}

// ============================================================================
// Daemon → Preview App protocol (TCP, ports 2106+)
// These types match waterui-preview/src/protocol.rs exactly for serde compat
// ============================================================================

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

/// How to load the dylib in the preview app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DylibSource {
    /// Send new dylib bytes (first request or after code change).
    Bytes(Vec<u8>),
    /// Reuse the cached dylib (no code change since last request).
    Cached,
}

/// Request from daemon to preview app (TCP).
/// Must match `waterui_preview::protocol::PreviewRequest` for serde compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppRequest {
    /// Render a view from a dylib.
    Render {
        /// Dylib source - either new bytes or reuse cached.
        dylib: DylibSource,
        /// Symbol name (e.g., "waterui_preview_dashboard_admin_card").
        symbol: String,
        /// Frame size for rendering.
        frame: Size,
    },
    /// Shutdown the preview app.
    Shutdown,
}

/// Successful render output from preview app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppOutput {
    /// PNG image bytes.
    pub png_data: Vec<u8>,
}

/// Errors that can occur during preview rendering in the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppError {
    /// No cached dylib available - client should resend with `DylibSource::Bytes`.
    NoCachedDylib,
    /// Failed to load dylib.
    DylibLoad(String),
    /// Symbol not found in dylib.
    SymbolNotFound(String),
    /// Rendering failed.
    RenderFailed(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCachedDylib => write!(f, "No cached dylib available"),
            Self::DylibLoad(e) => write!(f, "Failed to load dylib: {e}"),
            Self::SymbolNotFound(s) => write!(f, "Symbol not found: {s}"),
            Self::RenderFailed(e) => write!(f, "Render failed: {e}"),
        }
    }
}

/// Response from preview app to daemon (TCP).
pub type AppResponse = Result<AppOutput, AppError>;

// ============================================================================
// Utility functions
// ============================================================================

/// Convert a function path to a symbol name.
///
/// Example: `test_preview` with crate `together-app` -> `waterui_preview_together_app_test_preview`
#[must_use]
pub fn function_path_to_symbol(crate_name: &str, function_path: &str) -> String {
    let fn_name = function_path
        .rsplit("::")
        .next()
        .unwrap_or(function_path);
    // Replace dashes with underscores (Cargo uses dashes, Rust uses underscores)
    let crate_name = crate_name.replace('-', "_");
    format!("waterui_preview_{crate_name}_{fn_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_path_to_symbol() {
        assert_eq!(
            function_path_to_symbol("my_crate", "sidebar"),
            "waterui_preview_my_crate_sidebar"
        );
        assert_eq!(
            function_path_to_symbol("together-app", "test_preview"),
            "waterui_preview_together_app_test_preview"
        );
    }
}
