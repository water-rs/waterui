//! The contract a backend implements to realize this crate's leaves.
//!
//! Nothing here is for application code. A backend — Apple, Hydrolysis, or
//! one in its own repository — realizes the scene, picture and filter leaves
//! as native views, registers the resources their recordings name with its
//! render target, and records or runs them through the engine. This module
//! holds every type that work names, together with the engine vocabulary the
//! contract's signatures are spelled in, so a backend links the same copies
//! the framework records with.

/// The recorder vocabulary [`SceneContent`] records into and [`Picture`]
/// holds: `Recorder`, `Draw`, `Content`, `Paint`, the shapes, the colour
/// types, the resource ids and the `kurbo` geometry.
pub use cherenkov_record as draw;

/// The engine [`SceneBackend`] and [`ShaderBackend`] are implemented for:
/// `Engine`, `Surface` and the frame and error types a host that owns an
/// engine drives.
#[cfg(feature = "cherenkov")]
pub use cherenkov;

/// The GPU engine backend a host builds its `Engine` on: `Gpu`, its
/// configuration and the device interop.
#[cfg(feature = "gpu")]
pub use cherenkov_gpu;

/// The CPU raster engine backend, for hosts that render without a GPU.
#[cfg(feature = "cpu")]
pub use cherenkov_cpu;

/// The filter library a [`FilteredView`]'s effect is described and run in:
/// the stage, kind and parameter types a backend lowers into its own
/// primitives, and the `Effect` contexts it executes.
#[cfg(feature = "effects")]
pub use filtrate;

pub use crate::scene::picture::{Picture, PictureRecording, PictureSource};
#[cfg(feature = "cherenkov")]
pub use crate::scene::resources::SceneCaps;
pub use crate::scene::resources::{
    Handle, HeldResources, PlainId, RecordingResources, Registered, RegisteredText, ResourceHandle,
    SceneBackend, SceneResources, ShaderBackend, TextBackend, TextRegistration, TextRequest,
};
pub use crate::scene::scene_view::{
    NextFrame, SceneContent, SceneFrame, SceneInvalidator, SceneUpdate, SceneView,
    SceneViewMergeToParent, invalidate_on_change, resolve_scene_proposal, scene_stretch_axis,
};
pub use crate::scene::source::{
    FontSource, Format, ImageColorSpace, ImageData, ImageFormat, ResourceError, Rgba8, Rgba16F,
    ShaderLanguage, ShaderSource,
};

/// CPU rasterisation of a [`Picture`], for hosts whose image views take
/// bitmaps.
#[cfg(feature = "cpu")]
pub use crate::scene::raster;

#[cfg(feature = "gpu")]
pub use crate::effects::filter_view::ErasedEffect;
#[cfg(feature = "effects")]
pub use crate::effects::filter_view::{
    AnyEffect, FilterDescription, FilterSignal, FilteredView, OutputSize, ParamGuards, Reactive,
};

use waterui_core::AnyView;

/// The name of `view` when it is a leaf that needs a GPU device to render,
/// and `None` for any other view.
///
/// The leaves are `GpuContentView` — user GPU content, which
/// `ShaderPaintView`, `FlowingGradient` and the GPU video player render
/// through — and `ExternalFrameView`. A render target without a device
/// refuses them by this name. Whether they exist depends on this crate's
/// `gpu` feature, which any crate in the build can enable, so a backend
/// built without a GPU asks here instead of naming the types.
#[cfg(feature = "gpu")]
#[must_use]
pub fn gpu_device_view(view: &AnyView) -> Option<&'static str> {
    use waterui_core::Native;

    if view.is::<Native<crate::GpuContentView>>() {
        Some("GpuContentView")
    } else if view.is::<Native<crate::ExternalFrameView>>() {
        Some("ExternalFrameView")
    } else {
        None
    }
}

/// Always `None`: without the `gpu` feature this crate defines no leaf that
/// needs a GPU device.
#[cfg(not(feature = "gpu"))]
#[must_use]
pub const fn gpu_device_view(_view: &AnyView) -> Option<&'static str> {
    None
}
