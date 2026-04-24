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
// Re-exported from `waterui-preview-protocol` for serde compat across CLI/app
// ============================================================================

pub use waterui_preview_protocol::{
    DylibId, DylibSource, PreviewError as AppError, PreviewOutput as AppOutput,
    PreviewRequest as AppRequest, PreviewResponse as AppResponse, PreviewRuntimePlatform, Size,
};

pub use waterui_preview_protocol::tcp::PreviewTcpConfig;

// ============================================================================
// Utility functions
// ============================================================================

/// Convert a function path to preview export symbol.
///
/// Example:
/// - `sidebar` with crate `my-crate` -> `waterui_preview_my_crate_sidebar`
/// - `dashboard::admin::card_preview` with crate `my-crate`
///   -> `waterui_preview_my_crate_dashboard_admin_card_preview`
#[must_use]
pub fn function_path_to_symbol(crate_name: &str, function_path: &str) -> String {
    // Replace dashes with underscores (Cargo uses dashes, Rust uses underscores)
    let crate_name = crate_name.replace('-', "_");
    let full_path = function_path.replace("::", "_");
    format!("waterui_preview_{crate_name}_{full_path}")
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
            function_path_to_symbol("my-crate", "dashboard::admin::card_preview"),
            "waterui_preview_my_crate_dashboard_admin_card_preview"
        );
    }
}
