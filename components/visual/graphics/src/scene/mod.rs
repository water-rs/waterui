/// 2D scene model types.
pub mod scene2d;
/// 2D scenes over the CPU/GPU split renderer.
#[cfg(feature = "vello-scene")]
pub mod scene2d_hybrid;
/// 2D scenes over the Vello compute renderer.
#[cfg(feature = "vello-scene")]
pub mod scene2d_vello;
/// `GpuSurface` realization of a scene view.
#[cfg(feature = "gpu")]
pub mod scene_surface;
/// `WaterUI` view wrapper for scene rendering.
pub mod scene_view;
