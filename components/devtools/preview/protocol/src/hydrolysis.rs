//! Offscreen Hydrolysis preview protocol.
//!
//! The CLI drives the generated Hydrolysis preview binary with a single JSON
//! run configuration (passed as a file path through
//! [`PREVIEW_RUN_CONFIG_ENV`]).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Environment variable carrying the path of the JSON-encoded
/// [`PreviewRunConfig`] for the generated preview binary.
pub const PREVIEW_RUN_CONFIG_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_RUN_CONFIG";

/// One offscreen Hydrolysis preview invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewRunConfig {
    /// Viewport width in logical units.
    pub width: f32,
    /// Viewport height in logical units.
    pub height: f32,
    /// What the run produces.
    pub mode: PreviewRunMode,
}

/// What a preview run produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreviewRunMode {
    /// A single PNG capture after the view tree has mounted.
    Image {
        /// Destination PNG path.
        output: PathBuf,
    },
    /// A timeline capture: frames at `captures_ms` with `events` replayed at
    /// their timestamps.
    Scenario {
        /// Directory receiving `frame-XXXXms.png` captures.
        output_dir: PathBuf,
        /// Capture timestamps in milliseconds from scenario start.
        captures_ms: Vec<u64>,
        /// Input events sorted by timestamp.
        events: Vec<ScenarioEvent>,
    },
    /// Semantic accessibility-tree assertions (no render target).
    Semantic,
}

/// One input event in a preview scenario timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScenarioEvent {
    /// Event timestamp in milliseconds from scenario start.
    pub at_ms: u64,
    /// Event kind.
    pub kind: ScenarioEventKind,
    /// Pointer x coordinate in logical units.
    pub x: f32,
    /// Pointer y coordinate in logical units.
    pub y: f32,
    /// Pointer button for down/up events.
    pub button: ScenarioPointerButton,
    /// Scroll delta along the x axis for scroll events.
    pub dx: f32,
    /// Scroll delta along the y axis for scroll events.
    pub dy: f32,
    /// Whether scroll delta values are line units instead of logical units.
    pub is_line_delta: bool,
}

/// Event kind in a preview scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScenarioEventKind {
    /// Move the pointer without pressing.
    PointerMove,
    /// Press a pointer button.
    PointerDown,
    /// Release a pointer button.
    PointerUp,
    /// Cancel the active pointer.
    PointerCancel,
    /// Dispatch a wheel or trackpad scroll event.
    Scroll,
}

/// Pointer button identifier for scenario events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScenarioPointerButton {
    /// The primary button.
    #[default]
    Primary,
    /// The secondary button.
    Secondary,
    /// The middle button.
    Middle,
}
