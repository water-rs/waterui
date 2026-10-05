//! The `gpu_surface` leaf: `Native<GpuContentView>` and
//! `Native<ExternalFrameView>` and `Native<SceneView>` as a kit surface view
//! presenting through a `CAMetalLayer` driven by a `CAMetalDisplayLink` —
//! the platform's own presentation primitive (#1683).
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
    GpuContentView, GpuRuntime, RedrawHandle, SharedGpuContext,
};
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::offscreen::OffscreenSize;
use waterui_graphics::wgpu;

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

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
    fn renderer(
        &mut self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
    ) -> Box<dyn HostedRenderer>;
}

/// The engine half of a mounted GPU surface: one device generation's layer,
/// rebuilt from the [`HostedView`] after device loss.
trait HostedRenderer {
    /// The context generation the renderer was built under.
    fn generation(&self) -> u64;
    /// Renders and composites into the host's texture. `time` is the
    /// frame's target presentation timestamp — animations key to the moment
    /// the frame is displayed, not to `now`.
    fn present(&mut self, target: &wgpu::Texture, display: Display, time: FrameTime) -> Next;
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
    ) -> Box<dyn HostedRenderer> {
        // `engine_content` answers the same content object every time, so a
        // renderer rebuilt after device loss re-installs it with its state.
        let redraw = redraw.clone();
        let producer = waterui_graphics::cherenkov_gpu::interop::GpuContentBox::new(
            self.engine_content(),
            move || redraw.request_redraw(),
        );
        Box::new(GpuContentRenderer::new(
            runtime,
            context.clone(),
            producer,
            size,
        ))
    }
}

impl HostedRenderer for GpuContentRenderer {
    fn generation(&self) -> u64 {
        Self::generation(self)
    }

    fn present(&mut self, target: &wgpu::Texture, display: Display, _time: FrameTime) -> Next {
        // The shared graphics contract carries no target time yet — the GPU
        // content renderer keeps its signature and ignores it (#1725).
        Self::present(self, target, display)
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
    ) -> Box<dyn HostedRenderer> {
        let stream: ExternalFrameStream = self.stream();
        Box::new(ExternalFrameRenderer::new(
            runtime,
            context.clone(),
            &stream,
            size,
            redraw.clone(),
        ))
    }
}

impl HostedRenderer for ExternalFrameRenderer {
    fn generation(&self) -> u64 {
        Self::generation(self)
    }

    fn present(&mut self, target: &wgpu::Texture, display: Display, _time: FrameTime) -> Next {
        // The shared graphics contract carries no target time yet — the
        // external frame renderer keeps its signature and ignores it (#1725).
        Self::present(self, target, display)
    }
}

