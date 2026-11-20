//! Embedding `android.view.View` instances through [`AndroidContent`] and
//! [`AndroidView`].
//!
//! A crate that wraps a platform-coupled Android SDK view — an advertising SDK's
//! `AdView` or `NativeAdView`, `CameraX`'s `PreviewView`, a map view — implements
//! [`AndroidContent`] in Rust against [`jni`] and places it in a view tree with
//! [`AndroidView`]. Both live outside `NativeBackend`'s list of first-party
//! components: [`AndroidView`] reaches `HydroBackend`
//! through `NativeBackend::provided` on the static path (monomorphized for `T`
//! without extra boxing) and `NativeBackend::provided_dyn` on the
//! [`Erased`](waterui_core::backend::Erased) path, answers
//! [`waterui_backend_core::ProvidedSemantics`] on semantic test runtimes
//! without creating an Android `View`, and reports a contract violation when
//! reaching any other presentation backend.

use core::any::Any;
use core::cell::RefCell;
use core::fmt;
use std::rc::Rc;
#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
use std::sync::{Arc, Weak};

pub use jni;
use jni::Env;
use jni::objects::{Global, JObject};
use jni::vm::JavaVM;
use jni::{jni_sig, jni_str};
use nami::Signal;
use nami::watcher::Context;
use waterui::Str;
use waterui::accessibility::AccessibilityRole;
use waterui_backend_core::ProvidedSemantics;
#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
use waterui_core::contract_violation;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_core::{Computed, Environment, Native, NativeView, View};

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
use crate::platform_view::{PlatformViewPlacement, PlatformViewSink, PlatformViewSource};
#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
use crate::renderer::{
    HydroState, HydrolysisRenderer, RenderContext, SafeAreaLayout, WidgetBehavior,
};
#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
use crate::runner::android::host::AndroidHostBridge;
#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
use crate::theme::WidgetTheme;

/// Host `Activity` and embedding lifecycle transitions delivered to a mounted
/// [`AndroidContent`] view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AndroidLifecycle {
    /// The host entered `onStart`, or the view mounted while already started.
    Start,
    /// The host entered `onResume`, or the view mounted while already resumed.
    Resume,
    /// The host entered `onPause`.
    Pause,
    /// The host entered `onStop`.
    Stop,
    /// The view is leaving the tree or the host session is tearing down.
    Destroy,
}

/// Keeps reactive watcher guards and auxiliary host state alive as long as
/// a mounted [`AndroidContent`] view remains in the tree, dropping them in
/// reverse insertion order before lifecycle teardown and instance removal.
#[derive(Default)]
struct KeepAlive {
    items: Vec<Box<dyn Any>>,
}

impl KeepAlive {
    fn keep(&mut self, value: impl Any) {
        self.items.push(Box::new(value));
    }

    fn watch<S: Signal>(&mut self, signal: &S, watcher: impl Fn(Context<S::Output>) + 'static) {
        let guard = signal.watch(watcher);
        self.keep(guard);
    }

    fn bind<S: Signal>(&mut self, signal: &S, apply: impl Fn(S::Output) + 'static) {
        apply(signal.snapshot());
        self.watch(signal, move |ctx| apply(ctx.into_value()));
    }
}

impl Drop for KeepAlive {
    fn drop(&mut self) {
        while let Some(item) = self.items.pop() {
            drop(item);
        }
    }
}

/// Invalidates layout or display for a mounted [`AndroidContent`] view when a
/// reactive input changes what the view measures or draws.
#[derive(Clone)]
pub struct AndroidInvalidator {
    mark_layout: Rc<dyn Fn()>,
    mark_paint: Rc<dyn Fn()>,
    #[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
    host_invalidate: Rc<dyn Fn()>,
}

impl fmt::Debug for AndroidInvalidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AndroidInvalidator").finish_non_exhaustive()
    }
}

impl AndroidInvalidator {
    /// Marks this leaf's layout dirty up the ancestor chain and schedules a
    /// host frame update.
    pub fn invalidate_layout(&self) {
        (self.mark_layout)();
    }

