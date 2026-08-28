//! Hydrolysis backend.
//!
//! `HydrolysisExt` provides `.hydrolysis()` to wrap any cloneable view into
//! a `GpuSurface` rendered by hydrolysis.

mod engine;
mod env;
mod gpu_view;
mod localization;
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
/// The W3C UI Events key vocabulary this backend speaks, re-exported so hosts
/// that synthesize key events use the same version of it.
pub use keyboard_types;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use platform::BrowserWindow;
#[cfg(feature = "winit")]
pub use platform::WinitWindow;
pub use platform::{
    InputEvent, KeyCode, KeyState, Modifiers, OffscreenGpuContext, OffscreenSurface,
    OffscreenWindow, PlatformWindow, PointerButton, PointerKind, SurfaceError, SurfaceFrame,
    SurfaceProvider, TextInputPurpose, TextInputState, TouchPhase,
};
pub use renderer::{HydroState, HydrolysisRenderTarget, HydrolysisRenderer, RenderContext};
pub use runner::run;
#[cfg(not(target_arch = "wasm32"))]
pub use runner::{
    FrameCounters, FramePhases, FrameProfile, HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot,
};
pub use view_renderer::HydrolysisViewRenderer;
#[cfg(hydrolysis_macos_system_webview)]
pub use widgets::platform::webview::MacSystemWebViewController;
