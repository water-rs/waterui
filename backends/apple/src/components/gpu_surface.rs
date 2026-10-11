//! The `gpu_surface` leaf: `Native<GpuContentView>` and
//! `Native<ExternalFrameView>` and `Native<SceneView>` as a kit surface view
//! presenting through a `CAMetalLayer` driven by a `CAMetalDisplayLink` —
//! the platform's own presentation primitive.
//!
//! Cherenkov owns content rendering and composition: a
//! [`GpuContentRenderer`] (`ExternalFrameRenderer` for external frames) draws
//! each frame and composites it into the drawable the link delivers. The
//! link is the only drawable source — `nextDrawable` is never called — and
//! it is unpaused only while the surface is attached, effectively visible,
//! in an active scene and has demand. Surfaces also implement the kit's
//! [`CapturableSurface`] so an enclosing capture (`view_effect`,
//! `applied_filter`) can draw their content into its target.

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use core::fmt;
use core::num::NonZeroU32;

use cocoa_ui::Retained;
use cocoa_ui::metal_presenter::{DrawableFrame, MetalPresenter};
use objc2::MainThreadMarker;
use objc2_metal::{MTLPixelFormat, MTLTexture};
use waterui_core::NativeView;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_graphics::cherenkov::{Display, FrameTime, Next};
use waterui_graphics::draw::kurbo;
use waterui_graphics::gpu::{
    ExternalFrameRenderer, ExternalFrameStream, ExternalFrameView, GpuContentRenderer,
    GpuContentView, GpuRuntime, HostedLayerError, RedrawHandle, SharedGpuContext,
};
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::offscreen::OffscreenSize;
use waterui_graphics::wgpu;

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;
use crate::gpu_runtime::EngineGeneration;
#[cfg(all(feature = "native-test", target_os = "macos"))]
use crate::gpu_runtime::SceneEngine;
use crate::presentation_time::PresentationTime;
use crate::publication_park::PublicationPark;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::surface_view::SurfaceView;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::surface_view::SurfaceView;
}

use platform::SurfaceView;

#[path = "scene_surface.rs"]
mod scene;

/// The semantic half of a mounted GPU surface: what is drawn, measured and
/// fed input, shared by GPU producers, external frames and retained scenes.
trait HostedView {
    /// Measures the view against a layout proposal.
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions;
    /// Which axes the view stretches to fill.
    fn stretch_axis(&self) -> StretchAxis;
    /// The accessibility name.
    fn accessibility_label(&self) -> Option<String>;
    /// The accessibility value.
    fn accessibility_value(&self) -> Option<String>;
    /// Starts content-owned frame sources for this mount.
    fn mount(&mut self, _redraw: &RedrawHandle) {}
    /// Stops content-owned frame sources before releasing its renderer.
    fn unmount(&mut self) {}
    /// Whether the view takes input events.
    fn wants_input_events(&self) -> bool;
    /// The dynamic range the view resolves to, when it declares one.
    fn resolved_hdr_preference(&self) -> Option<bool>;
    /// Routes an input event; only called when the view takes input.
    fn input(&self, event: &SurfaceInputEvent);
    /// The view's text caret, in logical view-local coordinates.
    fn ime_caret(&self) -> Option<kurbo::Rect>;
    /// Runs the view's per-frame UI hook before the engine pass.
    fn before_frame(&self);
    /// Fires the view's invalidation hook, when it has one — the same
    /// notification a content signal runs. Views without one (GPU
    /// content, external frames) invalidate nothing.
    #[cfg_attr(
        not(all(feature = "native-test", target_os = "macos")),
        expect(
            dead_code,
            reason = "only the native-test fixture drives scene invalidation"
        )
    )]
    fn invalidate(&self) {}
    /// Whether the view's declared measurement dependency changed since the
    /// last invalidation it emitted — the `takeMeasurementInvalidation` hook.
    ///
    /// `None` lets the surface keep its proposal/answer baseline check, which
    /// is all a view whose `measure` is an open function of the proposal can
    /// express. A view whose measurement depends on an explicit semantic
    /// input overrides this and answers against that dependency instead: a
    /// delivered proposal that names every axis keeps the under-proposal
    /// answer identical on both sides of the input's change, so the
    /// response-delta baseline can never observe it.
    fn measurement_dependency_invalidated(&self) -> Option<bool> {
        None
    }
    /// Builds the view's engine layer on `context` — the exact generation
    /// the caller is holding for the frame this renderer presents.
    /// `failure_sink` is the owner's routed-failure channel: a shared
    /// engine generation delivers a batch failure to every mounted
    /// participant through it, and the sink enqueues that owner's
    /// `settle_failed` on the main queue.
    ///
    /// # Errors
    ///
    /// [`HostedError`] when the layer or its engine cannot be created.
    fn renderer(
        &mut self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        failure_sink: &Rc<dyn Fn(Arc<HostedLayerError>)>,
    ) -> Result<Box<dyn HostedRenderer>, HostedError>;
}

/// The engine half of a mounted GPU surface: one device generation's layer,
/// rebuilt from the [`HostedView`] after device loss.
trait HostedRenderer {
    /// Renders and composites into the host's texture for the production
    /// target timestamp `target_time`.
    ///
    /// # Errors
    ///
    /// [`HostedError`] when preparation or the frame fails.
    fn present(
        &mut self,
        target: &wgpu::Texture,
        display: Display,
        target_time: FrameTime,
    ) -> Result<Next, HostedError>;

    /// The retained generation evidence a submission's completion checks:
    /// the scene engine's production generation — whose sealed outcome is
    /// immutable once set — or `None` for renderers whose exact validity
    /// is the context generation alone (generic GPU content).
    fn submission_evidence(&self) -> Option<Rc<EngineGeneration>> {
        None
    }

    /// Whether the frame `present` returned for `target_time` wrote the
    /// target. A scene mounted or invalidated after the produced batch
    /// answers `Next::At` without compositing — `false` here marks the
    /// target unwritten, and the external capture defers rather than
    /// stamping untouched pixels as this surface's frame.
    fn wrote_target(&self, _target_time: FrameTime) -> bool {
        true
    }
}

/// What building or presenting a hosted renderer's frame can fail with —
/// the one typed failure channel of the private HostedView/HostedRenderer/
/// frame path. A failure settles the surface through
/// [`settle_failed`]: reported once as a native rendering failure, then
/// the instance stops scheduling until a new context generation
/// legitimately rebinds it — never a same-context retry.
#[derive(Debug)]
enum HostedError {
    /// The shared scene engine generation failed — the generation
    /// owner's owned carrier, so every affected participant and later
    /// mount sees the same typed failure without rerunning it.
    Scene(Arc<HostedLayerError>),
    /// The hosted cherenkov layer failed.
    Layer(HostedLayerError),
}

impl fmt::Display for HostedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(error) => write!(f, "{error}"),
            Self::Layer(error) => write!(f, "{error}"),
        }
    }
}

impl From<Arc<HostedLayerError>> for HostedError {
    fn from(error: Arc<HostedLayerError>) -> Self {
        Self::Scene(error)
    }
}

impl From<HostedLayerError> for HostedError {
    fn from(error: HostedLayerError) -> Self {
        Self::Layer(error)
    }
}

impl HostedView for GpuContentView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        Self::measure(self, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(self)
    }

    fn accessibility_label(&self) -> Option<String> {
        Self::accessibility_label(self).map(str::to_owned)
    }

    fn accessibility_value(&self) -> Option<String> {
        Self::accessibility_value(self).map(str::to_owned)
    }

    fn wants_input_events(&self) -> bool {
        Self::wants_input_events(self)
    }

    fn resolved_hdr_preference(&self) -> Option<bool> {
        Self::resolved_hdr_preference(self)
    }

    fn input(&self, event: &SurfaceInputEvent) {
        Self::input(self, event);
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        Self::ime_caret(self)
    }

    fn before_frame(&self) {
        self.frame();
    }

    fn renderer(
        &mut self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        _failure_sink: &Rc<dyn Fn(Arc<HostedLayerError>)>,
    ) -> Result<Box<dyn HostedRenderer>, HostedError> {
        // `engine_content` answers the same content object every time, so a
        // renderer rebuilt after device loss re-installs it with its state.
        let producer =
            waterui_graphics::cherenkov_gpu::interop::GpuContentBox::new(self.engine_content());
        Ok(Box::new(GpuContentRenderer::new(
            runtime,
            context.clone(),
            producer,
            size,
            redraw.clone(),
        )?))
    }
}

impl HostedRenderer for GpuContentRenderer {
    fn present(
        &mut self,
        target: &wgpu::Texture,
        display: Display,
        target_time: FrameTime,
    ) -> Result<Next, HostedError> {
        Ok(Self::present(self, target, display, target_time)?)
    }
}

impl HostedView for ExternalFrameView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        Self::measure(self, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(self)
    }

    fn accessibility_label(&self) -> Option<String> {
        Self::accessibility_label(self).map(str::to_owned)
    }

    fn accessibility_value(&self) -> Option<String> {
        Self::accessibility_value(self).map(str::to_owned)
    }

    fn wants_input_events(&self) -> bool {
        false
    }

    fn resolved_hdr_preference(&self) -> Option<bool> {
        Self::resolved_hdr_preference(self)
    }

    fn input(&self, _event: &SurfaceInputEvent) {
        unreachable!("an ExternalFrameView takes no input; hosts check wants_input_events first");
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        None
    }

    fn before_frame(&self) {}

    fn renderer(
        &mut self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        _failure_sink: &Rc<dyn Fn(Arc<HostedLayerError>)>,
    ) -> Result<Box<dyn HostedRenderer>, HostedError> {
        let stream: ExternalFrameStream = self.stream();
        Ok(Box::new(ExternalFrameRenderer::new(
            runtime,
            context.clone(),
            &stream,
            size,
            redraw.clone(),
        )?))
    }
}

impl HostedRenderer for ExternalFrameRenderer {
    fn present(
        &mut self,
        target: &wgpu::Texture,
        display: Display,
        target_time: FrameTime,
    ) -> Result<Next, HostedError> {
        Ok(Self::present(self, target, display, target_time)?)
    }
}

/// Everything the leaf owns for one mounted hosted view — the
/// `WuiGpuSurfaceState` equivalent, plus the presentation resources the
/// Swift side split across its own files.
struct SurfaceState {
    /// The environment's GPU runtime; `context()` follows device rebuilds.
    runtime: GpuRuntime,
    /// The environment's shared presentation-time anchor — the display
    /// link's target timestamp maps through it onto the engine's clock.
    presentation_time: Rc<PresentationTime>,
    /// The presentation lifecycle's independent axes: which epoch owns
    /// the drawable path and whether a capture scope routes redraws.
    /// The third axis — a parked wait or settled failure gating
    /// recovery — is `park`: events land in any order, so each axis
    /// reads alone, whatever the order of events.
    presentation: RefCell<Presentation>,
    /// The parked wait on a context publication — `Parked` after device
    /// loss or `Failed` after a typed frame failure. While it stands the
    /// link stays paused and delivered frames drop; the publication's
    /// wake clears it and replays the owed work.
    park: Rc<PublicationPark>,
    /// The hosted semantic view.
    view: RefCell<Box<dyn HostedView>>,
    /// The view's engine layer bound to the context that built it —
    /// context-scope, not attach-scope: it survives detach and external
    /// rendering, and the pair is replaced whole whenever the runtime
    /// publishes a new context. Owning the context beside the renderer
    /// is the binding — a stale pair is dropped, never re-derived.
    bound: RefCell<Option<BoundRenderer>>,
    /// Self-reference for the failure sink a bound renderer captures —
    /// the only way a route inside `produce` can reach this owner
    /// without borrowing presentation state mid-render.
    weak_self: Weak<Self>,
    /// Physical size the presentation buffers are configured at.
    current_width: Cell<u32>,
    /// Physical size the presentation buffers are configured at.
    current_height: Cell<u32>,
    /// Redraw handle the content and external producers wake the host
    /// through.
    redraw_handle: RedrawHandle,
    /// Whether the hosted view takes its own input — captured once at
    /// creation, since `wants_input_events` is a registration-time question.
    wants_input_events: bool,
    /// Whether the content asked for a frame since the last render —
    /// content-scope like `bound`: set by any redraw wake, cleared by
    /// the render that answers it, alive across presentation transitions.
    dirty: Cell<bool>,
    /// Outstanding capture-suppression scopes — the presentation layer is
    /// hidden while nonzero (`beginCaptureSuppression`). The layer is the
    /// view's, not the presentation state's, so this stays a plain field.
    capture_suppression: Cell<u32>,
    /// The explicit HDR preference `resolved_hdr_preference` produced.
    explicit_range: Option<cocoa_ui::dynamic_range::DynamicRange>,
    /// `rendererDynamicRange`'s latch: wgpu keeps the negotiated format
    /// across attach cycles, so the first answer stands.
    latched_renderer_range: Cell<Option<cocoa_ui::dynamic_range::DynamicRange>>,
    /// The presentation range `applyDynamicRange` was last run with.
    configured_range: Cell<Option<cocoa_ui::dynamic_range::DynamicRange>>,
    /// The format the presented surfaces carry — `configureDynamicRange`'s
    /// answer; `capturePixelFormat` reads it.
    presentation_format: Cell<Option<MTLPixelFormat>>,
    /// The scale the current bounds were last resolved at.
    current_scale: Cell<f64>,
    /// Wakers of tasks waiting on the first presented frame.
    ready_waiters: RefCell<Vec<std::task::Waker>>,
    /// Whether content accessibility republishes after the next frame.
    needs_a11y_refresh: Cell<bool>,
    /// Window/app observers re-arming presentation edges.
    observers: RefCell<Vec<cocoa_ui::notification::NotificationObserver>>,
    /// The proposal the surface was last measured under.
    last_proposal: Cell<Option<ProposalSize>>,
    /// The last measurement the renderer answered — reused while setup
    /// owns the semantic renderer (`deferredMeasurementInvalidation`).
    last_resolved_size: RefCell<Option<Size>>,
}