    /// Requests redraw of the mounted `android.view.View` and marks paint dirty
    /// on the host tree without invalidating layout.
    pub fn invalidate_display(&self) {
        #[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
        (self.host_invalidate)();
        (self.mark_paint)();
    }
}

/// The UI-thread mount context handed to [`AndroidContent::make_view`].
///
/// Supplies the active [`Environment`], the host `Context` (a rebind-aware
/// wrapper over the live `Activity` that updates across configuration changes),
/// the [`JavaVM`], an [`AndroidInvalidator`] for reactive size or display
/// changes, and a keep-alive store whose guards drop when the mounted view
/// leaves the tree.
pub struct AndroidMount<'a> {
    env: &'a Environment,
    context: &'a JObject<'a>,
    vm: &'a JavaVM,
    invalidator: AndroidInvalidator,
    keepalive: &'a mut KeepAlive,
}

impl fmt::Debug for AndroidMount<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AndroidMount").finish_non_exhaustive()
    }
}

impl AndroidMount<'_> {
    /// The environment at this view's position in the tree.
    #[must_use]
    pub const fn env(&self) -> &Environment {
        self.env
    }

    /// The host `android.content.Context` for constructing the embedded
    /// `android.view.View`.
    ///
    /// Backed by a `MutableContextWrapper` tracked by the session so
    /// configuration changes update its base `Activity` without leaking the old
    /// one.
    #[must_use]
    pub const fn context(&self) -> &JObject<'_> {
        self.context
    }

    /// The process [`JavaVM`] for signal watchers that attach back into JNI.
    #[must_use]
    pub const fn vm(&self) -> &JavaVM {
        self.vm
    }

    /// A cloneable handle that invalidates layout or view display when a
    /// signal watcher mutates the mounted `View`.
    #[must_use]
    pub fn invalidator(&self) -> AndroidInvalidator {
        self.invalidator.clone()
    }

    /// Retains `value` — a watcher guard, JNI global reference, or auxiliary
    /// state — with the mounted view.
    ///
    /// Kept values drop in reverse insertion order before [`AndroidLifecycle::Destroy`].
    pub fn keep(&mut self, value: impl Any) {
        self.keepalive.keep(value);
    }

    /// Subscribes `watcher` to `signal` and retains the guard with the mounted
    /// view.
    pub fn watch<S: Signal>(&mut self, signal: &S, watcher: impl Fn(Context<S::Output>) + 'static) {
        self.keepalive.watch(signal, watcher);
    }

    /// Applies `signal`'s current value immediately, then subscribes `apply` to
    /// every subsequent change and retains the guard with the mounted view.
    pub fn bind<S: Signal>(&mut self, signal: &S, apply: impl Fn(S::Output) + 'static) {
        self.keepalive.bind(signal, apply);
    }
}

/// Supplies an `android.view.View` for [`AndroidView`].
pub trait AndroidContent: 'static {
    /// Creates the `android.view.View` on the UI thread with the host
    /// `Context` supplied by [`AndroidMount::context`], subscribing any
    /// reactive inputs through `mount`.
    ///
    /// # Errors
    ///
    /// Returns a JNI error if Java class lookup, construction, or configuration
    /// fails.
    fn make_view(
        &mut self,
        env: &mut Env<'_>,
        mount: &mut AndroidMount<'_>,
    ) -> jni::errors::Result<Global<JObject<'static>>>;

    /// Measures the mounted `view` against `proposal` in logical dp.
    ///
    /// Defaults to measuring `view` with Android `MeasureSpec` (`AT_MOST` for
    /// finite proposals, `UNSPECIFIED` otherwise), converting measured pixels
    /// back to logical dp using the view's display density, and expanding
    /// along axes [`stretch_axis`](Self::stretch_axis) stretches when `proposal`
    /// offers more finite space.
    fn measure(
        &self,
        env: &mut Env<'_>,
        view: &JObject<'_>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        default_measure(env, view, self.stretch_axis(), proposal)
    }

    /// Which axis (or axes) this content stretches to fill available space.
    #[must_use]
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    /// Receives host `Activity` / embedding lifecycle transitions
    /// ([`AndroidLifecycle`]).
    fn on_lifecycle(&mut self, _env: &mut Env<'_>, _view: &JObject<'_>, _event: AndroidLifecycle) {}

    /// The semantic role this content declares for semantic test runtimes
    /// (`SemanticRuntime` / `waterui-testing`).
    #[must_use]
    fn accessibility_role(&self) -> Option<AccessibilityRole> {
        None
    }

    /// The spoken label this content declares for semantic test runtimes
    /// (`SemanticRuntime` / `waterui-testing`).
    #[must_use]
    fn accessibility_label(&self) -> Option<Computed<Str>> {
        None
    }
}

