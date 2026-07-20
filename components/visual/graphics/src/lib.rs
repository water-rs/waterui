#![doc = "Graphics primitives for `WaterUI`."]

extern crate alloc;

/// Color types and conversion utilities.
pub mod color;
#[cfg(feature = "gpu")]
mod effects;
#[cfg(feature = "gpu")]
mod gpu;
#[cfg(feature = "gpu")]
mod gradients;
#[cfg(feature = "gpu")]
mod image;
#[cfg(feature = "gpu")]
mod scene;
#[cfg(feature = "gpu")]
pub mod shader_types;

pub use color::{Color, Colorspace, ResolvedColor};
#[cfg(feature = "gpu")]
pub use effects::{filter_view, view_effect};
#[cfg(feature = "gpu")]
pub use gpu::{gpu_surface, reactive_color, shader_source, shader_surface, shared_context};
#[cfg(feature = "gpu")]
pub use gradients::{animated_mesh_gradient, flowing_gradient, gradient_renderer};
#[cfg(feature = "gpu")]
pub use image::{image_analysis, image_decode, image_generator};
#[cfg(feature = "gpu")]
pub use scene::{scene_view, scene2d, scene2d_vello};

/// Shared shader sources.
#[cfg(feature = "gpu")]
pub mod shaders;
/// Multi-input filters live in the `filtrate` crate; this alias keeps the
/// historical `waterui_graphics::multi_input_filter::*` import path working.
#[cfg(feature = "gpu")]
pub use filtrate::multi_input as multi_input_filter;

// Re-export key types for user convenience.
#[cfg(feature = "gpu")]
pub use gpu_surface::{
    GpuContext, GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenRenderError,
    OffscreenRenderOutput, OffscreenRenderOutputHdr, OffscreenSize, PointerState, RedrawHandle,
};

#[cfg(feature = "gpu")]
pub use shader_surface::ShaderSurface;
#[cfg(feature = "gpu")]
pub use shared_context::{GpuRuntime, SharedContextError, SharedGpuContext};

#[cfg(feature = "gpu")]
pub use animated_mesh_gradient::{
    ANIMATED_MESH_PALETTE_LEN, AnimatedMeshGradient, AnimatedMeshGradientConfig,
};
#[cfg(feature = "gpu")]
pub use gradient_renderer::{
    Gradient, GradientConfig, GradientType, MeshGradient, ResolvedGradient, ResolvedGradientStop,
};

#[cfg(feature = "gpu")]
pub use view_effect::{
    EffectRenderer, OutputSize, ViewEffect, ViewEffectContext, ViewEffectInput, ViewEffectOutput,
};

#[cfg(feature = "gpu")]
pub use filter_view::{
    AppliedFilter, Bloom, Blur, Brightness, BumpDistortion, ColorMatrix, Contrast, Crystallize,
    DotHalftone, EdgeWork, Exposure, FilterAdapter, FilterViewExt, Filtered, Gamma, GaussianBlur,
    Gloom, Grayscale, HdrPolicy, HighlightsShadows, HueRotation, Invert, Kaleidoscope,
    LineHalftone, MirrorTile, MotionBlur, PerspectiveCorrection, PerspectiveTransform,
    PinchDistortion, Pixellate, Saturation, Sepia, Sharpen, TemperatureTint, TwirlDistortion,
    UnsharpMask, Vibrance, Vignette, VortexDistortion, WhitePoint, ZoomBlur,
};
#[cfg(feature = "gpu")]
pub use multi_input_filter::{
    BackgroundReplace, BackgroundReplaceFilter, BlendMode, BlendWithImage, BlendWithImageFilter,
    DepthAwareBlur, DepthAwareBlurFilter, DisplacementTransitionToImage,
    DisplacementTransitionToImageFilter, DisplacementWarp, DisplacementWarpFilter, FilterImage,
    GuidedSmooth, GuidedSmoothFilter, LutColorGrade, LutColorGradeFilter, LutImage, MaskedBlur,
    MaskedBlurFilter, MultiInputFilter, RadialTransitionToImage, RadialTransitionToImageFilter,
    SwipeTransitionToImage, SwipeTransitionToImageFilter, TemporalDenoise, TemporalDenoiseFilter,
    ToneCurve, ToneCurveFilter, TransitionDirection, TransitionToImage, TransitionToImageFilter,
    ZoomTransitionToImage, ZoomTransitionToImageFilter, background_replace_filter,
    blend_with_image_filter, depth_aware_blur_filter, displacement_transition_to_image_filter,
    displacement_warp_filter, guided_smooth_filter, lut_color_grade_filter, masked_blur_filter,
    radial_transition_to_image_filter, swipe_transition_to_image_filter, temporal_denoise_filter,
    tone_curve_filter, transition_to_image_filter, zoom_transition_to_image_filter,
};

#[cfg(feature = "gpu")]
pub use image_analysis::{DominantColor, Histogram, ImageAnalysis, MinMaxLuma};
#[cfg(feature = "gpu")]
pub use image_generator::{
    CheckerboardGenerator, DotGridGenerator, GeneratedImage, ImageGenerator,
    LinearGradientGenerator, NoiseGenerator, RadialGradientGenerator, StripeGenerator,
};

#[cfg(feature = "gpu")]
pub use scene_view::{SceneContent, SceneInvalidator, SceneView, SceneViewMergeToParent};
#[cfg(feature = "gpu")]
pub use scene2d::Scene2D;
#[cfg(feature = "gpu")]
pub use scene2d_vello::VelloScene2D;

// Re-export dependencies used by macros
#[cfg(feature = "gpu")]
pub use rayon;

/// Re-export bytemuck for safe byte conversions in GPU programming.
#[cfg(feature = "gpu")]
pub use bytemuck;
