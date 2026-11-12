#[path = "engine/cherenkov_backend.rs"]
pub mod cherenkov;

#[cfg(all(target_os = "android", not(hydrolysis_hwui)))]
pub use cherenkov::AndroidCherenkovSurface;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use cherenkov::DomCherenkovSurface;
pub use cherenkov::EngineSurface;
#[cfg(all(target_os = "macos", hydrolysis_winit))]
pub use cherenkov::WindowCherenkovSurface;
#[cfg(hydrolysis_gpu_painter)]
pub use cherenkov::{GpuEngine, SharedEngineState, TextureCherenkovSurface, format_output_color};
#[cfg(hydrolysis_cpu_painter)]
pub use cherenkov::{RasterCherenkovSurface, RasterEngine, RasterEngineState};
// `macro_rules!` re-exports cap at `pub(crate)` — see cherenkov_backend.
pub(crate) use cherenkov::{cfg_async_fn, engine_await};
