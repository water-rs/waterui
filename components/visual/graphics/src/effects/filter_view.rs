//! Filters on views: a `filtrate` filter applied to a view's rendered subtree.
//!
//! [`Filtered`] pairs a view with a [`Filter`]. Its body erases the filter
//! into a [`FilteredView`] carrying an [`AnyEffect`], the form a backend
//! receives.
//!
//! An [`AnyEffect`] made from a filter is portable: its
//! [`description`](AnyEffect::description) is the filter as `filtrate-core`
//! describes it — the stages that apply it in order, each with the WGSL
//! snippet that is its shader-source contract, the flattened parameters the
//! stages index, the reactive parameters behind them, and the auxiliary
//! images the stages bind. A render target that lowers filters into its own
//! primitives reads that description and needs no GPU. With the `gpu`
//! feature, `AnyEffect::build` lowers the same description into
//! `filtrate`'s executor on the render thread, and `AnyEffect::new` erases
//! an arbitrary GPU effect, which has no portable description.
//!
//! Reactive parameters are [`Reactive`] slots: a nami signal on the UI side
//! feeds a `Send` value slot the render side samples, and a change carrying
//! a public [`Animation`] in its metadata hands an interpolator to every
//! watcher. The engine consumes its own `crate::draw::Animation`, so the
//! metadata type is mapped through the public `curve()`/`duration()`
//! contract at the watcher boundary.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use arc_swap::ArcSwap;
use core::any::Any;
use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use crate::draw::{curve_value, settled, spring_step};

/// Values crossing the render-thread boundary, mirroring the engine's
/// `RenderTransfer` marker so `effects` builds without it.
#[cfg(not(target_arch = "wasm32"))]
pub trait RenderTransfer: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> RenderTransfer for T {}

/// Values retained by the local browser executor; they need not be `Send`.
#[cfg(target_arch = "wasm32")]
pub trait RenderTransfer {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> RenderTransfer for T {}
pub use filtrate::filters::{BlendMode, TransitionDirection};
use filtrate::{
    AnimatedCallback, AnimatedTarget, AuxData, AuxImage, Chain, ColorStage, Filter, FilterExt as _,
    FilterLink, FilterParam, ImageVisitor, Interpolator, LinkVisitor, ParamArray, Placed,
    SignalVisitor, SpatialStage, StageCollector, WatchGuard,
};
pub use filtrate::{FilterImage, LutImage};
use nami::{Signal, signal::IntoComputed};
use waterui_core::animation::Animation;
use waterui_core::easing::EasingCurve;
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, IntoSignalF32, View};

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::ErasedEffect;

/// A filter parameter fed by a nami signal.
///
/// The slot is `Send + Sync`; the UI-side subscription that writes it lives
/// in the view's [`ParamGuards`].
#[derive(Clone)]
pub struct Reactive(Arc<Slot>);

struct Slot {
    value: AtomicU32,
    watchers: Arc<Watchers<AnimatedFn>>,
}

/// The callback a [`Reactive`] slot hands every new target value.
type AnimatedFn = dyn Fn(AnimatedTarget) + Send + Sync;

impl fmt::Debug for Reactive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Reactive").field(&self.snapshot()).finish()
    }
}

/// Callbacks subscribed through [`WatchGuard`]s, each kept until its guard
/// drops.
///
/// Subscribing never displaces another subscriber, and the list is read
/// lock-free by whichever thread fires the callbacks.
struct Watchers<C: ?Sized>(ArcSwap<Vec<Arc<C>>>);

impl<C: ?Sized + 'static> Watchers<C> {
    fn new() -> Arc<Self> {
        Arc::new(Self(ArcSwap::from_pointee(Vec::new())))
    }

    /// Adds `callback` until the returned subscription drops.
    fn subscribe(self: &Arc<Self>, callback: Arc<C>) -> Subscription<C> {
        self.0.rcu(|callbacks| {
            let mut next = (**callbacks).clone();
            next.push(Arc::clone(&callback));
            next
        });
        Subscription {
            watchers: Arc::downgrade(self),
            callback,
        }
    }

    /// Calls `fire` with every current subscriber.
    fn for_each(&self, mut fire: impl FnMut(&C)) {
        for callback in self.0.load().iter() {
            fire(callback);
        }
    }
}

/// One entry of a [`Watchers`] list, removed when it drops.
///
/// It is `Send + Sync` whenever the callback is, unlike the [`WatchGuard`]
/// the public API wraps it in, so an effect holding one keeps its own
/// thread-safety.
struct Subscription<C: ?Sized> {
    watchers: Weak<Watchers<C>>,
    callback: Arc<C>,
}