/// Converts a logical dp axis proposal into an Android `View.MeasureSpec`
/// using `AT_MOST` for finite offers and `UNSPECIFIED` otherwise.
#[must_use]
pub(crate) fn proposal_to_measure_spec(offered: Option<f32>, density: f32) -> i32 {
    const MODE_MASK: i32 = 0x3 << 30;
    const AT_MOST: i32 = 0x2 << 30;
    const UNSPECIFIED: i32 = 0;
    match offered {
        Some(dp) if dp.is_finite() => {
            let scale = if density.is_finite() && density > 0.0 {
                density
            } else {
                1.0
            };
            let px = (dp.max(0.0) * scale).round();
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "Android MeasureSpec packs a 30-bit clamped non-negative pixel size"
            )]
            let clamped = (px.min(((1_u32 << 30) - 1) as f32) as i32) & !MODE_MASK;
            clamped | AT_MOST
        }
        _ => UNSPECIFIED,
    }
}

/// Applies Android measured pixel dimensions and `stretch` to produce a finite
/// `ViewDimensions` in logical dp.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "Android measured pixel counts fit f32 logical dp"
)]
pub(crate) fn apply_android_measure(
    measured_width_px: i32,
    measured_height_px: i32,
    density: f32,
    stretch: StretchAxis,
    proposal: ProposalSize,
) -> ViewDimensions {
    let scale = if density.is_finite() && density > 0.0 {
        density
    } else {
        1.0
    };
    let fit_width = (measured_width_px.max(0) as f32) / scale;
    let fit_height = (measured_height_px.max(0) as f32) / scale;
    let width = if stretch.stretches_horizontal() {
        proposal
            .width
            .filter(|offered| offered.is_finite())
            .map_or(fit_width, |offered| offered.max(fit_width))
    } else {
        fit_width
    };
    let height = if stretch.stretches_vertical() {
        proposal
            .height
            .filter(|offered| offered.is_finite())
            .map_or(fit_height, |offered| offered.max(fit_height))
    } else {
        fit_height
    };
    ViewDimensions::new(Size::new(width, height))
}

fn measure_with_jni(
    env: &mut Env<'_>,
    view: &JObject<'_>,
    stretch: StretchAxis,
    proposal: ProposalSize,
) -> jni::errors::Result<ViewDimensions> {
    let resources = env
        .call_method(
            view,
            jni_str!("getResources"),
            jni_sig!("()Landroid/content/res/Resources;"),
            &[],
        )?
        .l()?;
    let metrics = env
        .call_method(
            &resources,
            jni_str!("getDisplayMetrics"),
            jni_sig!("()Landroid/util/DisplayMetrics;"),
            &[],
        )?
        .l()?;
    let density = env
        .get_field(&metrics, jni_str!("density"), jni_sig!("F"))?
        .f()?;
    let width_spec = proposal_to_measure_spec(proposal.width, density);
    let height_spec = proposal_to_measure_spec(proposal.height, density);
    env.call_method(
        view,
        jni_str!("measure"),
        jni_sig!("(II)V"),
        &[
            jni::objects::JValue::Int(width_spec),
            jni::objects::JValue::Int(height_spec),
        ],
    )?;
    let width_px = env
        .call_method(view, jni_str!("getMeasuredWidth"), jni_sig!("()I"), &[])?
        .i()?;
    let height_px = env
        .call_method(view, jni_str!("getMeasuredHeight"), jni_sig!("()I"), &[])?
        .i()?;
    Ok(apply_android_measure(
        width_px, height_px, density, stretch, proposal,
    ))
}

