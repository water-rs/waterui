#![doc = "Graphics primitives for `WaterUI`, recording Cherenkov `Content`."]

extern crate alloc;

/// Color types and conversion utilities.
pub mod color;
#[cfg(feature = "effects")]
mod effects;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod gradient;
#[cfg(feature = "gpu")]
mod image;
pub mod input;
#[cfg(any(feature = "gpu", feature = "cpu"))]
pub mod offscreen;
mod scene;
#[cfg(feature = "gpu")]
pub mod shader_paint;

/// The recorder vocabulary this crate records with: `Recorder`, `Draw`,
/// `Content`, `Paint`, `Shape`, the colour types and the plain resource ids.
/// Always available — it carries no engine.
pub use cherenkov_record as draw;

/// The engine this crate records for, re-exported so consumers that own an
/// engine name the same `Engine`, `Surface` and backend types.
#[cfg(feature = "cherenkov")]
pub use cherenkov;

pub use color::{Color, ColorScheme, Colorspace, CurrentColorScheme, WorkingColor};
#[cfg(feature = "effects")]
pub use effects::filter_view;
#[cfg(feature = "effects")]
pub use filter_view::{
    AnyEffect, BackgroundReplace, BlendWithImage, Bloom, Blur, Brightness, BumpDistortion,
    ColorMatrix, Contrast, Convolution3x3, Convolution5x5, Crystallize, DepthAwareBlur,
    DisplacementTransitionToImage, DisplacementWarp, DotHalftone, EdgeWork, Exposure,
    FilterDescription, FilterSignal, FilterViewExt, Filtered, FilteredView, Gamma, GaussianBlur,
    Gloom, Grayscale, GuidedSmooth, HighlightsShadows, HueRotation, Invert, Kaleidoscope,
    LineHalftone, LutColorGrade, MaskedBlur, Median3x3, MirrorTile, MorphologyGradient,
    MorphologyMax, MorphologyMin, MotionBlur, OutputSize, ParamGuards, PerspectiveCorrection,
    PerspectiveTransform, PhotoEffectChrome, PhotoEffectFade, PhotoEffectInstant, PhotoEffectMono,
    PhotoEffectNoir, PhotoEffectProcess, PhotoEffectTonal, PhotoEffectTransfer, PinchDistortion,
    Pixellate, Prewitt, RadialTransitionToImage, Reactive, RenderTransfer, Saturation, Sepia,
    Sharpen, Sobel, SwipeTransitionToImage, TemperatureTint, TemporalDenoise, ToneCurve,
    TransitionToImage, TwirlDistortion, UnsharpMask, Vibrance, Vignette, VortexDistortion,
    WhitePoint, ZoomBlur, ZoomTransitionToImage,
};
#[cfg(feature = "gpu")]
pub use gpu::{
    CaretQuery, Context, DeviceLoss, ExternalFrameSource, ExternalFrameStream, ExternalFrameView,
    Frame, FrameHook, FrameOutput, GpuContent, GpuContentHandle, GpuContentView, GpuRuntime,
    InputHandler, RedrawHandle, SharedGpuContext,
};
#[cfg(feature = "gpu")]
pub use gradient::{
    ANIMATED_MESH_PALETTE_LEN, AnimatedMeshGradient, AnimatedMeshGradientConfig, FlowingGradient,
};
pub use gradient::{Gradient, GradientType, MeshGradient};
#[cfg(feature = "gpu")]
pub use image::image_decode;
pub use input::{
    Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};
#[cfg(any(feature = "gpu", feature = "cpu"))]
pub use offscreen::{OffscreenError, OffscreenImage, OffscreenRenderer, OffscreenSize};
pub use scene::picture::{Picture, PictureRecording, PictureSource};
#[cfg(feature = "cherenkov")]
pub use scene::resources::SceneCaps;
pub use scene::resources::{
    Handle, HeldResources, PlainId, RecordingResources, Registered, ResourceHandle, SceneBackend,
    SceneResources, ShaderBackend,
};
pub use scene::scene_view::{
    SceneContent, SceneInvalidator, SceneView, SceneViewMergeToParent, invalidate_on_change,
    resolve_scene_proposal, scene_stretch_axis,
};
pub use scene::source::{
    FontSource, Format, ImageColorSpace, ImageData, ImageFormat, ResourceError, Rgba8, Rgba16F,
    ShaderLanguage, ShaderSource,
};
pub use scene::{picture, resources, scene_view, source};
#[cfg(feature = "gpu")]
pub use shader_paint::ShaderPaintView;

/// The CPU raster backend behind [`OffscreenRenderer::cpu`].
#[cfg(feature = "cpu")]
pub use cherenkov_cpu;
/// The CPU picture rasteriser used by hosts whose image views take bitmaps.
#[cfg(feature = "cpu")]
pub use scene::raster;

/// The GPU backend for hosts that own a retained engine.
#[cfg(feature = "gpu")]
pub use cherenkov_gpu;

/// The filter library `.filter(F)` and `.effect(E)` take, re-exported so a
/// filter written against it is the one the engine runs.
#[cfg(feature = "effects")]
pub use filtrate;

/// The exact `wgpu` this build links, re-exported so applications implementing
/// [`GpuContent`] cannot end up with a version-mismatched `wgpu`.
#[cfg(feature = "gpu")]
pub use wgpu;

/// Plain-bytes casts for the uniform/vertex data [`GpuContent`] writes to GPU
/// buffers, re-exported so a content author uses one version of it.
#[cfg(feature = "gpu")]
pub use bytemuck;
