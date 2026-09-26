//! Filters on views: a `filtrate` filter applied to a view's rendered subtree.
//!
//! [`Filtered`] pairs a view with a [`Filter`]. Its body erases the filter
//! into a [`FilteredView`] carrying an [`AnyEffect`]: a `Send` source the
//! backend builds on its render thread into the filter behind `filtrate`'s
//! [`Executor`], attached to the layer rendering the view.
//!
//! Reactive parameters are [`Reactive`] slots: a nami signal on the UI side
//! feeds a `Send` value slot the executor samples on the render side, and a
//! change carrying an [`Animation`] in its metadata interpolates on the
//! render clock.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use arc_swap::ArcSwap;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use cherenkov::{Animation, curve_value, settled, spring_step};
pub use filtrate::filters::{BlendMode, TransitionDirection};
use filtrate::{
    AnimatedCallback, AnimatedTarget, Chain, Effect, EffectContext, EffectInput, EffectOutput,
    EffectRedrawCallback, EffectRenderResult, EffectSetupResult, Executor, Filter, FilterExt as _,
    FilterParam, Interpolator, WatchGuard,
};
pub use filtrate::{FilterImage, LutImage};
use nami::Signal;
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, IntoSignalF32, View};

/// A filter parameter fed by a nami signal.
///
/// The slot is `Send + Sync`; the UI-side subscription that writes it lives
/// in the view's [`ParamGuards`].
#[derive(Clone)]
pub struct Reactive(Arc<Slot>);

struct Slot {
    value: AtomicU32,
    callbacks: ArcSwap<Vec<Arc<AnimatedCallback>>>,
}

impl fmt::Debug for Reactive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Reactive").field(&self.snapshot()).finish()
    }
}

struct Subscription {
    slot: Weak<Slot>,
    callback: Arc<AnimatedCallback>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.upgrade() {
            slot.callbacks.rcu(|callbacks| {
                callbacks
                    .iter()
                    .filter(|callback| !Arc::ptr_eq(callback, &self.callback))
                    .cloned()
                    .collect::<Vec<_>>()
            });
        }
    }
}

impl FilterParam for Reactive {
    fn snapshot(&self) -> f32 {
        f32::from_bits(self.0.value.load(Ordering::Acquire))
    }

    fn watch_animated(&self, callback: AnimatedCallback) -> WatchGuard {
        let callback = Arc::new(callback);
        self.0.callbacks.rcu(|callbacks| {
            let mut next = (**callbacks).clone();
            next.push(Arc::clone(&callback));
            next
        });
        WatchGuard::new(Subscription {
            slot: Arc::downgrade(&self.0),
            callback,
        })
    }
}

/// The UI-side subscriptions keeping a filter's [`Reactive`] parameters fed.
///
/// Dropping the guards freezes the parameters at their last value.
#[derive(Default)]
pub struct ParamGuards(Vec<Box<dyn core::any::Any>>);

impl fmt::Debug for ParamGuards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ParamGuards").field(&self.0.len()).finish()
    }
}

impl ParamGuards {
    /// Binds a signal to a new [`Reactive`] slot, keeping the subscription here.
    pub fn bind(&mut self, value: impl IntoSignalF32) -> Reactive {
        let signal = value.into_signal_f32();
        let slot = Arc::new(Slot {
            value: AtomicU32::new(signal.snapshot().to_bits()),
            callbacks: ArcSwap::from_pointee(Vec::new()),
        });
        let target = Arc::clone(&slot);
        let guard = signal.watch(move |context| {
            let animation = context.metadata().try_get::<Animation>();
            let value = context.into_value();
            target.value.store(value.to_bits(), Ordering::Release);
            for callback in target.callbacks.load().iter() {
                callback(AnimatedTarget {
                    value,
                    interpolator: animation.map(|animation| {
                        Box::new(AnimationInterpolator(animation)) as Box<dyn Interpolator>
                    }),
                });
            }
        });
        self.0.push(Box::new(guard));
        Reactive(slot)
    }

    fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

/// A Cherenkov [`Animation`] driving a scalar filter parameter.
struct AnimationInterpolator(Animation);

const SPRING_STEP: f64 = 1.0 / 240.0;
const SPRING_STEP_NANOS: u128 = 1_000_000_000 / 240;
const SPRING_LIMIT: Duration = Duration::from_secs(10);

#[allow(clippy::cast_possible_truncation)]
impl AnimationInterpolator {
    fn spring_at(spring: &cherenkov::Spring, from: f32, to: f32, elapsed: Duration) -> (f64, bool) {
        let (mut pos, mut velocity) = ([f64::from(from)], [0.0]);
        let target = [f64::from(to)];
        let steps = (elapsed.min(SPRING_LIMIT).as_nanos() / SPRING_STEP_NANOS) + 1;
        for _ in 0..steps {
            (pos, velocity) = spring_step(pos, velocity, target, spring, SPRING_STEP);
            if settled(pos, velocity, target) {
                return (target[0], true);
            }
        }
        (pos[0], false)
    }
}

#[allow(clippy::cast_possible_truncation)]
impl Interpolator for AnimationInterpolator {
    fn duration(&self) -> Duration {
        match &self.0 {
            Animation::Curve(curve) => curve.duration,
            Animation::Spring(_) => SPRING_LIMIT,
            Animation::Decay(_) => Duration::ZERO,
        }
    }