/// The presentation lifecycle: the drawable-path epoch and the open
/// external-render scope. Two independent facts about one surface —
/// events land in any order (a failure during a capture, a publication
/// during a capture, a capture over a parked wait), so each axis is
/// read and written on its own; the wait itself is `SurfaceState::park`.
struct Presentation {
    /// Who owns the drawable path right now.
    epoch: Epoch,
    /// Capture-owned rendering: the enclosing capture's redraw and the
    /// open scopes. Independent of the epoch — a mid-capture settle
    /// lands on the park without disturbing the scope, and the scope's
    /// balance check never inspects a hold.
    capture: Option<External>,
}

/// The drawable-path epoch.
enum Epoch {
    /// Nothing to present into: no window, or a window that cannot
    /// present. No presenter, no link, no drawable pool.
    Detached,
    /// Windowed and bound to a context generation — the display link
    /// issues drawables the frame path renders and presents.
    Attached(Attached),
}

/// The windowed state: the `CAMetalLayer` presenter whose link issues
/// drawables, the owned context generation it is bound to, and the work
/// the link must answer.
struct Attached {
    /// The presenter driving this window's drawables — its destruction
    /// invalidates the link, so the epoch IS the attach flag.
    presenter: MetalPresenter,
    /// The context generation this attach is bound to. Owning the
    /// context is the generation binding — `ensure_presenter` swaps in
    /// a newer publication and rebinds the presenter's device.
    context: Arc<SharedGpuContext>,
    /// What the link's next update must answer this epoch.
    demand: Demand,
}

/// One device generation's renderer bound to the context that built it
/// — the pair is the binding: the renderer can never disagree with the
/// context it names, and a published newer context replaces the pair
/// whole rather than re-binding by a generation counter.
struct BoundRenderer {
    /// The exact context generation `renderer` was created on — its
    /// device, queue and generation are what the renderer presents
    /// through.
    context: Arc<SharedGpuContext>,
    /// The hosted renderer built on `context`.
    renderer: Box<dyn HostedRenderer>,
}

/// The demand bits an attach epoch owns — everything the link's next
/// update answers, reset by detach because the epoch dies with it.
struct Demand {
    /// A frame asked for while one could not be drawn — owed, not
    /// dropped; the next link update answers it.
    frame_owed: Cell<bool>,
    /// The attach-time first frame a window's reveal waits on: it draws
    /// through the visibility gate exactly once so a window ordered at
    /// alpha 0 still receives its first presented frame. Cleared by the
    /// `PresentedFrame` receipt.
    first_paint_owed: Cell<bool>,
    /// Continuous render demand — `keepRedrawing`.
    keep_redrawing: Cell<bool>,
    /// Whether one on-screen frame landed this attach epoch —
    /// first-paint readiness credits only a real `PresentedFrame`.
    presented_once: Cell<bool>,
}

impl Demand {
    /// An attach epoch fresh from `attach`: the first frame is owed.
    const fn new(first_paint: bool) -> Self {
        Self {
            frame_owed: Cell::new(true),
            first_paint_owed: Cell::new(first_paint),
            keep_redrawing: Cell::new(false),
            presented_once: Cell::new(false),
        }
    }

    /// Whether the link must run at all — any of the demand kinds.
    const fn any(&self, dirty: bool) -> bool {
        self.keep_redrawing.get() || self.frame_owed.get() || self.first_paint_owed.get() || dirty
    }
}

/// Capture-owned rendering: `redraw` receives this surface's redraw
/// requests while a scope is open, and `depth` counts the open scopes —
/// nesting is legal.
struct External {
    /// The enclosing capture's redraw target.
    redraw: Rc<dyn Fn()>,
    /// Outstanding external-rendering scopes.
    depth: NonZeroU32,
}

impl Presentation {
    /// A surface with no presentation at all — the mount-time state and
    /// the teardown write.
    const fn detached() -> Self {
        Self {
            epoch: Epoch::Detached,
            capture: None,
        }
    }

    /// The attach resources while the epoch is `Attached`.
    const fn attached(&self) -> Option<&Attached> {
        match &self.epoch {
            Epoch::Attached(attached) => Some(attached),
            Epoch::Detached => None,
        }
    }

    /// The mutable counterpart of [`Presentation::attached`].
    const fn attached_mut(&mut self) -> Option<&mut Attached> {
        match &mut self.epoch {
            Epoch::Attached(attached) => Some(attached),
            Epoch::Detached => None,
        }
    }

    /// The enclosing capture's redraw contract while a scope is open.
    const fn external(&self) -> Option<&External> {
        self.capture.as_ref()
    }

    /// Ends the attach epoch: the presenter, link and demand drop back
    /// to `Detached`; an open capture scope and a parked wait are
    /// untouched.
    fn detach(&mut self) {
        self.epoch = Epoch::Detached;
    }
}

impl core::fmt::Debug for SurfaceState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SurfaceState")
            .field("current_width", &self.current_width)
            .field("current_height", &self.current_height)
            .finish_non_exhaustive()
    }
}

impl SurfaceState {
    /// Builds the state inside `Rc::new_cyclic`: the display-link clock's
    /// frame callback needs the `Weak` this construction produces.
    fn new(
        weak: &Weak<Self>,
        runtime: GpuRuntime,
        view: Box<dyn HostedView>,
        platform_view: &Retained<SurfaceView>,
        mtm: cocoa_ui::MainThreadMarker,
        env: &waterui_backend_core::Environment,
    ) -> Self {
        let wants_input_events = view.wants_input_events();
        let explicit_range = view.resolved_hdr_preference().map(|high| {
            if high {
                cocoa_ui::dynamic_range::DynamicRange::High
            } else {
                cocoa_ui::dynamic_range::DynamicRange::Standard
            }
        });
        // The redraw waker: external capture intercepts while it owns
        // rendering; otherwise the request is handled on the main queue.
        // The handle fires on arbitrary producer threads and its last
        // drop can land on the cherenkov render thread, so the weak and
        // the view travel in a `MainQueueOwned` — the clone and upgrade
        // happen only once the work item lands on the main queue, and an
        // off-main release enqueues the payload's drop instead of
        // blocking on `exec_sync`.
        let redraw_weak = crate::main_queue_owned::shared(weak.clone(), mtm);
        let redraw_view = crate::main_queue_owned::shared(platform_view.clone(), mtm);
        let redraw_handle = RedrawHandle::new(move || {
            let weak = Arc::clone(&redraw_weak);
            let view = Arc::clone(&redraw_view);
            cocoa_ui::main_queue::enqueue(move |mtm| {
                if let Some(state) = weak.get(mtm).upgrade() {
                    handle_redraw_request(&state, view.get(mtm));
                }
            });
        });
        Self {
            runtime,
            presentation_time: PresentationTime::get(env),
            presentation: RefCell::new(Presentation::detached()),
            park: Rc::new(PublicationPark::new()),
            view: RefCell::new(view),
            bound: RefCell::new(None),
            weak_self: weak.clone(),
            current_width: Cell::new(0),
            current_height: Cell::new(0),
            redraw_handle,
            wants_input_events,
            dirty: Cell::new(false),
            capture_suppression: Cell::new(0),
            explicit_range,
            latched_renderer_range: Cell::new(None),
            configured_range: Cell::new(None),
            presentation_format: Cell::new(None),
            current_scale: Cell::new(1.0),
            ready_waiters: RefCell::new(Vec::new()),
            needs_a11y_refresh: Cell::new(true),
            observers: RefCell::new(Vec::new()),
            last_proposal: Cell::new(None),
            last_resolved_size: RefCell::new(None),
        }
    }

    /// Every target — drawable or external capture — is allocated in the
    /// surface's declared presentation format.
    fn assert_declared_format(&self, pixel_format: MTLPixelFormat) {
        assert_eq!(
            pixel_format,
            state_format(self),
            "GpuSurface target must be in the surface's declared presentation format"
        );
    }

    /// Renders through the retained engine and presents its composed texture
    /// — the ffi `render_into`.
    ///
    /// The renderer — engine, surface and the view's layer — is built lazily
    /// on the first frame and rebuilt from the view whenever the runtime's
    /// context generation moves on.
    ///
    /// # Errors
    ///
    /// [`HostedError`] when creating the renderer or presenting the frame
    /// fails.
    fn render_into(
        &self,
        view: &Retained<SurfaceView>,
        context: &Arc<SharedGpuContext>,
        texture: &wgpu::Texture,
        display: Display,
        target_time: FrameTime,
    ) -> Result<(bool, bool, Option<Rc<EngineGeneration>>), HostedError> {
        self.dirty.set(false);
        self.view.borrow().before_frame();
        // A renderer bound to a context generation that has since been lost
        // and rebuilt holds a dead device; the pair is replaced whole on
        // `context`, the generation this frame is rendering under — the
        // stored `context` is the binding, no second counter compares.
        let mut slot = self.bound.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|bound| !Arc::ptr_eq(&bound.context, context))
        {
            *slot = None;
        }
        if slot.is_none() {
            let size = OffscreenSize::try_from_pixels(texture.width(), texture.height())
                .expect("a wgpu texture has a non-zero extent");
            *slot = Some(BoundRenderer {
                context: context.clone(),
                renderer: self.view.borrow_mut().renderer(
                    &self.runtime,
                    context,
                    &self.redraw_handle,
                    size,
                    &self.failure_sink(view, context.generation()),
                )?,
            });
        }
        let bound = slot.as_mut().expect("the bound slot was filled above");
        let submitted = bound.renderer.present(texture, display, target_time)?;
        // The submission retains the generation evidence its completion
        // checks: on the scene path the production generation itself —
        // sealed outcome immutable — so a late completion can never be
        // told productive readiness by a mutable owner flag alone.
        let scene_generation = bound.renderer.submission_evidence();
        let wrote_target = bound.renderer.wrote_target(target_time);
        Ok((
            submitted != Next::Idle || self.dirty.get(),
            wrote_target,
            scene_generation,
        ))
    }

    /// The routed-failure channel a bound renderer captures at creation:
    /// a shared engine generation delivers a batch failure to every
    /// mounted participant, and the sink enqueues the owner's own settle
    /// on the main queue — `settle_failed` keyed on the generation bound
    /// at sink creation, so a settle that already landed for it is a
    /// no-op.
    fn failure_sink(
        &self,
        view: &Retained<SurfaceView>,
        generation: u64,
    ) -> Rc<dyn Fn(Arc<HostedLayerError>)> {
        let weak = self.weak_self.clone();
        let view = objc2::rc::Weak::new(&**view);
        Rc::new(move |failure: Arc<HostedLayerError>| {
            let weak = weak.clone();
            let view = view.clone();
            // The sink itself runs where `produce`/`prepare` call it —
            // the main thread — so the settle hops through the local
            // queue and needs no `Send` wrapper.
            let mtm =
                MainThreadMarker::new().expect("the routed-failure sink runs on the main thread");
            cocoa_ui::main_queue::enqueue_local(mtm, move |_mtm| {
                let (Some(state), Some(view)) = (weak.upgrade(), view.load()) else {
                    return;
                };
                settle_failed(&state, &view, generation, HostedError::Scene(failure));
            });
        })
    }
}

/// The wgpu texture format an `MTLTexture` imports at — the ffi
/// `metal_texture_format`.
fn metal_texture_format(
    texture: &objc2::runtime::ProtocolObject<dyn MTLTexture>,
) -> wgpu::TextureFormat {
    match texture.pixelFormat() {
        MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => panic!("GpuSurface external Metal texture has unsupported format {other:?}"),
    }
}

/// The display's HDR headroom for the view's current screen — `1.0` on an
/// SDR target.
fn display_headroom(format: wgpu::TextureFormat, view: &Retained<SurfaceView>) -> f32 {
    if format != wgpu::TextureFormat::Rgba16Float {
        return 1.0;
    }
    let Some(window) = cocoa_ui::view::window(view.as_platform_view()) else {
        return 1.0;
    };
    #[cfg(target_os = "macos")]
    {
        window.screen().map_or(1.0, |screen| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "EDR headroom is a small display constant"
            )]
            let headroom = screen.maximumExtendedDynamicRangeColorComponentValue() as f32;
            headroom
        })
    }
    #[cfg(target_os = "ios")]
    {
        window.windowScene().map_or(1.0, |scene| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "EDR headroom is a small display constant"
            )]
            let headroom = scene.screen().currentEDRHeadroom() as f32;
            headroom
        })
    }
}

