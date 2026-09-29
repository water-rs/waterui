/// A recorded drawing shown as a static image.
pub mod picture;
/// CPU rasterisation of a [`picture::Picture`] through `cherenkov_cpu`.
#[cfg(feature = "cpu")]
pub mod raster;
/// Engine-scoped resource registration for mounted scene content.
pub mod resources;
/// `WaterUI` view wrapper for scene rendering.
pub mod scene_view;
