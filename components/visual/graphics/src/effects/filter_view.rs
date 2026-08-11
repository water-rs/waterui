//! GPU filter processing for captured view content.
//!
//! This module provides the `Effect` trait for implementing GPU-based filters
//! that process captured view textures. Native backends capture child views to
//! textures, pass them to Rust for GPU processing via wgpu.
//!
//! # Architecture
//!
//! 1. Native backend captures child view to a texture
//! 2. Native calls into Rust with input texture handle
//! 3. Rust applies filter pipeline via wgpu
//! 4. Result written to output texture for native to display
//!
//! # Animation Support
//!
//! Filters support Rust-side animation interpolation. When a reactive value
//! changes with animation metadata, the filter smoothly interpolates between
//! values and signals `needs_redraw = true` until the animation completes.

extern crate alloc;

use alloc::{boxed::Box, sync::Arc};
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::time::Duration;
// Keep the filter authoring and runtime contracts in one public module.
pub use filtrate::effect::EffectRedrawCallback;
pub use filtrate::{
    Effect, EffectContext, EffectFrameClock, EffectFrameTiming, EffectInput, EffectOutput,
    EffectRenderResult, EffectSetupResult, FilterAdapter, HdrPolicy, WgslModuleCache,
};
use filtrate_core::{
    AnimatedCallback, AnimatedTarget, Chain, Filter, FilterParam, Interpolator, WatchGuard,
};
use nami::{Computed, Signal, SignalExt as _};
use waterui_core::animation::Animation as WuiAnimation;
use waterui_core::layout::StretchAxis;
use waterui_core::metadata::MetadataKey;
use waterui_core::{Environment, IntoSignalF32, Metadata, View};

use crate::gpu_surface::RedrawHandle;

type ErasedEffectSetupFuture<'a> = Pin<Box<dyn Future<Output = EffectSetupResult> + 'a>>;

trait ErasedEffect: 'static {
    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> ErasedEffectSetupFuture<'a>;
    fn render(&mut self, input: &EffectInput, output: &EffectOutput) -> EffectRenderResult;
    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult;
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32);
    fn redraw_hint(&self) -> bool;
}

impl<T: Effect> ErasedEffect for T {
    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> ErasedEffectSetupFuture<'a> {
        Box::pin(Effect::setup(self, ctx))
    }

    fn render(&mut self, input: &EffectInput, output: &EffectOutput) -> EffectRenderResult {
        Effect::render(self, input, output)
    }

    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        Effect::encode_render(self, input, output, encoder)
    }

    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        Effect::output_size(self, input_width, input_height)
    }

    fn redraw_hint(&self) -> bool {
        Effect::redraw_hint(self)
    }
}

type ReactiveParam = Reactive<Computed<f32>>;
type ChainedFilter<V, F, Next> = Filtered<V, FilterAdapter<Chain<F, Next>>>;
type TemperatureTintFilter = filtrate::filters::TemperatureTint<ReactiveParam, ReactiveParam>;
type HighlightsShadowsFilter = filtrate::filters::HighlightsShadows<ReactiveParam, ReactiveParam>;
type VignetteFilter = filtrate::filters::Vignette<ReactiveParam, ReactiveParam>;
type MotionBlurFilter = filtrate::filters::MotionBlur<ReactiveParam, ReactiveParam>;
type WhitePointFilter = filtrate::filters::WhitePoint<ReactiveParam, ReactiveParam, ReactiveParam>;
type ZoomBlurFilter = filtrate::filters::ZoomBlur<ReactiveParam, ReactiveParam, ReactiveParam>;

const fn temperature_tint_filter(
    temperature: ReactiveParam,
    tint: ReactiveParam,
) -> TemperatureTintFilter {
    filtrate::filters::TemperatureTint(temperature, tint)
}

/// Type-erased filter for FFI boundary.
///
/// This owns the private type-erased effect shim used by native backends.
pub struct AppliedFilter {
    filter: Box<dyn ErasedEffect>,
    redraw_handle: RedrawHandle,
}

impl fmt::Debug for AppliedFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppliedFilter").finish_non_exhaustive()
    }
}

impl MetadataKey for AppliedFilter {}

impl AppliedFilter {
    /// Create a new `AppliedFilter` from a GPU filter.
    pub fn new<F: Effect>(mut filter: F) -> Self {
        let redraw_handle = RedrawHandle::new();
        let effect_redraw_handle = redraw_handle.clone();
        filter.set_redraw_callback(Arc::new(move || {
            effect_redraw_handle.request_redraw();
        }));
        Self {
            filter: Box::new(filter),
            redraw_handle,
        }
    }

    /// Returns a clone of the handle used to wake the native renderer.
    #[must_use]
    pub fn redraw_handle(&self) -> RedrawHandle {
        self.redraw_handle.clone()
    }