/// Everything the leaf owns for one mounted hosted view — the
/// `WuiGpuSurfaceState` equivalent, plus the presentation resources the
/// Swift side split across its own files.
struct SurfaceState {
    /// The environment's GPU runtime; `context()` follows device rebuilds.
    runtime: GpuRuntime,
    /// The hosted semantic view.
    view: RefCell<Box<dyn HostedView>>,
    /// The view's engine layer on the runtime's current context generation.
    renderer: RefCell<Option<Box<dyn HostedRenderer>>>,
    /// The format the content was set up for; `None` until the first target
    /// declares one. It never changes afterwards.
    format: Cell<Option<wgpu::TextureFormat>>,
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
    /// Whether the content asked for a frame since the last render.
    dirty: Cell<bool>,
    /// The `CAMetalLayer` presenter — created on attach, dropped on detach.
    presenter: RefCell<Option<MetalPresenter>>,
    /// Outstanding capture-suppression scopes — the presentation layer is
    /// hidden while nonzero (`beginCaptureSuppression`).
    capture_suppression: Cell<u32>,
    /// Outstanding external-rendering scopes; while nonzero the surface's
    /// redraws go to `external_redraw` and it presents nowhere itself.
    external_count: Cell<u32>,
    /// The redraw target external capture installed.
    external_redraw: RefCell<Option<Rc<dyn Fn()>>>,
    /// A frame asked for while one could not be drawn — owed, not dropped;
    /// the next link update answers it.
    frame_owed: Cell<bool>,
    /// The attach-time first frame a window's reveal waits on — the retired
    /// `force` render: it draws through the visibility gate exactly once so
    /// a window ordered at alpha 0 still receives its first presented
    /// frame. Cleared by the `PresentedFrame` receipt and by detach.
    first_paint_owed: Cell<bool>,
    /// Whether the link should keep delivering updates — `keepRedrawing`.
    keep_redrawing: Cell<bool>,
    /// The environment's shared presentation-time anchor — link update
    /// timestamps map through it to `Instant`.
    presentation_time: Rc<crate::presentation_time::PresentationTime>,
    /// The environment's capture registry — this leaf registers/unregisters
    /// on mount/unmount.
    registry: Rc<crate::capture_registry::CaptureRegistry>,
    /// Whether at least one frame has been presented on-screen — first-paint
    /// readiness.
    presented_once: Cell<bool>,
    /// The explicit HDR preference `resolved_hdr_preference` produced.
    explicit_range: Option<cocoa_ui::dynamic_range::DynamicRange>,
    /// `rendererDynamicRange`'s latch: wgpu keeps the negotiated format
    /// across attach cycles, so the first answer stands.
    latched_renderer_range: Cell<Option<cocoa_ui::dynamic_range::DynamicRange>>,
    /// The presentation range `applyDynamicRange` was last run with.
    configured_range: Cell<Option<cocoa_ui::dynamic_range::DynamicRange>>,
    /// Whether the presenter and renderer are attached for this window.
    attached: Cell<bool>,
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
    /// The context generation `buffers` was built under — the `IOSurface`
    /// ring is recreated when the runtime publishes a new context.
    gpu_generation: Cell<Option<u64>>,
    /// The parked wait on the next context publication, armed while a
    /// frame's context reported device loss. Stored so a newer wait
    /// replaces it and dropping the state cancels it.
    context_watch: RefCell<Option<executor_core::AnyLocalExecutorTask<()>>>,
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
        // blocking on `exec_sync` (#1776).
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
            view: RefCell::new(view),
            renderer: RefCell::new(None),
            format: Cell::new(None),
            current_width: Cell::new(0),
            current_height: Cell::new(0),
            redraw_handle,
            wants_input_events,
            dirty: Cell::new(false),
            presenter: RefCell::new(None),
            capture_suppression: Cell::new(0),
            external_count: Cell::new(0),
            external_redraw: RefCell::new(None),
            frame_owed: Cell::new(false),
            first_paint_owed: Cell::new(false),
            keep_redrawing: Cell::new(false),
            presentation_time: crate::presentation_time::PresentationTime::get(env),
            registry: crate::capture_registry::CaptureRegistry::get(env),
            presented_once: Cell::new(false),
            explicit_range,
            latched_renderer_range: Cell::new(None),
            configured_range: Cell::new(None),
            attached: Cell::new(false),
            presentation_format: Cell::new(None),
            current_scale: Cell::new(1.0),
            ready_waiters: RefCell::new(Vec::new()),
            needs_a11y_refresh: Cell::new(true),
            observers: RefCell::new(Vec::new()),
            last_proposal: Cell::new(None),
            last_resolved_size: RefCell::new(None),
            gpu_generation: Cell::new(None),
            context_watch: RefCell::new(None),
        }
    }

    /// Latches the target format — the ffi `prepare_format`: the first
    /// target's format stands for the surface's lifetime.
    fn prepare_format(&self, format: wgpu::TextureFormat) {
        if let Some(existing) = self.format.get() {
            assert_eq!(existing, format, "native target format changed after setup");
        } else {
            self.format.set(Some(format));
            self.redraw_handle.request_redraw();
        }
    }

    /// Renders through the retained engine and presents its composed texture
    /// — the ffi `render_into`.
    ///
    /// The renderer — engine, surface and the view's layer — is built lazily
    /// on the first frame and rebuilt from the view whenever the runtime's
    /// context generation moves on.
    fn render_into(
        &self,
        context: &Arc<SharedGpuContext>,
        texture: &wgpu::Texture,
        (width, height): (u32, u32),
        display: Display,
        time: FrameTime,
    ) -> bool {
        self.dirty.set(false);
        self.view.borrow().before_frame();
        // A renderer bound to a context generation that has since been lost
        // and rebuilt holds a dead device; recreate it on `context`, the
        // generation this frame is rendering under.
        let generation = context.generation();
        let mut slot = self.renderer.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|renderer| renderer.generation() != generation)
        {
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(
                self.view.borrow_mut().renderer(
                    &self.runtime,
                    context,
                    &self.redraw_handle,
                    OffscreenSize::try_from_pixels(width, height)
                        .expect("native target must be nonempty"),
                ),
            );
        }
        let renderer = slot.as_mut().expect("renderer created above");
        renderer.present(texture, display, time) != Next::Idle || self.dirty.get()
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
    },
}

