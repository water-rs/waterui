//! Hydrolysis backend.
//!
//! The legacy `Node`/`RenderCommand` prototype has been removed.

mod animation;
mod platform;
mod renderer;
mod runner;
mod scroll;
mod view_renderer;

#[cfg(feature = "winit")]
pub use platform::WinitWindow;
pub use platform::{
    InputEvent, KeyCode, KeyState, Modifiers, OffscreenSurface, OffscreenWindow, PlatformWindow,
    PointerButton, SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose, TextInputState,
};
pub use renderer::{HydroState, HydrolysisRenderer, RenderContext};
pub use runner::run;
pub use view_renderer::HydrolysisViewRenderer;