/// What one call to [`render_to_metal_texture`] did with the frame.
enum FrameRender {
    /// The context's device was already reported lost, so no submission was
    /// made — nothing may touch the dead device, and the caller must not
    /// latch a frame or present a ring slot nothing was rendered into.
    PendingRebuild,
    /// The frame's work was submitted on the context the caller passed in;
    /// that exact context owns the completion marker and callback.
    Submitted {
        /// Whether another frame should be scheduled.
        needs_redraw: bool,
        /// Whether `present` wrote the target — `false` when a scene
        /// mounted or invalidated after the produced batch answered
        /// `Next::At` without compositing.
        wrote_target: bool,
        /// The production generation's own sealed-outcome evidence,
        /// retained until the completion runs: a scene batch's
        /// [`EngineGeneration`] — `is_failed` is immutable once set — or
        /// `None`, when the submission's validity is its exact context
        /// generation alone.
        scene_generation: Option<Rc<EngineGeneration>>,
    },
}

/// Imports an `MTLTexture` as a wgpu texture, renders a frame into it and
/// submits; reports whether the frame submitted or the context is dead — the
/// `waterui_gpu_content_render_to_metal_texture` half of the ffi entry
/// point.
#[expect(
    clippy::too_many_arguments,
    reason = "the ffi render entry takes the whole frame contract directly"
)]
fn render_to_metal_texture(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    context: &Arc<SharedGpuContext>,
    metal_texture: Retained<objc2::runtime::ProtocolObject<dyn MTLTexture>>,
    width: u32,
    height: u32,
    scale: f64,
    target_time: FrameTime,
) -> Result<FrameRender, HostedError> {
    state.assert_declared_format(metal_texture.pixelFormat());
    let format = metal_texture_format(&metal_texture);
    if context.device_lost_reason().is_some() {
        return Ok(FrameRender::PendingRebuild);
    }
    // SAFETY: these presentation buffers belong to this device and are
    // handed over as color attachments after their preceding frame completed.
    let wgpu_texture = unsafe {
        cocoa_ui::metal::import_texture(
            context.device(),
            metal_texture,
            format,
            width,
            height,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            wgpu::TextureUses::COLOR_TARGET,
            "GpuSurface Imported Metal Texture",
        )
    };
    let display = Display {
        scale,
        headroom: display_headroom(format, view),
    };
    // The completion marker submitted by the caller orders the frame's work
    // ahead of the callback that presents it.
    let (needs_redraw, wrote_target, scene_generation) =
        state.render_into(view, context, &wgpu_texture, display, target_time)?;
    Ok(FrameRender::Submitted {
        needs_redraw,
        wrote_target,
        scene_generation,
    })
}

/// The external arm's answer to a typed render failure: the failure
/// settles the surface like any frame-path failure — then the capture's
/// completion answers the terminal outcome the `Failed` hold now
/// carries, not a deferral that would wait forever for a redraw no
/// failed context can issue.
fn fail_external_frame(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    context: &Arc<SharedGpuContext>,
    error: HostedError,
    completion: cocoa_ui::capture::SurfaceCaptureCompletion,
) {
    settle_failed(state, view, context.generation(), error);
    completion(Err(state.park.capture_error()));
}

/// The frame just examined carries no pixels for this surface — mark it
/// owed so the demand loop re-renders it on the next scheduled pass.
/// Scheduling the replay is the caller's job: a capture redraw, a
/// publication wake or `update_presentation_demand` each owns a channel.
fn owe_frame(state: &Rc<SurfaceState>) {
    if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
        attached.demand.frame_owed.set(true);
    }
}

/// Owe the frame and re-evaluate the surface's scheduling — the arm's own
/// replay channel when no capture scope, park or publication wake already
/// owns it.
fn owe_and_reschedule(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    owe_frame(state);
    update_presentation_demand(state, view);
}

/// The external arm's answer to a `present` that returned `Next::At`
/// because the scene mounted or invalidated after the produced batch:
/// the target carries no pixels, so the frame stays owed through the
/// surface's own demand — the next production re-renders it — and the
/// caller's completion defers rather than stamping unwritten content as
/// this surface's frame. The replay runs through a redraw request:
/// `update_presentation_demand` is a no-op while the capture scope owns
/// the surface's work, while `handle_redraw_request` fires the
/// enclosing capture's `redraw` and re-renders the deferred frame.
fn defer_unwritten_external_frame(state: &Rc<SurfaceState>) {
    owe_frame(state);
    state.redraw_handle.request_redraw();
}

/// The frame-path answer to a `PendingRebuild`: the lost context never
/// receives work again — the frame stays owed while the surface parks
/// until the runtime publishes the rebuilt context, whose wake replays
/// the owed work through `update_presentation_demand` — the owed path
/// schedules an on-demand render even with the clock stopped.
fn park_unrenderable_frame(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    generation: u64,
) {
    owe_frame(state);
    park_for_publication(state, view, generation);
}

/// The onscreen arm's answer to a `present` that returned `Next::At`:
/// the target carries no pixels — the drawable drops without submitting
/// or presenting — and the frame stays owed through the surface's own
/// demand, mirroring the external path's [`defer_unwritten_external_frame`].
fn defer_unwritten_drawable_frame(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    owe_and_reschedule(state, view);
}

// MARK: - Presentation lifecycle (WuiGpuSurface + WuiSurfacePresentation)

/// The renderer's latched target range — `rendererDynamicRange`.
fn renderer_dynamic_range(
    state: &SurfaceState,
    presentation: cocoa_ui::dynamic_range::DynamicRange,
) -> cocoa_ui::dynamic_range::DynamicRange {
    if let Some(latched) = state.latched_renderer_range.get() {
        return latched;
    }
    let mode = state.explicit_range.unwrap_or(presentation);
    state.latched_renderer_range.set(Some(mode));
    mode
}

/// Applies `presentation` to the view and settles the pixel format the
/// frames are rendered and composited in — `configureDynamicRange`.
fn configure_dynamic_range(
    state: &SurfaceState,
    view: &Retained<SurfaceView>,
    presentation: cocoa_ui::dynamic_range::DynamicRange,
    renderer: cocoa_ui::dynamic_range::DynamicRange,
) {
    if state.configured_range.get() == Some(presentation) {
        return;
    }
    debug_assert!(
        state.presentation.borrow().attached().is_none(),
        "GpuSurface dynamic range cannot change while attached"
    );
    debug_assert!(
        presentation == cocoa_ui::dynamic_range::DynamicRange::Standard
            || renderer == cocoa_ui::dynamic_range::DynamicRange::High,
        "an HDR presentation requires an HDR-capable renderer target"
    );
    cocoa_ui::dynamic_range::apply_to_view(presentation, view.as_platform_view());
    let format = match renderer {
        cocoa_ui::dynamic_range::DynamicRange::High => MTLPixelFormat::RGBA16Float,
        cocoa_ui::dynamic_range::DynamicRange::Standard => MTLPixelFormat::BGRA8Unorm_sRGB,
    };
    // The view's persistent `CAMetalLayer` carries the presentation
    // contract — the negotiated pixel format, the surface colour space and
    // the EDR flag — whether or not a presenter is attached; every presenter
    // is built over this layer, so they change only while detached.
    let layer = view.presentation_layer();
    layer.setPixelFormat(format);
    let colorspace = cocoa_ui::metal::color_space(format);
    layer.setColorspace(Some(&colorspace));
    layer.setWantsExtendedDynamicRangeContent(
        presentation == cocoa_ui::dynamic_range::DynamicRange::High,
    );
    state.presentation_format.set(Some(format));
    state.configured_range.set(Some(presentation));
}

/// Whether this surface's window can put a frame in front of someone —
/// `canPresentNow`.
fn can_present_now(view: &Retained<SurfaceView>) -> bool {
    let Some(window) = cocoa_ui::view::window(view.as_platform_view()) else {
        return false;
    };
    #[cfg(target_os = "macos")]
    {
        if window.isMiniaturized() {
            return false;
        }
        cocoa_ui::appkit::is_visible(&window)
    }
    #[cfg(target_os = "ios")]
    {
        let _ = window;
        cocoa_ui::uikit::application_is_active()
    }
}

/// Whether this surface takes part in its window's first-paint readiness —
/// `participatesInFirstPaintReady`. A view whose window cannot present has
/// no first frame to wait for and owes none: hidden, zero-alpha or
/// degenerate geometry on the view itself, an invisible ancestor (a hidden
/// or alpha-0 parent — distinct from a *window* at alpha 0, which a reveal
/// still presents through), or no presentable window.
fn participates_in_first_paint(view: &Retained<SurfaceView>) -> bool {
    let platform = view.as_platform_view();
    let bounds = cocoa_ui::view::bounds(platform);
    cocoa_ui::view::window(platform).is_some()
        && !cocoa_ui::view::is_hidden(platform)
        && cocoa_ui::view::alpha(platform) > 0.01
        && bounds.size.width > 0.5
        && bounds.size.height > 0.5
        && cocoa_ui::view::has_visible_ancestry(&cocoa_ui::view::retain_base(view))
        && can_present_now(view)
}

/// Whether the surface draws — `isEffectivelyVisible`: narrower than
/// `can_present_now` on the states that announce when they clear.
fn is_effectively_visible(view: &Retained<SurfaceView>) -> bool {
    let Some(window) = cocoa_ui::view::window(view.as_platform_view()) else {
        return false;
    };
    if !cocoa_ui::view::has_visible_ancestry(&cocoa_ui::view::retain_base(view)) {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        // A window that is on no display cannot present; gating here keeps
        // an offscreen window from rendering frames nobody sees.
        if window.screen().is_none() {
            return false;
        }
        if window.isMiniaturized() {
            return false;
        }
        cocoa_ui::appkit::is_visible(&window)
    }
    #[cfg(target_os = "ios")]
    {
        let _ = window;
        cocoa_ui::uikit::application_is_active()
    }
}

/// Positions the presentation layer and tells Core Animation the frames are
/// already at device-pixel size — `updatePresentationFrame`.
fn update_presentation_frame(state: &SurfaceState, view: &Retained<SurfaceView>) {
    let layer = view.presentation_layer();
    let bounds = view.bounds_size();
    cocoa_ui::core_animation::without_animation(|| {
        cocoa_ui::core_animation::set_frame(
            &layer,
            cocoa_ui::Rect::new(0.0, 0.0, bounds.width, bounds.height),
        );
        cocoa_ui::core_animation::set_contents_scale(&layer, state.current_scale.get());
    });
}

/// Geometry + deferred allocation — `initializeGpuIfNeeded`.
fn initialize_gpu(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    let bounds = view.bounds_size();
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        // A genuinely empty view cannot present at all: the live epoch
        // drops rather than carry a zero size beside a configured
        // drawable path — restored bounds re-enter through `attach`
        // below.
        state.current_width.set(0);
        state.current_height.set(0);
        detach_if_attached(state);
        return;
    }
    let Some(scale) = view.backing_scale() else {
        return;
    };
    if !prepare_presentation(state, view, scale) {
        return;
    }
    // The presenter is created where a frame could actually be shown:
    // laying out a covered window bought a link and drawables for frames
    // that never came. A `Detached` epoch under an open capture scope or
    // a parked wait does not re-enter through this door: the scope's or
    // the wait's own contract resumes it.
    if !can_present_now(view) {
        return;
    }
    {
        let slot = state.presentation.borrow();
        if !matches!(slot.epoch, Epoch::Detached) || slot.capture.is_some() || state.park.is_held()
        {
            return;
        }
    }
    attach(state, view);
}

/// The geometry and dynamic-range half of `initialize_gpu` — the part the
/// capture drive shares. `scale` is the scale the presentation renders at:
/// the window's backing scale on screen, the capture's off it.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the zero-bound guard keeps bounds positive, and a window's pixel size at its scale fits u32"
)]
fn prepare_presentation(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    scale: f64,
) -> bool {
    let bounds = view.bounds_size();
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return false;
    }

    let requested = state
        .explicit_range
        .unwrap_or_else(|| cocoa_ui::dynamic_range::require_inherited(view.as_platform_view()));
    let renderer = renderer_dynamic_range(state, requested);
    // An SDR renderer target has no extended range to present, so a surface
    // that latched SDR stays SDR even once its window reaches an HDR display.
    let presentation = if renderer == cocoa_ui::dynamic_range::DynamicRange::Standard {
        cocoa_ui::dynamic_range::DynamicRange::Standard
    } else {
        requested
    };
    if state.configured_range.get() != Some(presentation) {
        detach_if_attached(state);
        configure_dynamic_range(state, view, presentation, renderer);
    }

    state.current_scale.set(scale);
    // Device pixels round up: a positive sub-pixel bound still covers a
    // real pixel row, never a truncated zero.
    let width = (bounds.width * scale).ceil() as u32;
    let height = (bounds.height * scale).ceil() as u32;
    let size_changed = state.current_width.get() != width || state.current_height.get() != height;
    state.current_width.set(width);
    state.current_height.set(height);
    if size_changed && let Some(attached) = state.presentation.borrow_mut().attached_mut() {
        attached.demand.keep_redrawing.set(true);
    }
    update_presentation_frame(state, view);
    if let Some(attached) = state.presentation.borrow().attached() {
        attached
            .presenter
            .set_drawable_size(cocoa_ui::metal_presenter::drawable_size(width, height));
    }
    true
}