    /// Calls `setup` on the filter, returning a future that completes when ready.
    #[expect(
        clippy::future_not_send,
        reason = "effect setup is driven by the UI-local GPU host executor and intentionally accepts non-Send effects"
    )]
    pub fn setup<'a>(
        &'a mut self,
        ctx: &'a EffectContext<'a>,
    ) -> impl Future<Output = EffectSetupResult> + 'a {
        self.filter.setup(ctx)
    }

    /// Calls `render` on the filter.
    ///
    /// Returns `Ok(true)` if another frame is needed (animation in progress).
    ///
    /// # Errors
    ///
    /// Propagates the wrapped filter's render failure.
    pub fn render(&mut self, input: &EffectInput, output: &EffectOutput) -> EffectRenderResult {
        let _ = self.redraw_handle.take_dirty();
        let result = self.filter.render(input, output);
        let callback_requested_redraw = self.redraw_handle.take_dirty();
        result.map(|needs_redraw| needs_redraw || callback_requested_redraw)
    }

    /// Encodes `render` into an existing command encoder.
    ///
    /// Returns `Ok(true)` if another frame is needed (animation in progress).
    ///
    /// # Errors
    ///
    /// Propagates the wrapped filter's render failure.
    pub fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        let _ = self.redraw_handle.take_dirty();
        let result = self.filter.encode_render(input, output, encoder);
        let callback_requested_redraw = self.redraw_handle.take_dirty();
        result.map(|needs_redraw| needs_redraw || callback_requested_redraw)
    }

    /// Resolve the current output dimensions from snapped filter state.
    #[must_use]
    pub fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        self.filter.output_size(input_width, input_height)
    }

    /// Query whether this filter needs a redraw even without layout changes.
    #[must_use]
    pub fn redraw_hint(&self) -> bool {
        self.redraw_handle.is_dirty() || self.filter.redraw_hint()
    }
}

/// Public filter wrapper returned by `ViewExt` APIs.
///
/// `Filtered` preserves the concrete content type for fluent chaining and lowers
/// directly to the backend [`AppliedFilter`] metadata node.
pub struct Filtered<V: View, F: Effect> {
    content: V,
    filter: F,
}

impl<V: View, F: Effect> fmt::Debug for Filtered<V, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Filtered").finish_non_exhaustive()
    }
}

impl<V: View, F: Effect> Filtered<V, F> {
    /// Create a new filtered view with a `Effect`.
    #[must_use]
    pub const fn new(content: V, filter: F) -> Self {
        Self { content, filter }
    }
}

#[allow(private_bounds)]
impl<V: View, F: Filter> Filtered<V, FilterAdapter<F>> {
    /// Chain another filter onto this view.
    ///
    /// Returns a new `Filtered` with the filters chained together.
    /// Consecutive color-only filters will be fused into a single GPU pass.
    ///
    /// # Example
    ///
    /// ```ignore
    /// my_view
    ///     .blur(10.0)
    ///     .then(Brightness(0.2))
    ///     .then(Contrast(1.5))
    /// ```
    #[must_use]
    pub fn then<F2: Filter>(self, filter: F2) -> Filtered<V, FilterAdapter<Chain<F, F2>>> {
        Filtered::new(self.content, self.filter.then(filter))
    }

    /// Set HDR behavior policy for this filtered view.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn hdr_policy(mut self, policy: HdrPolicy) -> Self {
        self.filter = self.filter.hdr_policy(policy);
        self
    }

    /// Require HDR intermediates; setup fails if unsupported.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn require_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::RequireHdr)
    }

    /// Prefer HDR intermediates with automatic LDR fallback.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn prefer_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::PreferHdr)
    }

    /// Force LDR intermediates for compatibility/performance.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn force_ldr(self) -> Self {
        self.hdr_policy(HdrPolicy::ForceLdr)
    }
}

// ============================================================================
// Auto-fusion inherent methods
// ============================================================================
//
// `Filtered<V, FilterAdapter<F>>` lets users continue chaining built-in filters
// without losing fusion. When the receiver of `.brightness(0.2)` is already a
// `Filtered<V, FilterAdapter<F>>`, Rust's method resolution picks these
// inherent methods over the trait-method counterparts on `FilterViewExt`,
// so `view.blur(5).brightness(0.2)` extends the existing chain instead of
// wrapping the whole filtered view in a second adapter.

/// Internal helper: declare an inherent auto-fusion method on
/// `Filtered<V, FilterAdapter<F>>` that appends a single-parameter built-in
/// filter to the chain.
macro_rules! inherent_single_param_filter {
    ($method:ident, $filter:ident) => {
        #[doc = concat!("Append a `", stringify!($filter), "` filter to the existing chain.")]
        ///
        /// This extends the running `Filtered<V, FilterAdapter<...>>` instead
        /// of starting a new adapter, preserving compile-time fusion.
        #[must_use]
        pub fn $method<P: IntoSignalF32>(
            self,
            value: P,
        ) -> Filtered<V, FilterAdapter<Chain<F, filtrate::filters::$filter<Reactive<Computed<f32>>>>>>
        {
            self.then(filtrate::filters::$filter(Reactive(
                value.into_signal_f32().computed(),
            )))
        }
    };
}

#[allow(private_bounds)]
impl<V: View, F: Filter> Filtered<V, FilterAdapter<F>> {
    inherent_single_param_filter!(blur, Blur);
    inherent_single_param_filter!(brightness, Brightness);
    inherent_single_param_filter!(contrast, Contrast);
    inherent_single_param_filter!(crystallize, Crystallize);
    inherent_single_param_filter!(exposure, Exposure);
    inherent_single_param_filter!(gamma, Gamma);
    inherent_single_param_filter!(gaussian_blur, GaussianBlur);
    inherent_single_param_filter!(grayscale, Grayscale);
    inherent_single_param_filter!(hue_rotation, HueRotation);
    inherent_single_param_filter!(pixellate, Pixellate);
    inherent_single_param_filter!(saturation, Saturation);
    inherent_single_param_filter!(sepia, Sepia);
    inherent_single_param_filter!(sharpen, Sharpen);
    inherent_single_param_filter!(vibrance, Vibrance);

    /// Append an `Invert` filter to the chain (zero parameters).
    #[must_use]
    pub fn invert(self) -> Filtered<V, FilterAdapter<Chain<F, filtrate::filters::Invert>>> {
        self.then(filtrate::filters::Invert)
    }