    fn interpolate(&self, from: f32, to: f32, elapsed: Duration) -> f32 {
        match &self.0 {
            Animation::Curve(curve) => {
                let t = if curve.duration.is_zero() {
                    1.0
                } else {
                    (elapsed.as_secs_f64() / curve.duration.as_secs_f64()).min(1.0)
                };
                let k = curve_value(curve, t);
                (f64::from(to) - f64::from(from)).mul_add(k, f64::from(from)) as f32
            }
            Animation::Spring(spring) => Self::spring_at(spring, from, to, elapsed).0 as f32,
            Animation::Decay(_) => to,
        }
    }

    fn is_complete(&self, elapsed: Duration) -> bool {
        match &self.0 {
            Animation::Spring(spring) => {
                elapsed >= SPRING_LIMIT || Self::spring_at(spring, 0.0, 1.0, elapsed).1
            }
            _ => elapsed >= self.duration(),
        }
    }
}

type BoxedSetup<'a> = Pin<Box<dyn Future<Output = EffectSetupResult> + 'a>>;

/// Object-safe [`Effect`], built on the render thread by [`AnyEffect::build`].
///
/// `Effect::setup` returns an `impl Future`, so the trait itself cannot be
/// boxed; this form boxes the future. It is not `Send`: `filtrate`'s
/// [`Executor`] lives on the thread that built it.
pub trait ErasedEffect {
    /// Installs the callback the effect fires when it needs another frame.
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback);
    /// Creates pipelines and resources; once, before the first render.
    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> BoxedSetup<'a>;
    /// Encodes one frame of effect work.
    ///
    /// # Errors
    /// The effect's own render error, surfaced by the host as a frame failure.
    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult;
    /// Whether the effect wants another frame (an animating parameter).
    fn redraw_hint(&self) -> bool;
}

impl<E: Effect> ErasedEffect for E {
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        Effect::set_redraw_callback(self, callback);
    }

    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> BoxedSetup<'a> {
        Box::pin(Effect::setup(self, ctx))
    }

    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        Effect::encode_render(self, input, output, encoder)
    }

    fn redraw_hint(&self) -> bool {
        Effect::redraw_hint(self)
    }
}

trait EffectSource: Send {
    fn build(self: Box<Self>) -> Box<dyn ErasedEffect>;
}

struct FromFilter<F>(F);

impl<F: Filter + Send> EffectSource for FromFilter<F> {
    fn build(self: Box<Self>) -> Box<dyn ErasedEffect> {
        Box::new(Executor::new(self.0))
    }
}

struct FromEffect<E>(E);

impl<E: Effect + Send> EffectSource for FromEffect<E> {
    fn build(self: Box<Self>) -> Box<dyn ErasedEffect> {
        Box::new(self.0)
    }
}

/// A filter or effect, erased and `Send`, as a backend receives it.
///
/// The backend moves it to its render thread and calls [`build`](Self::build)
/// there; the result runs against the engine's device.
pub struct AnyEffect(Box<dyn EffectSource>);

impl fmt::Debug for AnyEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyEffect").finish_non_exhaustive()
    }
}

impl AnyEffect {
    /// Erases a custom effect.
    pub fn new(effect: impl Effect + Send) -> Self {
        Self(Box::new(FromEffect(effect)))
    }

    /// Erases a filter, to run through `filtrate`'s [`Executor`].
    pub fn filter(filter: impl Filter + Send) -> Self {
        Self(Box::new(FromFilter(filter)))
    }

    /// Builds the effect on the render thread.
    #[must_use]
    pub fn build(self) -> Box<dyn ErasedEffect> {
        self.0.build()
    }
}

/// A view with a filter applied to its rendered subtree.
#[derive(Debug)]
pub struct Filtered<V, F> {
    view: V,
    filter: F,
    guards: ParamGuards,
}

impl<V: View, F: Filter + Send> Filtered<V, F> {
    /// Applies `filter` to `view`.
    pub fn new(view: V, filter: F) -> Self {
        Self::bound(view, filter, ParamGuards::default())
    }

    /// Applies `filter` to `view`, keeping the subscriptions of its reactive parameters.
    pub const fn bound(view: V, filter: F, guards: ParamGuards) -> Self {
        Self {
            view,
            filter,
            guards,
        }
    }

    /// Appends `next` to the filter chain.
    pub fn then<G: Filter + Send>(self, next: G) -> Filtered<V, Chain<F, G>> {
        self.then_bound(next, ParamGuards::default())
    }

    fn then_bound<G: Filter + Send>(
        mut self,
        next: G,
        guards: ParamGuards,
    ) -> Filtered<V, Chain<F, G>> {
        self.guards.extend(guards);
        Filtered {
            view: self.view,
            filter: self.filter.then(next),
            guards: self.guards,
        }
    }
}

