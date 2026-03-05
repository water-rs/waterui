#![doc = "Graphics primitives for `WaterUI`."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// Color types and conversion utilities.
pub mod color;
pub use color::{Color, Colorspace, ResolvedColor};

/// High-performance GPU rendering surface using wgpu (advanced API).
pub mod gpu_surface;

/// Simplified shader-based GPU surface (intermediate API).
pub mod shader_surface;

/// GPU-animated mesh gradient.
pub mod animated_mesh_gradient;
/// GPU-animated flowing gradient.
pub mod flowing_gradient;
/// Gradient primitives (native linear/radial/angular + GPU mesh gradients).
pub mod gradient_renderer;

/// Shared GPU context for efficient multi-view rendering.
pub mod shared_context;

/// Shader pre-warming functionality.
pub mod prewarm;

/// Shared shader sources.
pub mod shaders;

/// Engine-neutral 2D scene abstraction.
pub mod scene2d;

/// Vello-backed Scene2D implementation.
pub mod scene2d_vello;

/// Scene-content view abstraction and GPU-backed fallback renderer.
pub mod scene_view;

/// GPU effect rendering for captured view content.
pub mod view_effect;

/// Filter-based view effects using the Filter trait system.
pub mod filter_view;
/// Multi-input filters (blend/mask/transition/displacement/depth/temporal).
pub mod multi_input_filter;

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
    EffectContext, EffectInput, EffectOutput, EffectRenderer, OutputSize, ViewEffect,
};

pub use filter_view::{
    AppliedFilter, Blur, Brightness, Contrast, FilterAdapter, FilterContext, FilterInput,
    FilterOutput, FilterViewExt, Filtered, FilteredView, GpuFilter, Grayscale, HdrPolicy,
    HueRotation, Invert, Saturation, Sepia, Sharpen, Vignette,
};
pub use multi_input_filter::{
    BackgroundReplace, BackgroundReplaceFilter, BlendMode, BlendWithImage, BlendWithImageFilter,
    DepthAwareBlur, DepthAwareBlurFilter, DisplacementWarp, DisplacementWarpFilter, FilterImage,
    GuidedSmooth, GuidedSmoothFilter, LutColorGrade, LutColorGradeFilter, LutImage, MaskedBlur,
    MaskedBlurFilter, MultiInputFilter, TemporalDenoise, TemporalDenoiseFilter, ToneCurve,
    ToneCurveFilter, TransitionToImage, TransitionToImageFilter, background_replace_filter,
    blend_with_image_filter, depth_aware_blur_filter, displacement_warp_filter,
    guided_smooth_filter, lut_color_grade_filter, masked_blur_filter, temporal_denoise_filter,
    tone_curve_filter, transition_to_image_filter,
};

pub use scene_view::{SceneContent, SceneInvalidator, SceneView, SceneViewMergeToParent};
pub use scene2d::Scene2D;
pub use scene2d_vello::VelloScene2D;

// Re-export dependencies used by macros
pub use inventory;
pub use rayon;

// Re-export wgpu and bytemuck for users to access GPU types directly.
pub use wgpu;

/// Re-export bytemuck for safe byte conversions in GPU programming.
pub use bytemuck;

pub use pollster;

#[inline]
pub(crate) fn ready_now_or_panic<F>(future: F, scope: &'static str) -> F::Output
where
    F: core::future::Future,
{
    use core::pin::Pin;
    use core::task::{Context, Poll};

    let mut future = future;
    let mut future = unsafe { Pin::new_unchecked(&mut future) };
    let mut cx = Context::from_waker(core::task::Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("{scope}: future returned Pending in synchronous path"),
    }
}

#[inline]
pub(crate) fn pop_error_scope_now(
    device: &wgpu::Device,
    scope: &'static str,
) -> Option<wgpu::Error> {
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
