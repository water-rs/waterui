/// A recorded drawing shown as a static image.
pub mod picture;
/// CPU rasterisation of a [`picture::Picture`] through `cherenkov_cpu`.
#[cfg(feature = "cpu")]
pub mod raster;
/// Target-scoped resource registration for scene content.
pub mod resources;
/// `WaterUI` view wrapper for scene rendering.
pub mod scene_view;
/// The resource-source vocabulary [`resources::SceneResources`] registers.
pub mod source;