impl<V: View, F: Filter + Send> View for Filtered<V, F> {
    fn body(self, _env: &Environment) -> impl View {
        FilteredView {
            content: AnyView::new(self.view),
            effect: AnyEffect::filter(self.filter),
            guards: self.guards,
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.view.stretch_axis()
    }
}

/// A view with an effect on its rendered subtree, as a backend receives it.
///
/// The backend registers `effect` with its engine (`Engine::effect`), sets
/// the returned `Filter` on the layer rendering `content`, and keeps
/// `guards` alive as long as the filter.
#[derive(Debug)]
pub struct FilteredView {
    /// The filtered subtree.
    pub content: AnyView,
    /// The effect to register.
    pub effect: AnyEffect,
    /// The subscriptions feeding the effect's reactive parameters.
    pub guards: ParamGuards,
}

impl FilteredView {
    /// Applies a custom effect to `view`.
    pub fn new(view: impl View, effect: impl Effect + Send) -> Self {
        Self {
            content: AnyView::new(view),
            effect: AnyEffect::new(effect),
            guards: ParamGuards::default(),
        }
    }
}

waterui_core::raw_view!(FilteredView);

macro_rules! inherent_single_param_filter {
    ($method:ident, $filter:ident) => {
        #[doc = concat!("Append a `", stringify!($filter), "` filter to the chain.")]
        #[must_use]
        pub fn $method<P: IntoSignalF32>(
            self,
            value: P,
        ) -> Filtered<V, Chain<F, filtrate::filters::$filter<Reactive>>> {
            let mut guards = ParamGuards::default();
            let filter = filtrate::filters::$filter(guards.bind(value));
            self.then_bound(filter, guards)
        }
    };
}

impl<V: View, F: Filter + Send> Filtered<V, F> {
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