/// Imports an `MTLTexture` as a wgpu texture, renders a frame into it and
/// submits; reports whether the frame submitted or the context is dead — the
/// `waterui_gpu_content_render_to_metal_texture` half of the ffi entry
/// point.
#[expect(
    clippy::too_many_arguments,
    reason = "the ffi entry point's parameters arrive as one call, not a config"
)]
fn render_to_metal_texture(
    state: &Rc<SurfaceState>,
    view: &Retained<SurfaceView>,
    context: &Arc<SharedGpuContext>,
    metal_texture: Retained<objc2::runtime::ProtocolObject<dyn MTLTexture>>,
    width: u32,
    height: u32,
    scale: f64,
    time: FrameTime,
) -> FrameRender {
    let format = metal_texture_format(&metal_texture);
    state.prepare_format(format);
    if context.device_lost_reason().is_some() {
        return FrameRender::PendingRebuild;
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
    FrameRender::Submitted {
        needs_redraw: state.render_into(context, &wgpu_texture, (width, height), display, time),
    }
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
        !state.attached.get(),
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
    // The `CAMetalLayer` carries the same contract the `IOSurface` ring
    // presented at: the negotiated pixel format, the surface colour space
    // and the EDR flag. Applying them to a live presenter retires every
    // frame its old configuration issued.
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        let layer = presenter.layer();
        layer.setPixelFormat(format);
        let colorspace = cocoa_ui::metal::color_space(format);
        layer.setColorspace(Some(&colorspace));
        layer.setWantsExtendedDynamicRangeContent(
            presentation == cocoa_ui::dynamic_range::DynamicRange::High,
        );
        presenter.advance_generation();
    }
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

/// Whether the frame clock ticks — `isEffectivelyVisible`: narrower than
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
        // A window that is on no display cannot present; the frame clock
        // falls back to the run loop for those, so gating here keeps an
        // offscreen window from rendering frames nobody sees.
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
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn initialize_gpu(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    let bounds = view.bounds_size();
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return;
    }
    let Some(scale) = view.backing_scale() else {
        return;
    };

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
    let width = (bounds.width * scale) as u32;
    let height = (bounds.height * scale) as u32;
    let size_changed = state.current_width.get() != width || state.current_height.get() != height;
    state.current_width.set(width);
    state.current_height.set(height);
    if size_changed {
        state.keep_redrawing.set(true);
    }
    update_presentation_frame(state, view);
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        presenter.set_drawable_size(cocoa_ui::metal_presenter::drawable_size(width, height));
    }

    // The presenter is created where a frame could actually be shown:
    // laying out a covered window bought a link and drawables for frames
    // that never came.
    if !can_present_now(view) {
        return;
    }
    if state.attached.get() {
        return;
    }
    attach(state, view);
}

/// Builds the `CAMetalLayer` presenter on the current context and owes the
/// first frame — the link's next update answers it.
fn attach(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    let layer = view.presentation_layer();
    let weak = Rc::downgrade(state);
    let frame_view = view.clone();
    let presenter = MetalPresenter::new(
        layer,
        Rc::new(move |frame| {
            if let Some(state) = weak.upgrade() {
                render_drawable(&state, &frame_view, frame);
            }
        }),
    );
    let context = state.runtime.context();
    let device = crate::gpu_runtime::raw_metal_device(&context);
    presenter.set_device(&device);
    state.gpu_generation.set(Some(context.generation()));
    presenter.set_drawable_size(cocoa_ui::metal_presenter::drawable_size(
        state.current_width.get(),
        state.current_height.get(),
    ));
    // `configure_dynamic_range` latches the range without a live presenter;
    // a fresh layer still needs the negotiated format/colour-space/EDR.
    let configured = state.configured_range.get();
    state.configured_range.set(None);
    if let Some(range) = configured {
        configure_dynamic_range(state, view, range, renderer_dynamic_range(state, range));
    }
    if let Some(maximum) = display_rate(view) {
        presenter.set_display_rate(maximum);
    }
    *state.presenter.borrow_mut() = Some(presenter);
    state.attached.set(true);
    // The first update answers the owed first frame — the window's reveal
    // waits on it. The owed first paint draws through the visibility gate
    // exactly once (the retired `force` render) so an alpha-0 reveal window
    // still gets its frame; a presented surface re-attaching owes only the
    // ordinary frame.
    state.frame_owed.set(true);
    // Only a first-paint participant owes the reveal: a surface mounted
    // behind hidden or zero-alpha ancestors parks until it is shown.
    state
        .first_paint_owed
        .set(!state.presented_once.get() && participates_in_first_paint(view));
    update_presentation_demand(state, view);
}

