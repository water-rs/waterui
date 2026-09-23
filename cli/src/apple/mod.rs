//! Apple platform support.

/// Apple backend implementation.
pub mod backend;
/// Apple device detection and management.
pub mod device;
pub(crate) mod dynamic_runtime;
/// macOS local device gestures and screenshot.
#[cfg(target_os = "macos")]
pub mod local;
/// Non-macOS stub for local Apple automation APIs.
#[cfg(not(target_os = "macos"))]
pub mod local {
    use std::path::Path;

    use color_eyre::eyre::{self, eyre};

    /// Information about a macOS window.
    #[derive(Debug, Clone)]
    pub struct WindowInfo {
        /// The window ID (used for screencapture -l).
        pub window_id: u32,
        /// The window title/name.
        pub name: String,
        /// The owning process ID.
        pub owner_pid: i32,
        /// The owning application name.
        pub owner_name: String,
        /// Window layer (0 = normal windows).
        pub layer: i32,
    }

    fn unsupported() -> eyre::Report {
        eyre!("apple::local is only supported on macOS")
    }

    /// Stub: local window enumeration is unavailable outside macOS.
    pub fn list_windows_by_pid(_pid: i32) -> eyre::Result<Vec<WindowInfo>> {
        Err(unsupported())
    }

    /// Stub: local screenshot is unavailable outside macOS.
    pub async fn screenshot(_output: &Path) -> eyre::Result<()> {
        Err(unsupported())
    }

    /// Stub: local screenshot is unavailable outside macOS.
    pub async fn screenshot_bytes() -> eyre::Result<Vec<u8>> {
        Err(unsupported())
    }

    /// Stub: local screenshot is unavailable outside macOS.
    pub async fn screenshot_window(_window_id: u32, _output: &Path) -> eyre::Result<()> {
        Err(unsupported())
    }

    /// Stub: local screenshot is unavailable outside macOS.
    pub async fn screenshot_window_bytes(_window_id: u32) -> eyre::Result<Vec<u8>> {
        Err(unsupported())
    }

    /// Stub: local tap is unavailable outside macOS.
    pub async fn tap(_x: u32, _y: u32) -> eyre::Result<()> {
        Err(unsupported())
    }

    /// Stub: local swipe is unavailable outside macOS.
    pub async fn swipe(
        _from: (u32, u32),
        _to: (u32, u32),
        _duration_ms: Option<u32>,
    ) -> eyre::Result<()> {
        Err(unsupported())
    }

    /// Stub: local text input is unavailable outside macOS.
    pub async fn text(_input: &str) -> eyre::Result<()> {
        Err(unsupported())
    }
}
/// Apple platform configuration.
pub mod platform;
/// Apple toolchain management.
pub mod toolchain;