    /// Append an `Invert` filter to the chain.
    #[must_use]
    pub fn invert(self) -> Filtered<V, Chain<F, filtrate::filters::Invert>> {
        self.then(filtrate::filters::Invert)
    }
}

// Concrete filter aliases with stable type identities.
/// Alias for the `Blur` filter.
pub type Blur = filtrate::filters::Blur<Reactive>;
/// Alias for the `Brightness` filter.
pub type Brightness = filtrate::filters::Brightness<Reactive>;
/// Alias for the `Contrast` filter.
pub type Contrast = filtrate::filters::Contrast<Reactive>;
/// Alias for the `Exposure` filter.
pub type Exposure = filtrate::filters::Exposure<Reactive>;
/// Alias for the `ColorMatrix` filter.
pub type ColorMatrix = filtrate::filters::ColorMatrix<f32>;
/// Alias for the `Gamma` filter.
pub type Gamma = filtrate::filters::Gamma<Reactive>;
/// Alias for the `GaussianBlur` filter.
pub type GaussianBlur = filtrate::filters::GaussianBlur<Reactive>;
/// Alias for the `Saturation` filter.
pub type Saturation = filtrate::filters::Saturation<Reactive>;
/// Alias for the `TemperatureTint` filter.
pub type TemperatureTint = filtrate::filters::TemperatureTint<Reactive, Reactive>;
/// Alias for the `Grayscale` filter.
pub type Grayscale = filtrate::filters::Grayscale<Reactive>;
/// Alias for the `Bloom` filter.
pub type Bloom = filtrate::filters::Bloom<Reactive>;
/// Alias for the `Gloom` filter.
pub type Gloom = filtrate::filters::Gloom<Reactive>;
/// Alias for the `HighlightsShadows` filter.
pub type HighlightsShadows = filtrate::filters::HighlightsShadows<Reactive, Reactive>;
/// Alias for the `HueRotation` filter.
pub type HueRotation = filtrate::filters::HueRotation<Reactive>;
/// Alias for the `Invert` filter.
pub type Invert = filtrate::filters::Invert;
/// Alias for the `Sobel` filter.
pub type Sobel = filtrate::filters::Sobel;
/// Alias for the `Prewitt` filter.
pub type Prewitt = filtrate::filters::Prewitt;
/// Alias for the `Median3x3` filter.
pub type Median3x3 = filtrate::filters::Median3x3;
/// Alias for the `Convolution3x3` filter.
pub type Convolution3x3 = filtrate::filters::Convolution3x3<Reactive>;
/// Alias for the `Convolution5x5` filter.
pub type Convolution5x5 = filtrate::filters::Convolution5x5<Reactive>;
/// Alias for the `MorphologyMin` filter.
pub type MorphologyMin = filtrate::filters::MorphologyMin;
/// Alias for the `MorphologyMax` filter.
pub type MorphologyMax = filtrate::filters::MorphologyMax;
/// Alias for the `MorphologyGradient` filter.
pub type MorphologyGradient = filtrate::filters::MorphologyGradient;
/// Alias for the `PhotoEffectMono` filter.
pub type PhotoEffectMono = filtrate::filters::PhotoEffectMono;
/// Alias for the `PhotoEffectNoir` filter.
pub type PhotoEffectNoir = filtrate::filters::PhotoEffectNoir;
/// Alias for the `PhotoEffectChrome` filter.
pub type PhotoEffectChrome = filtrate::filters::PhotoEffectChrome;
/// Alias for the `PhotoEffectInstant` filter.
pub type PhotoEffectInstant = filtrate::filters::PhotoEffectInstant;
/// Alias for the `PhotoEffectFade` filter.
pub type PhotoEffectFade = filtrate::filters::PhotoEffectFade;
/// Alias for the `PhotoEffectProcess` filter.
pub type PhotoEffectProcess = filtrate::filters::PhotoEffectProcess;
/// Alias for the `PhotoEffectTonal` filter.
pub type PhotoEffectTonal = filtrate::filters::PhotoEffectTonal;
/// Alias for the `PhotoEffectTransfer` filter.
pub type PhotoEffectTransfer = filtrate::filters::PhotoEffectTransfer;
/// Alias for the `MotionBlur` filter.
pub type MotionBlur = filtrate::filters::MotionBlur<Reactive, Reactive>;
/// Alias for the `BumpDistortion` filter.
pub type BumpDistortion = filtrate::filters::BumpDistortion<Reactive>;
/// Alias for the `PinchDistortion` filter.
pub type PinchDistortion = filtrate::filters::PinchDistortion<Reactive>;
/// Alias for the `TwirlDistortion` filter.
pub type TwirlDistortion = filtrate::filters::TwirlDistortion<Reactive>;
/// Alias for the `VortexDistortion` filter.
pub type VortexDistortion = filtrate::filters::VortexDistortion<Reactive>;
/// Alias for the `PerspectiveTransform` filter.
pub type PerspectiveTransform = filtrate::filters::PerspectiveTransform<f32>;
/// Alias for the `PerspectiveCorrection` filter.
pub type PerspectiveCorrection = filtrate::filters::PerspectiveCorrection<f32>;
/// Alias for the `Sepia` filter.
pub type Sepia = filtrate::filters::Sepia<Reactive>;
/// Alias for the `Vibrance` filter.
pub type Vibrance = filtrate::filters::Vibrance<Reactive>;
/// Alias for the `Pixellate` filter.
pub type Pixellate = filtrate::filters::Pixellate<Reactive>;
/// Alias for the `Crystallize` filter.
pub type Crystallize = filtrate::filters::Crystallize<Reactive>;
/// Alias for the `EdgeWork` filter.
pub type EdgeWork = filtrate::filters::EdgeWork<Reactive>;
/// Alias for the `DotHalftone` filter.
pub type DotHalftone = filtrate::filters::DotHalftone<Reactive>;
/// Alias for the `LineHalftone` filter.
pub type LineHalftone = filtrate::filters::LineHalftone<Reactive>;
/// Alias for the `Kaleidoscope` filter.
pub type Kaleidoscope = filtrate::filters::Kaleidoscope<Reactive>;
/// Alias for the `MirrorTile` filter.
pub type MirrorTile = filtrate::filters::MirrorTile<Reactive>;
/// Alias for the `UnsharpMask` filter.
pub type UnsharpMask = filtrate::filters::UnsharpMask<Reactive>;
/// Alias for the `Sharpen` filter.
pub type Sharpen = filtrate::filters::Sharpen<Reactive>;
/// Alias for the `Vignette` filter.
pub type Vignette = filtrate::filters::Vignette<Reactive, Reactive>;
/// Alias for the `WhitePoint` filter.
pub type WhitePoint = filtrate::filters::WhitePoint<Reactive, Reactive, Reactive>;
/// Alias for the `ZoomBlur` filter.
pub type ZoomBlur = filtrate::filters::ZoomBlur<Reactive, Reactive, Reactive>;
/// Alias for the `BlendWithImage` filter with reactive parameters.
pub type BlendWithImage = filtrate::filters::BlendWithImage<Reactive>;
/// Alias for the `MaskedBlur` filter with reactive parameters.
pub type MaskedBlur = filtrate::filters::MaskedBlur<Reactive>;
/// Alias for the `TransitionToImage` filter with reactive parameters.
pub type TransitionToImage = filtrate::filters::TransitionToImage<Reactive>;
/// Alias for the `SwipeTransitionToImage` filter with reactive parameters.
pub type SwipeTransitionToImage = filtrate::filters::SwipeTransitionToImage<Reactive>;
/// Alias for the `RadialTransitionToImage` filter with reactive parameters.
pub type RadialTransitionToImage = filtrate::filters::RadialTransitionToImage<Reactive>;
/// Alias for the `ZoomTransitionToImage` filter with reactive parameters.
pub type ZoomTransitionToImage = filtrate::filters::ZoomTransitionToImage<Reactive>;
/// Alias for the `DisplacementTransitionToImage` filter with reactive parameters.
pub type DisplacementTransitionToImage = filtrate::filters::DisplacementTransitionToImage<Reactive>;
/// Alias for the `DisplacementWarp` filter with reactive parameters.
pub type DisplacementWarp = filtrate::filters::DisplacementWarp<Reactive>;
/// Alias for the `GuidedSmooth` filter with reactive parameters.
pub type GuidedSmooth = filtrate::filters::GuidedSmooth<Reactive>;
/// Alias for the `DepthAwareBlur` filter with reactive parameters.
pub type DepthAwareBlur = filtrate::filters::DepthAwareBlur<Reactive>;
/// Alias for the `TemporalDenoise` filter with reactive parameters.
pub type TemporalDenoise = filtrate::filters::TemporalDenoise<Reactive>;
/// Alias for the `BackgroundReplace` filter with reactive parameters.
pub type BackgroundReplace = filtrate::filters::BackgroundReplace<Reactive>;
/// Alias for the `LutColorGrade` filter with reactive parameters.
pub type LutColorGrade = filtrate::filters::LutColorGrade<Reactive>;
/// Alias for the `ToneCurve` filter with reactive parameters.
pub type ToneCurve = filtrate::filters::ToneCurve<Reactive>;

/// Filters on any view: `.filter(F)`, `.effect(E)` and the named shortcuts.
pub trait FilterViewExt: View + Sized {
    /// Apply a `filtrate` filter to this view.
    fn filter<F: Filter + Send>(self, filter: F) -> Filtered<Self, F> {
        Filtered::new(self, filter)
    }