/// Default leaf measurement calling `view.measure(widthSpec, heightSpec)` in
/// physical pixels and converting back to logical dp.
#[must_use]
pub(crate) fn default_measure(
    env: &mut Env<'_>,
    view: &JObject<'_>,
    stretch: StretchAxis,
    proposal: ProposalSize,
) -> ViewDimensions {
    match measure_with_jni(env, view, stretch, proposal) {
        Ok(dimensions) => dimensions,
        Err(error) => {
            if env.exception_check() {
                env.exception_clear();
            }
            tracing::error!(
                target: "waterui::hydrolysis::android",
                %error,
                "AndroidContent::measure failed via JNI; reporting zero dimensions"
            );
            ViewDimensions::new(Size::zero())
        }
    }
}

/// A component Hydrolysis provides on Android to embed [`AndroidContent`]
/// (`android.view.View`) inside a `WaterUI` view tree.
///
/// Generic over `T` so static dispatch through
/// `HydroBackend` is monomorphized without
/// boxing `T`; `T` is boxed only when the caller erases the subtree with
/// [`AnyView`](waterui_core::AnyView). Reaching any other presentation backend
/// reports a contract violation.
#[must_use]
pub struct AndroidView<T: AndroidContent> {
    content: T,
}

impl<T: AndroidContent> fmt::Debug for AndroidView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AndroidView")
            .field("content", &core::any::type_name::<T>())
            .finish()
    }
}

impl<T: AndroidContent> AndroidView<T> {
    /// Wraps `content` in an Android-backend leaf view.
    pub const fn new(content: T) -> Self {
        Self { content }
    }

    /// Consumes the view and returns the inner [`AndroidContent`].
    #[must_use]
    pub fn into_inner(self) -> T {
        self.content
    }

    /// Borrows the inner [`AndroidContent`].
    #[must_use]
    pub const fn as_inner(&self) -> &T {
        &self.content
    }

    /// Mutably borrows the inner [`AndroidContent`].
    #[must_use]
    pub const fn as_inner_mut(&mut self) -> &mut T {
        &mut self.content
    }
}

impl<T: AndroidContent> NativeView for AndroidView<T> {
    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }

    fn realize_provided(self, context: &mut dyn Any, env: &Environment) -> Result<(), Self> {
        #[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
        if let Some(slot) = context.downcast_mut::<AndroidProvidedSlot>() {
            let stretch = self.content.stretch_axis();
            let leaf = realize_android_view(
                self.content,
                env,
                &slot.bridge,
                Rc::clone(&slot.mark_layout),
                Rc::clone(&slot.mark_paint),
            );
            slot.realized = Some((leaf, stretch));
            return Ok(());
        }
        let _ = env;
        if let Some(semantics) = context.downcast_mut::<ProvidedSemantics>() {
            semantics.role = self.content.accessibility_role();
            semantics.label = self.content.accessibility_label();
            return Ok(());
        }
        Err(self)
    }
}

impl<T: AndroidContent> View for AndroidView<T> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }
}

/// Private receiver [`HydroBackend`](crate::renderer::HydroBackend) passes into
/// [`NativeView::realize_provided`] to realize [`AndroidView`] monomorphized on
/// the static path without naming the text engine type parameter `E`.
#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
pub(crate) struct AndroidProvidedSlot {
    pub(crate) bridge: AndroidHostBridge,
    pub(crate) mark_layout: Rc<dyn Fn()>,
    pub(crate) mark_paint: Rc<dyn Fn()>,
    pub(crate) realized: Option<(Option<Rc<dyn AndroidLeafBehavior>>, StretchAxis)>,
}