impl<C: ?Sized> Drop for Subscription<C> {
    fn drop(&mut self) {
        if let Some(watchers) = self.watchers.upgrade() {
            watchers.0.rcu(|callbacks| {
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
        WatchGuard::new(self.0.watchers.subscribe(Arc::from(callback)))
    }
}

/// The UI-side subscriptions keeping a filter's [`Reactive`] parameters fed.
///
/// Dropping the guards freezes the parameters at their last value.
#[derive(Default)]
pub struct ParamGuards(Vec<Box<dyn Any>>);

impl fmt::Debug for ParamGuards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ParamGuards").field(&self.0.len()).finish()
    }
}

/// The pixel dimensions of a filtered view's output texture.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OutputSize {
    /// Match the captured input dimensions.
    #[default]
    MatchInput,
    /// Use fixed pixel dimensions.
    Fixed {
        /// Output width in pixels.
        width: u32,
        /// Output height in pixels.
        height: u32,
    },
    /// Scale both input dimensions by a factor.
    Scale(f32),
}

nami::impl_constant!(OutputSize);

impl OutputSize {
    /// Computes output dimensions from the captured input dimensions.
    #[must_use]
    pub fn compute(self, input_width: u32, input_height: u32) -> (u32, u32) {
        match self {
            Self::MatchInput => (input_width, input_height),
            Self::Fixed { width, height } => (width, height),
            Self::Scale(factor) => (
                scaled_dimension(input_width, factor),
                scaled_dimension(input_height, factor),
            ),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is rounded and clamped to the u32 range before the cast"
)]
fn scaled_dimension(value: u32, factor: f32) -> u32 {
    (f64::from(value) * f64::from(factor))
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

type ChangeCallback = dyn Fn() + Send + Sync;

/// Thread-safe output-size policy shared by the UI and render threads.
///
/// Its watchers are independent subscriptions: a backend's watcher and the
/// redraw callback of the built GPU effect each hold their own guard, so
/// neither replaces the other.
#[derive(Clone)]
struct OutputSizeState {
    policy: Arc<ArcSwap<Option<OutputSize>>>,
    watchers: Arc<Watchers<ChangeCallback>>,
}

impl OutputSizeState {
    fn new() -> Self {
        Self {
            policy: Arc::new(ArcSwap::from_pointee(None)),
            watchers: Watchers::new(),
        }
    }

    /// The declared policy; `None` until a view declares one.
    fn declared(&self) -> Option<OutputSize> {
        **self.policy.load()
    }

    /// Subscribes `callback`, fired after the declared policy changes, until
    /// the returned subscription drops.
    fn watch(&self, callback: Arc<ChangeCallback>) -> Subscription<ChangeCallback> {
        self.watchers.subscribe(callback)
    }

    fn bind(&self, value: impl IntoComputed<OutputSize>, guards: &mut ParamGuards) {
        let value = value.into_computed();
        self.policy.store(Arc::new(Some(value.snapshot())));
        let policy = Arc::clone(&self.policy);
        let watchers = Arc::clone(&self.watchers);
        let guard = value.watch(move |context| {
            policy.store(Arc::new(Some(context.into_value())));
            watchers.for_each(|callback| callback());
        });
        guards.0.push(Box::new(guard));
    }
}

impl ParamGuards {
    /// Binds a signal to a new [`Reactive`] slot, keeping the subscription here.
    pub fn bind(&mut self, value: impl IntoSignalF32) -> Reactive {
        let signal = value.into_signal_f32();
        let slot = Arc::new(Slot {
            value: AtomicU32::new(signal.snapshot().to_bits()),
            watchers: Watchers::new(),
        });
        let target = Arc::clone(&slot);
        let guard = signal.watch(move |context| {
            let animation = context
                .metadata()
                .try_get::<Animation>()
                .map(|animation| cherenkov_animation(&animation));
            let value = context.into_value();
            target.value.store(value.to_bits(), Ordering::Release);
            target.watchers.for_each(|callback| {
                callback(AnimatedTarget {
                    value,
                    interpolator: animation.map(|animation| {
                        Box::new(AnimationInterpolator(animation)) as Box<dyn Interpolator>
                    }),
                });
            });
        });
        self.0.push(Box::new(guard));
        Reactive(slot)
    }

    /// Retains a UI-side subscription until this filtered view is dropped.
    pub fn retain(&mut self, guard: impl Any) {
        self.0.push(Box::new(guard));
    }

    fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

/// Maps the public [`Animation`] onto the engine's execution type through
/// its `curve()`/`duration()` contract — [`Animation::Default`] resolves
/// through those accessors to the documented ease-in-out 250 ms.
fn cherenkov_animation(animation: &Animation) -> crate::draw::Animation {
    match animation.curve() {
        EasingCurve::CubicBezier(x1, y1, x2, y2) => {
            crate::draw::Animation::Curve(crate::draw::Curve::bezier(
                animation.duration(),
                f64::from(x1),
                f64::from(y1),
                f64::from(x2),
                f64::from(y2),
            ))
        }
        EasingCurve::Spring { stiffness, damping } => crate::draw::Animation::Spring(
            crate::draw::Spring::from_physics(f64::from(stiffness), f64::from(damping)),
        ),
    }
}

/// A Cherenkov animation driving a scalar filter parameter.
struct AnimationInterpolator(crate::draw::Animation);

const SPRING_STEP: f64 = 1.0 / 240.0;
const SPRING_STEP_NANOS: u128 = 1_000_000_000 / 240;
const SPRING_LIMIT: Duration = Duration::from_secs(10);

#[allow(clippy::cast_possible_truncation)]
impl AnimationInterpolator {
    fn spring_at(
        spring: &crate::draw::Spring,
        from: f32,
        to: f32,
        elapsed: Duration,
    ) -> (f64, bool) {
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
            crate::draw::Animation::Curve(curve) => curve.duration,
            crate::draw::Animation::Spring(_) => SPRING_LIMIT,
            crate::draw::Animation::Decay(_) => Duration::ZERO,
        }
    }

    fn interpolate(&self, from: f32, to: f32, elapsed: Duration) -> f32 {
        match &self.0 {
            crate::draw::Animation::Curve(curve) => {
                let t = if curve.duration.is_zero() {
                    1.0
                } else {
                    (elapsed.as_secs_f64() / curve.duration.as_secs_f64()).min(1.0)
                };
                let k = curve_value(curve, t);
                (f64::from(to) - f64::from(from)).mul_add(k, f64::from(from)) as f32
            }
            crate::draw::Animation::Spring(spring) => {
                Self::spring_at(spring, from, to, elapsed).0 as f32
            }
            crate::draw::Animation::Decay(_) => to,
        }
    }

    fn is_complete(&self, elapsed: Duration) -> bool {
        match &self.0 {
            crate::draw::Animation::Spring(spring) => {
                elapsed >= SPRING_LIMIT || Self::spring_at(spring, 0.0, 1.0, elapsed).1
            }
            _ => elapsed >= self.duration(),
        }
    }
}

/// Object-safe form of a [`Filter`], behind [`FilterDescription`].
///
/// `Filter`'s visitor methods are generic, so the trait itself cannot be
/// boxed; this form takes `dyn` sinks and adapts them back to `filtrate`'s
/// visitor traits inside the blanket implementation, where the concrete
/// filter type is known.
trait FilterSource: RenderTransfer {
    /// The flattened parameter values, in the order the stages index them.
    fn dyn_params(&self) -> Vec<f32>;
    fn dyn_collect_stages(&self, collector: &mut dyn StageCollector);
    fn dyn_visit_links(&self, visit: &mut dyn FnMut(FilterLink<'_>));
    fn dyn_visit_signals(&self, visit: &mut dyn FnMut(FilterSignal<'_>));
    fn dyn_visit_images(&self, visit: &mut dyn FnMut(usize, &dyn AuxImage));
    /// Lowers the filter into `filtrate`'s executor on the render thread.
    #[cfg(feature = "gpu")]
    fn build(self: Box<Self>, output_size: OutputSizeState) -> Box<dyn ErasedEffect>;
}

impl<F: Filter + RenderTransfer> FilterSource for F {
    fn dyn_params(&self) -> Vec<f32> {
        let mut values = alloc::vec![0.0; <F::Params as ParamArray>::LEN];
        self.params().write_to(&mut values);
        values
    }

    fn dyn_collect_stages(&self, collector: &mut dyn StageCollector) {
        self.collect_stages(&mut DynStages(collector));
    }

    fn dyn_visit_links(&self, visit: &mut dyn FnMut(FilterLink<'_>)) {
        self.visit_links(&mut DynLinks(visit));
    }

    fn dyn_visit_signals(&self, visit: &mut dyn FnMut(FilterSignal<'_>)) {
        self.visit_signals(&mut DynSignals(visit));
    }

    fn dyn_visit_images(&self, visit: &mut dyn FnMut(usize, &dyn AuxImage)) {
        self.visit_images(&mut DynImages(visit));
    }

    #[cfg(feature = "gpu")]
    fn build(self: Box<Self>, output_size: OutputSizeState) -> Box<dyn ErasedEffect> {
        gpu::lower_filter(*self, output_size)
    }
}

/// A [`LinkVisitor`] handing each link to a `dyn` sink.
struct DynLinks<'a>(&'a mut dyn FnMut(FilterLink<'_>));

impl LinkVisitor for DynLinks<'_> {
    fn link(&mut self, link: FilterLink<'_>) {
        (self.0)(link);
    }
}

/// A sized [`StageCollector`] forwarding to a `dyn` one, since
/// [`Filter::collect_stages`] takes its collector by type parameter.
struct DynStages<'a>(&'a mut dyn StageCollector);

impl StageCollector for DynStages<'_> {
    fn color(&mut self, stage: Placed<ColorStage>) {
        self.0.color(stage);
    }

    fn spatial(&mut self, stage: Placed<SpatialStage>) {
        self.0.spatial(stage);
    }
}

/// A [`SignalVisitor`] handing each parameter to a `dyn` sink as a
/// [`FilterSignal`].
struct DynSignals<'a>(&'a mut dyn FnMut(FilterSignal<'_>));

impl SignalVisitor for DynSignals<'_> {
    fn visit<P: FilterParam + ?Sized>(&mut self, index: usize, param: &P) {
        (self.0)(FilterSignal {
            index,
            param: &ParamRef(param),
        });
    }
}

/// An [`ImageVisitor`] handing each image to a `dyn` sink.
struct DynImages<'a>(&'a mut dyn FnMut(usize, &dyn AuxImage));

impl ImageVisitor for DynImages<'_> {
    fn visit<I: AuxImage + ?Sized>(&mut self, index: usize, image: &I) {
        (self.0)(index, &ImageRef(image));
    }
}

/// A borrowed parameter of any (possibly unsized) [`FilterParam`] type, as a
/// sized value that erases to `dyn SignalSource`.
///
/// It cannot implement [`FilterParam`] itself, whose `'static` bound a borrow
/// does not meet; that bound is also why [`FilterDescription::visit_signals`]
/// cannot accept a `filtrate` [`SignalVisitor`].
struct ParamRef<'a, P: ?Sized>(&'a P);

/// Object-safe access to one parameter, behind [`FilterSignal`].
trait SignalSource {
    fn snapshot(&self) -> f32;
    fn watch_animated(&self, callback: AnimatedCallback) -> WatchGuard;
}

impl<P: FilterParam + ?Sized> SignalSource for ParamRef<'_, P> {
    fn snapshot(&self) -> f32 {
        self.0.snapshot()
    }

    fn watch_animated(&self, callback: AnimatedCallback) -> WatchGuard {
        self.0.watch_animated(callback)
    }
}

/// A borrowed image of any (possibly unsized) [`AuxImage`] type, as a sized
/// value that erases to `dyn AuxImage`.
struct ImageRef<'a, I: ?Sized>(&'a I);

impl<I: AuxImage + ?Sized> AuxImage for ImageRef<'_, I> {
    fn width(&self) -> u32 {
        self.0.width()
    }