    /// Append a two-parameter `TemperatureTint` filter to the chain.
    #[must_use]
    pub fn temperature_tint<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        temperature: T,
        tint: U,
    ) -> ChainedFilter<V, F, TemperatureTintFilter> {
        self.then(temperature_tint_filter(
            Reactive(temperature.into_signal_f32().computed()),
            Reactive(tint.into_signal_f32().computed()),
        ))
    }

    /// Append a `HighlightsShadows` filter to the chain.
    #[must_use]
    pub fn highlights_shadows<H: IntoSignalF32, S: IntoSignalF32>(
        self,
        highlights: H,
        shadows: S,
    ) -> ChainedFilter<V, F, HighlightsShadowsFilter> {
        self.then(filtrate::filters::HighlightsShadows(
            Reactive(highlights.into_signal_f32().computed()),
            Reactive(shadows.into_signal_f32().computed()),
        ))
    }

    /// Append a `Vignette` filter to the chain.
    #[must_use]
    pub fn vignette<R: IntoSignalF32, S: IntoSignalF32>(
        self,
        radius: R,
        softness: S,
    ) -> ChainedFilter<V, F, VignetteFilter> {
        self.then(filtrate::filters::Vignette(
            Reactive(radius.into_signal_f32().computed()),
            Reactive(softness.into_signal_f32().computed()),
        ))
    }

    /// Append a directional `MotionBlur` filter to the chain.
    #[must_use]
    pub fn motion_blur<R: IntoSignalF32, A: IntoSignalF32>(
        self,
        radius: R,
        angle: A,
    ) -> ChainedFilter<V, F, MotionBlurFilter> {
        self.then(filtrate::filters::MotionBlur(
            Reactive(radius.into_signal_f32().computed()),
            Reactive(angle.into_signal_f32().computed()),
        ))
    }
}

impl<V: View, F: Effect> View for Filtered<V, F> {
    fn body(self, _env: &Environment) -> impl View {
        Metadata::new(self.content, AppliedFilter::new(self.filter))
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }
}

// ============================================================================
// FilterParam ↔ nami signal bridge
// ============================================================================

/// Wraps any `nami::Signal<Output = f32>` so it can be used as a
/// [`FilterParam`] in `filtrate-core` filter structs without coupling
/// `filtrate-core` to nami.
///
/// Produced internally by view-level modifiers (`view.blur(...)` etc.); end
/// users do not normally name this type.
#[derive(Debug, Clone, Copy)]
pub struct Reactive<S>(pub S);

struct WaterUiAnimationInterpolator(WuiAnimation);

impl Interpolator for WaterUiAnimationInterpolator {
    fn duration(&self) -> Duration {
        self.0.duration()
    }
    fn interpolate(&self, from: f32, to: f32, elapsed: Duration) -> f32 {
        self.0.interpolate(&from, &to, elapsed)
    }
    fn is_complete(&self, elapsed: Duration) -> bool {
        self.0.is_complete(elapsed)
    }
}

impl<S> FilterParam for Reactive<S>
where
    S: Signal<Output = f32> + 'static,
    S::Guard: 'static,
{
    fn snapshot(&self) -> f32 {
        self.0.get()
    }

    fn watch_animated(&self, callback: AnimatedCallback) -> WatchGuard {
        let guard = self.0.watch(move |context| {
            let interpolator = context
                .metadata()
                .try_get::<WuiAnimation>()
                .map(|animation| {
                    Box::new(WaterUiAnimationInterpolator(animation)) as Box<dyn Interpolator>
                });
            let value = context.into_value();
            callback(AnimatedTarget {
                value,
                interpolator,
            });
        });
        WatchGuard::new(guard)
    }
}

