//! Authoring-side [`GpuContent`] contract and [`GpuView`] builder.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::any::type_name;
use core::fmt;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering, fence};

use arc_swap::ArcSwapOption;
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal};
use waterui_core::layout::{ProposalSize, Size, ViewDimensions};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, Native, Str, View};

use super::leaf::GpuContentLeaf;
use super::{Context, Frame, RedrawHandle, SurfaceHooks, measure_by_intrinsic_size};
use crate::draw::kurbo;
use crate::input::SurfaceInputEvent;

/// GPU work the engine composites as a layer.
///
/// Content that takes no reactive parameters sets `type Params = ();` and
/// mounts directly as a [`View`] inside [`GpuView`]. Parameterized content sets
/// `type Params = MyParams;`, overrides [`update`](Self::update), and binds a
/// reactive signal through [`GpuView::params`] before mounting as a [`View`].
pub trait GpuContent: Send + 'static {
    /// Parameter type delivered from the UI thread to [`update`](Self::update).
    ///
    /// Set `type Params = ();` for content without reactive parameters.
    type Params: Send + 'static;

    /// Creates pipelines and resources against the engine's device.
    ///
    /// Called on the render thread before the first [`render`](Self::render),
    /// and again on each new device after the GPU runtime replaces a lost one.
    /// Each call replaces every device resource the previous call created:
    /// resources from the old device must not be used after it.
    fn setup(&mut self, gpu: &Context<'_>);

    /// Receives updated parameters delivered from the UI thread before the next
    /// [`render`](Self::render).
    ///
    /// For parameterized content bound through [`GpuView::params`], the runtime
    /// delivers the initial snapshot after [`setup`](Self::setup) and before
    /// the first [`render`](Self::render), re-delivers the latest value after a
    /// device-loss re-setup, and delivers any newer value whenever the bound
    /// signal changes.
    fn update(&mut self, _params: Self::Params) {}

    /// Draws one frame into `frame`'s texture.
    ///
    /// Call [`Frame::request_redraw`] when the content animates and needs
    /// the next frame too.
    fn render(&mut self, frame: &mut Frame<'_>);

    /// Whether every pixel the content draws is opaque, letting the engine
    /// skip blending underneath it.
    fn is_opaque(&self) -> bool {
        false
    }

    /// The size the content is naturally, in logical points; `None` takes
    /// whatever the layout gives.
    ///
    /// Content whose natural size only resolves asynchronously — a decoder
    /// that learns the stream dimensions after setup — leaves this `None` and
    /// answers [`measure`](Self::measure) instead.
    fn intrinsic_size(&self) -> Option<Size> {
        None
    }

    /// Measures the content against a layout proposal.
    ///
    /// The default answers [`intrinsic_size`](Self::intrinsic_size) when it
    /// knows one and fills the proposal otherwise. Override it for content
    /// whose size resolves at runtime: the measurement is read again for every
    /// layout pass, so an asynchronous source reports its size as soon as it
    /// has one — announce the change through [`Context::redraw`].
    ///
    /// This is layout-only and must not touch GPU or render state.
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure_by_intrinsic_size(self.intrinsic_size(), proposal)
    }

    /// Which dynamic range the content prefers its presentation target to
    /// carry.
    ///
    /// - `Some(true)`: prefer an HDR target.
    /// - `Some(false)`: prefer an SDR target.
    /// - `None` — the default — means follow the host's surrounding policy.
    ///
    /// The view's own [`prefer_hdr_surface`](GpuView::prefer_hdr_surface)
    /// overrides this answer.
    fn preferred_surface_hdr(&self) -> Option<bool> {
        None
    }
}

/// Crate-internal dyn-compatible GPU content contract used exclusively for late
/// type erasure inside [`GpuContentLeaf`] and [`GpuContentHandle`](super::GpuContentHandle).
pub trait ErasedGpuContent: Send + 'static {
    fn setup(&mut self, gpu: &Context<'_>);
    fn render(&mut self, frame: &mut Frame<'_>);
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions;
    fn preferred_surface_hdr(&self) -> Option<bool>;
}

pub(super) struct Erase<T>(pub(super) T);

impl<T: GpuContent<Params = ()>> ErasedGpuContent for Erase<T> {
    fn setup(&mut self, gpu: &Context<'_>) {
        self.0.setup(gpu);
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        self.0.render(frame);
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.0.measure(proposal)
    }

    fn preferred_surface_hdr(&self) -> Option<bool> {
        self.0.preferred_surface_hdr()
    }
}

/// Lock-free single-slot mailbox transferring owned `P` values from the UI
/// thread to the render thread without requiring `P: Sync`.
struct AtomicSlot<P: Send + 'static> {
    ptr: AtomicPtr<P>,
}

// SAFETY: `AtomicSlot` never shares `&P` across threads; every stored `Box<P>`
// transfers sole ownership into the slot and either out to the render thread on
// `take` or directly to `drop` when overwritten or when the slot drops.
unsafe impl<P: Send + 'static> Sync for AtomicSlot<P> {}