    fn height(&self) -> u32 {
        self.0.height()
    }

    fn data(&self) -> Option<AuxData<'_>> {
        self.0.data()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        self.0.as_any()
    }
}

/// One reactive parameter of a filter, visited by
/// [`FilterDescription::visit_signals`].
///
/// It borrows the parameter for the duration of the visit: read its current
/// value with [`snapshot`](Self::snapshot) and subscribe to its changes with
/// [`watch_animated`](Self::watch_animated), keeping the returned guard.
pub struct FilterSignal<'a> {
    index: usize,
    param: &'a dyn SignalSource,
}

impl fmt::Debug for FilterSignal<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterSignal")
            .field("index", &self.index)
            .field("value", &self.snapshot())
            .finish()
    }
}

impl FilterSignal<'_> {
    /// The parameter's index in the flattened parameters
    /// ([`FilterDescription::params`]).
    ///
    /// A stage addresses this index as its [`Placed::param_base`] plus the
    /// index of a [`ParamSource::Param`](filtrate::ParamSource::Param) it
    /// declares, not by the declared index alone: in
    /// `brightness(..).blur(..)`, the blur stage declares `Param(0)` with
    /// `param_base == 1`, which is index 1.
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    /// The parameter's current value.
    #[must_use]
    pub fn snapshot(&self) -> f32 {
        self.param.snapshot()
    }

    /// Subscribes to the parameter's changes.
    ///
    /// `callback` receives every new target value, with the interpolator of
    /// the animation the change carries, if any. Dropping the returned guard
    /// cancels the subscription.
    #[must_use = "dropping the guard cancels the subscription"]
    pub fn watch_animated(
        &self,
        callback: impl Fn(AnimatedTarget) + Send + Sync + 'static,
    ) -> WatchGuard {
        self.param.watch_animated(Box::new(callback))
    }
}