/// Builds the `CAMetalLayer` presenter on the current context and owes the
/// first frame — the link's next update answers it.
fn attach(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    debug_assert!(
        matches!(
            *state.presentation.borrow(),
            Presentation {
                epoch: Epoch::Detached,
                capture: None,
            }
        ) && !state.park.is_held(),
        "attach only ever opens on a free surface"
    );
    let layer = view.presentation_layer();
    let weak = Rc::downgrade(state);
    let frame_view = view.clone();
    let on_frame = Rc::new(move |frame| {
        if let Some(state) = weak.upgrade() {
            render_drawable(&state, &frame_view, frame);
        }
    });
    let presenter = MetalPresenter::new(layer, on_frame);
    let context = state.runtime.context();
    let device = crate::gpu_runtime::raw_metal_device(&context);
    presenter.set_device(&device);
    presenter.set_drawable_size(cocoa_ui::metal_presenter::drawable_size(
        state.current_width.get(),
        state.current_height.get(),
    ));
    if let Some(maximum) = crate::gpu_runtime::display_rate(view.as_platform_view()) {
        presenter.set_display_rate(maximum);
    }
    // The first update answers the owed first frame — the window's reveal
    // waits on it. The owed first paint draws through the visibility gate
    // exactly once so an alpha-0 reveal window still gets its frame; only
    // a first-paint participant owes the reveal: a surface mounted behind
    // hidden or zero-alpha ancestors parks until it is shown.
    let first_paint = participates_in_first_paint(view);
    state.presentation.borrow_mut().epoch = Epoch::Attached(Attached {
        presenter,
        context,
        demand: Demand::new(first_paint),
    });
    update_presentation_demand(state, view);
}

/// Detaches the presenter for a window or range change: dropping the
/// `Attached` state releases the link and the demand epoch with it —
/// readiness re-arms and the next attach forces one frame through the
/// gates again. A stale frame's lease release can never reach the next
/// presenter's pool.
fn detach_if_attached(state: &SurfaceState) {
    state.presentation.borrow_mut().detach();
}

// MARK: - Frame scheduling (WuiDisplayLinkDriver + WuiRedrawCallback)

/// Re-runs `initialize_gpu` once a frame could be shown again, then drives
/// the clock and replays an owed frame — `updateDisplayLinkState`.
fn update_presentation_demand(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    {
        let slot = state.presentation.borrow();
        if slot.capture.is_some() || state.park.is_held() {
            // A capture scope or a parked wait owns the surface's work
            // right now — the attach door and the link demand are theirs
            // to reopen, never this call's.
            return;
        }
    }
    if matches!(state.presentation.borrow().epoch, Epoch::Detached) && can_present_now(view) {
        initialize_gpu(state, view);
    }
    // Demand is `keepRedrawing`, an owed frame or a dirty redraw — the link
    // unpauses only while all visibility gates hold and answers the demand
    // at the next drawable delivery, at most one display interval later.
    let slot = state.presentation.borrow();
    if let Epoch::Attached(attached) = &slot.epoch {
        let demand = state.runtime.context().device_lost_reason().is_none()
            && (is_effectively_visible(view) || attached.demand.first_paint_owed.get())
            && attached.demand.any(state.dirty.get());
        attached.presenter.set_paused(!demand);
    }
}

/// Rebinds the presenter to a newly published context generation even
/// when the underlying Metal device remains the same.
fn ensure_presenter(state: &SurfaceState, context: &Arc<SharedGpuContext>) {
    let mut slot = state.presentation.borrow_mut();
    let Some(attached) = slot.attached_mut() else {
        return;
    };
    if Arc::ptr_eq(&attached.context, context) {
        return;
    }
    // Context replacement keeps the layer and the link; the device swap
    // advances the presenter generation so frames from the old context
    // can never present.
    let device = crate::gpu_runtime::raw_metal_device(context);
    attached.presenter.set_device(&device);
    attached.context = context.clone();
}

/// The publication wake's replay — the `context_after` wait's wake runs
/// it once it clears the hold: a new context generation is the one
/// legitimate recovery, never a retry of the same frame on the
/// generation that failed. An enclosing capture is asked for a fresh
/// frame through its redraw contract, a windowed surface gets the owed
/// frame back as link demand — the next update answers it, ticking or
/// not — and a detached surface owes the frame through `attach` anyway.
fn publication_replay(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
) -> impl FnOnce() + 'static {
    let weak = Rc::downgrade(state);
    let view = view.clone();
    move || {
        if let Some(state) = weak.upgrade() {
            // A capture scope open over the hold is asked for the owed
            // frame through the enclosing capture's redraw contract.
            let replay = state
                .presentation
                .borrow()
                .external()
                .map(|external| external.redraw.clone());
            if let Some(redraw) = replay {
                redraw();
            } else {
                owe_and_reschedule(&state, &view);
            }
        }
    }
}

/// Parks the surface on the `generation` publication wait — the
/// device-lost paths' `PendingRebuild`/`DeviceLost` settle, distinct
/// from a typed failure: readiness keeps waiting for a real frame. A
/// hold already parked or failed only re-arms on the newer generation;
/// an open capture scope survives either way.
fn park_for_publication(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>, generation: u64) {
    state.park.park(
        &state.runtime,
        generation,
        || {
            // A live link is paused — a stale delivery still arriving
            // drops its drawable in `render_drawable`.
            if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
                attached.presenter.set_paused(true);
            }
        },
        publication_replay(state, view),
    );
}

/// Renders one issued drawable and settles its receipt after GPU completion.
fn render_drawable(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>, frame: DrawableFrame) {
    let external_redraw = {
        let slot = state.presentation.borrow();
        if let Some(external) = &slot.capture {
            // Capture-owned surfaces present nowhere themselves: the
            // delivered drawable is released unpresented and the redraw
            // goes to the enclosing capture — cloned out so the callback
            // cannot re-enter a live borrow.
            Some(external.redraw.clone())
        } else {
            // A stale delivery — the link kept a queued update across a
            // detach, a failure settle or a device-loss park: release the
            // drawable's lease silently.
            if state.park.is_held() || matches!(slot.epoch, Epoch::Detached) {
                return;
            }
            None
        }
    };
    if let Some(redraw) = external_redraw {
        drop(frame);
        redraw();
        return;
    }
    let first_paint_pending = state
        .presentation
        .borrow()
        .attached()
        .is_some_and(|attached| attached.demand.first_paint_owed.get());
    if !is_effectively_visible(view) && !first_paint_pending {
        owe_frame(state);
        return;
    }

    // One context generation covers the render, the completion marker, and
    // the callback registration — a rebuild mid-frame cannot split them.
    let context = state.runtime.context();
    ensure_presenter(state, &context);

    // This drawable answers the work that was owed before submission.
    // A redraw arriving during its GPU work records fresh demand independently.
    if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
        attached.demand.frame_owed.set(false);
    }
    let (width, height) = frame.drawable_size();
    let target_time = state.presentation_time.map(frame.target_time());
    let outcome = match render_to_metal_texture(
        state,
        view,
        &context,
        frame.texture(),
        width,
        height,
        state.current_scale.get(),
        target_time,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            settle_failed(state, view, context.generation(), error);
            return;
        }
    };
    let FrameRender::Submitted {
        needs_redraw,
        wrote_target,
        scene_generation,
    } = outcome
    else {
        park_unrenderable_frame(state, view, context.generation());
        return;
    };

    if !wrote_target {
        // A `Next::At` frame never wrote its target — drop it and owe it.
        defer_unwritten_drawable_frame(state, view);
        return;
    }

    if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
        attached.demand.keep_redrawing.set(needs_redraw);
    }
    publish_content_accessibility(state, view);
    update_presentation_demand(state, view);

    let weak = Sendable(Rc::downgrade(state));
    let view = Sendable(view.clone());
    // The owned drawable crosses the completion-driver hop and returns to
    // the main queue before it is presented or released.
    let mut frame_slot = Some(Sendable(frame));
    let scene_generation = Sendable(scene_generation);
    let submitted_context = context.clone();
    let marker = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu surface frame completion marker"),
        });
    crate::gpu_completion::submit_with_completion(
        MainThreadMarker::new().expect("GpuSurface frames render on the main thread"),
        marker,
        &context,
        move |result| {
            cocoa_ui::main_queue::enqueue(move |_mtm| {
                let Some(state) = weak.get().upgrade() else {
                    return;
                };
                let Sendable(frame) = frame_slot.take().expect("completion fires once");
                settle_submitted_frame(
                    &state,
                    view.get(),
                    frame,
                    scene_generation.get().as_ref(),
                    &submitted_context,
                    &result,
                );
            });
        },
    );
}

/// The onscreen completion's main-queue settle: the submission's own
/// result is matched first — `Err(SubmissionFailed)` is the same
/// device-loss settle the `PendingRebuild` render arm takes — then a
/// live submission settles its owned slot by the
/// [`CompletionValidity`] contract.
fn settle_submitted_frame(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    frame: DrawableFrame,
    scene_generation: Option<&Rc<EngineGeneration>>,
    submitted_context: &Arc<SharedGpuContext>,
    result: &Result<(), crate::gpu_completion::SubmissionFailed>,
) {
    match result {
        Err(_) => {
            // The submitted generation died in flight — the fence
            // settled on a dead queue and the drawable holds no ready
            // pixels. Park until publication replays the owed frame on
            // a live context.
            owe_frame(state);
            park_for_publication(state, view, submitted_context.generation());
        }
        Ok(()) => {
            settle_frame_completion(state, view, frame, scene_generation, submitted_context);
        }
    }
}

/// What a still-open in-flight submission may do with the current epoch.
enum CompletionValidity {
    /// The submitted context generation is still current: the frame's
    /// ownership, generation evidence and device health were all checked
    /// and the completion may settle readiness normally.
    Current,
    /// A newer publication already landed: the completion is stale — it
    /// releases only its own lease. It must never settle or overwrite a
    /// newer epoch's failure flag, readiness or watch; the owed work
    /// replays on the current generation.
    Obsolete,
    /// The still-current generation's own failure, settled through
    /// `settle_failed` against the exact generation it belongs to.
    Failed {
        /// The generation the failure belongs to — the renderer's own
        /// or the sealed production generation's context generation.
        generation: u64,
        /// The typed error to report once.
        error: HostedError,
    },
}

/// The exact-generation validity check shared by the onscreen and
/// external-capture completions, so neither can diverge: obsolescence
/// first — a stale submission releases only its own lease — then, for a
/// still-current generation only, the renderer's routed failure and the
/// retained production generation's immutable sealed outcome. A failed
/// submission never reaches here: the completion answers its own
/// `Err(SubmissionFailed)` before validity runs.
fn submission_validity(
    state: &Rc<SurfaceState>,
    scene_generation: Option<&Rc<EngineGeneration>>,
    submitted_context: &Arc<SharedGpuContext>,
) -> CompletionValidity {
    // A newer publication before this completion makes the submission
    // obsolete — even when it still carries old failure evidence, that
    // evidence belongs to the dead epoch and never marks the current
    // owner failed, completes its readiness or replaces its watch.
    if submitted_context.generation() != state.runtime.context().generation() {
        return CompletionValidity::Obsolete;
    }
    // A failure routed to this renderer between submission and
    // completion settles on the owner through the sink's queued
    // `settle_failed`; the batch sealed the retained production
    // generation first, so the record below also covers the failure
    // this completion would miss.
    // A sealed production generation rejects its own completion —
    // immutable on the retained generation the submission carried, not
    // on whatever state the surface holds now.
    if let Some(generation) = scene_generation
        && let Some(failure) = generation.failure()
    {
        return CompletionValidity::Failed {
            generation: generation.context().generation(),
            error: HostedError::Scene(failure),
        };
    }
    CompletionValidity::Current
}

/// A submitted onscreen frame's main-queue completion: releases the
/// in-flight lease, then settles the owned frame's slot by the
/// [`CompletionValidity`] contract — a stale completion only owes the
/// work on the current generation, and a still-current one presents
/// only after its failure and device evidence pass.
fn settle_frame_completion(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    frame: DrawableFrame,
    scene_generation: Option<&Rc<EngineGeneration>>,
    submitted_context: &Arc<SharedGpuContext>,
) {
    match submission_validity(state, scene_generation, submitted_context) {
        CompletionValidity::Obsolete => {
            // Stale: drop the slot — the owed frame replays on the
            // current context; readiness, failure and watch untouched.
            owe_and_reschedule(state, view);
            return;
        }
        CompletionValidity::Failed { generation, error } => {
            settle_failed(state, view, generation, error);
            return;
        }
        CompletionValidity::Current => {}
    }
    // A frame may only present on a free `Attached` epoch: a capture
    // scope or a parked wait already owns the redraw or the recovery,
    // so this completion owes the frame back instead of presenting.
    let presentable = {
        let slot = state.presentation.borrow();
        slot.capture.is_none()
            && !state.park.is_held()
            && matches!(&slot.epoch, Epoch::Attached(attached)
                if state.capture_suppression.get() == 0
                    && can_present_now(view)
                    && (is_effectively_visible(view)
                        || attached.demand.first_paint_owed.get()))
    };
    if !presentable {
        owe_and_reschedule(state, view);
        return;
    }
    let presented = frame.present();
    if let Some(_receipt) = presented {
        // A presented frame makes this generation productive —
        // the runtime's unproductive-loss detector keys on it.
        submitted_context.note_frame_presented();
        if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
            attached.demand.first_paint_owed.set(false);
            attached.demand.presented_once.set(true);
        }
        complete_ready(state);
    } else {
        // The buffers were replaced while this frame was in flight —
        // owe it again rather than reveal a hole.
        owe_frame(state);
    }
    update_presentation_demand(state, view);
}