/// The display's maximum frame rate for `view`'s current screen — the cadence
/// the retired `FrameClock` requested.
fn display_rate(view: &Retained<SurfaceView>) -> Option<f32> {
    let window = cocoa_ui::view::window(view.as_platform_view())?;
    #[cfg(target_os = "macos")]
    {
        window.screen().map(|screen| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "frame rates fit comfortably in f32"
            )]
            let rate = screen.maximumFramesPerSecond().max(1) as f32;
            rate
        })
    }
    #[cfg(target_os = "ios")]
    {
        window.windowScene().map(|scene| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "frame rates fit comfortably in f32"
            )]
            let rate = scene.screen().maximumFramesPerSecond().max(1) as f32;
            rate
        })
    }
}

/// Detaches presenter and renderer for a window or range change: dropping
/// the presenter invalidates its link, and a stale frame's lease release
/// can never reach the next presenter's pool.
fn detach_if_attached(state: &SurfaceState) {
    if !state.attached.get() {
        return;
    }
    state.attached.set(false);
    state.presenter.borrow_mut().take();
    // The presentation epoch ends with the presenter: readiness re-arms and
    // the next attach forces one frame through the gates again.
    state.first_paint_owed.set(false);
    state.presented_once.set(false);
}

// MARK: - Frame scheduling (WuiDisplayLinkDriver + WuiRedrawCallback)

/// Re-runs `initialize_gpu` once a frame could be shown again, then drives
/// the clock and replays an owed frame — `updateDisplayLinkState`.
fn update_presentation_demand(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    if !state.attached.get() && can_present_now(view) {
        initialize_gpu(state, view);
    }
    // Demand is `keepRedrawing`, an owed frame or a dirty redraw — the link
    // unpauses only while all visibility gates hold and answers the demand
    // at the next drawable delivery, at most one display interval later.
    let demand = state.attached.get()
        && state.external_count.get() == 0
        && (is_effectively_visible(view) || state.first_paint_owed.get())
        && (state.keep_redrawing.get()
            || state.frame_owed.get()
            || state.dirty.get()
            || state.first_paint_owed.get());
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        presenter.set_paused(!demand);
    }
}

/// `ensurePresenterGeneration` — `buffers` is an `IOSurface` ring on the
/// context's Metal device; recreate it when the runtime publishes a new
/// context generation (the ring predates the new context even when the
/// underlying `MTLDevice` is unchanged).
fn ensure_presenter(state: &SurfaceState, context: &Arc<SharedGpuContext>) {
    if state.gpu_generation.get() == Some(context.generation()) {
        return;
    }
    // Context replacement keeps the layer and the link; the device swap
    // advances the presenter generation so frames from the old context
    // can never present.
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        let device = crate::gpu_runtime::raw_metal_device(context);
        presenter.set_device(&device);
    }
    state.gpu_generation.set(Some(context.generation()));
}

/// `awaitNextContextGeneration` — parks the surface until the runtime
/// publishes a context newer than the lost one, the publication wake a
/// lost-context frame is owed. Stored on the state so replacing the wait
/// or dropping the state cancels it.
fn arm_context_watch(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>, generation: u64) {
    let runtime = state.runtime.clone();
    let weak = Rc::downgrade(state);
    let view = view.clone();
    *state.context_watch.borrow_mut() = Some(executor_core::spawn_local(async move {
        let _published = runtime.context_after(generation).await;
        if let Some(state) = weak.upgrade() {
            if state.external_count.get() > 0 {
                // Externally rendered surfaces own no presentation to
                // replay: the owed frame replays by asking the enclosing
                // capture for a fresh frame through the redraw contract.
                notify_external_redraw(&state);
            } else {
                // The owed frame replays as demand — the next link update
                // answers it, ticking or not.
                state.frame_owed.set(true);
                update_presentation_demand(&state, &view);
            }
        }
    }));
}