    /// Apply a custom `filtrate` effect to this view.
    fn effect(self, effect: impl Effect + Send) -> FilteredView {
        FilteredView::new(self, effect)
    }

    /// Apply a blur filter.
    ///
    /// Accepts reactive values that will be automatically animated.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Static value
    /// # fn fixed(my_view: impl View) -> impl View {
    /// my_view.blur(10.0)
    /// # }
    ///
    /// // Reactive value with animation
    /// # fn animated(my_view: impl View) -> impl View {
    /// let radius: Binding<f32> = binding(10.0);
    /// my_view.blur(radius.animated())
    /// # }
    /// ```
    fn blur<T: IntoSignalF32>(self, radius: T) -> Filtered<Self, Blur> {
        let mut guards = ParamGuards::default();
        Filtered::bound(self, filtrate::filters::Blur(guards.bind(radius)), guards)
    }

    /// Apply a brightness filter.
    fn brightness<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Brightness> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Brightness(guards.bind(amount)),
            guards,
        )
    }

    /// Apply an exposure filter in photographic stops.
    fn exposure<T: IntoSignalF32>(self, ev: T) -> Filtered<Self, Exposure> {
        let mut guards = ParamGuards::default();
        Filtered::bound(self, filtrate::filters::Exposure(guards.bind(ev)), guards)
    }

    /// Apply a gamma adjustment filter.
    fn gamma<T: IntoSignalF32>(self, gamma: T) -> Filtered<Self, Gamma> {
        let mut guards = ParamGuards::default();
        Filtered::bound(self, filtrate::filters::Gamma(guards.bind(gamma)), guards)
    }

    /// Apply a contrast filter.
    fn contrast<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Contrast> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Contrast(guards.bind(amount)),
            guards,
        )
    }

    /// Apply a saturation filter.
    fn saturation<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Saturation> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Saturation(guards.bind(amount)),
            guards,
        )
    }

    /// Apply a vibrance filter.
    fn vibrance<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Vibrance> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Vibrance(guards.bind(amount)),
            guards,
        )
    }

    /// Apply a grayscale filter.
    fn grayscale<T: IntoSignalF32>(self, intensity: T) -> Filtered<Self, Grayscale> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Grayscale(guards.bind(intensity)),
            guards,
        )
    }

    /// Apply a hue rotation filter.
    fn hue_rotation<T: IntoSignalF32>(self, angle: T) -> Filtered<Self, HueRotation> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::HueRotation(guards.bind(angle)),
            guards,
        )
    }

    /// Apply an invert filter.
    fn invert(self) -> Filtered<Self, Invert> {
        Filtered::new(self, filtrate::filters::Invert)
    }

    /// Apply a Sobel edge-detection filter (3x3, gradient magnitude).
    fn sobel(self) -> Filtered<Self, Sobel> {
        Filtered::new(self, filtrate::filters::Sobel)
    }

    /// Apply a Prewitt edge-detection filter (3x3, uniform-weight kernels).
    fn prewitt(self) -> Filtered<Self, Prewitt> {
        Filtered::new(self, filtrate::filters::Prewitt)
    }

    /// Apply a 3x3 per-channel median filter for salt-and-pepper denoising.
    fn median3x3(self) -> Filtered<Self, Median3x3> {
        Filtered::new(self, filtrate::filters::Median3x3)
    }

    /// Apply a 3x3 convolution filter with a caller-supplied kernel
    /// (row-major, top-left to bottom-right).
    fn convolution3x3<P: IntoSignalF32 + Copy>(
        self,
        kernel: [P; 9],
    ) -> Filtered<Self, Convolution3x3> {
        let mut guards = ParamGuards::default();
        let signals: [Reactive; 9] = core::array::from_fn(|i| guards.bind(kernel[i]));
        Filtered::bound(self, filtrate::filters::Convolution3x3(signals), guards)
    }

    /// Apply a 5x5 convolution filter with a caller-supplied 25-element
    /// kernel (row-major).
    fn convolution5x5<P: IntoSignalF32 + Copy>(
        self,
        kernel: [P; 25],
    ) -> Filtered<Self, Convolution5x5> {
        let mut guards = ParamGuards::default();
        let signals: [Reactive; 25] = core::array::from_fn(|i| guards.bind(kernel[i]));
        Filtered::bound(self, filtrate::filters::Convolution5x5(signals), guards)
    }

    /// Apply a 3x3 morphological erosion (per-channel minimum).
    fn morphology_min(self) -> Filtered<Self, MorphologyMin> {
        Filtered::new(self, filtrate::filters::MorphologyMin)
    }

    /// Apply a 3x3 morphological dilation (per-channel maximum).
    fn morphology_max(self) -> Filtered<Self, MorphologyMax> {
        Filtered::new(self, filtrate::filters::MorphologyMax)
    }

    /// Apply a 3x3 morphological gradient (per-channel max minus min).
    fn morphology_gradient(self) -> Filtered<Self, MorphologyGradient> {
        Filtered::new(self, filtrate::filters::MorphologyGradient)
    }

    /// Apply the monochrome photo preset.
    fn photo_effect_mono(self) -> Filtered<Self, PhotoEffectMono> {
        Filtered::new(self, filtrate::filters::PhotoEffectMono)
    }

    /// Apply the noir photo preset.
    fn photo_effect_noir(self) -> Filtered<Self, PhotoEffectNoir> {
        Filtered::new(self, filtrate::filters::PhotoEffectNoir)
    }

    /// Apply the chrome photo preset.
    fn photo_effect_chrome(self) -> Filtered<Self, PhotoEffectChrome> {
        Filtered::new(self, filtrate::filters::PhotoEffectChrome)
    }

    /// Apply the instant photo preset.
    fn photo_effect_instant(self) -> Filtered<Self, PhotoEffectInstant> {
        Filtered::new(self, filtrate::filters::PhotoEffectInstant)
    }

    /// Apply the fade photo preset.
    fn photo_effect_fade(self) -> Filtered<Self, PhotoEffectFade> {
        Filtered::new(self, filtrate::filters::PhotoEffectFade)
    }

    /// Apply the process photo preset.
    fn photo_effect_process(self) -> Filtered<Self, PhotoEffectProcess> {
        Filtered::new(self, filtrate::filters::PhotoEffectProcess)
    }

    /// Apply the tonal photo preset.
    fn photo_effect_tonal(self) -> Filtered<Self, PhotoEffectTonal> {
        Filtered::new(self, filtrate::filters::PhotoEffectTonal)
    }

    /// Apply the transfer photo preset.
    fn photo_effect_transfer(self) -> Filtered<Self, PhotoEffectTransfer> {
        Filtered::new(self, filtrate::filters::PhotoEffectTransfer)
    }

    /// Apply a sepia filter.
    fn sepia<T: IntoSignalF32>(self, intensity: T) -> Filtered<Self, Sepia> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Sepia(guards.bind(intensity)),
            guards,
        )
    }

    /// Apply a sharpen filter.
    fn sharpen<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Sharpen> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Sharpen(guards.bind(amount)),
            guards,
        )
    }

    /// Apply a temperature/tint white-balance adjustment.
    fn temperature_tint<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        temperature: T,
        tint: U,
    ) -> Filtered<Self, TemperatureTint> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::TemperatureTint(guards.bind(temperature), guards.bind(tint)),
            guards,
        )
    }

    /// Recover highlights while lifting shadows.
    fn highlights_shadows<H: IntoSignalF32, S: IntoSignalF32>(
        self,
        highlights: H,
        shadows: S,
    ) -> Filtered<Self, HighlightsShadows> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::HighlightsShadows(guards.bind(highlights), guards.bind(shadows)),
            guards,
        )
    }

    /// Apply directional motion blur.
    fn motion_blur<R: IntoSignalF32, A: IntoSignalF32>(
        self,
        radius: R,
        angle: A,
    ) -> Filtered<Self, MotionBlur> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::MotionBlur(guards.bind(radius), guards.bind(angle)),
            guards,
        )
    }

    /// Apply a vignette filter.
    fn vignette<R: IntoSignalF32, S: IntoSignalF32>(
        self,
        radius: R,
        softness: S,
    ) -> Filtered<Self, Vignette> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Vignette(guards.bind(radius), guards.bind(softness)),
            guards,
        )
    }

    /// Adjust color balance using an explicit white point triplet.
    fn white_point<R: IntoSignalF32, G: IntoSignalF32, B: IntoSignalF32>(
        self,
        red: R,
        green: G,
        blue: B,
    ) -> Filtered<Self, WhitePoint> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::WhitePoint(guards.bind(red), guards.bind(green), guards.bind(blue)),
            guards,
        )
    }

    /// Apply radial zoom blur around a focal point.
    fn zoom_blur<A: IntoSignalF32, X: IntoSignalF32, Y: IntoSignalF32>(
        self,
        amount: A,
        center_x: X,
        center_y: Y,
    ) -> Filtered<Self, ZoomBlur> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::ZoomBlur(
                guards.bind(amount),
                guards.bind(center_x),
                guards.bind(center_y),
            ),
            guards,
        )
    }

    /// Apply a gaussian blur filter.
    fn gaussian_blur<T: IntoSignalF32>(self, sigma: T) -> Filtered<Self, GaussianBlur> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::GaussianBlur(guards.bind(sigma)),
            guards,
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
        Filtered::new(self, filtrate::filters::ColorMatrix(params))
    }

    /// Apply bloom around bright regions.
    fn bloom<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32>(
        self,
        radius: T,
        intensity: U,
        threshold: V,
    ) -> Filtered<Self, Bloom> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Bloom {
                radius: guards.bind(radius),
                intensity: guards.bind(intensity),
                threshold: guards.bind(threshold),
            },
            guards,
        )
    }

    /// Apply gloom around bright regions.
    fn gloom<T: IntoSignalF32, U: IntoSignalF32, V: IntoSignalF32>(
        self,
        radius: T,
        intensity: U,
        threshold: V,
    ) -> Filtered<Self, Gloom> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Gloom {
                radius: guards.bind(radius),
                intensity: guards.bind(intensity),
                threshold: guards.bind(threshold),
            },
            guards,
        )
    }

    /// Apply an unsharp mask.
    fn unsharp_mask<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        radius: T,
        amount: U,
    ) -> Filtered<Self, UnsharpMask> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::UnsharpMask {
                radius: guards.bind(radius),
                intensity: guards.bind(amount),
            },
            guards,
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
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::BumpDistortion([
                guards.bind(center_x),
                guards.bind(center_y),
                guards.bind(radius),
                guards.bind(scale),
            ]),
            guards,
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
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::PinchDistortion([
                guards.bind(center_x),
                guards.bind(center_y),
                guards.bind(radius),
                guards.bind(scale),
            ]),
            guards,
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
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::TwirlDistortion([
                guards.bind(center_x),
                guards.bind(center_y),
                guards.bind(radius),
                guards.bind(angle),
            ]),
            guards,
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
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::VortexDistortion([
                guards.bind(center_x),
                guards.bind(center_y),
                guards.bind(radius),
                guards.bind(angle),
            ]),
            guards,
        )
    }

    /// Warp a source quadrilateral into the output rectangle.
    fn perspective_transform(self, quad: [[f32; 2]; 4]) -> Filtered<Self, PerspectiveTransform> {
        let params = [
            quad[0][0], quad[0][1], quad[1][0], quad[1][1], quad[2][0], quad[2][1], quad[3][0],
            quad[3][1],
        ];
        Filtered::new(self, filtrate::filters::PerspectiveTransform(params))
    }

    /// Correct a perspective-skewed quadrilateral back to a rectangle.
    fn perspective_correction(self, quad: [[f32; 2]; 4]) -> Filtered<Self, PerspectiveCorrection> {
        let params = [
            quad[0][0], quad[0][1], quad[1][0], quad[1][1], quad[2][0], quad[2][1], quad[3][0],
            quad[3][1],
        ];
        Filtered::new(self, filtrate::filters::PerspectiveCorrection(params))
    }

    /// Apply a pixellate effect.
    fn pixellate<T: IntoSignalF32>(self, size: T) -> Filtered<Self, Pixellate> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Pixellate(guards.bind(size)),
            guards,
        )
    }

    /// Apply a crystallize effect.
    fn crystallize<T: IntoSignalF32>(self, size: T) -> Filtered<Self, Crystallize> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Crystallize(guards.bind(size)),
            guards,
        )
    }

    /// Apply an edge-work effect.
    fn edge_work<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        radius: T,
        amount: U,
    ) -> Filtered<Self, EdgeWork> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::EdgeWork([guards.bind(radius), guards.bind(amount)]),
            guards,
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
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::DotHalftone([
                guards.bind(scale),
                guards.bind(angle),
                guards.bind(center_x),
                guards.bind(center_y),
            ]),
            guards,
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
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::LineHalftone([
                guards.bind(scale),
                guards.bind(angle),
                guards.bind(center_x),
                guards.bind(center_y),
            ]),
            guards,
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
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::Kaleidoscope([
                guards.bind(segments),
                guards.bind(angle),
                guards.bind(center_x),
                guards.bind(center_y),
            ]),
            guards,
        )
    }

    /// Apply mirrored tiling.
    fn mirror_tile<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        repeat_x: T,
        repeat_y: U,
    ) -> Filtered<Self, MirrorTile> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::MirrorTile([guards.bind(repeat_x), guards.bind(repeat_y)]),
            guards,
        )
    }

    /// Blend the current content with an auxiliary image.
    fn blend_with_image<T: IntoSignalF32>(
        self,
        image: FilterImage,
        amount: T,
        mode: BlendMode,
    ) -> Filtered<Self, BlendWithImage> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::BlendWithImage {
                image,
                amount: guards.bind(amount),
                mode,
            },
            guards,
        )
    }

    /// Apply masked blur using an auxiliary mask image.
    fn masked_blur<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        mask: FilterImage,
        radius: T,
        strength: U,
    ) -> Filtered<Self, MaskedBlur> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::MaskedBlur {
                mask,
                radius: guards.bind(radius),
                strength: guards.bind(strength),
            },
            guards,
        )
    }

    /// Transition to another image.
    fn transition_to_image<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        target: FilterImage,
        progress: T,
        softness: U,
    ) -> Filtered<Self, TransitionToImage> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::TransitionToImage {
                target,
                progress: guards.bind(progress),
                softness: guards.bind(softness),
            },
            guards,
        )
    }

    /// Transition to another image with a directional swipe.
    fn swipe_transition_to_image<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        target: FilterImage,
        progress: T,
        softness: U,
        direction: TransitionDirection,
    ) -> Filtered<Self, SwipeTransitionToImage> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::SwipeTransitionToImage {
                target,
                progress: guards.bind(progress),
                softness: guards.bind(softness),
                direction,
            },
            guards,
        )
    }

    /// Transition to another image from a radial reveal center.
    fn radial_transition_to_image<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        target: FilterImage,
        progress: T,
        softness: U,
        center_x: f32,
        center_y: f32,
    ) -> Filtered<Self, RadialTransitionToImage> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::RadialTransitionToImage {
                target,
                progress: guards.bind(progress),
                softness: guards.bind(softness),
                center_x: guards.bind(center_x),
                center_y: guards.bind(center_y),
            },
            guards,
        )
    }

    /// Transition to another image with a zooming blend.
    fn zoom_transition_to_image<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        target: FilterImage,
        progress: T,
        amount: U,
        center_x: f32,
        center_y: f32,
    ) -> Filtered<Self, ZoomTransitionToImage> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::ZoomTransitionToImage {
                target,
                progress: guards.bind(progress),
                amount: guards.bind(amount),
                center_x: guards.bind(center_x),
                center_y: guards.bind(center_y),
            },
            guards,
        )
    }

    /// Transition to another image driven by a displacement map.
    fn displacement_transition_to_image<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        target: FilterImage,
        map: FilterImage,
        progress: T,
        scale: U,
    ) -> Filtered<Self, DisplacementTransitionToImage> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::DisplacementTransitionToImage {
                target,
                map,
                progress: guards.bind(progress),
                scale: guards.bind(scale),
            },
            guards,
        )
    }

    /// Warp with an auxiliary displacement map.
    fn displacement_warp<T: IntoSignalF32, U: IntoSignalF32>(
        self,
        map: FilterImage,
        scale_x: T,
        scale_y: U,
    ) -> Filtered<Self, DisplacementWarp> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::DisplacementWarp {
                map,
                scale_x: guards.bind(scale_x),
                scale_y: guards.bind(scale_y),
            },
            guards,
        )
    }

    /// Apply guide-image-aware smoothing.
    fn guided_smooth<T: IntoSignalF32, U: IntoSignalF32, W: IntoSignalF32>(
        self,
        guide: FilterImage,
        radius: T,
        range_sigma: U,
        amount: W,
    ) -> Filtered<Self, GuidedSmooth> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::GuidedSmooth {
                guide,
                radius: guards.bind(radius),
                range_sigma: guards.bind(range_sigma),
                amount: guards.bind(amount),
            },
            guards,
        )
    }

    /// Apply depth-aware blur using a depth map.
    fn depth_aware_blur<T: IntoSignalF32, U: IntoSignalF32, W: IntoSignalF32>(
        self,
        depth: FilterImage,
        focus_depth: T,
        aperture: U,
        max_radius: W,
    ) -> Filtered<Self, DepthAwareBlur> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::DepthAwareBlur {
                depth,
                focus_depth: guards.bind(focus_depth),
                aperture: guards.bind(aperture),
                max_radius: guards.bind(max_radius),
            },
            guards,
        )
    }

    /// Temporal denoise/stabilize using history and motion maps.
    fn temporal_denoise<T: IntoSignalF32>(
        self,
        history: FilterImage,
        motion: FilterImage,
        history_weight: T,
    ) -> Filtered<Self, TemporalDenoise> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::TemporalDenoise {
                history,
                motion,
                history_weight: guards.bind(history_weight),
            },
            guards,
        )
    }

    /// Replace background using matte and background images.
    fn replace_background<T: IntoSignalF32>(
        self,
        matte: FilterImage,
        background: FilterImage,
        edge_softness: T,
    ) -> Filtered<Self, BackgroundReplace> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::BackgroundReplace {
                matte,
                background,
                edge_softness: guards.bind(edge_softness),
            },
            guards,
        )
    }

    /// Apply a 3D LUT color transform encoded as a 2D strip (`size*size x size`).
    fn lut_color_grade<T: IntoSignalF32>(
        self,
        lut: LutImage,
        intensity: T,
    ) -> Filtered<Self, LutColorGrade> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::LutColorGrade {
                lut,
                intensity: guards.bind(intensity),
            },
            guards,
        )
    }

    /// Apply a simple master tone curve.
    fn tone_curve<
        T: IntoSignalF32,
        U: IntoSignalF32,
        W: IntoSignalF32,
        X: IntoSignalF32,
        Y: IntoSignalF32,
    >(
        self,
        shadows: T,
        midtones: U,
        highlights: W,
        gamma: X,
        amount: Y,
    ) -> Filtered<Self, ToneCurve> {
        let mut guards = ParamGuards::default();
        Filtered::bound(
            self,
            filtrate::filters::ToneCurve {
                shadows: guards.bind(shadows),
                midtones: guards.bind(midtones),
                highlights: guards.bind(highlights),
                gamma: guards.bind(gamma),
                amount: guards.bind(amount),
            },
            guards,
        )
    }
}