/// Settles a typed [`HostedError`] as an explicit native rendering
/// failure: reported through tracing (the native `os_log` channel), native
/// readiness resolves as failure so `ready` waiters never hang, and the
/// instance stops scheduling — the publication park gates out every
/// later tick, on-demand render and redraw wake. There is no
/// automatic retry and no fallback presentation; the only legitimate
/// recovery is a new context generation, which the parked
/// `context_after` wake rebinds against — `settle_failed` arms that
/// subscription itself so a failure on an otherwise healthy context
/// still reaches the recovery path.
fn settle_failed(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    generation: u64,
    error: HostedError,
) {
    // The terminal carrier the `Failed` hold stores — the capture's
    // terminal outcome forwards this same typed failure. The shared
    // scene failure is already `Arc`'d on the generation; a hosted-layer
    // failure wraps here — one record per owner.
    let failure: Arc<dyn std::error::Error + Send + Sync> = match error {
        HostedError::Scene(shared) => shared,
        HostedError::Layer(error) => Arc::new(error),
    };
    // The failure IS the hold transition: the parked wait is armed on
    // the exact generation the failure happened under — `context_after`
    // never resolves on it, so the failed one is never retried, and a
    // later — including already-published — rebuild rebinds the surface.
    // The failed frame's pixels never landed — the wake replays the owed
    // work through the surface's own contract. An open capture scope is
    // untouched: its `end` still balances.
    let settled = state.park.fail(
        &state.runtime,
        generation,
        failure.clone(),
        || {
            // A failure supersedes a park — the parked wait is replaced
            // by the failed one, cancelling the previous task.
            if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
                attached.demand.keep_redrawing.set(false);
                attached.presenter.set_paused(true);
            }
        },
        publication_replay(state, view),
    );
    if !settled {
        // The owner already settled this failure — the requesting
        // participant's own `Err` arm settled synchronously while its
        // routed copy was still queued. No second log, no re-armed
        // watch, no second readiness resolution.
        return;
    }
    tracing::error!(
        "native rendering failed; the surface stops scheduling until a new context generation rebinds it: {failure}"
    );
    complete_ready(state);
    update_presentation_demand(state, view);
}

/// `handleRedrawRequest`: the redraw waker's main-queue body — republishes
/// accessibility, re-measures against the last proposal, then renders.
fn handle_redraw_request(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    state.dirty.set(true);
    state.needs_a11y_refresh.set(true);
    if take_measurement_invalidation(state) {
        crate::invalidation::invalidate_layout_hierarchy(view.as_platform_view());
    }
    let external_redraw = state
        .presentation
        .borrow()
        .external()
        .map(|external| external.redraw.clone());
    if let Some(redraw) = external_redraw {
        redraw();
    } else {
        // A redraw request is demand: the next link update answers it.
        update_presentation_demand(state, view);
    }
}

/// Whether the host laid this surface out with a measurement the renderer no
/// longer gives — `takeMeasurementInvalidation`.
fn take_measurement_invalidation(state: &SurfaceState) -> bool {
    // A hosted view with an explicit measurement dependency owns its own
    // baseline; it answers before the proposal/answer check does.
    if let Some(changed) = state.view.borrow().measurement_dependency_invalidated() {
        return changed;
    }
    let Some(proposal) = state.last_proposal.get() else {
        return false;
    };
    let measured = state.view.borrow().measure(proposal);
    let changed = state
        .last_resolved_size
        .borrow()
        .is_some_and(|last| last != measured.size);
    if changed {
        *state.last_resolved_size.borrow_mut() = Some(measured.size);
    }
    changed
}

/// Publishes the content's label and value — `publishContentAccessibility`:
/// an application-set value on this very view always wins.
fn publish_content_accessibility(state: &SurfaceState, view: &Retained<SurfaceView>) {
    if !state.needs_a11y_refresh.replace(false) {
        return;
    }
    let (label, value) = {
        let view = state.view.borrow();
        (view.accessibility_label(), view.accessibility_value())
    };
    publish_accessibility(view.as_platform_view(), label.as_deref(), value.as_deref());
}

/// Writes label/value through the platform accessibility channel — the two
/// `publishContentAccessibility*` halves.
fn publish_accessibility(view: &cocoa_ui::PlatformView, label: Option<&str>, value: Option<&str>) {
    cocoa_ui::view::set_accessibility_content(view, label, value);
}

/// Runs the frame after which `ready` waiters wake — `completeReady`.
fn complete_ready(state: &SurfaceState) {
    for waker in state.ready_waiters.borrow_mut().drain(..) {
        waker.wake();
    }
}

// MARK: - Window observers (WuiWindowOcclusion)

/// (Re)arms the occlusion / miniaturization / activation observers for the
/// window `view` now sits in — `updateWindowObservers`.
fn update_window_observers(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    let mut observers = state.observers.borrow_mut();
    observers.clear();
    let Some(window) = cocoa_ui::view::window(view.as_platform_view()) else {
        return;
    };
    #[cfg(target_os = "ios")]
    let _ = &window;
    let mtm = cocoa_ui::MainThreadMarker::new().expect("main thread");
    let fire = {
        let weak = Rc::downgrade(state);
        let view = view.clone();
        move || {
            if let Some(state) = weak.upgrade() {
                update_presentation_demand(&state, &view);
            }
        }
    };
    #[cfg(target_os = "macos")]
    {
        observers.push(cocoa_ui::appkit::watch_occlusion(mtm, &window, {
            let fire = fire.clone();
            move || fire()
        }));
        for notification in [
            // SAFETY: the notification names are system constants.
            unsafe { cocoa_ui::objc2_app_kit::NSWindowDidMiniaturizeNotification },
            // SAFETY: the notification names are system constants.
            unsafe { cocoa_ui::objc2_app_kit::NSWindowDidDeminiaturizeNotification },
            // SAFETY: the notification names are system constants.
            unsafe { cocoa_ui::objc2_app_kit::NSWindowDidChangeScreenNotification },
        ] {
            let name = notification;
            observers.push(cocoa_ui::notification::observe_object(
                mtm,
                &cocoa_ui::notification::NotificationName::framework(name),
                window.as_ref(),
                {
                    let fire = fire.clone();
                    move || fire()
                },
            ));
        }
    }
    #[cfg(target_os = "ios")]
    {
        for notification in [
            // SAFETY: the notification names are system constants.
            unsafe { cocoa_ui::objc2_ui_kit::UIApplicationDidBecomeActiveNotification },
            // SAFETY: the notification names are system constants.
            unsafe { cocoa_ui::objc2_ui_kit::UIApplicationWillResignActiveNotification },
        ] {
            let name = notification;
            observers.push(cocoa_ui::notification::observe(
                mtm,
                &cocoa_ui::notification::NotificationName::framework(name),
                {
                    let fire = fire.clone();
                    move || fire()
                },
            ));
        }
    }
}

// MARK: - Input (WuiGpuSurfaceInput)

/// The input responder overlay `wants_input_events` installs — the kit's
/// `InputView` forwarding `SurfaceEvent`s translated for the semantic view.
fn install_input(
    view: &Retained<SurfaceView>,
    state: &Rc<SurfaceState>,
) -> Option<Retained<cocoa_ui::PlatformView>> {
    if !state.wants_input_events {
        return None;
    }
    let mtm = cocoa_ui::MainThreadMarker::new().expect("main thread");
    let input = platform_input_view(mtm);
    let weak = Rc::downgrade(state);
    // The input responder is a subview of `view`: the view retains it, its
    // handler must not retain the view back — `host → subtree → input →
    // handler → host` would pin the whole graph past teardown. Both
    // captures stay weak.
    let host = objc2::rc::Weak::new(&**view);
    input.set_event_handler(move |event| {
        let (Some(state), Some(host)) = (weak.upgrade(), host.load()) else {
            return;
        };
        let event = crate::gpu_input::translate(&event);
        state.view.borrow().input(&event);
        // The event's frame request rides the same coalescing as a redraw
        // request.
        let external_redraw = state
            .presentation
            .borrow()
            .external()
            .map(|external| external.redraw.clone());
        if let Some(redraw) = external_redraw {
            redraw();
        } else {
            if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
                attached.demand.keep_redrawing.set(true);
            }
            update_presentation_demand(&state, &host);
        }
    });
    input.set_caret_provider({
        let weak = Rc::downgrade(state);
        move || {
            weak.upgrade().and_then(|state| {
                state.view.borrow().ime_caret().map(|rect| {
                    cocoa_ui::Rect::new(
                        rect.origin().x,
                        rect.origin().y,
                        rect.size().width,
                        rect.size().height,
                    )
                })
            })
        }
    });
    // The responder fills the surface and sits on top of it.
    cocoa_ui::view::add_subview(view.as_platform_view(), input.as_ref());
    Some(Retained::into_super(input))
}

// MARK: - Capturable (WuiMetalViewCapture's surface half)

/// The surface installed on the view's capturable slot, for
/// `ViewCapture`'s resolver, `view.ready()` and the capture drive's
/// [`Presentation`].
struct Capturable {
    state: Rc<SurfaceState>,
    view: Retained<SurfaceView>,
}

impl Capturable {
    /// Whether one on-screen frame has landed — readiness comes only from a
    /// `PresentedFrame` receipt, never from an offscreen capture completion.
    fn presented(&self) -> bool {
        self.state
            .presentation
            .borrow()
            .attached()
            .is_some_and(|attached| attached.demand.presented_once.get())
    }

    /// Whether this surface gates its window's first-paint readiness —
    /// see [`participates_in_first_paint`].
    fn participates(&self) -> bool {
        !self.state.park.is_failed() && participates_in_first_paint(&self.view)
    }

    /// Registers `waker` and requests the owed first frame: lay out, make
    /// sure the GPU context is up, arm the reveal wait so the link answers
    /// it instead of parking forever.
    fn register_waiter(&self, waker: std::task::Waker) {
        if self.presented() || self.state.park.is_failed() {
            return;
        }
        cocoa_ui::view::layout_immediately(self.view.as_platform_view());
        self.state.ready_waiters.borrow_mut().push(waker);
        if let Some(attached) = self.state.presentation.borrow_mut().attached_mut() {
            attached.demand.first_paint_owed.set(true);
        }
        owe_frame(&self.state);
        update_presentation_demand(&self.state, &self.view);
    }
}

impl cocoa_ui::capture::CapturableSurface for Capturable {
    fn capture_pixel_format(&self) -> MTLPixelFormat {
        state_format(&self.state)
    }

    fn content_bounds(&self, relative_to: &cocoa_ui::PlatformView) -> cocoa_ui::Rect {
        self.view.bounds_in(relative_to)
    }

    fn begin_capture_suppression(&self) {
        let count = self.state.capture_suppression.get() + 1;
        self.state.capture_suppression.set(count);
        if count == 1 {
            set_presentation_hidden(&self.view, true);
        }
    }

    fn end_capture_suppression(&self) {
        let count = self.state.capture_suppression.get();
        assert!(
            count > 0,
            "GpuSurface capture suppression scopes are unbalanced"
        );
        self.state.capture_suppression.set(count - 1);
        if count == 1 {
            set_presentation_hidden(&self.view, false);
        }
    }

    fn begin_external_rendering(&self, on_redraw: Rc<dyn Fn()>) {
        let mut slot = self.state.presentation.borrow_mut();
        if let Some(external) = &mut slot.capture {
            // A nested scope only deepens the existing capture — the
            // redraw contract is the outermost scope's.
            external.depth = external
                .depth
                .checked_add(1)
                .expect("external rendering depth overflowed NonZeroU32");
            return;
        }
        // The capture owns rendering now: the link is paused and
        // continuous demand stops — the epoch resumes exactly as left
        // when the last scope ends, whatever a mid-scope settle landed.
        if let Some(attached) = slot.attached_mut() {
            attached.demand.keep_redrawing.set(false);
            attached.presenter.set_paused(true);
        }
        slot.capture = Some(External {
            redraw: on_redraw,
            depth: NonZeroU32::MIN,
        });
    }

    fn end_external_rendering(&self, resume: bool) {
        {
            let mut slot = self.state.presentation.borrow_mut();
            let Some(external) = &mut slot.capture else {
                panic!("GpuSurface external rendering scopes are unbalanced");
            };
            if let Some(remaining) = NonZeroU32::new(external.depth.get() - 1) {
                external.depth = remaining;
                return;
            }
            slot.capture = None;
        }
        if resume {
            owe_and_reschedule(&self.state, &self.view);
        }
    }

    fn prepare_external_render(
        &self,
        texture: &objc2::runtime::ProtocolObject<dyn MTLTexture>,
    ) -> bool {
        self.state.assert_declared_format(texture.pixelFormat());
        true
    }

    fn has_presented_frame(&self) -> bool {
        self.presented()
    }

    fn participates_in_first_paint(&self) -> bool {
        self.participates()
    }

    fn register_ready_waiter(&self, waker: std::task::Waker) {
        self.register_waiter(waker);
    }

    fn presentation_failed(&self) -> bool {
        self.state.park.is_failed()
    }