/// One issued `DrawableFrame`: render into it, submit, and on completion
/// present through the receipt contract — `renderFrame` as a link callback.
fn render_drawable(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>, frame: DrawableFrame) {
    if state.external_count.get() > 0 {
        // Externally rendered surfaces present nowhere themselves: the
        // delivered drawable is released unpresented and the redraw goes to
        // the enclosing capture.
        drop(frame);
        notify_external_redraw(state);
        return;
    }
    if !is_effectively_visible(view) && !state.first_paint_owed.get() {
        state.frame_owed.set(true);
        return;
    }

    // One context generation covers the render, the completion marker, and
    // the callback registration — a rebuild mid-frame cannot split them.
    let context = state.runtime.context();
    ensure_presenter(state, &context);

    let (width, height) = frame.drawable_size();
    let time = state.presentation_time.map(frame.target_time());
    let FrameRender::Submitted { needs_redraw } = render_to_metal_texture(
        state,
        view,
        &context,
        frame.texture(),
        width,
        height,
        state.current_scale.get(),
        time,
    ) else {
        // The lost context never receives work again; the dropped frame
        // releases its lease, the frame is owed, and publication replays it
        // through `update_presentation_demand`.
        drop(frame);
        state.frame_owed.set(true);
        arm_context_watch(state, view, context.generation());
        return;
    };

    state.keep_redrawing.set(needs_redraw);
    publish_content_accessibility(state, view);
    update_presentation_demand(state, view);

    let weak = Sendable(Rc::downgrade(state));
    let view = Sendable(view.clone());
    // `Sendable` carries the lease across the completion-driver hop; the
    // main-queue closure takes it back out whole so `present` can consume
    // it — the completion fires exactly once.
    let mut frame_slot = Some(Sendable(frame));
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
        move || {
            cocoa_ui::main_queue::enqueue(move |_mtm| {
                let Some(state) = weak.get().upgrade() else {
                    return;
                };
                let Sendable(frame) = frame_slot.take().expect("completion fires once");
                if submitted_context.device_lost_reason().is_some() {
                    // The submitted generation died in flight — the slot
                    // holds no ready pixels; park until publication replays
                    // the owed frame on a live context.
                    state.frame_owed.set(true);
                    arm_context_watch(&state, view.get(), submitted_context.generation());
                    return;
                }
                // `present` returns the receipt only when the frame is this
                // presenter's current generation and configuration — a
                // late or stale frame settles its lease and shows nothing.
                let presented = state
                    .presenter
                    .borrow()
                    .as_ref()
                    .and_then(|presenter| presenter.present(frame));
                if let Some(_receipt) = presented {
                    // A presented frame makes this generation productive —
                    // the runtime's unproductive-loss detector keys on it.
                    submitted_context.note_frame_presented();
                    state.first_paint_owed.set(false);
                    state.presented_once.set(true);
                    complete_ready(&state, true);
                } else {
                    // The presenter moved on while this frame was in flight —
                    // owe it again rather than reveal a hole.
                    state.frame_owed.set(true);
                }
                update_presentation_demand(&state, view.get());
            });
        },
    );
}

/// `handleRedrawRequest`: the redraw waker's main-queue body — republishes
/// accessibility, re-measures against the last proposal, then renders.
fn handle_redraw_request(state: &Rc<SurfaceState>, view: &Retained<SurfaceView>) {
    state.dirty.set(true);
    state.needs_a11y_refresh.set(true);
    if take_measurement_invalidation(state) {
        crate::invalidation::invalidate_layout_hierarchy(view.as_platform_view());
    }
    if state.external_count.get() > 0 {
        notify_external_redraw(state);
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
    #[cfg(target_os = "macos")]
    {
        cocoa_ui::view::set_accessibility_content(view, label, value);
    }
    #[cfg(target_os = "ios")]
    {
        cocoa_ui::view::set_accessibility_content(view, label, value);
    }
}

/// The redraw callback's target — external capture gets the call while it
/// owns rendering, otherwise the main queue drives `handleRedrawRequest`.
fn notify_external_redraw(state: &SurfaceState) {
    if let Some(callback) = state.external_redraw.borrow().as_ref() {
        callback();
    }
}

/// Runs the frame after which `ready` waiters wake — `completeReady`.
fn complete_ready(state: &SurfaceState, _presented: bool) {
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
    let host = view.clone();
    input.set_event_handler(move |event| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let event = crate::gpu_input::translate(&event);
        state.view.borrow().input(&event);
        // The event's frame request rides the same coalescing as a redraw
        // request.
        if state.external_count.get() > 0 {
            notify_external_redraw(&state);
        } else {
            state.keep_redrawing.set(true);
            update_presentation_demand(&state, &host);
        }
    });
    input.set_caret_provider({
        let state = state.clone();
        move || {
            state.view.borrow().ime_caret().map(|rect| {
                cocoa_ui::Rect::new(
                    rect.origin().x,
                    rect.origin().y,
                    rect.size().width,
                    rect.size().height,
                )
            })
        }
    });
    // The responder fills the surface and sits on top of it.
    cocoa_ui::view::add_subview(view.as_platform_view(), input.as_ref());
    Some(Retained::into_super(input))
}