/// The portable description of a filter, read without a GPU.
///
/// It mirrors `filtrate-core`'s [`Filter`]: the stages in application order,
/// the flattened parameter values they index, the reactive parameters behind
/// those values, and the auxiliary images the stages bind. Each stage's
/// [`ColorStage::source`] / [`SpatialStage::source`] is the WGSL snippet
/// that is its shader-source contract. The description holds no GPU context
/// and records no GPU work.
#[derive(Clone, Copy)]
pub struct FilterDescription<'a>(&'a dyn FilterSource);

impl fmt::Debug for FilterDescription<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterDescription")
            .field("params", &self.params())
            .finish_non_exhaustive()
    }
}

impl FilterDescription<'_> {
    /// The current parameter values, flattened across every stage.
    ///
    /// A stage's <code>[ParamSource::Param](filtrate::ParamSource::Param)(i)</code>
    /// reads the value at `param_base + i`, where `param_base` is the
    /// [`Placed::param_base`] that [`collect_stages`](Self::collect_stages)
    /// reports with the stage. A `vecN<f32>` member reads `N` consecutive
    /// values from there.
    #[must_use]
    pub fn params(&self) -> Vec<f32> {
        self.0.dyn_params()
    }

    /// Reports every stage, in application order, with the offsets that
    /// place its parameter and image indices.
    pub fn collect_stages(&self, collector: &mut impl StageCollector) {
        self.0.dyn_collect_stages(collector);
    }

    /// Visits every link of the filter in application order.
    ///
    /// A link's current values are
    /// <code>[params](Self::params)()[link.param_base..]</code>, and its
    /// reactive updates arrive through [`visit_signals`](Self::visit_signals)
    /// at those indices.
    pub fn visit_links(&self, mut visit: impl FnMut(FilterLink<'_>)) {
        self.0.dyn_visit_links(&mut visit);
    }

    /// Visits every reactive parameter, by index in [`params`](Self::params).
    ///
    /// A parameter that is not visited is constant at its value in
    /// [`params`](Self::params).
    pub fn visit_signals(&self, mut visit: impl FnMut(FilterSignal<'_>)) {
        self.0.dyn_visit_signals(&mut visit);
    }

    /// Visits the auxiliary images, by index in the flattened images of the
    /// whole filter.
    ///
    /// A spatial stage's <code>[AuxSource::Image](filtrate::AuxSource::Image)(i)</code>
    /// or <code>[AuxSource::Texture](filtrate::AuxSource::Texture)(i)</code> binds the
    /// image at `image_base + i`, where `image_base` is the
    /// [`Placed::image_base`] that [`collect_stages`](Self::collect_stages)
    /// reports with the stage.
    pub fn visit_images(&self, visitor: &mut impl ImageVisitor) {
        self.0
            .dyn_visit_images(&mut |index, image: &dyn AuxImage| visitor.visit(index, image));
    }
}

/// What an [`AnyEffect`] carries: a closed choice between a portable filter
/// and an effect that only a GPU can run.
enum EffectSource {
    /// A filter, described by its stages, parameters and images.
    Filter(Box<dyn FilterSource>),
    /// An arbitrary GPU effect, which has no portable description.
    #[cfg(feature = "gpu")]
    Gpu(Box<dyn gpu::GpuEffect>),
}

/// A filter or effect, erased for the render thread, as a backend receives it.
///
/// A backend that lowers filters into its own primitives reads the
/// [`description`](Self::description). With the `gpu` feature, a backend
/// that runs `filtrate`'s executor moves the effect to its render thread and
/// calls `build` there; the result runs against the engine's
/// device.
pub struct AnyEffect {
    source: EffectSource,
    output_size: OutputSizeState,
}

impl fmt::Debug for AnyEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyEffect")
            .field("description", &self.description())
            .field("output_size", &self.output_size())
            .finish()
    }
}

