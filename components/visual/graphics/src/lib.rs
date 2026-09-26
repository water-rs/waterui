#![doc = "Graphics primitives for `WaterUI`, recording Cherenkov `Content`."]

extern crate alloc;

/// Color types and conversion utilities.
pub mod color;
/// cbindgen:ignore
#[cfg(feature = "gpu")]
mod effects;
#[cfg(feature = "gpu")]
pub mod gpu;
mod gradients;
#[cfg(feature = "gpu")]
mod image;
pub mod input;
#[cfg(any(feature = "gpu", feature = "cpu"))]
pub mod offscreen;
mod scene;
pub mod shader_paint;

/// The engine this crate records for, re-exported so every consumer names
/// the same `Content`, `Paint`, `Shape` and colour types.
pub use cherenkov;

pub use color::{Color, ColorScheme, Colorspace, CurrentColorScheme, WorkingColor};
#[cfg(feature = "gpu")]
pub use effects::filter_view;
#[cfg(feature = "gpu")]
pub use filter_view::{
    AnyEffect, BackgroundReplace, BlendWithImage, Bloom, Blur, Brightness, BumpDistortion,
    ColorMatrix, Contrast, Convolution3x3, Convolution5x5, Crystallize, DepthAwareBlur,
    DisplacementTransitionToImage, DisplacementWarp, DotHalftone, EdgeWork, Exposure,
    FilterViewExt, Filtered, FilteredView, Gamma, GaussianBlur, Gloom, Grayscale, GuidedSmooth,
    HighlightsShadows, HueRotation, Invert, Kaleidoscope, LineHalftone, LutColorGrade, MaskedBlur,
    Median3x3, MirrorTile, MorphologyGradient, MorphologyMax, MorphologyMin, MotionBlur,
    ParamGuards, PerspectiveCorrection, PerspectiveTransform, PhotoEffectChrome, PhotoEffectFade,
    PhotoEffectInstant, PhotoEffectMono, PhotoEffectNoir, PhotoEffectProcess, PhotoEffectTonal,
    PhotoEffectTransfer, PinchDistortion, Pixellate, Prewitt, RadialTransitionToImage, Reactive,
    Saturation, Sepia, Sharpen, Sobel, SwipeTransitionToImage, TemperatureTint, TemporalDenoise,
    ToneCurve, TransitionToImage, TwirlDistortion, UnsharpMask, Vibrance, Vignette,
    VortexDistortion, WhitePoint, ZoomBlur, ZoomTransitionToImage,
};
#[cfg(feature = "gpu")]
pub use gpu::{
    CaretQuery, Context, Frame, FrameHook, GpuContent, GpuContentView, InputHandler, RedrawHandle,
};
pub use gradients::gradient::{Gradient, GradientType};
#[cfg(feature = "gpu")]
pub use image::image_decode;
pub use input::{
    Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};
pub use scene::picture::Picture;
pub use scene::resources::{Scene, SceneBackend, SceneResources};
pub use scene::scene_view::{
    SceneContent, SceneInvalidator, SceneView, invalidate_on_change, resolve_scene_proposal,
    scene_stretch_axis,
};
pub use scene::{picture, resources, scene_view};
pub use shader_paint::ShaderPaintView;

/// The CPU raster backend behind [`OffscreenRenderer::cpu`](offscreen::OffscreenRenderer::cpu).
#[cfg(feature = "cpu")]
pub use cherenkov_cpu;

/// The GPU backend for hosts that own a retained engine.
#[cfg(feature = "gpu")]
pub use cherenkov_gpu;

/// The filter library `.filter(F)` and `.effect(E)` take, re-exported so a
/// filter written against it is the one the engine runs.
#[cfg(feature = "gpu")]
pub use filtrate;

/// The exact `wgpu` this build links, re-exported so applications implementing
/// [`GpuContent`] cannot end up with a version-mismatched `wgpu`.
#[cfg(feature = "gpu")]
pub use wgpu;