/// Non-generic widget behavior for one realized [`AndroidView<T>`] leaf,
/// adapted to [`WidgetBehavior<E>`] in [`HydroBackend`](crate::renderer::HydroBackend).
#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
pub(crate) trait AndroidLeafBehavior {
    fn measure_leaf(&self, proposal: ProposalSize) -> ViewDimensions;

    fn placement(&self) -> (&PlatformViewSink, u64);
}

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
pub(crate) struct AndroidWidgetAdapter(pub(crate) Rc<dyn AndroidLeafBehavior>);

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
impl<E: crate::text::TextEngine> WidgetBehavior<E> for AndroidWidgetAdapter {
    fn render(
        self: Rc<Self>,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        _env: &Environment,
        _safe_area: Option<SafeAreaLayout>,
    ) {
        let (sink, id) = self.0.placement();
        renderer.register_platform_view_placement(
            ctx.bounds(),
            Rc::clone(&sink.table),
            PlatformViewPlacement::new(id, PlatformViewSource::Instance { instance: id }),
        );
    }

    fn measure(
        &self,
        _state: &mut HydroState<E>,
        proposal: ProposalSize,
        _env: &Environment,
        _theme: &Rc<dyn WidgetTheme>,
    ) -> ViewDimensions {
        self.0.measure_leaf(proposal)
    }
}

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
struct AndroidLeafState<T: AndroidContent> {
    content: RefCell<T>,
    view: Arc<Global<JObject<'static>>>,
    placement_id: u64,
    sink: PlatformViewSink,
    bridge: AndroidHostBridge,
    lifecycle_token: RefCell<Option<u64>>,
    keepalive: RefCell<Option<KeepAlive>>,
}

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
impl<T: AndroidContent> AndroidLeafBehavior for AndroidLeafState<T> {
    fn measure_leaf(&self, proposal: ProposalSize) -> ViewDimensions {
        let mut measured = None;
        let result = self.bridge.vm().with_top_local_frame(|jni_env| {
            let content = self.content.borrow();
            measured = Some(content.measure(jni_env, self.view.as_obj(), proposal));
            Ok::<_, jni::errors::Error>(())
        });
        if let Err(error) = result {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                %error,
                "AndroidContent::measure failed to attach JNI env"
            );
            return ViewDimensions::new(Size::zero());
        }
        measured.unwrap_or_else(|| ViewDimensions::new(Size::zero()))
    }

    fn placement(&self) -> (&PlatformViewSink, u64) {
        (&self.sink, self.placement_id)
    }
}

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
impl<T: AndroidContent> Drop for AndroidLeafState<T> {
    fn drop(&mut self) {
        if let Some(token) = self.lifecycle_token.borrow_mut().take() {
            self.bridge.unsubscribe_lifecycle(token);
        }
        // Drop reactive guards and auxiliary state in reverse insertion order
        // before calling lifecycle teardown or unregistering the View.
        drop(self.keepalive.borrow_mut().take());

        let (started, resumed) = self.bridge.lifecycle_state();
        let placement_id = self.placement_id;
        let bridge = &self.bridge;
        let view = &self.view;
        let mut content = self.content.borrow_mut();
        let result = bridge.vm().with_top_local_frame(|jni_env| {
            if resumed {
                content.on_lifecycle(jni_env, view.as_obj(), AndroidLifecycle::Pause);
            }
            if started {
                content.on_lifecycle(jni_env, view.as_obj(), AndroidLifecycle::Stop);
            }
            content.on_lifecycle(jni_env, view.as_obj(), AndroidLifecycle::Destroy);
            bridge.unregister_instance(jni_env, placement_id)?;
            Ok::<_, jni::errors::Error>(())
        });
        if let Err(error) = result {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                placement_id,
                %error,
                "AndroidView teardown failed across JNI"
            );
        }
    }
}

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
fn realize_android_view<T: AndroidContent>(
    mut content: T,
    env: &Environment,
    bridge: &AndroidHostBridge,
    mark_layout: Rc<dyn Fn()>,
    mark_paint: Rc<dyn Fn()>,
) -> Option<Rc<dyn AndroidLeafBehavior>> {
    let Some(sink) = env.get::<PlatformViewSink>().cloned() else {
        contract_violation!(
            "hydrolysis android: AndroidView realized without a PlatformViewSink in Environment"
        );
        return None;
    };
    let placement_id = sink.next_id();
    let view_ref: Rc<RefCell<Option<Weak<Global<JObject<'static>>>>>> = Rc::new(RefCell::new(None));
    let invalidator_view = Rc::clone(&view_ref);
    let invalidator_bridge = bridge.clone();
    let invalidator = AndroidInvalidator {
        mark_layout,
        mark_paint,
        host_invalidate: Rc::new(move || {
            let Some(view) = invalidator_view.borrow().as_ref().and_then(Weak::upgrade) else {
                return;
            };
            if let Err(error) = invalidator_bridge.vm().with_top_local_frame(|jni_env| {
                invalidator_bridge.post_invalidate(jni_env, view.as_obj())
            }) {
                tracing::error!(
                    target: "waterui::hydrolysis::android",
                    %error,
                    "AndroidInvalidator::invalidate_display failed"
                );
            }
        }),
    };

    let mut keepalive = KeepAlive::default();
    let mut created: Option<Global<JObject<'static>>> = None;
    let jni_result = bridge.vm().with_top_local_frame(|jni_env| {
        let host_context = bridge.acquire_instance_context(jni_env, placement_id)?;
        let mut mount = AndroidMount {
            env,
            context: &host_context,
            vm: bridge.vm(),
            invalidator,
            keepalive: &mut keepalive,
        };
        let view = content.make_view(jni_env, &mut mount)?;
        bridge.register_instance(jni_env, placement_id, view.as_obj())?;
        created = Some(view);
        Ok::<_, jni::errors::Error>(())
    });

    let view = match (jni_result, created) {
        (Ok(()), Some(view)) => Arc::new(view),
        (Err(error), _) => {
            drop(keepalive);
            let _ = bridge.vm().with_top_local_frame(|jni_env| {
                if jni_env.exception_check() {
                    jni_env.exception_clear();
                }
                bridge.unregister_instance(jni_env, placement_id)
            });
            contract_violation!(
                "hydrolysis android: AndroidContent::make_view failed for {}: {error}",
                core::any::type_name::<T>()
            );
            return None;
        }
        (Ok(()), None) => unreachable!("make_view succeeded without producing a view"),
    };

    *view_ref.borrow_mut() = Some(Arc::downgrade(&view));

    let state = Rc::new(AndroidLeafState {
        content: RefCell::new(content),
        view,
        placement_id,
        sink,
        bridge: bridge.clone(),
        lifecycle_token: RefCell::new(None),
        keepalive: RefCell::new(Some(keepalive)),
    });
    subscribe_android_lifecycle(&state, bridge, placement_id);
    Some(state)
}