/// Concrete filter aliases with stable type identities.
///
/// These aliases normalize reactive parameters to `Reactive<Computed<f32>>` so
/// fluent filter chains keep stable concrete types.
/// Alias for a box-blur filter.
pub type Blur = FilterAdapter<filtrate::filters::Blur<Reactive<Computed<f32>>>>;
/// Alias for a brightness adjustment filter.
pub type Brightness = FilterAdapter<filtrate::filters::Brightness<Reactive<Computed<f32>>>>;
/// Alias for a contrast adjustment filter.
pub type Contrast = FilterAdapter<filtrate::filters::Contrast<Reactive<Computed<f32>>>>;
/// Alias for an exposure adjustment filter.
pub type Exposure = FilterAdapter<filtrate::filters::Exposure<Reactive<Computed<f32>>>>;
/// Alias for a 4x5 color-matrix filter.
pub type ColorMatrix = FilterAdapter<filtrate::filters::ColorMatrix<f32>>;
/// Alias for a gamma adjustment filter.
pub type Gamma = FilterAdapter<filtrate::filters::Gamma<Reactive<Computed<f32>>>>;
/// Alias for a Gaussian blur filter.
pub type GaussianBlur = FilterAdapter<filtrate::filters::GaussianBlur<Reactive<Computed<f32>>>>;
/// Alias for a saturation adjustment filter.
pub type Saturation = FilterAdapter<filtrate::filters::Saturation<Reactive<Computed<f32>>>>;
/// Alias for a temperature/tint adjustment filter.
pub type TemperatureTintFilterAdapter = FilterAdapter<TemperatureTintFilter>;
pub use TemperatureTintFilterAdapter as TemperatureTint;
/// Alias for a grayscale mix filter.
pub type Grayscale = FilterAdapter<filtrate::filters::Grayscale<Reactive<Computed<f32>>>>;
/// Alias for a bloom filter.
pub type Bloom = FilterAdapter<filtrate::filters::Bloom<Reactive<Computed<f32>>>>;
/// Alias for a gloom filter.
pub type Gloom = FilterAdapter<filtrate::filters::Gloom<Reactive<Computed<f32>>>>;
/// Alias for a highlights/shadows adjustment filter.
pub type HighlightsShadowsFilterAdapter = FilterAdapter<HighlightsShadowsFilter>;
pub use HighlightsShadowsFilterAdapter as HighlightsShadows;
/// Alias for a hue-rotation filter.
pub type HueRotation = FilterAdapter<filtrate::filters::HueRotation<Reactive<Computed<f32>>>>;
/// Alias for a color inversion filter.
pub type Invert = FilterAdapter<filtrate::filters::Invert>;
/// Alias for a Sobel edge-detection filter.
pub type Sobel = FilterAdapter<filtrate::filters::Sobel>;
/// Alias for a Prewitt edge-detection filter.
pub type Prewitt = FilterAdapter<filtrate::filters::Prewitt>;
/// Alias for a 3x3 median filter.
pub type Median3x3 = FilterAdapter<filtrate::filters::Median3x3>;
/// Alias for a 3x3 convolution filter (caller-supplied kernel).
pub type Convolution3x3 = FilterAdapter<filtrate::filters::Convolution3x3<Reactive<Computed<f32>>>>;
/// Alias for a 5x5 convolution filter (caller-supplied kernel).
pub type Convolution5x5 = FilterAdapter<filtrate::filters::Convolution5x5<Reactive<Computed<f32>>>>;
/// Alias for a 3x3 morphological erosion filter (per-channel minimum).
pub type MorphologyMin = FilterAdapter<filtrate::filters::MorphologyMin>;
/// Alias for a 3x3 morphological dilation filter (per-channel maximum).
pub type MorphologyMax = FilterAdapter<filtrate::filters::MorphologyMax>;
/// Alias for a 3x3 morphological gradient filter (per-channel max minus min).
pub type MorphologyGradient = FilterAdapter<filtrate::filters::MorphologyGradient>;
/// Alias for the monochrome photo preset.
pub type PhotoEffectMono = FilterAdapter<filtrate::filters::PhotoEffectMono>;
/// Alias for the noir photo preset.
pub type PhotoEffectNoir = FilterAdapter<filtrate::filters::PhotoEffectNoir>;
/// Alias for the chrome photo preset.
pub type PhotoEffectChrome = FilterAdapter<filtrate::filters::PhotoEffectChrome>;
/// Alias for the instant photo preset.
pub type PhotoEffectInstant = FilterAdapter<filtrate::filters::PhotoEffectInstant>;
/// Alias for the fade photo preset.
pub type PhotoEffectFade = FilterAdapter<filtrate::filters::PhotoEffectFade>;
/// Alias for the process photo preset.
pub type PhotoEffectProcess = FilterAdapter<filtrate::filters::PhotoEffectProcess>;
/// Alias for the tonal photo preset.
pub type PhotoEffectTonal = FilterAdapter<filtrate::filters::PhotoEffectTonal>;
/// Alias for the transfer photo preset.
pub type PhotoEffectTransfer = FilterAdapter<filtrate::filters::PhotoEffectTransfer>;
/// Alias for a motion blur filter.
pub type MotionBlurFilterAdapter = FilterAdapter<MotionBlurFilter>;
pub use MotionBlurFilterAdapter as MotionBlur;
/// Alias for a bump distortion filter.
pub type BumpDistortion = FilterAdapter<filtrate::filters::BumpDistortion<Reactive<Computed<f32>>>>;
/// Alias for a pinch distortion filter.
pub type PinchDistortion =
    FilterAdapter<filtrate::filters::PinchDistortion<Reactive<Computed<f32>>>>;
/// Alias for a twirl distortion filter.
pub type TwirlDistortion =
    FilterAdapter<filtrate::filters::TwirlDistortion<Reactive<Computed<f32>>>>;
/// Alias for a vortex distortion filter.
pub type VortexDistortion =
    FilterAdapter<filtrate::filters::VortexDistortion<Reactive<Computed<f32>>>>;
/// Alias for a perspective transform filter.
pub type PerspectiveTransform = FilterAdapter<filtrate::filters::PerspectiveTransform<f32>>;
/// Alias for a perspective correction filter.
pub type PerspectiveCorrection = FilterAdapter<filtrate::filters::PerspectiveCorrection<f32>>;
/// Alias for a sepia-toning filter.
pub type Sepia = FilterAdapter<filtrate::filters::Sepia<Reactive<Computed<f32>>>>;
/// Alias for a vibrance adjustment filter.
pub type Vibrance = FilterAdapter<filtrate::filters::Vibrance<Reactive<Computed<f32>>>>;
/// Alias for a pixellation filter.
pub type Pixellate = FilterAdapter<filtrate::filters::Pixellate<Reactive<Computed<f32>>>>;
/// Alias for a crystallize filter.
pub type Crystallize = FilterAdapter<filtrate::filters::Crystallize<Reactive<Computed<f32>>>>;
/// Alias for an edge-work stylization filter.
pub type EdgeWork = FilterAdapter<filtrate::filters::EdgeWork<Reactive<Computed<f32>>>>;
/// Alias for a dot-halftone filter.
pub type DotHalftone = FilterAdapter<filtrate::filters::DotHalftone<Reactive<Computed<f32>>>>;
/// Alias for a line-halftone filter.
pub type LineHalftone = FilterAdapter<filtrate::filters::LineHalftone<Reactive<Computed<f32>>>>;
/// Alias for a kaleidoscope filter.
pub type Kaleidoscope = FilterAdapter<filtrate::filters::Kaleidoscope<Reactive<Computed<f32>>>>;
/// Alias for a mirror-tile filter.
pub type MirrorTile = FilterAdapter<filtrate::filters::MirrorTile<Reactive<Computed<f32>>>>;
/// Alias for an unsharp-mask filter.
pub type UnsharpMask = FilterAdapter<filtrate::filters::UnsharpMask<Reactive<Computed<f32>>>>;
/// Alias for a sharpen filter.
pub type Sharpen = FilterAdapter<filtrate::filters::Sharpen<Reactive<Computed<f32>>>>;
/// Alias for a vignette filter.
pub type VignetteFilterAdapter = FilterAdapter<VignetteFilter>;
pub use VignetteFilterAdapter as Vignette;
/// Alias for a white-point adjustment filter.
pub type WhitePointFilterAdapter = FilterAdapter<WhitePointFilter>;
pub use WhitePointFilterAdapter as WhitePoint;
/// Alias for a zoom-blur filter.
pub type ZoomBlurFilterAdapter = FilterAdapter<ZoomBlurFilter>;
pub use ZoomBlurFilterAdapter as ZoomBlur;

