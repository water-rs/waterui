#![doc = "Graphics primitives for `WaterUI`."]

extern crate alloc;

/// Color types and conversion utilities.
pub mod color;
mod effects;
mod gpu;
mod gradients;
mod image;
mod scene;

pub use color::{Color, Colorspace, ResolvedColor};
pub use effects::{filter_parity, filter_registry, filter_view, view_effect};
pub use gpu::{gpu_surface, prewarm, shader_surface, shared_context};
pub use gradients::{animated_mesh_gradient, flowing_gradient, gradient_renderer};
pub use image::{image_analysis, image_decode, image_generator};
pub use scene::{scene_view, scene2d, scene2d_vello};

/// Shared shader sources.
pub mod shaders;
/// Multi-input filters live in the `filtrate` crate; this alias keeps the
/// historical `waterui_graphics::multi_input_filter::*` import path working.
pub use filtrate::multi_input as multi_input_filter;

// Re-export key types for user convenience.
pub use gpu_surface::{
    GpuContext, GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenRenderError,
    OffscreenRenderOutput, OffscreenRenderOutputHdr, OffscreenSize, PointerState, RedrawHandle,
};

pub use shader_surface::ShaderSurface;

pub use animated_mesh_gradient::{
    ANIMATED_MESH_PALETTE_LEN, AnimatedMeshGradient, AnimatedMeshGradientConfig,
};
pub use gradient_renderer::{
    Gradient, GradientConfig, GradientType, MeshGradient, ResolvedGradient, ResolvedGradientStop,
};

pub use view_effect::{
    EffectRenderer, OutputSize, ViewEffect, ViewEffectContext, ViewEffectInput, ViewEffectOutput,
};

pub use filter_view::{
    AppliedFilter, Bloom, Blur, Brightness, BumpDistortion, ColorMatrix, Contrast, Crystallize,
    DotHalftone, EdgeWork, Exposure, FilterAdapter, FilterViewExt, Filtered, FilteredView, Gamma,
    GaussianBlur, Gloom, Grayscale, HdrPolicy, HighlightsShadows, HueRotation, Invert,
    Kaleidoscope, LineHalftone, MirrorTile, MotionBlur, PerspectiveCorrection,
    PerspectiveTransform, PinchDistortion, Pixellate, Saturation, Sepia, Sharpen, TemperatureTint,
    TwirlDistortion, UnsharpMask, Vibrance, Vignette, VortexDistortion, WhitePoint, ZoomBlur,
};
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

pub use image_analysis::{DominantColor, Histogram, ImageAnalysis, MinMaxLuma};
pub use image_generator::{
    CheckerboardGenerator, DotGridGenerator, GeneratedImage, ImageGenerator,
    LinearGradientGenerator, NoiseGenerator, RadialGradientGenerator, StripeGenerator,
};

pub use scene_view::{SceneContent, SceneInvalidator, SceneView, SceneViewMergeToParent};
pub use scene2d::Scene2D;
pub use scene2d_vello::VelloScene2D;

// Re-export dependencies used by macros
pub use inventory;
pub use rayon;

/// Re-export bytemuck for safe byte conversions in GPU programming.
pub use bytemuck;

pub use pollster;

/// Synchronously resolves the current top `wgpu` error scope by polling the device once.
///
/// This is intended for setup-time validation paths where `WaterUI` needs the scope result
/// immediately and can safely force a device poll.
///
/// # Panics
///
/// Panics if the error-scope future is still pending after `device.poll(Poll)`.
#[inline]
#[must_use]
pub fn pop_error_scope_now(device: &wgpu::Device, scope: &'static str) -> Option<wgpu::Error> {
    use core::future::Future as _;
    use core::pin::Pin;
    use core::task::{Context, Poll};

    let mut future = device.pop_error_scope();
    let mut future = unsafe { Pin::new_unchecked(&mut future) };
    let mut cx = Context::from_waker(core::task::Waker::noop());

    if let Poll::Ready(result) = future.as_mut().poll(&mut cx) {
        return result;
    }

    let _ = device.poll(wgpu::PollType::Poll);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(result) => result,
        Poll::Pending => {
            panic!("{scope}: pop_error_scope remained pending after device.poll(Poll)")
        }
    }
}
