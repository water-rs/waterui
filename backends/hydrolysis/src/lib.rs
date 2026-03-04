//! Hydrolysis backend.
//!
//! The legacy `Node`/`RenderCommand` prototype has been removed.
//!
//! `HydrolysisExt` provides `.hydrolysis()` to wrap any cloneable view into
//! a `GpuSurface` rendered by hydrolysis.

mod animation;
mod gesture;
mod gpu_view;
mod platform;
mod renderer;
mod runner;
mod scroll;
mod view_renderer;

pub use gpu_view::{HydrolysisExt, HydrolysisGpuView};
#[cfg(feature = "winit")]
pub use platform::WinitWindow;
pub use platform::{
    InputEvent, KeyCode, KeyState, Modifiers, OffscreenSurface, OffscreenWindow, PlatformWindow,
    PointerButton, SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose, TextInputState,
};
pub use renderer::{HydroState, HydrolysisRenderer, RenderContext};
pub use runner::run;
pub use view_renderer::HydrolysisViewRenderer;
