//! Hydrolysis backend.
//!
//! The legacy `Node`/`RenderCommand` prototype has been removed.

mod animation;
mod platform;
mod renderer;
mod runner;

#[cfg(feature = "winit")]
pub use platform::WinitWindow;
pub use platform::{
    InputEvent, KeyCode, KeyState, Modifiers, OffscreenSurface, OffscreenWindow, PlatformWindow,
    PointerButton, SurfaceError, SurfaceFrame, SurfaceProvider,
};
pub use renderer::{HydroState, HydrolysisRenderer, RenderContext};
pub use runner::run;