impl<P: Send + 'static> AtomicSlot<P> {
    fn new(initial: P) -> Self {
        Self {
            ptr: AtomicPtr::new(Box::into_raw(Box::new(initial))),
        }
    }

    fn store(&self, value: P) {
        let next = Box::into_raw(Box::new(value));
        let prev = self.ptr.swap(next, Ordering::AcqRel);
        if !prev.is_null() {
            // SAFETY: `prev` was produced by `Box::into_raw` in `new`/`store`,
            // and swapping it out transfers exclusive ownership here.
            drop(unsafe { Box::from_raw(prev) });
        }
    }

    fn take(&self) -> Option<P> {
        let prev = self.ptr.swap(ptr::null_mut(), Ordering::AcqRel);
        if prev.is_null() {
            None
        } else {
            // SAFETY: `prev` was produced by `Box::into_raw` in `new`/`store`,
            // and swapping it out transfers exclusive ownership here.
            Some(unsafe { *Box::from_raw(prev) })
        }
    }
}

impl<P: Send + 'static> Drop for AtomicSlot<P> {
    fn drop(&mut self) {
        let ptr = *self.ptr.get_mut();
        if !ptr.is_null() {
            // SAFETY: `ptr` was produced by `Box::into_raw` and `&mut self`
            // guarantees exclusive access on drop.
            drop(unsafe { Box::from_raw(ptr) });
        }
    }
}

/// Lock-free parameter channel paired with the render host's wake handle.
struct ParamMailbox<P: Send + 'static> {
    pending: AtomicSlot<P>,
    redraw: ArcSwapOption<RedrawHandle>,
}

impl<P: Send + 'static> ParamMailbox<P> {
    fn new(initial: P) -> Self {
        Self {
            pending: AtomicSlot::new(initial),
            redraw: ArcSwapOption::empty(),
        }
    }

    fn post(&self, value: P) {
        self.pending.store(value);
        // Pair with `set_redraw`'s store + fence + `take_pending` so an update
        // concurrent with `setup` either is observed by `setup` or sees the
        // installed `RedrawHandle` and wakes the host.
        fence(Ordering::SeqCst);
        if let Some(redraw) = self.redraw.load_full() {
            redraw.request_redraw();
        }
    }

    fn set_redraw(&self, redraw: RedrawHandle) {
        self.redraw.store(Some(Arc::new(redraw)));
        fence(Ordering::SeqCst);
    }

    fn take_pending(&self) -> Option<P> {
        self.pending.take()
    }
}

/// Internal adapter wrapping a [`GpuContent`] bound to a parameter signal via
/// [`GpuView::params`].
struct WithParams<T: GpuContent> {
    inner: T,
    mailbox: Arc<ParamMailbox<T::Params>>,
    last_params: Option<T::Params>,
}

impl<T: GpuContent> fmt::Debug for WithParams<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WithParams")
            .field("inner", &type_name::<T>())
            .field("params", &type_name::<T::Params>())
            .finish_non_exhaustive()
    }
}

impl<T> GpuContent for WithParams<T>
where
    T: GpuContent,
    T::Params: Clone,
{
    type Params = ();

    fn setup(&mut self, gpu: &Context<'_>) {
        self.inner.setup(gpu);
        self.mailbox.set_redraw(gpu.redraw().clone());
        if let Some(latest) = self.mailbox.take_pending() {
            self.last_params = Some(latest.clone());
            self.inner.update(latest);
        } else if let Some(last) = self.last_params.clone() {
            self.inner.update(last);
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        if let Some(latest) = self.mailbox.take_pending() {
            self.last_params = Some(latest.clone());
            self.inner.update(latest);
        }
        self.inner.render(frame);
    }

    fn is_opaque(&self) -> bool {
        self.inner.is_opaque()
    }

    fn intrinsic_size(&self) -> Option<Size> {
        self.inner.intrinsic_size()
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.inner.measure(proposal)
    }

    fn preferred_surface_hdr(&self) -> Option<bool> {
        self.inner.preferred_surface_hdr()
    }
}

/// A view whose pixels come from [`GpuContent`].
///
/// Parameterless content (`type Params = ();`) mounts directly as a [`View`].
/// Parameterized content (`type Params = MyParams;`) must bind a reactive
/// signal through [`GpuView::params`] before it implements [`View`]:
///
/// ```compile_fail
/// use waterui_core::View;
/// use waterui_graphics::{Context, Frame, GpuContent, GpuView};
///
/// #[derive(Clone)]
/// struct Tint(f32);
///
/// struct Tinted;
/// impl GpuContent for Tinted {
///     type Params = Tint;
///
///     fn setup(&mut self, _gpu: &Context<'_>) {}
///     fn update(&mut self, _params: Tint) {}
///     fn render(&mut self, _frame: &mut Frame<'_>) {}
/// }
///
/// fn assert_view<V: View>(_view: V) {}
///
/// // Missing `.params(...)`: a parameterized `GpuView` does not implement `View`.
/// assert_view(GpuView::new(Tinted));
/// ```
///
/// # Content lifetime
///
/// The view owns one [`GpuContent`] instance for its whole lifetime, and
/// [`GpuContent::setup`] is where that instance creates its persistent GPU
/// resources. When a parent rebuild tears the view down, the content goes
/// with it and the rebuilt view starts a new instance. Content state does not
/// move into hidden shared caches to survive that.
///
/// # Layout Behavior
///
/// Stretches on both axes unless the content reports an intrinsic size, in
/// which case it is that size and does not stretch.
pub struct GpuView<T: GpuContent> {
    content: T,
    intrinsic_size: Option<Size>,
    opaque: bool,
    surface_prefers_hdr: Option<bool>,
    hooks: SurfaceHooks,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
}

impl<T: GpuContent> fmt::Debug for GpuView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuView")
            .field("content", &type_name::<T>())
            .field("intrinsic_size", &self.intrinsic_size)
            .field("opaque", &self.opaque)
            .finish_non_exhaustive()
    }
}