impl AnyEffect {
    fn from_source(source: EffectSource) -> Self {
        Self {
            source,
            output_size: OutputSizeState::new(),
        }
    }

    /// Erases a filter, keeping its portable description.
    pub fn filter(filter: impl Filter + RenderTransfer) -> Self {
        Self::from_source(EffectSource::Filter(Box::new(filter)))
    }

    /// The portable description of the filter this effect applies, or `None`
    /// for an arbitrary GPU effect, which only a GPU can run.
    #[must_use]
    pub fn description(&self) -> Option<FilterDescription<'_>> {
        match &self.source {
            EffectSource::Filter(filter) => Some(FilterDescription(&**filter)),
            #[cfg(feature = "gpu")]
            EffectSource::Gpu(_) => None,
        }
    }

    /// The output-size policy the view declared, or `None` when the output
    /// matches the effect's own size for its input.
    #[must_use]
    pub fn output_size(&self) -> Option<OutputSize> {
        self.output_size.declared()
    }

    /// Subscribes `callback`, fired after the declared
    /// [`output_size`](Self::output_size) changes, until the returned guard
    /// drops.
    ///
    /// Any number of watchers may subscribe. None replaces another, and none
    /// is replaced by the redraw callback the engine installs on the effect
    /// that the GPU `build` produces: a subscription made before `build`
    /// keeps firing afterwards.
    #[must_use = "dropping the guard cancels the subscription"]
    pub fn watch_output_size(&self, callback: impl Fn() + Send + Sync + 'static) -> WatchGuard {
        WatchGuard::new(self.output_size.watch(Arc::new(callback)))
    }

    fn bind_output_size(&self, size: impl IntoComputed<OutputSize>, guards: &mut ParamGuards) {
        self.output_size.bind(size, guards);
    }
}

