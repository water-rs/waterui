//! Hydrolysis backend.
//!
//! The legacy `Node`/`RenderCommand` prototype has been removed.

mod renderer;
mod platform;
mod runner;

pub use platform::{
    InputEvent, KeyCode, KeyState, Modifiers, OffscreenSurface, OffscreenWindow, PlatformWindow,
    PointerButton, SurfaceError, SurfaceFrame, SurfaceProvider,
};
#[cfg(feature = "winit")]
pub use platform::WinitWindow;
pub use renderer::{HydroState, HydrolysisRenderer, RenderContext};
pub use runner::run;
