#![doc = "Graphics primitives for `WaterUI`, recording Cherenkov `Content`."]

extern crate alloc;

/// The bridge from the framework's animations to the engine's.
pub mod animation;
pub mod backend;
pub mod canvas;
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

// Crate-internal short paths: the recorder vocabulary this crate records
// with and the scene module's submodules. They are public only through
// [`backend`].
use cherenkov_record as draw;
use scene::{scene_view, source};

#[cfg(feature = "effects")]
pub use effects::filter_view;
#[cfg(feature = "gpu")]
pub use filter_view::WithEffect;
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
pub use gradient::FlowingGradient;
pub use gradient::{ANIMATED_MESH_PALETTE_LEN, AnimatedMeshGradient, AnimatedMeshGradientConfig};
pub use gradient::{Gradient, GradientType, MeshGradient};
#[cfg(feature = "gpu")]
pub use image::image_decode;
pub use input::{
    Code, GesturePhase, Key, Modifiers, NamedKey, ScrollPhase, ScrollUnit, SurfaceInputEvent,
    SurfacePointerButton,
};
#[cfg(any(feature = "gpu", feature = "cpu"))]
pub use offscreen::{OffscreenError, OffscreenImage, OffscreenRenderer, OffscreenSize};
#[cfg(feature = "gpu")]
pub use shader_paint::ShaderPaintView;
/// Color types and conversion utilities.
pub use waterui_color as color;
pub use waterui_color::{Color, ColorScheme, Colorspace, CurrentColorScheme, WorkingColor};