/// A view with a filter applied to its rendered subtree.
#[derive(Debug)]
pub struct Filtered<V, F> {
    view: V,
    filter: F,
    guards: ParamGuards,
}

impl<V: View, F: Filter + RenderTransfer> Filtered<V, F> {
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
    pub fn then<G: Filter + RenderTransfer>(self, next: G) -> Filtered<V, Chain<F, G>> {
        self.then_bound(next, ParamGuards::default())
    }

    fn then_bound<G: Filter + RenderTransfer>(
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

    /// Erases the view and the filter into the form a backend receives.
    fn erase(self) -> FilteredView {
        FilteredView {
            content: AnyView::new(self.view),
            effect: AnyEffect::filter(self.filter),
            guards: self.guards,
        }
    }
}

impl<V: View, F: Filter + RenderTransfer> View for Filtered<V, F> {
    fn body(self, _env: &Environment) -> impl View {
        self.erase()
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.view.stretch_axis()
    }
}

/// A view with an effect on its rendered subtree, as a backend receives it.
///
/// The backend realizes `effect` on the layer rendering `content` — from its
/// [`description`](AnyEffect::description), or, with the `gpu` feature, by
/// registering it with its engine (`Engine::effect`) — and keeps `guards`
/// alive as long as the realization.
#[derive(Debug)]
pub struct FilteredView {
    /// The filtered subtree.
    pub content: AnyView,
    /// The effect to realize.
    pub effect: AnyEffect,
    /// The subscriptions feeding the effect's reactive parameters.
    pub guards: ParamGuards,
}

impl FilteredView {
    /// Sets the output texture dimensions without changing layout.
    #[must_use]
    pub fn output_size(mut self, size: impl IntoComputed<OutputSize>) -> Self {
        self.effect.bind_output_size(size, &mut self.guards);
        self
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

impl<V: View, F: Filter + RenderTransfer> Filtered<V, F> {
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
    fn filter<F: Filter + RenderTransfer>(self, filter: F) -> Filtered<Self, F> {
        Filtered::new(self, filter)
    }

    /// Apply a custom `filtrate` effect to this view.
    ///
    /// An arbitrary effect has no portable description, so only a backend
    /// that runs effects on a GPU can realize it.
    #[cfg(feature = "gpu")]
    fn effect(self, effect: impl filtrate::Effect + RenderTransfer) -> FilteredView {
        FilteredView::new(self, effect)
    }

    /// Apply a blur filter.
    ///
    /// Accepts reactive values that will be automatically animated.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nami::binding;
    /// use waterui_core::View;
    /// use waterui_graphics::filter_view::FilterViewExt;
    ///
    /// // Static value
    /// # fn fixed(my_view: impl View) -> impl View {
    /// my_view.blur(10.0)
    /// # }
    ///
    /// // Reactive value
    /// # fn reactive(my_view: impl View) -> impl View {
    /// let radius: nami::Binding<f32> = binding(10.0f32);
    /// my_view.blur(radius)
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
    #[cfg(feature = "gpu")]
    use super::AnyEffect;
    use super::{
        AnimationInterpolator, Brightness, ColorStage, FilterLink, FilterParam as _, FilterSignal,
        FilterViewExt as _, FilteredView, GaussianBlur, OutputSize, ParamGuards, Placed,
        SPRING_LIMIT, SpatialStage, StageCollector, cherenkov_animation,
    };
    use core::time::Duration;
    use filtrate::Interpolator as _;
    use nami::SignalExt as _;
    #[cfg(feature = "gpu")]
    use std::sync::Arc;
    #[cfg(feature = "gpu")]
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    #[cfg(feature = "gpu")]
    use waterui_core::AnyView;
    use waterui_core::animation::Animation;

    /// Records each stage's kind, name and parameter offset, and checks that
    /// it carries its shader source.
    #[derive(Default)]
    struct Stages(Vec<(&'static str, &'static str, usize)>);

    impl StageCollector for Stages {
        fn color(&mut self, stage: Placed<ColorStage>) {
            assert_ne!(stage.stage.source, "");
            self.0.push(("color", stage.stage.name, stage.param_base));
        }

        fn spatial(&mut self, stage: Placed<SpatialStage>) {
            assert_ne!(stage.stage.source, "");
            self.0.push(("spatial", stage.stage.name, stage.param_base));
        }
    }

    #[test]
    fn filter_chain_describes_its_stages_in_order() {
        let FilteredView { effect, guards, .. } = ().brightness(0.25_f32).blur(4.0_f32).erase();
        let description = effect.description().expect("a filter is portable");
        let mut stages = Stages::default();
        description.collect_stages(&mut stages);
        assert_eq!(
            stages.0,
            [
                ("color", "Brightness", 0),
                ("spatial", "box_blur_horizontal", 1),
                ("spatial", "box_blur_vertical", 1),
            ]
        );
        assert_eq!(description.params(), [0.25, 4.0]);
        drop(guards);
    }

    #[test]
    fn described_filter_links_are_the_concrete_filters() {
        let FilteredView { effect, guards, .. } =
            ().gaussian_blur(1.5_f32).brightness(0.25_f32).erase();
        let description = effect.description().expect("a filter is portable");

        let mut links = Vec::new();
        description.visit_links(|link: FilterLink<'_>| {
            links.push((
                link.downcast_ref::<GaussianBlur>().is_some(),
                link.downcast_ref::<Brightness>().is_some(),
                link.param_base,
                link.image_base,
            ));
        });
        assert_eq!(links, [(true, false, 0, 0), (false, true, 1, 0)]);
        assert_eq!(description.params(), [1.5, 0.25]);
        drop(guards);
    }

    #[test]
    fn described_filter_signals_follow_their_bindings() {
        let amount = nami::binding(0.25_f32);
        let FilteredView { effect, guards, .. } =
            ().brightness(amount.clone()).blur(4.0_f32).erase();
        let description = effect.description().expect("a filter is portable");

        let (send, receive) = mpsc::channel();
        let mut subscriptions = Vec::new();
        let mut visited = Vec::new();
        description.visit_signals(|signal: FilterSignal<'_>| {
            visited.push((signal.index(), signal.snapshot()));
            let send = send.clone();
            let index = signal.index();
            subscriptions.push(signal.watch_animated(move |target| {
                send.send((index, target.value)).expect("receiver exists");
            }));
        });
        assert_eq!(visited, [(0, 0.25), (1, 4.0)]);

        amount.set(0.5);
        assert_eq!(receive.try_recv().expect("brightness update"), (0, 0.5));
        assert!(receive.try_recv().is_err());
        assert_eq!(description.params(), [0.5, 4.0]);
        drop(subscriptions);
        drop(guards);
    }