    fn render_prepared_external_texture(
        &self,
        texture: &objc2::runtime::ProtocolObject<dyn MTLTexture>,
        width: u32,
        height: u32,
        completion: cocoa_ui::capture::SurfaceCaptureCompletion,
    ) {
        if self.state.park.is_held() {
            // A parked or failed surface owes its next frame to the
            // publication wake, not to a new render on the same context:
            // re-rendering here would retry the settled frame, log again
            // and re-arm the watch. The hold's outcome answers the
            // capture — terminal `Failed` for a settled failure, deferred
            // for a park — and the wake replays through `capture.redraw`.
            completion(Err(self.state.park.capture_error()));
            return;
        }
        // SAFETY: `texture` is the live texture the capture pipeline retained
        // for this call; `retain` takes our own reference.
        let texture = unsafe {
            Retained::<objc2::runtime::ProtocolObject<dyn MTLTexture>>::retain(
                std::ptr::from_ref(texture).cast_mut(),
            )
        }
        .expect("GpuSurface external render received a null texture");
        let context = self.state.runtime.context();
        // An explicit capture renders at the shared anchor's capture time —
        // its own exact instant, never a borrowed or approximated frame
        // timestamp.
        let target_time = self.state.presentation_time.capture_time();
        let outcome = match render_to_metal_texture(
            &self.state,
            &self.view,
            &context,
            texture,
            width,
            height,
            self.state.current_scale.get(),
            target_time,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                fail_external_frame(&self.state, &self.view, &context, error, completion);
                return;
            }
        };
        let FrameRender::Submitted {
            wrote_target,
            scene_generation,
            ..
        } = outcome
        else {
            complete_ready(&self.state);
            // The capture's deferred frame replays through the redraw
            // contract: publication resolves the parked wait, and the
            // externally rendered surface's wake turns it into a redraw
            // notification to the enclosing capture — the parent's owed
            // frame then re-renders.
            park_for_publication(&self.state, &self.view, context.generation());
            completion(Err(cocoa_ui::capture::CaptureError::Deferred));
            return;
        };
        if !wrote_target {
            defer_unwritten_external_frame(&self.state);
            completion(Err(cocoa_ui::capture::CaptureError::Deferred));
            return;
        }
        let weak = Sendable(Rc::downgrade(&self.state));
        let view = Sendable(self.view.clone());
        let scene_generation = Sendable(scene_generation);
        let submitted_context = context.clone();
        let marker = context
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gpu surface external completion marker"),
            });
        crate::gpu_completion::submit_with_completion(
            MainThreadMarker::new()
                .expect("GpuSurface external renders complete on the main thread"),
            marker,
            &context,
            move |result| {
                // `on_submitted_work_done` closures can run on any thread
                // that maintains the device — hop to the main queue before
                // touching the weak state handle.
                cocoa_ui::main_queue::enqueue(move |_mtm| {
                    let Some(state) = weak.get().upgrade() else {
                        // The capture's own lease retains its texture and
                        // submission, not this host's `SurfaceState` — a
                        // released native owner leaves the in-flight
                        // submission no current epoch to validate against
                        // and no readiness to settle.
                        // The pixels may exist, but nothing left can vouch
                        // for them: the submission settles as deferred —
                        // never a synthesized success.
                        completion(Err(cocoa_ui::capture::CaptureError::Deferred));
                        return;
                    };
                    match result {
                        Err(_) => {
                            // The generation that carried this frame died
                            // in flight: the fence settled on a dead queue,
                            // so there are no usable pixels to compose.
                            // Arm publication so the redraw contract wakes
                            // the parent once a live context lands.
                            complete_ready(&state);
                            park_for_publication(
                                &state,
                                view.get(),
                                submitted_context.generation(),
                            );
                            completion(Err(cocoa_ui::capture::CaptureError::Deferred));
                        }
                        // The same exact-generation contract as the
                        // onscreen completion: a stale submission releases
                        // only its own lease — its deferred result replays
                        // the capture on the current context — and never
                        // touches a newer epoch's readiness or watch.
                        Ok(()) => {
                            match submission_validity(
                                &state,
                                scene_generation.get().as_ref(),
                                &submitted_context,
                            ) {
                                CompletionValidity::Obsolete => {
                                    completion(Err(cocoa_ui::capture::CaptureError::Deferred));
                                }
                                CompletionValidity::Failed { generation, error } => {
                                    settle_failed(&state, view.get(), generation, error);
                                    // The batch failure this completion
                                    // carried: the typed `Failed` the
                                    // freshly-settled hold now answers.
                                    completion(Err(state.park.capture_error()));
                                }
                                CompletionValidity::Current => {
                                    if state.park.is_held() {
                                        // A hold landed between submission
                                        // and this completion — the
                                        // onscreen presentable check's
                                        // counterpart: a failed surface
                                        // answers its terminal outcome,
                                        // a parked one defers to the
                                        // publication wake.
                                        completion(Err(state.park.capture_error()));
                                        return;
                                    }
                                    // Offscreen pixels are usable, but only a real
                                    // PresentedFrame receipt settles onscreen readiness
                                    // and the runtime's productive-generation accounting.
                                    completion(Ok(()));
                                }
                            }
                        }
                    }
                });
            },
        );
    }
}

const fn state_format(state: &SurfaceState) -> MTLPixelFormat {
    state
        .presentation_format
        .get()
        .expect("GpuSurface must have a configured dynamic range before external capture")
}

/// Shows or hides the presentation layer — `setPresentationHidden`.
///
/// Called only inside `ViewCapture`'s outer disabled-actions transaction,
/// which owns the one commit covering suppression open, the native
/// raster draw and suppression close: this setter must not open, commit or
/// flush a transaction of its own — the suppressed state can then never
/// reach the on-screen tree.
fn set_presentation_hidden(view: &Retained<SurfaceView>, hidden: bool) {
    let layer = view.presentation_layer();
    if layer.isHidden() == hidden {
        return;
    }
    layer.setHidden(hidden);
}

/// The sendable wrapper for main-thread-only state crossing the redraw
/// waker and completion-driver boundaries.
struct Sendable<T>(T);

impl<T> Sendable<T> {
    /// Reads the wrapped value — method access keeps closure captures on the
    /// whole cell, where the `Send`/`Sync` contract lives.
    const fn get(&self) -> &T {
        &self.0
    }
}

// SAFETY: the wrapped value is only ever produced/consumed on the main
// thread — the drivers park the closure on theirs and fire it back through
// the main queue.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Send for Sendable<T> {}
// SAFETY: as `Send`.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Sync for Sendable<T> {}

/// The platform's input overlay view.
#[cfg(target_os = "macos")]
fn platform_input_view(
    mtm: cocoa_ui::MainThreadMarker,
) -> Retained<cocoa_ui::appkit::input_view::InputView> {
    cocoa_ui::appkit::input_view::InputView::new(mtm)
}

/// The platform's input overlay view.
#[cfg(target_os = "ios")]
fn platform_input_view(
    mtm: cocoa_ui::MainThreadMarker,
) -> Retained<cocoa_ui::uikit::input_view::InputView> {
    cocoa_ui::uikit::input_view::InputView::new(mtm)
}

/// Dropping tears down the mounted surface: the presentation detach,
/// content unmount and handler cleanup the mount itself owns.
struct MountGuard {
    view: Retained<SurfaceView>,
    state: Rc<SurfaceState>,
    input: Option<Retained<cocoa_ui::PlatformView>>,
}

impl fmt::Debug for MountGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MountGuard").finish_non_exhaustive()
    }
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        *self.state.presentation.borrow_mut() = Presentation::detached();
        self.state.park.clear();
        self.state.observers.borrow_mut().clear();
        self.state.view.borrow_mut().unmount();
        drop(self.state.bound.borrow_mut().take());
        if let Some(input) = &self.input {
            cocoa_ui::view::remove_from_superview(input);
        }
        // Remove callbacks that retain the mount and its native view.
        self.view.set_layout_handler(|| {});
        self.view.set_window_changed_handler(|| {});
        self.view.set_backing_changed_handler(|| {});
        self.view.set_visibility_changed_handler(|| {});
        self.view.capturable_slot().clear();
    }
}

/// Waits until every mounted capturable output inside `view`'s subtree —
/// GPU surfaces and filtered outputs alike — has presented a frame:
/// `WuiAnyView.ready()`.
#[allow(clippy::future_not_send)]
pub async fn wait_for_first_frames(view: &cocoa_ui::PlatformView) {
    // Non-participants (a hidden, clipped or unpresentable surface) never
    // gate the reveal — and never owe a frame.
    let mut surfaces = Vec::new();
    cocoa_ui::capture::collect_capturables(view, &mut |surface| {
        if surface.participates_in_first_paint() && !surface.has_presented_frame() {
            surfaces.push(Rc::clone(surface));
        }
    });
    wait_for_presented(&surfaces).await;
}

/// Waits until every surface in `surfaces` has presented its first
/// frame — the wait [`wait_for_first_frames`] runs, covering only the set
/// its caller collected. Each surface it waits on owes a frame whose
/// every deferral
/// — an in-flight replay, a parked device-loss rebuild — ends in a frame
/// or a settled terminal failure: the wait always has an end condition.
#[expect(
    clippy::future_not_send,
    reason = "the wait runs on the main thread; the Rc surfaces it holds are not Send"
)]
pub async fn wait_for_presented(surfaces: &[Rc<dyn cocoa_ui::capture::CapturableSurface>]) {
    core::future::poll_fn(|cx| {
        let mut pending = false;
        for surface in surfaces {
            if !surface.has_presented_frame() && !surface.presentation_failed() {
                surface.register_ready_waiter(cx.waker().clone());
                pending = true;
            }
        }
        if pending {
            core::task::Poll::Pending
        } else {
            core::task::Poll::Ready(())
        }
    })
    .await;
}

// MARK: - SubView (WuiGraphicsPrimitiveSizing)

/// The leaf's layout: the hosted view measures under the proposal it was
/// last given, reusing the stale box while a capture-owned frame runs.
struct SurfaceSubView {
    state: Rc<SurfaceState>,
}

impl SubView for SurfaceSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.last_proposal.set(Some(proposal));
        let measured = self.state.view.borrow().measure(proposal);
        *self.state.last_resolved_size.borrow_mut() = Some(measured.size);
        measured
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.view.borrow().stretch_axis()
    }

    fn priority(&self) -> i32 {
        0
    }
}

// MARK: - Install

/// Installs the `gpu_surface` handlers — `Native<GpuContentView>` and
/// `Native<ExternalFrameView>` and `Native<SceneView>` share presentation.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<GpuContentView>(build_surface);
    dispatcher.register_native::<ExternalFrameView>(build_surface);
    dispatcher.register_native::<waterui_graphics::scene_view::SceneView>(|view, ctx| {
        build_surface(
            scene::Scene::new(view, crate::gpu_runtime::scene_engine(ctx.env())),
            ctx,
        )
    });
}

/// The leaf construction the dispatcher's GPU-surface claims share —
/// `GpuContentView`, `ExternalFrameView`, and `SceneView`.
fn build_surface<V: HostedView + 'static>(
    view: V,
    ctx: &crate::contract::RenderContext<'_>,
) -> NativeLeaf {
    build_surface_parts(view, ctx).0
}

/// The single production mount path and its typed capture owner.
fn build_surface_parts<V: HostedView + 'static>(
    view: V,
    ctx: &crate::contract::RenderContext<'_>,
) -> (NativeLeaf, Rc<Capturable>) {
    {
        let mtm = ctx.mtm();
        let platform_view = SurfaceView::new(mtm);
        let runtime = crate::gpu_runtime::runtime(ctx.env());
        let state = Rc::new_cyclic(|weak| {
            SurfaceState::new(
                weak,
                runtime,
                Box::new(view),
                &platform_view,
                mtm,
                ctx.env(),
            )
        });
        state.view.borrow_mut().mount(&state.redraw_handle);

        // Captures resolve surfaces through the slot on the view itself;
        // the mount guard owns removal.
        let capturable = Rc::new(Capturable {
            state: state.clone(),
            view: platform_view.clone(),
        });
        let registered: Rc<dyn cocoa_ui::capture::CapturableSurface> = capturable.clone();
        platform_view.capturable_slot().install(&registered);

        wire_view_handlers(&platform_view, &state);
        let input_view = install_input(&platform_view, &state);

        let mount_guard = MountGuard {
            view: platform_view.clone(),
            state: state.clone(),
            input: input_view.clone(),
        };
        let mut leaf = NativeLeaf::new(&platform_view, SurfaceSubView { state });
        leaf.keep(platform_view);
        leaf.keep(registered);
        leaf.keep(mount_guard);
        if let Some(input) = input_view {
            leaf.keep(input);
        }
        (leaf, capturable)
    }
}

