//! Hydrolysis backend.
//!
//! `HydrolysisExt` provides `.hydrolysis()` to wrap any cloneable view into
//! a `GpuSurface` rendered by hydrolysis.

// Every `unsafe` here must say why it is sound, in a form the compiler checks.
// The crate is at zero; the FFI layer and the platform bridges are not yet, which
// is why this is set per-crate rather than in the workspace lint table.
#![warn(clippy::undocumented_unsafe_blocks)]

mod engine;
mod env;
mod gpu_view;
mod platform;
mod readback;
mod renderer;
mod runner;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod view_renderer;
mod widgets;

// Interaction/runtime layer shared with other self-drawn backends.
pub(crate) use waterui_backend_core::{animation, gesture, scroll, time};

pub use engine::{Brush, DrawContext, WidgetTheme};
pub use gpu_view::{HydrolysisExt, HydrolysisGpuView};
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use platform::BrowserWindow;
#[cfg(feature = "winit")]
pub use platform::WinitWindow;
pub use platform::{
    InputEvent, KeyCode, KeyState, Modifiers, OffscreenSurface, OffscreenWindow, PlatformWindow,
    PointerButton, PointerKind, SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose,
    TextInputState, TouchPhase,
};
pub use renderer::{HydroState, HydrolysisRenderTarget, HydrolysisRenderer, RenderContext};
pub use runner::run;
#[cfg(not(target_arch = "wasm32"))]
pub use runner::{
    FrameCounters, FramePhases, FrameProfile, HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot,
};
pub use view_renderer::HydrolysisViewRenderer;

/// Executes the process as a packaged CEF subprocess helper.
///
/// # Panics
///
/// Panics when this backend was built without a CEF-backed browser component,
/// or when the process was not launched with a Chromium subprocess type.
#[must_use]
pub fn run_browser_subprocess() -> i32 {
    #[cfg(any(hydrolysis_cef_webview, feature = "chromium"))]
    {
        return waterui_browser_cef::run_packaged_subprocess();
    }
    #[cfg(not(any(hydrolysis_cef_webview, feature = "chromium")))]
    panic!("this Hydrolysis backend was built without a CEF browser component")
}