    /// A GPU effect that renders nothing.
    #[cfg(feature = "gpu")]
    struct NoopEffect;

    #[cfg(feature = "gpu")]
    impl filtrate::Effect for NoopEffect {
        fn setup(
            &mut self,
            _ctx: &filtrate::EffectContext<'_>,
        ) -> impl Future<Output = filtrate::EffectSetupResult> {
            core::future::ready(Ok(()))
        }

        fn encode_render(
            &mut self,
            _input: &filtrate::EffectInput,
            _output: &filtrate::EffectOutput,
            _encoder: &mut wgpu::CommandEncoder,
        ) -> filtrate::EffectRenderResult {
            Ok(false)
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_effect_has_no_portable_description() {
        assert!(AnyEffect::new(NoopEffect).description().is_none());
        assert!(
            AnyEffect::filter(filtrate::filters::Invert)
                .description()
                .is_some()
        );
    }

    #[test]
    fn output_size_computes_declared_dimensions() {
        assert_eq!(OutputSize::MatchInput.compute(10, 20), (10, 20));
        assert_eq!(
            OutputSize::Fixed {
                width: 1920,
                height: 1080,
            }
            .compute(10, 20),
            (1920, 1080)
        );
        assert_eq!(OutputSize::Scale(1.5).compute(10, 20), (15, 30));
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn filtered_view_output_size_reaches_the_effect() {
        let executor = filtrate::Executor::new(filtrate::filters::Invert)
            .with_output_size(|width, height| (width * 3, height * 4));
        assert_eq!(filtrate::Effect::output_size(&executor, 10, 20), (30, 80));

        let filtered = FilteredView {
            content: AnyView::new(()),
            effect: AnyEffect::filter(filtrate::filters::Invert),
            guards: ParamGuards::default(),
        }
        .output_size(OutputSize::Fixed {
            width: 1920,
            height: 1080,
        });
        let FilteredView { effect, guards, .. } = filtered;
        let effect = effect.build();

        assert_eq!(effect.output_size(10, 20), (1920, 1080));
        drop(guards);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn filter_without_declared_size_matches_input() {
        let filtered = FilteredView {
            content: AnyView::new(()),
            effect: AnyEffect::filter(filtrate::filters::Invert),
            guards: ParamGuards::default(),
        };
        let FilteredView { effect, guards, .. } = filtered;
        let effect = effect.build();
        assert_eq!(effect.output_size(10, 20), (10, 20));
        drop(guards);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn output_size_signal_updates_without_rebuilding_the_effect() {
        let size = nami::binding(OutputSize::Scale(2.0));
        let filtered = FilteredView {
            content: AnyView::new(()),
            effect: AnyEffect::filter(filtrate::filters::Invert),
            guards: ParamGuards::default(),
        }
        .output_size(size.clone());
        let FilteredView { effect, guards, .. } = filtered;
        let mut effect = effect.build();
        let redraws = Arc::new(AtomicUsize::new(0));
        let callback_redraws = Arc::clone(&redraws);
        effect.set_redraw_callback(Arc::new(move || {
            callback_redraws.fetch_add(1, Ordering::Relaxed);
        }));

        assert_eq!(effect.output_size(10, 20), (20, 40));
        size.set(OutputSize::Fixed {
            width: 30,
            height: 40,
        });
        assert_eq!(effect.output_size(10, 20), (30, 40));
        assert_eq!(redraws.load(Ordering::Relaxed), 1);
        drop(guards);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn output_size_watchers_survive_the_gpu_redraw_callback() {
        let size = nami::binding(OutputSize::Scale(2.0));
        let FilteredView { effect, guards, .. } = FilteredView {
            content: AnyView::new(()),
            effect: AnyEffect::filter(filtrate::filters::Invert),
            guards: ParamGuards::default(),
        }
        .output_size(size.clone());
        let watched = Arc::new(AtomicUsize::new(0));
        let watcher_count = Arc::clone(&watched);
        let subscription = effect.watch_output_size(move || {
            watcher_count.fetch_add(1, Ordering::Relaxed);
        });
        let mut effect = effect.build();
        let redraws = Arc::new(AtomicUsize::new(0));
        let redraw_count = Arc::clone(&redraws);
        effect.set_redraw_callback(Arc::new(move || {
            redraw_count.fetch_add(1, Ordering::Relaxed);
        }));

        size.set(OutputSize::Fixed {
            width: 30,
            height: 40,
        });
        assert_eq!(watched.load(Ordering::Relaxed), 1);
        assert_eq!(redraws.load(Ordering::Relaxed), 1);

        drop(subscription);
        size.set(OutputSize::Scale(3.0));
        assert_eq!(watched.load(Ordering::Relaxed), 1);
        assert_eq!(redraws.load(Ordering::Relaxed), 2);
        drop(guards);
    }

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
        let second = parameter.watch_animated(Box::new(move |target| {
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

    #[test]
    fn animation_metadata_reaches_param_interpolators() {
        let value = nami::binding(0.25_f32);
        let mut guards = ParamGuards::default();
        let parameter = guards.bind(value.with(Animation::ease_in_out(Duration::from_millis(300))));
        let (send, receive) = mpsc::channel();
        let subscription = parameter.watch_animated(Box::new(move |target| {
            send.send(target).expect("receiver exists");
        }));
        value.set(0.5_f32);
        let target = receive.try_recv().expect("animated update");
        assert_eq!(target.value.to_bits(), 0.5_f32.to_bits());
        let interpolator = target.interpolator.expect("metadata interpolator");
        assert_eq!(interpolator.duration(), Duration::from_millis(300));
        let mid = interpolator.interpolate(0.0, 1.0, Duration::from_millis(150));
        assert!((mid - 0.5).abs() < 1e-3, "symmetric ease-in-out midpoint");
        assert!(interpolator.is_complete(Duration::from_millis(300)));
        drop(subscription);
        drop(guards);
    }

    #[test]
    fn animation_default_resolves_documented_ease_in_out() {
        let interpolator = AnimationInterpolator(cherenkov_animation(&Animation::Default));
        assert_eq!(interpolator.duration(), Duration::from_millis(250));
        let mid = interpolator.interpolate(0.0, 1.0, Duration::from_millis(125));
        assert!(
            (mid - 0.5).abs() < 1e-3,
            "Default ease-in-out midpoint, got {mid}"
        );
    }

    #[test]
    fn animation_bezier_preserves_duration_and_shape() {
        let interpolator = AnimationInterpolator(cherenkov_animation(&Animation::bezier(
            Duration::from_millis(400),
            0.25,
            0.1,
            0.25,
            1.0,
        )));
        assert_eq!(interpolator.duration(), Duration::from_millis(400));
        let quarter = interpolator.interpolate(0.0, 1.0, Duration::from_millis(100));
        assert!(
            quarter > 0.25,
            "ease curve leads linear early, got {quarter}"
        );
        assert_eq!(
            interpolator
                .interpolate(0.0, 1.0, Duration::from_millis(400))
                .to_bits(),
            1.0_f32.to_bits()
        );
    }

    #[test]
    fn animation_spring_uses_engine_physics() {
        let interpolator =
            AnimationInterpolator(cherenkov_animation(&Animation::spring(200.0, 15.0)));
        assert_eq!(interpolator.duration(), SPRING_LIMIT);
        let early = interpolator.interpolate(0.0, 1.0, Duration::from_millis(30));
        assert!(
            early > 0.0 && early < 1.0,
            "spring progresses without snapping, got {early}"
        );
        assert!(!interpolator.is_complete(Duration::from_millis(30)));
    }
}
