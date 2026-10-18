#[path = "engine/cherenkov_backend.rs"]
pub mod cherenkov;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use cherenkov::DomCherenkovSurface;
#[cfg(all(target_os = "macos", hydrolysis_winit))]
pub use cherenkov::WindowCherenkovSurface;
pub use cherenkov::{EngineSurface, TextureCherenkovSurface};
pub use cherenkov::{GpuEngine, SharedEngineState, format_output_color, shared_engine_state};
// `macro_rules!` re-exports cap at `pub(crate)` — see cherenkov_backend.
pub(crate) use cherenkov::{cfg_async_fn, engine_await};