impl<T: GpuContent> GpuView<T> {
    /// A view drawing `content`.
    #[must_use]
    pub fn new(content: T) -> Self {
        Self {
            intrinsic_size: content.intrinsic_size(),
            opaque: content.is_opaque(),
            content,
            surface_prefers_hdr: None,
            hooks: SurfaceHooks::default(),
            label: Computed::constant(None),
            value: Computed::constant(None),
        }
    }

    /// Receives pointer, keyboard and gesture events the backend routes to
    /// this view.
    #[must_use]
    pub fn on_input(mut self, handler: impl Fn(&SurfaceInputEvent) + 'static) -> Self {
        self.hooks.input = Some(Rc::new(handler));
        self
    }

    /// Runs `hook` on the UI thread once per frame before the content renders.
    #[must_use]
    pub fn on_frame(mut self, hook: impl Fn() + 'static) -> Self {
        self.hooks.frame = Some(Rc::new(hook));
        self
    }

    /// Answers where the content's text caret is, for input-method panels.
    #[must_use]
    pub fn on_ime_caret(mut self, query: impl Fn() -> Option<kurbo::Rect> + 'static) -> Self {
        self.hooks.caret = Some(Rc::new(query));
        self
    }

    /// The accessible name the content offers. An app's own `.a11y_label(…)`
    /// overrides it.
    #[must_use]
    pub fn offered_label(mut self, label: impl IntoComputed<Option<Str>>) -> Self {
        self.label = label.into_computed();
        self
    }

    /// The accessible value the content offers. An app's own `.a11y_value(…)`
    /// overrides it.
    #[must_use]
    pub fn offered_value(mut self, value: impl IntoComputed<Option<Str>>) -> Self {
        self.value = value.into_computed();
        self
    }

    /// Prefer HDR presentation formats for this content even when the
    /// surrounding platform style is SDR.
    ///
    /// This overrides the surrounding platform style for this surface only.
    #[must_use]
    pub const fn prefer_hdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(true);
        self
    }

    /// Prefer SDR presentation formats for this content even when HDR is
    /// available.
    ///
    /// This overrides the surrounding platform style for this surface only.
    #[must_use]
    pub const fn prefer_sdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(false);
        self
    }
}

impl<T> GpuView<T>
where
    T: GpuContent,
    T::Params: Clone,
{
    /// Binds a reactive parameter signal to this GPU content.
    ///
    /// The initial snapshot is delivered to [`GpuContent::update`] after
    /// [`GpuContent::setup`] and before the first [`GpuContent::render`]. When
    /// `params` changes later, the runtime posts the new value lock-free and
    /// wakes an idle host so [`GpuContent::update`] runs before the next frame.
    /// Composes in any order with `.on_frame(...)` and other `GpuView`
    /// modifiers.
    #[must_use]
    pub fn params(
        mut self,
        params: impl IntoComputed<T::Params>,
    ) -> GpuView<impl GpuContent<Params = ()>> {
        let params = params.into_computed();
        let mailbox = Arc::new(ParamMailbox::new(params.snapshot()));
        let watcher_mailbox = Arc::clone(&mailbox);
        let guard: BoxWatcherGuard = Box::new(params.watch(move |ctx| {
            watcher_mailbox.post(ctx.into_value());
        }));
        self.hooks.guards.push(guard);
        GpuView {
            content: WithParams {
                inner: self.content,
                mailbox,
                last_params: None,
            },
            intrinsic_size: self.intrinsic_size,
            opaque: self.opaque,
            surface_prefers_hdr: self.surface_prefers_hdr,
            hooks: self.hooks,
            label: self.label,
            value: self.value,
        }
    }
}

impl<T: GpuContent<Params = ()>> GpuView<T> {
    /// Lowers this authoring view into the backend leaf backends mount.
    pub(crate) fn into_leaf(self) -> GpuContentLeaf {
        GpuContentLeaf::from_parts(
            Box::new(Erase(self.content)),
            self.intrinsic_size,
            self.opaque,
            self.surface_prefers_hdr,
            self.hooks,
            self.label,
            self.value,
        )
    }
}

impl<T: GpuContent<Params = ()>> From<GpuView<T>> for GpuContentLeaf {
    fn from(view: GpuView<T>) -> Self {
        view.into_leaf()
    }
}

impl<T: GpuContent<Params = ()>> View for GpuView<T> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_leaf())
    }
}