/// `build_surface`'s platform-view handlers — layout, window moves, backing
/// changes and occlusion all route into the shared state.
fn wire_view_handlers(platform_view: &Retained<SurfaceView>, state: &Rc<SurfaceState>) {
    platform_view.set_layout_handler({
        let state = state.clone();
        let view = platform_view.clone();
        move || {
            initialize_gpu(&state, &view);
            update_presentation_demand(&state, &view);
        }
    });
    platform_view.set_window_changed_handler({
        let state = state.clone();
        let view = platform_view.clone();
        move || {
            let Some(window) = cocoa_ui::view::window(view.as_platform_view()) else {
                detach_if_attached(&state);
                complete_ready(&state);
                if let Some(attached) = state.presentation.borrow_mut().attached_mut() {
                    attached.demand.keep_redrawing.set(false);
                }
                state.observers.borrow_mut().clear();
                return;
            };
            if let Some(scale) = view.backing_scale() {
                state.current_scale.set(scale);
            }
            let _ = window;
            update_presentation_frame(&state, &view);
            update_window_observers(&state, &view);
            update_presentation_demand(&state, &view);
        }
    });
    #[cfg(target_os = "macos")]
    {
        platform_view.set_visibility_changed_handler({
            let state = state.clone();
            let view = platform_view.clone();
            move || update_presentation_demand(&state, &view)
        });
        platform_view.set_backing_changed_handler({
            let state = state.clone();
            let view = platform_view.clone();
            move || {
                if cocoa_ui::view::window(view.as_platform_view()).is_none() {
                    return;
                }
                if let Some(scale) = view.backing_scale() {
                    state.current_scale.set(scale);
                }
                initialize_gpu(&state, &view);
                update_presentation_demand(&state, &view);
            }
        });
    }
    #[cfg(target_os = "ios")]
    {
        platform_view.set_visibility_changed_handler({
            let state = state.clone();
            let view = platform_view.clone();
            move || update_presentation_demand(&state, &view)
        });
        platform_view.set_backing_changed_handler({
            let state = state.clone();
            let view = platform_view.clone();
            move || {
                initialize_gpu(&state, &view);
                update_presentation_demand(&state, &view);
            }
        });
    }
}

// MARK: - Native-test reach (Tests/native.rs)

/// Harness-only reach for `Tests/native.rs`, the harness whose cases run
/// on the process's real main thread under a true `MainThreadMarker`:
/// mounts a `SceneView` through [`build_surface`] — the production leaf
/// construction, so the `SurfaceState`, capturable registration, redraw
/// handle and view handlers are the ones a real mount installs — then
/// drives the completion settlement and routed-failure paths on it.
/// Compiled only with `native-test` on macOS; never in a production
/// build, never on iOS.
#[cfg(all(feature = "native-test", target_os = "macos"))]
pub mod native_test {
    use alloc::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::task::{Wake, Waker};

    use waterui_backend_core::Environment;
    use waterui_graphics::cherenkov::{Display, Draw, Recorder, SurfaceError, WorkingColor};
    use waterui_graphics::gpu::runtime::HostedLayerError;
    use waterui_graphics::resources::RecordingResources;
    use waterui_graphics::scene_view::{SceneContent, SceneView};

    use super::{
        Capturable, Dispatcher, DrawableFrame, FrameTime, GpuRuntime, HostedError,
        MainThreadMarker, NativeLeaf, PresentationTime, Rc, RefCell, Retained, SceneEngine,
        SurfaceState, SurfaceView, build_surface_parts, defer_unwritten_external_frame, fmt, kurbo,
        park_for_publication, render_drawable, scene, settle_failed, wgpu,
    };
    use crate::contract::RenderContext;

    /// The smallest real `SceneContent`: records one fill — a mounted
    /// scene that runs the true `Scene` → `ScenePart` → engine path.
    struct FixtureContent;

    impl SceneContent for FixtureContent {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            recorder.fill(
                kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                WorkingColor::WHITE,
            );
            false
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// Self-animating fixture content: `build_scene` always answers
    /// `true`, exercising the `again` path that schedules the next frame
    /// without un-producing this one.
    struct AnimatingFixtureContent;

    impl SceneContent for AnimatingFixtureContent {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            recorder.fill(
                kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                WorkingColor::WHITE,
            );
            true
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// The fixture environment every mount shares — runtime, engine
    /// owner and the `PresentationTime` anchor — so a mounted surface
    /// (or a pair) binds one engine generation.
    #[must_use]
    pub fn fixture_env(runtime: GpuRuntime) -> Environment {
        let mut env = Environment::new();
        env.insert(runtime);
        env.insert(Rc::new(SceneEngine::new()));
        PresentationTime::install(&mut env);
        env
    }

    /// A `SceneView` carrying [`FixtureContent`].
    ///
    /// The same GPU child the fixture mounts, as a value a
    /// `ViewRenderer::render` trial passes straight in, so the leaf mounts
    /// through the production `Native<SceneView>` path itself.
    #[must_use]
    pub fn fixture_scene_view() -> SceneView {
        SceneView::new(FixtureContent)
    }

    /// A mounted `SceneView` surface kept alive for a trial's assertions.
    ///
    /// `leaf` holds the surface registration, capturable and view
    /// handlers the production mount installed; the private fields keep
    /// the owned pieces each settlement path needs.
    #[must_use]
    pub struct MountedSceneSurface {
        /// The mounted leaf — dropping it unmounts the surface.
        pub leaf: NativeLeaf,
        state: Rc<SurfaceState>,
        view: Retained<SurfaceView>,
        capturable: Rc<Capturable>,
        runtime: GpuRuntime,
        /// The real window that makes the fixture's drawable source presentable.
        fixture_window: RefCell<Option<cocoa_ui::appkit::Window>>,
    }

    impl fmt::Debug for MountedSceneSurface {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("MountedSceneSurface")
                .field("presented", &self.frame_presented())
                .finish_non_exhaustive()
        }
    }

    /// A readiness waiter registered through the real
    /// `Capturable::register_waiter` — `wakes` counts how many times
    /// `complete_ready` drained it.
    pub struct WakeProbe(Arc<AtomicU32>);

    impl fmt::Debug for WakeProbe {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_tuple("WakeProbe").field(&self.wakes()).finish()
        }
    }

    impl WakeProbe {
        /// How many times the registered waker has fired.
        #[must_use]
        pub fn wakes(&self) -> u32 {
            self.0.load(Ordering::SeqCst)
        }
    }

    /// Counts wake deliveries — one `fetch_add` per `wake`, so an
    /// assertion reads both that it fired and that it fired once.
    struct ProbeWake(Arc<AtomicU32>);