// MARK: - Capturable (WuiMetalViewCapture's surface half)

/// The registered surface, for `ViewCapture`'s resolver and `view.ready()`.
struct Capturable {
    state: Rc<SurfaceState>,
    view: Retained<SurfaceView>,
}

impl Capturable {
    /// Whether one on-screen frame has landed — readiness comes only from a
    /// `PresentedFrame` receipt, never from an offscreen capture completion.
    fn presented(&self) -> bool {
        self.state.presented_once.get()
    }

    /// Whether this surface gates its window's first-paint readiness —
    /// see [`participates_in_first_paint`].
    fn participates(&self) -> bool {
        participates_in_first_paint(&self.view)
    }

    /// Registers `waker` and requests the owed first frame — the retired
    /// `request_ready_frame`: lay out, make sure the GPU context is up, arm
    /// the reveal wait so the link answers it instead of parking forever.
    fn register_waiter(&self, waker: std::task::Waker) {
        if self.presented() {
            return;
        }
        cocoa_ui::view::layout_immediately(self.view.as_platform_view());
        self.state.ready_waiters.borrow_mut().push(waker);
        self.state.first_paint_owed.set(true);
        self.state.frame_owed.set(true);
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
        if self.state.external_count.get() == 0 {
            *self.state.external_redraw.borrow_mut() = Some(on_redraw);
            self.state.keep_redrawing.set(false);
            if let Some(presenter) = self.state.presenter.borrow().as_ref() {
                presenter.set_paused(true);
            }
        }
        self.state
            .external_count
            .set(self.state.external_count.get() + 1);
    }

    fn end_external_rendering(&self, resume: bool) {
        let count = self.state.external_count.get();
        assert!(
            count > 0,
            "GpuSurface external rendering scopes are unbalanced"
        );
        self.state.external_count.set(count - 1);
        if count == 1 {
            *self.state.external_redraw.borrow_mut() = None;
            if resume {
                self.state.frame_owed.set(true);
                update_presentation_demand(&self.state, &self.view);
            }
        }
    }