/// Rebuilds the canonical blur filter adapter from a reactive radius signal.
#[must_use]
pub fn blur_from_radius_signal(radius: Reactive<Computed<f32>>) -> Blur {
    FilterAdapter::new(filtrate::filters::Blur(radius))
}

/// Extension methods for applying filters to views.
pub trait FilterViewExt: View + Sized {
    /// Apply a `Effect` to this view.
    ///
    /// For the high-level `Filter` API with automatic optimization,
    /// use convenience methods like `.blur()`, `.brightness()`, etc.
    fn filter<F: Effect>(self, filter: F) -> Filtered<Self, F> {
        Filtered::new(self, filter)
    }

    // ========================================================================
    // Convenience methods - return FilterAdapter which supports .then()
    // ========================================================================

    /// Apply a blur filter.
    ///
    /// Accepts reactive values that will be automatically animated.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Static value
    /// my_view.blur(10.0)
    ///
    /// // Reactive value with animation
    /// let radius = binding(10.0);
    /// my_view.blur(radius.animated())
    /// ```
    fn blur<T: IntoSignalF32>(self, radius: T) -> Filtered<Self, Blur> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Blur(Reactive(
                radius.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a brightness filter.
    fn brightness<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Brightness> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Brightness(Reactive(
                amount.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply an exposure filter in photographic stops.
    fn exposure<T: IntoSignalF32>(self, ev: T) -> Filtered<Self, Exposure> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Exposure(Reactive(
                ev.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a gamma adjustment filter.
    fn gamma<T: IntoSignalF32>(self, gamma: T) -> Filtered<Self, Gamma> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Gamma(Reactive(
                gamma.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a contrast filter.
    fn contrast<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Contrast> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Contrast(Reactive(
                amount.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a saturation filter.
    fn saturation<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Saturation> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Saturation(Reactive(
                amount.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a vibrance filter.
    fn vibrance<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Vibrance> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Vibrance(Reactive(
                amount.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a grayscale filter.
    fn grayscale<T: IntoSignalF32>(self, intensity: T) -> Filtered<Self, Grayscale> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Grayscale(Reactive(
                intensity.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a hue rotation filter.
    fn hue_rotation<T: IntoSignalF32>(self, angle: T) -> Filtered<Self, HueRotation> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::HueRotation(Reactive(
                angle.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply an invert filter.
    fn invert(self) -> Filtered<Self, Invert> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::Invert))
    }

    /// Apply a Sobel edge-detection filter (3x3, gradient magnitude).
    fn sobel(self) -> Filtered<Self, Sobel> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::Sobel))
    }

    /// Apply a Prewitt edge-detection filter (3x3, uniform-weight kernels).
    fn prewitt(self) -> Filtered<Self, Prewitt> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::Prewitt))
    }

    /// Apply a 3x3 per-channel median filter for salt-and-pepper denoising.
    fn median3x3(self) -> Filtered<Self, Median3x3> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::Median3x3))
    }

    /// Apply a 3x3 convolution filter with a caller-supplied kernel
    /// (row-major, top-left to bottom-right).
    fn convolution3x3<P: IntoSignalF32 + Copy>(
        self,
        kernel: [P; 9],
    ) -> Filtered<Self, Convolution3x3> {
        let signals: [Reactive<Computed<f32>>; 9] =
            core::array::from_fn(|i| Reactive(kernel[i].into_signal_f32().computed()));
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Convolution3x3(signals)),
        )
    }

    /// Apply a 5x5 convolution filter with a caller-supplied 25-element
    /// kernel (row-major).
    fn convolution5x5<P: IntoSignalF32 + Copy>(
        self,
        kernel: [P; 25],
    ) -> Filtered<Self, Convolution5x5> {
        let signals: [Reactive<Computed<f32>>; 25] =
            core::array::from_fn(|i| Reactive(kernel[i].into_signal_f32().computed()));
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Convolution5x5(signals)),
        )
    }

    /// Apply a 3x3 morphological erosion (per-channel minimum).
    fn morphology_min(self) -> Filtered<Self, MorphologyMin> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::MorphologyMin))
    }

    /// Apply a 3x3 morphological dilation (per-channel maximum).
    fn morphology_max(self) -> Filtered<Self, MorphologyMax> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::MorphologyMax))
    }

    /// Apply a 3x3 morphological gradient (per-channel max minus min).
    fn morphology_gradient(self) -> Filtered<Self, MorphologyGradient> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::MorphologyGradient),
        )
    }

    /// Apply the monochrome photo preset.
    fn photo_effect_mono(self) -> Filtered<Self, PhotoEffectMono> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::PhotoEffectMono))
    }

    /// Apply the noir photo preset.
    fn photo_effect_noir(self) -> Filtered<Self, PhotoEffectNoir> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::PhotoEffectNoir))
    }

    /// Apply the chrome photo preset.
    fn photo_effect_chrome(self) -> Filtered<Self, PhotoEffectChrome> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PhotoEffectChrome),
        )
    }

    /// Apply the instant photo preset.
    fn photo_effect_instant(self) -> Filtered<Self, PhotoEffectInstant> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PhotoEffectInstant),
        )
    }

    /// Apply the fade photo preset.
    fn photo_effect_fade(self) -> Filtered<Self, PhotoEffectFade> {
        Filtered::new(self, FilterAdapter::new(filtrate::filters::PhotoEffectFade))
    }

    /// Apply the process photo preset.
    fn photo_effect_process(self) -> Filtered<Self, PhotoEffectProcess> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PhotoEffectProcess),
        )
    }

    /// Apply the tonal photo preset.
    fn photo_effect_tonal(self) -> Filtered<Self, PhotoEffectTonal> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PhotoEffectTonal),
        )
    }

    /// Apply the transfer photo preset.
    fn photo_effect_transfer(self) -> Filtered<Self, PhotoEffectTransfer> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PhotoEffectTransfer),
        )
    }

    /// Apply a sepia filter.
    fn sepia<T: IntoSignalF32>(self, intensity: T) -> Filtered<Self, Sepia> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Sepia(Reactive(
                intensity.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a sharpen filter.
    fn sharpen<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Sharpen> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Sharpen(Reactive(
                amount.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a temperature/tint white-balance adjustment.
    fn temperature_tint<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        temperature: T,
        tint: U,
    ) -> Filtered<Self, TemperatureTint> {
        Filtered::new(
            self,
            FilterAdapter::new(temperature_tint_filter(
                Reactive(temperature.into_signal_f32().computed()),
                Reactive(tint.into_signal_f32().computed()),
            )),
        )
    }

    /// Recover highlights while lifting shadows.
    fn highlights_shadows<H: IntoSignalF32, S: IntoSignalF32>(
        self,
        highlights: H,
        shadows: S,
    ) -> Filtered<Self, HighlightsShadows> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::HighlightsShadows(
                Reactive(highlights.into_signal_f32().computed()),
                Reactive(shadows.into_signal_f32().computed()),
            )),
        )
    }

    /// Apply directional motion blur.
    fn motion_blur<R: IntoSignalF32, A: IntoSignalF32>(
        self,
        radius: R,
        angle: A,
    ) -> Filtered<Self, MotionBlur> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::MotionBlur(
                Reactive(radius.into_signal_f32().computed()),
                Reactive(angle.into_signal_f32().computed()),
            )),
        )
    }

    /// Apply a vignette filter.
    fn vignette<R: IntoSignalF32, S: IntoSignalF32>(
        self,
        radius: R,
        softness: S,
    ) -> Filtered<Self, Vignette> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Vignette(
                Reactive(radius.into_signal_f32().computed()),
                Reactive(softness.into_signal_f32().computed()),
            )),
        )
    }

    /// Adjust color balance using an explicit white point triplet.
    fn white_point<R: IntoSignalF32, G: IntoSignalF32, B: IntoSignalF32>(
        self,
        red: R,
        green: G,
        blue: B,
    ) -> Filtered<Self, WhitePoint> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::WhitePoint(
                Reactive(red.into_signal_f32().computed()),
                Reactive(green.into_signal_f32().computed()),
                Reactive(blue.into_signal_f32().computed()),
            )),
        )
    }

    /// Apply radial zoom blur around a focal point.
    fn zoom_blur<A: IntoSignalF32, X: IntoSignalF32, Y: IntoSignalF32>(
        self,
        amount: A,
        center_x: X,
        center_y: Y,
    ) -> Filtered<Self, ZoomBlur> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::ZoomBlur(
                Reactive(amount.into_signal_f32().computed()),
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
            )),
        )
    }

    /// Apply a gaussian blur filter.
    fn gaussian_blur<T: IntoSignalF32>(self, sigma: T) -> Filtered<Self, GaussianBlur> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::GaussianBlur(Reactive(
                sigma.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a 3x4 color matrix transform.
    fn color_matrix(self, matrix: [[f32; 4]; 3]) -> Filtered<Self, ColorMatrix> {
        let params = [
            matrix[0][0],
            matrix[0][1],
            matrix[0][2],
            matrix[0][3],
            matrix[1][0],
            matrix[1][1],
            matrix[1][2],
            matrix[1][3],
            matrix[2][0],
            matrix[2][1],
            matrix[2][2],
            matrix[2][3],
        ];
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::ColorMatrix(params)),
        )
    }

    /// Apply bloom around bright regions.
    fn bloom<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32>(
        self,
        radius: T,
        intensity: U,
        threshold: V,
    ) -> Filtered<Self, Bloom> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Bloom {
                radius: Reactive(radius.into_signal_f32().computed()),
                intensity: Reactive(intensity.into_signal_f32().computed()),
                threshold: Reactive(threshold.into_signal_f32().computed()),
            }),
        )
    }

    /// Apply gloom around bright regions.
    fn gloom<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32>(
        self,
        radius: T,
        intensity: U,
        threshold: V,
    ) -> Filtered<Self, Gloom> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Gloom {
                radius: Reactive(radius.into_signal_f32().computed()),
                intensity: Reactive(intensity.into_signal_f32().computed()),
                threshold: Reactive(threshold.into_signal_f32().computed()),
            }),
        )
    }

    /// Apply an unsharp mask.
    fn unsharp_mask<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        radius: T,
        amount: U,
    ) -> Filtered<Self, UnsharpMask> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::UnsharpMask {
                radius: Reactive(radius.into_signal_f32().computed()),
                intensity: Reactive(amount.into_signal_f32().computed()),
            }),
        )
    }

    /// Apply bump distortion around a center.
    fn bump_distortion<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32, W: IntoSignalF32>(
        self,
        center_x: T,
        center_y: U,
        radius: V,
        scale: W,
    ) -> Filtered<Self, BumpDistortion> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::BumpDistortion([
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
                Reactive(radius.into_signal_f32().computed()),
                Reactive(scale.into_signal_f32().computed()),
            ])),
        )
    }

    /// Apply pinch distortion around a center.
    fn pinch_distortion<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32, W: IntoSignalF32>(
        self,
        center_x: T,
        center_y: U,
        radius: V,
        scale: W,
    ) -> Filtered<Self, PinchDistortion> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PinchDistortion([
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
                Reactive(radius.into_signal_f32().computed()),
                Reactive(scale.into_signal_f32().computed()),
            ])),
        )
    }

    /// Apply twirl distortion around a center.
    fn twirl_distortion<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32, W: IntoSignalF32>(
        self,
        center_x: T,
        center_y: U,
        radius: V,
        angle: W,
    ) -> Filtered<Self, TwirlDistortion> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::TwirlDistortion([
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
                Reactive(radius.into_signal_f32().computed()),
                Reactive(angle.into_signal_f32().computed()),
            ])),
        )
    }

    /// Apply vortex distortion around a center.
    fn vortex_distortion<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32, W: IntoSignalF32>(
        self,
        center_x: T,
        center_y: U,
        radius: V,
        angle: W,
    ) -> Filtered<Self, VortexDistortion> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::VortexDistortion([
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
                Reactive(radius.into_signal_f32().computed()),
                Reactive(angle.into_signal_f32().computed()),
            ])),
        )
    }

    /// Warp a source quadrilateral into the output rectangle.
    fn perspective_transform(self, quad: [[f32; 2]; 4]) -> Filtered<Self, PerspectiveTransform> {
        let params = [
            quad[0][0], quad[0][1], quad[1][0], quad[1][1], quad[2][0], quad[2][1], quad[3][0],
            quad[3][1],
        ];
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PerspectiveTransform(params)),
        )
    }

    /// Correct a perspective-skewed quadrilateral back to a rectangle.
    fn perspective_correction(self, quad: [[f32; 2]; 4]) -> Filtered<Self, PerspectiveCorrection> {
        let params = [
            quad[0][0], quad[0][1], quad[1][0], quad[1][1], quad[2][0], quad[2][1], quad[3][0],
            quad[3][1],
        ];
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::PerspectiveCorrection(params)),
        )
    }

    /// Apply a pixellate effect.
    fn pixellate<T: IntoSignalF32>(self, size: T) -> Filtered<Self, Pixellate> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Pixellate(Reactive(
                size.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply a crystallize effect.
    fn crystallize<T: IntoSignalF32>(self, size: T) -> Filtered<Self, Crystallize> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Crystallize(Reactive(
                size.into_signal_f32().computed(),
            ))),
        )
    }

    /// Apply an edge-work effect.
    fn edge_work<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        radius: T,
        amount: U,
    ) -> Filtered<Self, EdgeWork> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::EdgeWork([
                Reactive(radius.into_signal_f32().computed()),
                Reactive(amount.into_signal_f32().computed()),
            ])),
        )
    }

    /// Apply a dot halftone effect.
    fn dot_halftone<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32, W: IntoSignalF32>(
        self,
        scale: T,
        angle: U,
        center_x: V,
        center_y: W,
    ) -> Filtered<Self, DotHalftone> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::DotHalftone([
                Reactive(scale.into_signal_f32().computed()),
                Reactive(angle.into_signal_f32().computed()),
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
            ])),
        )
    }

    /// Apply a line halftone effect.
    fn line_halftone<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32, W: IntoSignalF32>(
        self,
        scale: T,
        angle: U,
        center_x: V,
        center_y: W,
    ) -> Filtered<Self, LineHalftone> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::LineHalftone([
                Reactive(scale.into_signal_f32().computed()),
                Reactive(angle.into_signal_f32().computed()),
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
            ])),
        )
    }

    /// Apply a kaleidoscope effect.
    fn kaleidoscope<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32, W: IntoSignalF32>(
        self,
        segments: T,
        angle: U,
        center_x: V,
        center_y: W,
    ) -> Filtered<Self, Kaleidoscope> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::Kaleidoscope([
                Reactive(segments.into_signal_f32().computed()),
                Reactive(angle.into_signal_f32().computed()),
                Reactive(center_x.into_signal_f32().computed()),
                Reactive(center_y.into_signal_f32().computed()),
            ])),
        )
    }

    /// Apply mirrored tiling.
    fn mirror_tile<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        repeat_x: T,
        repeat_y: U,
    ) -> Filtered<Self, MirrorTile> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate::filters::MirrorTile([
                Reactive(repeat_x.into_signal_f32().computed()),
                Reactive(repeat_y.into_signal_f32().computed()),
            ])),
        )
    }

    /// Blend the current content with an auxiliary image.
    fn blend_with_image(
        self,
        image: crate::multi_input_filter::FilterImage,
        amount: f32,
        mode: crate::multi_input_filter::BlendMode,
    ) -> Filtered<Self, crate::multi_input_filter::BlendWithImageFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::blend_with_image_filter(image, amount, mode),
        )
    }

    /// Apply masked blur using an auxiliary mask image.
    fn masked_blur(
        self,
        mask: crate::multi_input_filter::FilterImage,
        radius: f32,
        strength: f32,
    ) -> Filtered<Self, crate::multi_input_filter::MaskedBlurFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::masked_blur_filter(mask, radius, strength),
        )
    }

    /// Transition to another image.
    fn transition_to_image(
        self,
        target: crate::multi_input_filter::FilterImage,
        progress: f32,
        softness: f32,
    ) -> Filtered<Self, crate::multi_input_filter::TransitionToImageFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::transition_to_image_filter(target, progress, softness),
        )
    }

    /// Transition to another image with a directional swipe.
    fn swipe_transition_to_image(
        self,
        target: crate::multi_input_filter::FilterImage,
        progress: f32,
        softness: f32,
        direction: crate::multi_input_filter::TransitionDirection,
    ) -> Filtered<Self, crate::multi_input_filter::SwipeTransitionToImageFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::swipe_transition_to_image_filter(
                target, progress, softness, direction,
            ),
        )
    }

    /// Transition to another image from a radial reveal center.
    fn radial_transition_to_image(
        self,
        target: crate::multi_input_filter::FilterImage,
        progress: f32,
        softness: f32,
        center_x: f32,
        center_y: f32,
    ) -> Filtered<Self, crate::multi_input_filter::RadialTransitionToImageFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::radial_transition_to_image_filter(
                target, progress, softness, center_x, center_y,
            ),
        )
    }

    /// Transition to another image with a zooming blend.
    fn zoom_transition_to_image(
        self,
        target: crate::multi_input_filter::FilterImage,
        progress: f32,
        amount: f32,
        center_x: f32,
        center_y: f32,
    ) -> Filtered<Self, crate::multi_input_filter::ZoomTransitionToImageFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::zoom_transition_to_image_filter(
                target, progress, amount, center_x, center_y,
            ),
        )
    }

    /// Transition to another image driven by a displacement map.
    fn displacement_transition_to_image(
        self,
        target: crate::multi_input_filter::FilterImage,
        map: crate::multi_input_filter::FilterImage,
        progress: f32,
        scale: f32,
    ) -> Filtered<Self, crate::multi_input_filter::DisplacementTransitionToImageFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::displacement_transition_to_image_filter(
                target, map, progress, scale,
            ),
        )
    }

    /// Warp with an auxiliary displacement map.
    fn displacement_warp(
        self,
        map: crate::multi_input_filter::FilterImage,
        scale_x: f32,
        scale_y: f32,
    ) -> Filtered<Self, crate::multi_input_filter::DisplacementWarpFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::displacement_warp_filter(map, scale_x, scale_y),
        )
    }

    /// Apply guide-image-aware smoothing.
    fn guided_smooth(
        self,
        guide: crate::multi_input_filter::FilterImage,
        radius: f32,
        range_sigma: f32,
        amount: f32,
    ) -> Filtered<Self, crate::multi_input_filter::GuidedSmoothFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::guided_smooth_filter(guide, radius, range_sigma, amount),
        )
    }

    /// Apply depth-aware blur using a depth map.
    fn depth_aware_blur(
        self,
        depth: crate::multi_input_filter::FilterImage,
        focus_depth: f32,
        aperture: f32,
        max_radius: f32,
    ) -> Filtered<Self, crate::multi_input_filter::DepthAwareBlurFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::depth_aware_blur_filter(
                depth,
                focus_depth,
                aperture,
                max_radius,
            ),
        )
    }

    /// Temporal denoise/stabilize using history and motion maps.
    fn temporal_denoise(
        self,
        history: crate::multi_input_filter::FilterImage,
        motion: crate::multi_input_filter::FilterImage,
        history_weight: f32,
    ) -> Filtered<Self, crate::multi_input_filter::TemporalDenoiseFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::temporal_denoise_filter(history, motion, history_weight),
        )
    }

    /// Replace background using matte and background images.
    fn replace_background(
        self,
        matte: crate::multi_input_filter::FilterImage,
        background: crate::multi_input_filter::FilterImage,
        edge_softness: f32,
    ) -> Filtered<Self, crate::multi_input_filter::BackgroundReplaceFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::background_replace_filter(matte, background, edge_softness),
        )
    }

    /// Apply a 3D LUT color transform encoded as a 2D strip (`size*size x size`).
    fn lut_color_grade(
        self,
        lut: crate::multi_input_filter::LutImage,
        intensity: f32,
    ) -> Filtered<Self, crate::multi_input_filter::LutColorGradeFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::lut_color_grade_filter(lut, intensity),
        )
    }

    /// Apply a simple master tone curve.
    fn tone_curve(
        self,
        shadows: f32,
        midtones: f32,
        highlights: f32,
        gamma: f32,
        amount: f32,
    ) -> Filtered<Self, crate::multi_input_filter::ToneCurveFilter> {
        Filtered::new(
            self,
            crate::multi_input_filter::tone_curve_filter(
                shadows, midtones, highlights, gamma, amount,
            ),
        )
    }
}

impl<V: View> FilterViewExt for V {}