    impl Wake for ProbeWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl MountedSceneSurface {
        /// Creates the shared GPU runtime and mounts a `SceneView`
        /// through [`build_surface`], the production leaf construction.
        /// Async because adapter/device creation is; the trial drives
        /// the future with its own executor (`pollster` is a dev-dep —
        /// unavailable in-crate).
        ///
        /// # Errors
        ///
        /// When the runner has no GPU adapter.
        ///
        /// # Panics
        ///
        /// When the surface's own mount machinery misbehaves (the
        /// production asserts it would hit on any real mount).
        #[expect(
            clippy::future_not_send,
            reason = "the mount is a main-thread fixture: Rc<SurfaceState>, the MainThreadMarker and the Objective-C views stay on the trial's main thread"
        )]
        pub async fn mount(mtm: MainThreadMarker) -> Result<Self, String> {
            // `startup::initialize` installs process-global state (the
            // tracing dispatcher, executors); the native-test harness
            // performs it once on this same main thread before any
            // trial runs — the mount relies on that explicit setup.
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            Self::mount_in(mtm, &fixture_env(runtime))
        }

        /// Mounts a surface whose content self-animates — `build_scene`
        /// always answers `true` — through the same production path as
        /// [`Self::mount`].
        ///
        /// # Errors
        ///
        /// When the runner has no GPU adapter.
        #[expect(
            clippy::future_not_send,
            reason = "the mount is a main-thread fixture: Rc<SurfaceState>, the MainThreadMarker and the Objective-C views stay on the trial's main thread"
        )]
        pub async fn mount_animating(mtm: MainThreadMarker) -> Result<Self, String> {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            Ok(Self::mount_in_view(
                mtm,
                &fixture_env(runtime),
                SceneView::new(AnimatingFixtureContent),
            ))
        }

        /// Mounts the surface under an environment the caller prepared —
        /// the shared `GpuRuntime`, `SceneEngine` and `PresentationTime`
        /// it already holds — so a fixture that mounts this surface as a
        /// child (the filtered-leaf trials) renders it on the parent's own
        /// runtime and context generation.
        ///
        /// # Errors
        ///
        /// When `env` carries no `GpuRuntime` or `SceneEngine`.
        pub fn mount_in(mtm: MainThreadMarker, env: &Environment) -> Result<Self, String> {
            Ok(Self::mount_in_view(
                mtm,
                env,
                SceneView::new(FixtureContent),
            ))
        }

        /// Mounts a surface around the caller's `SceneView` under `env` —
        /// the body [`Self::mount_in`] shares with mounts whose content
        /// differs (self-animating fixture content).
        ///
        /// # Panics
        ///
        /// When `env` carries no `GpuRuntime` or `SceneEngine`.
        fn mount_in_view(mtm: MainThreadMarker, env: &Environment, view: SceneView) -> Self {
            let runtime = crate::gpu_runtime::runtime(env);
            let engines = crate::gpu_runtime::scene_engine(env);
            let ctx = RenderContext::new(env, Rc::new(Dispatcher::new()), mtm);
            let (leaf, capturable) = build_surface_parts(scene::Scene::new(view, engines), &ctx);
            Self {
                state: capturable.state.clone(),
                view: capturable.view.clone(),
                capturable,
                leaf,
                runtime,
                fixture_window: RefCell::new(None),
            }
        }

        /// Installs the hosted scene renderer through the real
        /// [`SurfaceState::render_into`] on the runtime's live context —
        /// the production `HostedView::renderer` → `ScenePart::new` →
        /// `generation.mount` path `render_drawable` takes — presenting one
        /// real frame into a texture so the shared generation is
        /// genuinely produced.
        ///
        /// # Errors
        ///
        /// When renderer creation or the frame's encode fails.
        pub fn install_scene_renderer(&self) -> Result<(), String> {
            self.render_scene_frame_at(FrameTime(std::time::Instant::now()))
        }

        /// Mounts two surfaces into one environment — the same
        /// `SceneEngine` engine owner and `PresentationTime` anchor — so
        /// their renderers share the live [`EngineGeneration`]. The first
        /// member's `produce` then produces the batch the second member's
        /// `present` answers `Next::At` for — the mount-after-batch case
        /// an unwritten target comes from.
        ///
        /// # Errors
        ///
        /// When runtime creation or either mount fails.
        #[expect(
            clippy::future_not_send,
            reason = "the mount is a main-thread fixture: Rc<SurfaceState>, the MainThreadMarker and the Objective-C views stay on the trial's main thread"
        )]
        pub async fn mount_pair(mtm: MainThreadMarker) -> Result<(Self, Self), String> {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            let env = fixture_env(runtime);
            Ok((Self::mount_in(mtm, &env)?, Self::mount_in(mtm, &env)?))
        }

        /// Runs [`SurfaceState::render_into`] for one explicit
        /// `target_time` — the frame `produce` stamps the shared batch
        /// under — so a sibling surface's later render at the same
        /// timestamp reads the already-produced batch.
        ///
        /// # Errors
        ///
        /// When renderer creation or the frame's encode fails.
        pub fn render_scene_frame_at(&self, target_time: FrameTime) -> Result<(), String> {
            let context = self.runtime.context();
            let texture = context.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("gpu surface native-test scene target"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.state
                .render_into(
                    &self.view,
                    &context,
                    &texture,
                    Display::default(),
                    target_time,
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        }

        /// Maps a `CACurrentMediaTime` timestamp onto this surface's
        /// `FrameTime` axis — the same `presentation_time.map` a delivered
        /// drawable's `target_time` goes through — so a trial can produce
        /// the shared batch at the exact timestamp a delivered frame
        /// carries.
        pub fn map_frame_time(&self, media_time: f64) -> FrameTime {
            self.state.presentation_time.map(media_time)
        }

        /// Issues one real drawable frame through `render_drawable`
        /// itself at the given media timestamp. The drawable is checked
        /// out of a standalone `CAMetalLayer` on the live context's
        /// device — a link-bound layer forbids `nextDrawable` — so the
        /// frame carries an empty owner: `present` can never mint a
        /// receipt on it and its drop settles nothing on the surface's
        /// presenter. Answers whether a drawable was delivered.
        pub fn deliver_frame(&self, media_time: f64) -> bool {
            use cocoa_ui::objc2_core_foundation::CGSize;
            use cocoa_ui::objc2_quartz_core::CAMetalLayer;

            let device = crate::gpu_runtime::raw_metal_device(&self.runtime.context());
            let layer = CAMetalLayer::new();
            layer.setDevice(Some(&device));
            layer.setPixelFormat(crate::components::gpu_surface::state_format(&self.state));
            layer.setDrawableSize(CGSize::new(64.0, 64.0));
            let Some(drawable) = layer.nextDrawable() else {
                return false;
            };
            let frame = DrawableFrame::unowned_for_test(drawable, media_time);
            render_drawable(&self.state, &self.view, frame);
            true
        }

        /// Whether the attached epoch still owes a frame — the flag the
        /// unwritten-frame guard re-arms.
        pub fn frame_owed(&self) -> bool {
            self.state
                .presentation
                .borrow()
                .attached()
                .is_some_and(|attached| attached.demand.frame_owed.get())
        }

        /// What the bound renderer reports for `target_time` — `None`
        /// while no renderer is bound. An unwritten report is the input
        /// `render_drawable`'s drop-unpresented guard keys on; the probe
        /// proves the bound renderer itself produced the report.
        #[must_use]
        pub fn wrote_target(&self, target_time: FrameTime) -> Option<bool> {
            self.state
                .bound
                .borrow()
                .as_ref()
                .map(|bound| bound.renderer.wrote_target(target_time))
        }

        /// Fires the hosted scene's invalidator — the "invalidated
        /// after the batch" case: `produced_for` clears and a redraw is
        /// requested. Synchronous: nothing the link or a queued redraw
        /// did before matters, and nothing can re-prepare the scene
        /// before the trial's next synchronous render.
        pub fn invalidate_scene(&self) {
            self.state.view.borrow().invalidate();
        }

        /// Drives the production `defer_unwritten_external_frame` — the
        /// settle an external render runs when `present` answers
        /// `Next::At` — so the trial observes the deferred frame's replay
        /// through the capture scope's own redraw notification.
        pub fn defer_external_frame(&self) {
            defer_unwritten_external_frame(&self.state);
        }

        /// The surface's own platform view, retained — for fixtures that
        /// mount this surface as a child inside another leaf (the
        /// filtered-leaf trials place it in the filtered host's hidden
        /// content subtree).
        pub fn platform_view(&self) -> Retained<cocoa_ui::PlatformView> {
            cocoa_ui::view::retain_base(self.view.as_platform_view())
        }

        /// Whether the surface accepted a real `PresentedFrame` receipt.
        pub fn frame_presented(&self) -> bool {
            self.state
                .presentation
                .borrow()
                .attached()
                .is_some_and(|attached| attached.demand.presented_once.get())
        }

        /// The attached presenter's layer pixel format — the format every
        /// drawable it issues is allocated in.
        pub fn presenter_pixel_format(&self) -> Option<objc2_metal::MTLPixelFormat> {
            self.state
                .presentation
                .borrow()
                .attached()
                .map(|attached| attached.presenter.layer().pixelFormat())
        }

        /// The surface's declared presentation format, through the real
        /// `CapturableSurface::capture_pixel_format`.
        pub fn capture_pixel_format(&self) -> objc2_metal::MTLPixelFormat {
            cocoa_ui::capture::CapturableSurface::capture_pixel_format(&*self.capturable)
        }

        /// Registers a waiter through the real
        /// `Capturable::register_waiter` — the same slot
        /// `wait_for_first_frames` polls into — returning the probe.
        pub fn readiness_probe(&self) -> WakeProbe {
            let count = Arc::new(AtomicU32::new(0));
            self.capturable
                .register_waiter(Waker::from(Arc::new(ProbeWake(count.clone()))));
            WakeProbe(count)
        }

        /// Uses a real `AppKit` window — the attach door's only host
        /// requirement — without installing the fixture presenter, for
        /// trials that drive `initialize_gpu` themselves.
        ///
        /// # Panics
        ///
        /// When the trial is not running on the main thread, or the
        /// fixture window never reports visible.
        pub fn ensure_fixture_window(&self) {
            use cocoa_ui::objc2_foundation::{NSDate, NSRunLoop};
            if self.fixture_window.borrow().is_some() {
                return;
            }
            let mtm = MainThreadMarker::new().expect("the fixture runs on the main thread");
            let window = cocoa_ui::appkit::Window::new(
                mtm,
                cocoa_ui::Rect::new(0.0, 0.0, 64.0, 64.0),
                cocoa_ui::appkit::WindowStyle::TITLED,
            );
            window.set_content_view(self.view.as_platform_view());
            window.make_key_and_order_front();
            *self.fixture_window.borrow_mut() = Some(window);
            // The occlusion state lands after the window server composites
            // — drive the run loop until it reports, bounded by a deadline.
            let deadline = NSDate::dateWithTimeIntervalSinceNow(5.0);
            while !self.view.is_visible() && deadline.timeIntervalSinceNow() > 0.0 {
                NSRunLoop::currentRunLoop()
                    .runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
            }
            assert!(self.view.is_visible(), "the fixture window reports visible");
        }

        /// Attaches through the production door: the fixture window makes
        /// the surface presentable, then `initialize_gpu` builds the real
        /// `MetalPresenter` — the same path a layout drives on a live
        /// mount.
        ///
        /// # Panics
        ///
        /// When the attach door does not install the presentation epoch.
        pub fn ensure_attached(&self) {
            self.ensure_fixture_window();
            self.initialize_gpu();
            assert!(
                self.state.presentation.borrow().attached().is_some(),
                "the attach door installs the Attached epoch"
            );
        }

        /// Sets the view's frame — the logical size `initialize_gpu`
        /// computes the drawable extent from (bounds follow frame for
        /// an untransformed view).
        pub fn set_view_frame(&self, width: f64, height: f64) {
            cocoa_ui::view::set_frame(
                self.view.as_platform_view(),
                cocoa_ui::Rect::new(0.0, 0.0, width, height),
            );
        }

        /// Drives the production `initialize_gpu` — the geometry and
        /// deferred-allocation door a layout or backing change runs.
        pub fn initialize_gpu(&self) {
            super::initialize_gpu(&self.state, &self.view);
        }

        /// The context generation the runtime currently publishes — the
        /// key the parked watches arm on.
        pub fn current_generation(&self) -> u64 {
            self.runtime.context().generation()
        }

        /// Whether the surface still offers its first frame to an
        /// enclosing capture — the same observable the capture walk
        /// reads.
        pub fn first_paint_participation(&self) -> bool {
            use cocoa_ui::capture::CapturableSurface;
            self.capturable.participates_in_first_paint()
        }

        /// Opens a real capture scope on the surface — the
        /// `CapturableSurface` call a `ViewCapture` issues.
        pub fn begin_capture(&self, on_redraw: Rc<dyn Fn()>) {
            use cocoa_ui::capture::CapturableSurface;
            self.capturable.begin_external_rendering(on_redraw);
        }

        /// Closes the outermost capture scope — the `CapturableSurface`
        /// call the capture issues when it finishes.
        ///
        /// # Panics
        ///
        /// When no scope is open — the production balance assert.
        pub fn end_capture(&self, resume: bool) {
            use cocoa_ui::capture::CapturableSurface;
            self.capturable.end_external_rendering(resume);
        }

        /// Drives the production `park_for_publication` — the settle the
        /// frame path runs when its context generation must rebuild —
        /// parked on `generation`.
        pub fn park_on(&self, generation: u64) {
            park_for_publication(&self.state, &self.view, generation);
        }

        /// Runs the main run loop until `condition` holds or `seconds`
        /// elapse — the loop the executor's parked tasks and the main
        /// queue's enqueued work both ride.
        pub fn pump_main(&self, seconds: f64, mut condition: impl FnMut() -> bool) -> bool {
            use cocoa_ui::objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
            let deadline = NSDate::dateWithTimeIntervalSinceNow(seconds);
            while !condition() && deadline.timeIntervalSinceNow() > 0.0 {
                // A run-mode turn only iterates when a source wakes it;
                // queueing a block is what wakes it, and the same turn then
                // drains every queued main-queue block — executor tasks
                // included.
                cocoa_ui::main_queue::enqueue(|_| {});
                // SAFETY: `NSDefaultRunLoopMode` is a system-owned mode.
                NSRunLoop::currentRunLoop().runMode_beforeDate(
                    unsafe { NSDefaultRunLoopMode },
                    &NSDate::dateWithTimeIntervalSinceNow(0.02),
                );
            }
            condition()
        }

        /// The view's current bounds size — what `initialize_gpu` reads.
        pub fn view_bounds(&self) -> cocoa_ui::geometry::Size {
            self.view.bounds_size()
        }

        /// The view's backing scale — the factor `initialize_gpu`
        /// converts bounds with.
        pub fn backing_scale(&self) -> Option<f64> {
            self.view.backing_scale()
        }

        /// Renders one external-capture frame into a live texture the
        /// context's own device allocates, declared at `width`×`height`
        /// pixels, inside an already-open capture scope, then waits for
        /// the submission's completion (bounded).
        ///
        /// Answers the completion's own result — `Ok(())` when the shared
        /// validity contract judges the live submission current.
        ///
        /// # Errors
        ///
        /// [`cocoa_ui::capture::CaptureError`] when the render or the
        /// live submission settles stale, failed or device-lost — the
        /// contract's own answer.
        ///
        /// # Panics
        ///
        /// When the surface refuses the render target, or the completion
        /// never lands.
        pub fn render_capture_frame(
            &self,
            width: u32,
            height: u32,
        ) -> Result<(), cocoa_ui::capture::CaptureError> {
            use cocoa_ui::capture::CapturableSurface;
            use objc2_metal::MTLDevice;
            let context = self.runtime.context();
            let device = crate::gpu_runtime::raw_metal_device(&context);
            // SAFETY: a 2D descriptor is always valid to construct. The
            // allocation matches the declared extent — `import_texture`'s
            // safety contract requires the declared dims to be the real
            // texture's.
            let descriptor = unsafe {
                objc2_metal::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    self.capturable.capture_pixel_format(),
                    width as usize,
                    height as usize,
                    false,
                )
            };
            descriptor.setUsage(
                objc2_metal::MTLTextureUsage::ShaderRead
                    | objc2_metal::MTLTextureUsage::RenderTarget,
            );
            let texture = device
                .newTextureWithDescriptor(&descriptor)
                .expect("a capture target texture");
            assert!(
                self.capturable.prepare_external_render(&texture),
                "the surface accepts the external render target"
            );
            let (tx, rx) = std::sync::mpsc::channel();
            self.capturable.render_prepared_external_texture(
                &texture,
                width,
                height,
                Box::new(move |result| {
                    let _ = tx.send(result);
                }),
            );
            // The completion hops to the main queue before answering —
            // run the loop in bounded turns until it lands.
            let mut received = None;
            assert!(
                self.pump_main(10.0, || {
                    if let Ok(result) = rx.try_recv() {
                        received = Some(result);
                    }
                    received.is_some()
                }),
                "the submission completion lands within the deadline"
            );
            received.expect("the completion was received")
        }

        /// Drives the real external-capture sequence —
        /// `begin_external_rendering` → `prepare_external_render` →
        /// `render_prepared_external_texture` → `end_external_rendering` —
        /// into a live texture the context's own device allocates, then
        /// turns the main run loop until the submission's
        /// `on_submitted_work_done` completion lands (bounded).
        ///
        /// Answers the completion's own result — `Ok(())` when the shared
        /// validity contract judges the live submission current.
        ///
        /// # Errors
        ///
        /// [`cocoa_ui::capture::CaptureError`] when the live submission
        /// settles stale, failed or device-lost — the contract's own
        /// answer.
        pub fn capture_external_once(&self) -> Result<(), cocoa_ui::capture::CaptureError> {
            self.ensure_attached();
            self.begin_capture(Rc::new(|| {}));
            let result = self.render_capture_frame(64, 64);
            self.end_capture(true);
            result
        }

        /// Drives the production `settle_failed` synchronously — the
        /// settle the `Err` arms of `render_into` and
        /// `render_prepared_external_texture` run on the main queue — for
        /// `generation`. The error is the same typed carrier a rejected
        /// surface extent produces (`HostedLayerError::Surface`).
        pub fn settle_failure(&self, generation: u64) {
            settle_failed(&self.state, &self.view, generation, self.fixture_failure());
        }

        /// Routes a batch failure through the production `failure_sink` —
        /// the channel `EngineGeneration::settle_failed` delivers through
        /// — so the owner's `settle_failed` lands on the next main-queue
        /// turn exactly as a routed failure does. Pump the main queue to
        /// land it.
        pub fn route_failure(&self, generation: u64) {
            self.state.failure_sink(&self.view, generation)(self.fixture_layer_failure());
        }

        /// The typed carrier a real surface rejection produces — the same
        /// `HostedLayerError::Surface` `resize` answers for an extent over
        /// the device maximum. No renderable texture input can produce an
        /// `Err` from `render_prepared_external_texture` (the platform's
        /// maximum texture dimension equals the surface's rejection
        /// bound), so trials drive the settle entries directly with this
        /// real carrier.
        fn fixture_layer_failure(&self) -> Arc<HostedLayerError> {
            // The real rejection bound: `resize`'s `TooLarge` reports the
            // live device's limit, not a constant — the two must agree or
            // the typed carrier diverges from the surface's own error.
            let max = self
                .state
                .runtime
                .context()
                .device()
                .limits()
                .max_texture_dimension_2d;
            Arc::new(HostedLayerError::Surface(SurfaceError::TooLarge {
                width: u32::MAX,
                height: u32::MAX,
                max,
            }))
        }

        /// The `HostedError` `settle_failed` takes for the fixture carrier.
        fn fixture_failure(&self) -> HostedError {
            HostedError::Scene(self.fixture_layer_failure())
        }

        /// The drawable pixel size configured on the live presenter —
        /// `None` while the surface carries no attach: a genuinely empty
        /// view presents nothing at all.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a layer drawableSize fits in u32 and cannot be negative"
        )]
        pub fn drawable_size(&self) -> Option<(u32, u32)> {
            let presentation = self.state.presentation.borrow();
            let attached = presentation.attached()?;
            let size = attached.presenter.layer().drawableSize();
            Some((size.width.max(0.0) as u32, size.height.max(0.0) as u32))
        }
    }
}