    fn prepare_external_render(
        &self,
        texture: &objc2::runtime::ProtocolObject<dyn MTLTexture>,
    ) -> bool {
        self.state.prepare_format(metal_texture_format(texture));
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

    fn render_prepared_external_texture(
        &self,
        texture: &objc2::runtime::ProtocolObject<dyn MTLTexture>,
        width: u32,
        height: u32,
        completion: cocoa_ui::capture::SurfaceCaptureCompletion,
    ) {
        // SAFETY: `texture` is the live texture the capture pipeline retained
        // for this call; `retain` takes our own reference.
        let texture = unsafe {
            Retained::<objc2::runtime::ProtocolObject<dyn MTLTexture>>::retain(
                std::ptr::from_ref(texture).cast_mut(),
            )
        }
        .expect("GpuSurface external render received a null texture");
        let context = self.state.runtime.context();
        let FrameRender::Submitted { .. } = render_to_metal_texture(
            &self.state,
            &self.view,
            &context,
            texture,
            width,
            height,
            self.state.current_scale.get(),
            self.state.presentation_time.capture_time(),
        ) else {
            // The capture's deferred frame replays through the redraw
            // contract: publication resolves the watch, and an externally
            // rendered surface turns it into a redraw notification to the
            // enclosing capture — the parent's owed frame then re-renders.
            arm_context_watch(&self.state, &self.view, context.generation());
            completion(Err(cocoa_ui::capture::CaptureDeferred));
            return;
        };
        let weak = Sendable(Rc::downgrade(&self.state));
        let view = Sendable(self.view.clone());
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
            move || {
                // `on_submitted_work_done` closures can run on any thread
                // that maintains the device — hop to the main queue before
                // touching the weak state handle.
                cocoa_ui::main_queue::enqueue(move |_mtm| {
                    // The generation that carried this frame was lost in
                    // flight: the fence settles, but there are no usable
                    // pixels to compose. Arm publication so the redraw
                    // contract wakes the parent once a live context lands.
                    if submitted_context.device_lost_reason().is_some() {
                        if let Some(state) = weak.get().upgrade() {
                            arm_context_watch(&state, view.get(), submitted_context.generation());
                        }
                        completion(Err(cocoa_ui::capture::CaptureDeferred));
                        return;
                    }
                    // An offscreen capture completion is not a presentation
                    // receipt: it never mints readiness and never feeds the
                    // unproductive-loss detector — `PresentedFrame` alone
                    // does both.
                    completion(Ok(()));
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

/// Dropping unregisters the surface — `deinit`/`shutdown` on the Swift side.
struct RegistryGuard {
    view: Retained<SurfaceView>,
    state: Rc<SurfaceState>,
    input: Option<Retained<cocoa_ui::PlatformView>>,
}

impl fmt::Debug for RegistryGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistryGuard").finish_non_exhaustive()
    }
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        self.state.presenter.borrow_mut().take();
        self.state.observers.borrow_mut().clear();
        self.state.view.borrow_mut().unmount();
        drop(self.state.renderer.borrow_mut().take());
        if let Some(input) = &self.input {
            cocoa_ui::view::remove_from_superview(input);
        }
        // Remove callbacks that retain the mount and its native view.
        self.view.set_layout_handler(|| {});
        self.view.set_window_changed_handler(|| {});
        self.view.set_backing_changed_handler(|| {});
        self.view.set_visibility_changed_handler(|| {});
        self.state
            .registry
            .remove_capturable(self.view.as_platform_view());
    }
}

/// Waits until every mounted capturable output inside `view`'s subtree —
/// GPU surfaces and filtered outputs alike — has presented a frame:
/// `WuiAnyView.ready()`.
#[allow(clippy::future_not_send)]
pub async fn wait_for_first_frames(
    view: &cocoa_ui::PlatformView,
    env: &waterui_backend_core::Environment,
) {
    let registry = crate::capture_registry::CaptureRegistry::get(env);
    core::future::poll_fn(|cx| {
        let mut pending = false;
        registry.collect_capturables(view, &mut |surface| {
            // Non-participants (a hidden, clipped or unpresentable surface)
            // never gate the reveal — and never owe a frame.
            if surface.participates_in_first_paint() && !surface.has_presented_frame() {
                surface.register_ready_waiter(cx.waker().clone());
                pending = true;
            }
        });
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
        build_surface(scene::Scene::new(view), ctx)
    });
}

/// The leaf construction the dispatcher's GPU-surface claims share —
/// `GpuContentView`, `ExternalFrameView`, and `SceneView`.
fn build_surface<V: HostedView + 'static>(
    view: V,
    ctx: &crate::contract::RenderContext<'_>,
) -> NativeLeaf {
    {
        let mtm = ctx.mtm();
        let platform_view = SurfaceView::new(mtm);
        let runtime = crate::gpu_runtime::runtime(ctx.env());
        let env = ctx.env();
        let state = Rc::new_cyclic(|weak| {
            SurfaceState::new(weak, runtime, Box::new(view), &platform_view, mtm, env)
        });
        state.view.borrow_mut().mount(&state.redraw_handle);

        // Registry: captures resolve surfaces by their platform view. The
        // mount guard owns removal; entries are weak.
        let capturable: Rc<dyn cocoa_ui::capture::CapturableSurface> = Rc::new(Capturable {
            state: state.clone(),
            view: platform_view.clone(),
        });
        state
            .registry
            .insert_capturable(platform_view.as_platform_view(), &capturable);

        wire_view_handlers(&platform_view, &state);
        let input_view = install_input(&platform_view, &state);

        let registry_guard = RegistryGuard {
            view: platform_view.clone(),
            state: state.clone(),
            input: input_view.clone(),
        };
        let mut leaf = NativeLeaf::new(&platform_view, SurfaceSubView { state });
        leaf.keep(platform_view);
        leaf.keep(capturable);
        leaf.keep(registry_guard);
        if let Some(input) = input_view {
            leaf.keep(input);
        }
        leaf
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
                complete_ready(&state, false);
                state.keep_redrawing.set(false);
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