#[cfg(any(hydrolysis_gpu_painter, hydrolysis_hwui))]
fn subscribe_android_lifecycle<T: AndroidContent>(
    state: &Rc<AndroidLeafState<T>>,
    bridge: &AndroidHostBridge,
    placement_id: u64,
) {
    let weak_state = Rc::downgrade(state);
    let (token, started, resumed) = bridge.subscribe_lifecycle(Rc::new(move |jni_env, event| {
        let Some(state) = weak_state.upgrade() else {
            return;
        };
        state
            .content
            .borrow_mut()
            .on_lifecycle(jni_env, state.view.as_obj(), event);
    }));
    *state.lifecycle_token.borrow_mut() = Some(token);

    if started || resumed {
        let catchup = bridge.vm().with_top_local_frame(|jni_env| {
            let mut content = state.content.borrow_mut();
            if started {
                content.on_lifecycle(jni_env, state.view.as_obj(), AndroidLifecycle::Start);
            }
            if resumed {
                content.on_lifecycle(jni_env, state.view.as_obj(), AndroidLifecycle::Resume);
            }
            Ok::<_, jni::errors::Error>(())
        });
        if let Err(error) = catchup {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                placement_id,
                %error,
                "failed to deliver initial lifecycle state to AndroidContent"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::binding;
    use waterui_core::backend::ErasedPayload;

    struct DummyContent {
        stretch: StretchAxis,
        role: Option<AccessibilityRole>,
        label: Option<Computed<Str>>,
    }

    impl AndroidContent for DummyContent {
        fn make_view(
            &mut self,
            _env: &mut Env<'_>,
            _mount: &mut AndroidMount<'_>,
        ) -> jni::errors::Result<Global<JObject<'static>>> {
            unreachable!("SemanticRuntime must never mount a platform Android View")
        }

        fn stretch_axis(&self) -> StretchAxis {
            self.stretch
        }

        fn accessibility_role(&self) -> Option<AccessibilityRole> {
            self.role.clone()
        }

        fn accessibility_label(&self) -> Option<Computed<Str>> {
            self.label.clone()
        }
    }

    #[test]
    fn measure_spec_maps_finite_and_infinite_proposals() {
        const MODE_MASK: i32 = 0x3 << 30;
        const AT_MOST: i32 = 0x2 << 30;
        const UNSPECIFIED: i32 = 0;

        let spec = proposal_to_measure_spec(Some(120.0), 2.5);
        assert_eq!(spec & MODE_MASK, AT_MOST);
        assert_eq!(spec & !MODE_MASK, 300);

        assert_eq!(proposal_to_measure_spec(None, 2.5), UNSPECIFIED);
        assert_eq!(
            proposal_to_measure_spec(Some(f32::INFINITY), 2.5),
            UNSPECIFIED
        );
    }

    #[test]
    fn default_measure_respects_stretch_axis() {
        let dims = apply_android_measure(
            320,
            100,
            2.0,
            StretchAxis::Horizontal,
            ProposalSize::new(Some(300.0), Some(120.0)),
        );
        assert_eq!(dims.size, Size::new(300.0, 50.0));

        let unstretched = apply_android_measure(
            320,
            100,
            2.0,
            StretchAxis::None,
            ProposalSize::new(Some(300.0), Some(120.0)),
        );
        assert_eq!(unstretched.size, Size::new(160.0, 50.0));
    }

    #[test]
    fn android_view_realizes_provided_semantics_on_static_and_erased_paths() {
        let label = binding(Str::from_static("Sponsored ad"));
        let env = Environment::new();
        let view = AndroidView::new(DummyContent {
            stretch: StretchAxis::Horizontal,
            role: Some(AccessibilityRole::Container),
            label: Some(label.computed()),
        });
        assert_eq!(view.stretch_axis(), StretchAxis::Horizontal);

        let mut semantics = ProvidedSemantics::default();
        assert!(view.realize_provided(&mut semantics, &env).is_ok());
        assert_eq!(semantics.role, Some(AccessibilityRole::Container));
        assert_eq!(
            semantics
                .label
                .as_ref()
                .map(|computed| nami::Signal::snapshot(computed).as_str().to_owned()),
            Some("Sponsored ad".to_owned())
        );

        let erased = AndroidView::new(DummyContent {
            stretch: StretchAxis::None,
            role: Some(AccessibilityRole::Button),
            label: Some(binding(Str::from_static("Install")).computed()),
        });
        let payload = ErasedPayload::new_native(erased);
        let mut erased_semantics = ProvidedSemantics::default();
        assert!(payload.realize_provided(&mut erased_semantics, &env));
        assert_eq!(erased_semantics.role, Some(AccessibilityRole::Button));
    }

    #[test]
    fn android_view_rejects_unknown_backend_slot() {
        let env = Environment::new();
        let view = AndroidView::new(DummyContent {
            stretch: StretchAxis::None,
            role: None,
            label: None,
        });
        let mut unknown = ();
        assert!(view.realize_provided(&mut unknown, &env).is_err());
    }
}
