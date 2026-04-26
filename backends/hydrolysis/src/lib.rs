//! Hydrolysis backend.
//!
//! The legacy `Node`/`RenderCommand` prototype has been removed.
//!
//! `HydrolysisExt` provides `.hydrolysis()` to wrap any cloneable view into
//! a `GpuSurface` rendered by hydrolysis.

mod animation;
mod engine;
mod env;
mod gesture;
mod gpu_view;
mod platform;
mod renderer;
mod runner;
mod scroll;
mod time;
mod view_renderer;
mod widgets;

pub use engine::{Brush, DrawContext, MaterialTheme, WidgetTheme};
pub use gpu_view::{HydrolysisExt, HydrolysisGpuView};
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use platform::BrowserWindow;
#[cfg(feature = "winit")]
pub use platform::WinitWindow;
pub use platform::{
    InputEvent, KeyCode, KeyState, Modifiers, OffscreenSurface, OffscreenWindow, PlatformWindow,
    PointerButton, SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose, TextInputState,
    TouchPhase,
};
pub use renderer::{HydroState, HydrolysisRenderTarget, HydrolysisRenderer, RenderContext};
pub use runner::run;
#[cfg(not(target_arch = "wasm32"))]
pub use runner::{HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot};
pub use view_renderer::HydrolysisViewRenderer;