impl<V: View> FilterViewExt for V {}

#[cfg(test)]
mod tests {
    use super::{FilterParam as _, ParamGuards};
    use std::sync::mpsc;

    #[test]
    fn reactive_parameters_keep_independent_subscription_lifetimes() {
        let value = nami::binding(0.25_f32);
        let mut guards = ParamGuards::default();
        let parameter = guards.bind(value.clone());
        let (first_send, first_receive) = mpsc::channel();
        let (second_send, second_receive) = mpsc::channel();
        let first = parameter.watch_animated(Box::new(move |target| {
            first_send
                .send(target.value.to_bits())
                .expect("first receiver exists");
        }));
        let second = parameter.clone().watch_animated(Box::new(move |target| {
            second_send
                .send(target.value.to_bits())
                .expect("second receiver exists");
        }));
        value.set(0.5_f32);
        assert_eq!(
            first_receive.try_recv().expect("first update"),
            0.5_f32.to_bits()
        );
        assert_eq!(
            second_receive.try_recv().expect("second update"),
            0.5_f32.to_bits()
        );
        drop(first);
        value.set(0.75_f32);
        assert!(matches!(
            first_receive.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert_eq!(
            second_receive.try_recv().expect("remaining subscription"),
            0.75_f32.to_bits()
        );
        assert_eq!(parameter.snapshot().to_bits(), 0.75_f32.to_bits());
        drop(second);
        drop(guards);
        value.set(1.0_f32);
        assert_eq!(parameter.snapshot().to_bits(), 0.75_f32.to_bits());
    }
}
