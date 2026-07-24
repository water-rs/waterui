//! Hydrolysis backend.
//!
//! The legacy `Node`/`RenderCommand` prototype has been removed.
//!
//! `HydrolysisExt` provides `.hydrolysis()` to wrap any cloneable view into
//! a `GpuSurface` rendered by hydrolysis.

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
