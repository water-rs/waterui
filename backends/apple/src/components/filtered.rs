//! The filtered-content leaf: `Native<FilteredView>` — the
//! `WuiAppliedFilter`/`WuiViewEffect` port. The hidden content view is
//! captured offscreen through the kit's `ViewCapture`; each erased effect in
//! the chain encodes into one shared command buffer — innermost first,
//! chained through private intermediate targets — and the last pass lands in
//! the output view's `CAMetalLayer`, driven by a `CAMetalDisplayLink`
//! through `cocoa_ui::metal_presenter`. The output registers as a
//! `CapturableSurface` on its host view's slot, so a nested
//! filter never captures a blank Metal layer.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cocoa_ui::PlatformView;
use cocoa_ui::Retained;
use cocoa_ui::capture::{CapturableSurface, CaptureError, SurfaceCaptureCompletion};
use cocoa_ui::metal_presenter::{DrawableFrame, MetalPresenter};
use executor_core::spawn_local;
use futures::FutureExt;
use objc2::MainThreadMarker;
use objc2_metal::{MTLDevice as _, MTLTexture as _};
use objc2_quartz_core::CAMetalLayer;
use waterui_backend_core::{AnyView, View};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_graphics::filter_view::{AnyEffect, ErasedEffect, FilteredView, ParamGuards};
use waterui_graphics::filtrate::{
    EffectContext, EffectFrameClock, EffectFrameTiming, EffectInput, EffectOutput,
    EffectRedrawCallback, ShapeTextures,
};
use waterui_graphics::gpu::{GpuRuntime, SharedGpuContext};
use waterui_graphics::wgpu;

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::dispatch::{Dispatcher, is_native_boundary};

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::HostView;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::HostView;
}

use platform::HostView;

/// The format an Apple host captures and renders filtered content in —
/// `attach_host_textures` always takes the extended-range half-float target,
/// exactly as the `CAMetalLayer` path's unconditional `rendererMode: .high`
/// did.
const PRESENTATION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// One display-bound frame in flight: the drawable lease and the exact
/// context its capture was prepared under, carried together so the
/// completion never encodes, imports or presents on a re-fetched
/// generation.
struct PresentWork {
    /// The link-issued drawable lease — held across the hidden-content
    /// capture, the effect encode, the submission and its completion.
    /// Dropping it releases the lease without presenting.
    frame: DrawableFrame,
    /// The live context the frame was issued under.
    context: Arc<SharedGpuContext>,
    /// The exact native texture the child content was captured into for
    /// this frame — carried on the work item so an async resize or a
    /// context replacement can never pair a resized/new-generation slot
    /// with this frame's captured pixels.
    input: Retained<MetalTexture>,
    /// The drawable texture's actual pixel size, as issued.
    width: u32,
    height: u32,
}

/// Marks a value as crossing the GPU-completion → main-queue boundary; the
/// wrapped object is only dereferenced once the work item lands on the main
/// thread, where every receiver was created.
struct Sendable<T>(T);

impl<T> Sendable<T> {
    /// Reads the wrapped value — method access keeps closure captures on the
    /// whole cell, where the `Send`/`Sync` contract lives.
    const fn get(&self) -> &T {
        &self.0
    }
}

// SAFETY: the payload is only touched on the main thread: Metal completion
// handlers and `enqueue` consumers of these values both land there.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Send for Sendable<T> {}
// SAFETY: `&Sendable<T>` shared with the main queue is only read on it.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Sync for Sendable<T> {}

type MetalTexture = objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>;
type MetalDevice = objc2::runtime::ProtocolObject<dyn objc2_metal::MTLDevice>;

/// The erased effect chain the async setup publishes into: `None` while it
/// owns the Vec, `Some` once every effect finished `setup` on the live
/// context — innermost (content-adjacent) effect first.
type EffectSlot = Rc<RefCell<Option<Vec<Box<dyn ErasedEffect>>>>>;

/// A compositor target `prepare_external_render` bound, imported for wgpu
/// under the context `generation`.
#[derive(Clone)]
struct ExternalTarget {
    texture: Retained<MetalTexture>,
    output: wgpu::Texture,
    generation: u64,
}

/// What the external render that follows a preparation draws into its
/// target — decided at preparation from the view's state.
#[derive(Clone)]
enum ExternalRender {
    /// The effect chain over the captured content.
    Effects(ExternalTarget),
    /// A collapsed view has no content to capture: the producer texture is
    /// cleared to transparent and composited source-over, so the view
    /// contributes nothing to its region of the parent's target.
    Transparent(ExternalTarget),
}

impl ExternalRender {
    const fn target(&self) -> &ExternalTarget {
        match self {
            Self::Effects(target) | Self::Transparent(target) => target,
        }
    }
}

/// The filter host's shared state — `WuiAppliedFilterRenderState` plus the
/// view's frame bookkeeping.
pub struct FilteredState {
    /// The host view.
    view: Retained<HostView>,
    /// The output presentation view.
    output_view: Retained<PlatformView>,
    /// The hidden content view the capture reads — the same view
    /// `capture` was built over, framed with the output in one pass.
    content_view: Retained<PlatformView>,
    /// The presentation `CAMetalLayer` — a sublayer of `output_view`'s
    /// backing layer, persistent across presenters: the link is
    /// per-attach, the layer is not.
    presentation_layer: Retained<CAMetalLayer>,
    /// The shared Metal device — `metalDevice`. Recreated when the
    /// runtime publishes a new context generation.
    device: RefCell<Retained<MetalDevice>>,
    /// The erased effect chain, innermost (content-adjacent) first; `None`
    /// while asynchronous setup owns it — the ffi state's effect slot.
    effects: EffectSlot,
    /// The GPU runtime.
    runtime: GpuRuntime,
    /// The host-owned effect clock — `frame_clock` on the ffi state. It
    /// measures effect time only; it does not drive presentation.
    frame_clock: RefCell<EffectFrameClock>,
    /// Whether the host produced no frame since the effect timeline last
    /// went idle. A parked timeline's next `tick` measures the idle
    /// wall-clock gap rather than frame cadence, so the encode path primes
    /// the clock once before sampling the resumed frame.
    timeline_parked: Cell<bool>,
    /// The context generation the installed effects finished setup on —
    /// `setup_ready` keyed to the resources' generation. A frame may only
    /// encode through an effect bundle when this equals `gpu_generation`.
    setup_generation: Rc<Cell<Option<u64>>>,
    /// The imported capture texture of the live frame — `imported_texture`.
    imported_texture: RefCell<Option<wgpu::Texture>>,
    /// Attach-time input size — `input_width`/`input_height`; the output
    /// size equals it — the effects render at capture dimensions.
    input_size: Cell<(u32, u32)>,
    /// `isAttached`.
    attached: Cell<bool>,
    /// The display-link presenter on the output `CAMetalLayer` —
    /// `presenter`. `Some` only while attached: detach, drop and window
    /// change invalidate the link and retire it, and the next attach
    /// builds a fresh one on the same layer.
    presenter: RefCell<Option<MetalPresenter>>,
    /// Issues each link frame into this leaf's render path.
    frame_sink: Rc<dyn Fn(DrawableFrame)>,
    /// `captureTexture`.
    capture_texture: RefCell<Option<Retained<MetalTexture>>>,
    /// `framePresentationInFlight`.
    frame_presentation_in_flight: Cell<bool>,
    /// `renderInFlight`.
    render_in_flight: Cell<bool>,
    /// `detachAfterCapture`.
    detach_after_capture: Cell<bool>,
    /// `pendingDynamicRangeMode`.
    pending_dynamic_range: RefCell<Option<cocoa_ui::dynamic_range::DynamicRange>>,
    /// `configuredDynamicRangeMode`.
    configured_range: Cell<Option<cocoa_ui::dynamic_range::DynamicRange>>,
    /// `needsRender`.
    needs_render: Cell<bool>,
    /// The attach-time first paint a reveal window waits on: arms only for
    /// a first-paint participant, answers the visibility gate exactly once
    /// so a window ordered at alpha 0 still delivers. Cleared by the
    /// presented receipt and by detach.
    first_paint_owed: Cell<bool>,
    /// `outputRevealed`/`filteredOutputRevealed` — set only by a returned
    /// `PresentedFrame`: actual screen presentation, nothing else.
    output_revealed: Cell<bool>,
    /// The terminal capture failure this leaf settled, keyed to the context
    /// generation it happened under: one log and one settle per generation,
    /// the typed carrier `render_prepared_external_texture` hands parent
    /// captures, and the gate `update_link_demand`/
    /// `participates_in_first_paint_ready` hold scheduling dead on until a
    /// new context publication rebinds the view.
    render_failed: RefCell<Option<(u64, Arc<dyn std::error::Error + Send + Sync>)>>,
    /// `currentScaleFactor`.
    current_scale: Cell<f64>,
    /// `laidOutGeometry`: a layout pass only requests a frame when the
    /// geometry it produced is new — captures provoke layout passes of
    /// their own, so arming the link on every pass makes nested filters
    /// drive each other forever.
    laid_out_geometry: RefCell<Option<cocoa_ui::Rect>>,
    /// `contentChangedSinceCapture` — a filter is only ready once it has
    /// shown a frame of the content as it actually stands.
    content_changed_since_capture: Cell<bool>,
    /// First-paint waiters — `readyCompletions` in waker form; they answer
    /// on actual screen presentation only.
    ready_waiters: RefCell<Vec<std::task::Waker>>,
    /// The hidden content leaf's mount — read for measurement only;
    /// `content_view` carries its geometry.
    mounted: RefCell<Option<Mounted>>,
    /// The `ViewCapture` pipeline — `capturePipeline`.
    capture: Rc<cocoa_ui::capture::ViewCapture>,
    /// Nested native-pass suppression scopes — counted so overlapping
    /// captures keep the presentation layer hidden until the last one ends.
    capture_suppression: Cell<usize>,
    /// External-render scopes — counted so overlapping captures suspend
    /// autonomous presentation until the last one ends.
    external_count: Cell<usize>,
    /// While external, redraw requests go to this capture's hook.
    external_redraw: RefCell<Option<Rc<dyn Fn()>>>,
    /// What the next external render draws into the compositor target
    /// `prepare_external_render` last bound — cleared when the scope ends
    /// or the context generation that imported it is superseded.
    external_render: RefCell<Option<ExternalRender>>,
    /// Whether the host has no area — set where its bounds frame the
    /// content, so a preparation for a collapsed view yields
    /// [`ExternalRender::Transparent`] instead of a content capture.
    collapsed: Cell<bool>,
    /// Window observers — `occlusionObserver`/app-activation watchers.
    observers: RefCell<Vec<cocoa_ui::notification::NotificationObserver>>,
    /// The context generation `device`/`capture_texture`/`imported_texture`
    /// were built under — all are recreated when the runtime publishes a
    /// new context. The presenter's device swaps onto the same event.
    gpu_generation: Cell<Option<u64>>,
    /// The parked wait on the next context publication, armed when a frame
    /// finds the current context lost. Stored so replacing the wait or
    /// dropping the state cancels it.
    context_watch: RefCell<Option<executor_core::AnyLocalExecutorTask<()>>>,
    /// The in-flight effect setup. Stored so dropping the state cancels a
    /// setup parked on `context_after` instead of leaking the future.
    setup_task: RefCell<Option<executor_core::AnyLocalExecutorTask<()>>>,
}

impl fmt::Debug for FilteredState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilteredState")
            .field("attached", &self.attached)
            .field("setup_generation", &self.setup_generation)
            .finish_non_exhaustive()
    }
}

/// The host bounds in physical pixels, rounded up: a positive extent in
/// points always covers at least one device pixel, so "positive in points"
/// and "non-zero in pixels" are one predicate.
fn pixel_size(view: &PlatformView, scale: f64) -> (u32, u32) {
    let bounds = cocoa_ui::view::bounds(view);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a view's pixel dimensions fit u32 and are clamped non-negative"
    )]
    let (w, h) = (
        (bounds.size.width * scale).max(0.0).ceil() as u32,
        (bounds.size.height * scale).max(0.0).ceil() as u32,
    );
    (w, h)
}

/// `outputPixelFormat`: the Metal format the effects render their output in.
fn output_pixel_format() -> objc2_metal::MTLPixelFormat {
    cocoa_ui::metal::wgpu_to_metal_format(PRESENTATION_FORMAT)
}

/// `isPresentationOccluded`.
fn presentation_occluded(view: &PlatformView) -> bool {
    #[cfg(target_os = "macos")]
    {
        cocoa_ui::view::window(view).is_none_or(|window| !cocoa_ui::appkit::is_visible(&window))
    }
    #[cfg(target_os = "ios")]
    {
        let _ = view;
        !cocoa_ui::uikit::application_is_active()
    }
}

/// `canAttachNow`.
fn can_attach_now(view: &PlatformView) -> bool {
    cocoa_ui::view::window(view).is_some() && !presentation_occluded(view)
}

/// `configureDynamicRange` — applies to the host view's layer tree, which
/// reaches the presentation layer through `apply_to_view`'s recursive
/// sublayer walk.
fn configure_dynamic_range(state: &FilteredState, mode: cocoa_ui::dynamic_range::DynamicRange) {
    assert!(
        !state.attached.get(),
        "FilteredView dynamic range cannot change while attached"
    );
    cocoa_ui::dynamic_range::apply_to_view(mode, &state.view);
    apply_presentation_contract(state, mode);
    state.capture_texture.borrow_mut().take();
    hide_output(state);
    state.configured_range.set(Some(mode));
}

/// The `CAMetalLayer`'s colour contract for `mode` — the colour space of
/// the actual output pixel format and the EDR-content flag (distinct from
/// the preferred-dynamic-range tags `apply_to_view` writes). Applying it to
/// a live presenter retires frames the old configuration issued.
fn apply_presentation_contract(state: &FilteredState, mode: cocoa_ui::dynamic_range::DynamicRange) {
    let colorspace = cocoa_ui::metal::color_space(output_pixel_format());
    state.presentation_layer.setColorspace(Some(&colorspace));
    state
        .presentation_layer
        .setWantsExtendedDynamicRangeContent(mode == cocoa_ui::dynamic_range::DynamicRange::High);
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        presenter.advance_generation();
    }
}

/// `prepareDynamicRange` — parks the change while a frame is in flight.
fn prepare_dynamic_range(
    state: &FilteredState,
    mode: cocoa_ui::dynamic_range::DynamicRange,
) -> bool {
    if state.configured_range.get() == Some(mode) {
        state.pending_dynamic_range.borrow_mut().take();
        return true;
    }
    if state.render_in_flight.get() || state.frame_presentation_in_flight.get() {
        *state.pending_dynamic_range.borrow_mut() = Some(mode);
        return false;
    }
    detach_if_needed(state);
    configure_dynamic_range(state, mode);
    true
}

/// `ensureGpuGeneration` — every device-bound resource (`device`,
/// `capture_texture`, `imported_texture`, the set-up effects) predates a
/// newly published context and must not be reused, even when the
/// underlying `MTLDevice` is unchanged. A live presenter keeps its layer
/// and link and swaps only the device, advancing its generation.
fn ensure_filtered_generation(state: &Rc<FilteredState>, context: &SharedGpuContext) {
    if state.gpu_generation.get() == Some(context.generation()) {
        return;
    }
    let device = crate::gpu_runtime::raw_metal_device(context);
    *state.device.borrow_mut() = device.clone();
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        presenter.set_device(&device);
    }
    *state.capture_texture.borrow_mut() = None;
    *state.imported_texture.borrow_mut() = None;
    *state.external_render.borrow_mut() = None;
    state.gpu_generation.set(Some(context.generation()));
    // The effect pipeline was set up on the previous generation — set up
    // again on this one.
    state.setup_generation.set(None);
    start_setup(state);
}

/// The installed effects are ready only when they finished setup on the
/// generation the device-bound resources belong to.
fn effects_ready(state: &FilteredState) -> bool {
    state.setup_generation.get() == state.gpu_generation.get()
}

/// Parks the filter until the runtime publishes a context newer than the
/// lost one, then requests a frame. Stored on the state so replacing the
/// wait or dropping the state cancels it.
fn arm_filtered_context_watch(state: &Rc<FilteredState>, generation: u64) {
    let runtime = state.runtime.clone();
    let weak = Rc::downgrade(state);
    *state.context_watch.borrow_mut() = Some(spawn_local(async move {
        let _published = runtime.context_after(generation).await;
        if let Some(state) = weak.upgrade() {
            // A park or failure taken before attach re-runs the GPU
            // initialization so `attach_if_needed` retakes on the
            // published context — and a settled failure only ever
            // rebinds on a newer generation, so its gate clears here.
            state.render_failed.borrow_mut().take();
            initialize_gpu(&state);
            state.needs_render.set(true);
            update_link_demand(&state);
        }
    }));
}

/// `waterui_applied_filter_attach_host_textures` — the capture texture and
/// output format an attached presentation implies, always at the
/// extended-range target. `context` is the live context `initialize_gpu`
/// already verified and bound; attach never re-fetches it.
fn attach_if_needed(
    state: &Rc<FilteredState>,
    context: &Arc<SharedGpuContext>,
    width: u32,
    height: u32,
) {
    if state.attached.get() {
        return;
    }
    ensure_filtered_generation(state, context);
    // `assert_capture_usable_format`: the capture texture and the output
    // share one format, so the check runs at attach, not inside a frame.
    let capture_usages = wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::COPY_DST;
    assert!(
        context
            .adapter()
            .get_texture_format_features(PRESENTATION_FORMAT)
            .allowed_usages
            .contains(capture_usages),
        "filtered attach: output format {PRESENTATION_FORMAT:?} cannot be used for capture"
    );
    assert!(
        width > 0 && height > 0,
        "FilteredView attach: dimensions must be non-zero, got {width}x{height}"
    );
    state.input_size.set((width, height));
    // One presenter per attach: the persistent layer gets a fresh
    // `CAMetalDisplayLink` bound to the current context's device and the
    // drawable size this attach measures.
    let presenter = MetalPresenter::new(
        state.presentation_layer.clone(),
        Rc::clone(&state.frame_sink),
    );
    presenter.set_device(&crate::gpu_runtime::raw_metal_device(context));
    presenter.set_drawable_size(cocoa_ui::metal_presenter::drawable_size(width, height));
    // The display's maximum frame rate — the link is armed for its
    // cadence (`min 60..=max preferred max` inside `set_display_rate`).
    if let Some(maximum) = crate::gpu_runtime::display_rate(&state.view) {
        presenter.set_display_rate(maximum);
    }
    *state.presenter.borrow_mut() = Some(presenter);
    // A fresh presenter carries the latched colour contract — the layer is
    // persistent across attaches, but `advance_generation` above does not
    // reapply contract properties.
    if let Some(mode) = state.configured_range.get() {
        let colorspace = cocoa_ui::metal::color_space(output_pixel_format());
        state.presentation_layer.setColorspace(Some(&colorspace));
        state
            .presentation_layer
            .setWantsExtendedDynamicRangeContent(
                mode == cocoa_ui::dynamic_range::DynamicRange::High,
            );
    }
    state.attached.set(true);
    // Only a first-paint participant owes the reveal frame: a filter
    // mounted behind hidden or zero-alpha ancestors parks until shown.
    state
        .first_paint_owed
        .set(!state.output_revealed.get() && participates_in_first_paint_ready(state));
    if effects_ready(state) {
        request_render(state);
    } else {
        start_setup(state);
    }
}

/// `detachIfNeeded` — `waterui_applied_filter_detach`. Detaching retires
/// the presenter: dropping it invalidates the link and advances the
/// generation, so a late completion settles its lease without presenting.
fn detach_if_needed(state: &FilteredState) {
    if !state.attached.get() {
        return;
    }
    state.attached.set(false);
    state.presenter.borrow_mut().take();
    state.imported_texture.borrow_mut().take();
    state.capture_texture.borrow_mut().take();
    state.input_size.set((0, 0));
    // The presentation epoch ends with the presenter: readiness re-arms and
    // the next attach owes its first paint again.
    state.first_paint_owed.set(false);
    state.output_revealed.set(false);
}

/// `ensureCaptureTexture` — a private-storage render target in the
/// presentation format, reallocated on size change, allocated on the
/// explicit live `context` the caller bound this generation to.
fn ensure_capture_texture(
    state: &FilteredState,
    context: &SharedGpuContext,
    width: u32,
    height: u32,
) -> Retained<MetalTexture> {
    let pixel_format = output_pixel_format();
    if let Some(texture) = state.capture_texture.borrow().as_ref()
        && texture.width() == width as usize
        && texture.height() == height as usize
        && texture.pixelFormat() == pixel_format
    {
        return texture.clone();
    }
    // SAFETY: creates a valid descriptor; Metal validates the arguments.
    let descriptor = unsafe {
        objc2_metal::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            pixel_format,
            width as usize,
            height as usize,
            false,
        )
    };
    descriptor.setUsage(
        objc2_metal::MTLTextureUsage::ShaderRead | objc2_metal::MTLTextureUsage::RenderTarget,
    );
    descriptor.setStorageMode(objc2_metal::MTLStorageMode::Private);
    let texture = crate::gpu_runtime::raw_metal_device(context)
        .newTextureWithDescriptor(&descriptor)
        .expect("Failed to create the filtered-content capture texture");
    *state.capture_texture.borrow_mut() = Some(texture.clone());
    texture
}

/// `initializeGpuIfNeeded`. A zero-sized host is legitimate and detaches:
/// no link stays armed to issue a frame over content with nothing to
/// capture, and the layout that restores the bounds re-attaches.
fn initialize_gpu(state: &Rc<FilteredState>) {
    let bounds = cocoa_ui::view::bounds(&state.view);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        if state.render_in_flight.get() || state.frame_presentation_in_flight.get() {
            state.detach_after_capture.set(true);
        } else {
            detach_if_needed(state);
        }
        complete_ready(state);
        update_link_demand(state);
        return;
    }
    let Some(window) = cocoa_ui::view::window(&state.view) else {
        return;
    };
    // Cleared only with a window present: a windowless layout must not
    // cancel the detach a window-leave deferred past the frame in flight.
    state.detach_after_capture.set(false);
    let dynamic_range = cocoa_ui::dynamic_range::require_inherited(&state.view);
    if !prepare_dynamic_range(state, dynamic_range) {
        return;
    }
    #[cfg(target_os = "macos")]
    let scale = window.backingScaleFactor();
    #[cfg(target_os = "ios")]
    let scale = window.screen().scale();
    state.current_scale.set(scale);
    update_output_frame(state);

    // Attaching waits for a window that can present: a filter in a covered
    // window never captures anything, so the capture texture is only bought
    // once `update_link_demand` could arm the link.
    if !can_attach_now(&state.view) {
        return;
    }
    attach(state);
}

/// The attach the `can_attach_now` gate defers: one explicit live context
/// for everything below — on loss the whole initialization parks before
/// attach or any native allocation, and the publication watch re-runs
/// `initialize_gpu` on the rebuilt context.
fn attach(state: &Rc<FilteredState>) {
    let context = state.runtime.context();
    if context.device_lost_reason().is_some() {
        arm_filtered_context_watch(state, context.generation());
        return;
    }
    // Device-bound state always binds to this exact generation — including
    // when the view is already attached — so nothing later allocates on a
    // stale device.
    ensure_filtered_generation(state, &context);
    let (width, height) = pixel_size(&state.view, state.current_scale.get());
    attach_if_needed(state, &context, width, height);
    let _ = ensure_capture_texture(state, &context, width, height);
}

/// `updateOutputLayerFrame` — the hidden content view, the output view,
/// its backing layer and the presentation `CAMetalLayer` share one layout
/// pass, including the drawable size: a size change advances the
/// presenter's generation so a frame rendered for the previous size can
/// never present at the new one, and every path that attaches sizes the
/// content the capture reads in the same call.
fn update_output_frame(state: &FilteredState) {
    let bounds = cocoa_ui::view::bounds(&state.view);
    state
        .collapsed
        .set(bounds.size.width <= 0.0 || bounds.size.height <= 0.0);
    cocoa_ui::core_animation::without_animation(|| {
        cocoa_ui::view::set_frame(&state.content_view, bounds);
        cocoa_ui::view::set_frame(&state.output_view, bounds);
        if let Some(layer) = cocoa_ui::view::layer(&state.output_view) {
            cocoa_ui::core_animation::set_frame(&layer, bounds);
            cocoa_ui::core_animation::set_contents_scale(&layer, state.current_scale.get());
        }
        let mut frame = bounds;
        frame.origin.x = 0.0;
        frame.origin.y = 0.0;
        cocoa_ui::core_animation::set_frame(&state.presentation_layer, frame);
        cocoa_ui::core_animation::set_contents_scale(
            &state.presentation_layer,
            state.current_scale.get(),
        );
    });
    // A collapsed view keeps its last drawable size: a zero size would reach
    // the live `CAMetalLayer` while a deferred detach holds the presenter.
    if state.collapsed.get() {
        return;
    }
    let (width, height) = pixel_size(&state.view, state.current_scale.get());
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        presenter.set_drawable_size(cocoa_ui::metal_presenter::drawable_size(width, height));
    }
}

/// Whether the link may deliver frames — attached, effectively visible, in
/// an active scene and with demand (an animating, owed or dirty frame). An
/// in-flight frame, native-pass suppression or external rendering suspends
/// it; no display-interval retry loop, no timer, no polling — a paused
/// link simply delivers nothing until demand returns. A capture-driven
/// filter's never-ordered window gets no updates at all, so its demand is
/// answered by a queued on-demand issue instead — the same drawable
/// lease a link update delivers.
fn update_link_demand(state: &Rc<FilteredState>) {
    let demand = state.attached.get()
        && effects_ready(state)
        && state.render_failed.borrow().is_none()
        && cocoa_ui::view::window(&state.view).is_some()
        && (state.needs_render.get() || state.first_paint_owed.get())
        && !state.render_in_flight.get()
        && !state.frame_presentation_in_flight.get()
        && state.capture_suppression.get() == 0
        && state.external_count.get() == 0
        && ((cocoa_ui::view::has_visible_ancestry(&state.view)
            && !presentation_occluded(&state.view))
            || state.first_paint_owed.get());
    if let Some(presenter) = state.presenter.borrow().as_ref() {
        presenter.set_paused(!demand);
    }
    if !demand && !state.render_in_flight.get() && !state.frame_presentation_in_flight.get() {
        // Nothing armed and nothing in flight leaves the effect timeline
        // idle; its next tick measures restart latency, not park duration.
        state.timeline_parked.set(true);
    }
}

/// `requestRenderIfNeeded` — a redraw request records demand and is
/// answered by the next drawable delivery, at most one display interval
/// later. While an external capture owns this output, the request goes to
/// the capture's redraw hook instead and the link stays suspended.
fn request_render(state: &Rc<FilteredState>) {
    state.needs_render.set(true);
    if state.external_count.get() > 0 {
        if let Some(on_redraw) = state.external_redraw.borrow().as_ref() {
            on_redraw();
        }
        return;
    }
    update_link_demand(state);
}

/// `requestRenderIfGeometryChanged` — only a pass that produced new
/// geometry arms the link; see `laid_out_geometry`.
fn request_render_if_geometry_changed(state: &Rc<FilteredState>) {
    let geometry = cocoa_ui::view::bounds(&state.view);
    if state
        .laid_out_geometry
        .borrow()
        .is_some_and(|g| g == geometry)
    {
        update_link_demand(state);
        return;
    }
    *state.laid_out_geometry.borrow_mut() = Some(geometry);
    request_render(state);
}

/// The link-issued frame: `frame` is the unique drawable lease — held
/// across the hidden-content capture, the effect encode, the submission
/// and its completion. Dropping it at any gate releases the lease without
/// presenting and the next delivery answers the still-owed demand.
fn render_frame(state: &Rc<FilteredState>, frame: DrawableFrame) {
    let context = state.runtime.context();
    if context.device_lost_reason().is_some() {
        // Nothing prepares, captures, or submits on a dead device — the
        // frame drops unpresented and the leaf parks until the rebuilt
        // context is published.
        drop(frame);
        arm_filtered_context_watch(state, context.generation());
        return;
    }
    // A new context generation rebuilds the device-bound resources and
    // re-arms the effect setup before the gates below run.
    ensure_filtered_generation(state, &context);
    if !effects_ready(state)
        || !state.needs_render.get()
        || state.render_in_flight.get()
        || state.frame_presentation_in_flight.get()
    {
        // The frame drops unpresented; `needs_render` (or the setup's own
        // re-request) keeps the demand that re-issues it.
        drop(frame);
        update_link_demand(state);
        return;
    }
    let (width, height) = frame.drawable_size();
    state.needs_render.set(false);
    state.render_in_flight.set(true);
    // A frame in flight pauses the link: `deliver_update` returns early
    // while the lease is held, so nothing else stops the next vsync.
    update_link_demand(state);
    let input = ensure_capture_texture(state, &context, width, height);
    let generation = context.generation();
    let work = PresentWork {
        frame,
        context,
        input,
        width,
        height,
    };
    let capture_input = work.input.clone();
    let weak = Sendable(Rc::downgrade(state));
    // The capture completion is `Fn` — the work crosses it inside a slot.
    let work = std::sync::Mutex::new(Some(Sendable(work)));
    state
        .capture
        .capture(&capture_input, generation, move |outcome| {
            if let Some(state) = weak.get().upgrade() {
                let work = work.lock().expect("capture fires once").take();
                if let Some(work) = work {
                    finish_captured_frame(&state, work.0, &outcome);
                }
            }
            // A dropped state's slot drops the frame here — the lease releases
            // unpresented on the main thread the completion contract
            // guarantees.
        });
}

/// `finishCapturedFrame`.
fn finish_captured_frame(
    state: &Rc<FilteredState>,
    work: PresentWork,
    outcome: &Result<(), CaptureError>,
) {
    if state.detach_after_capture.get() {
        state.render_in_flight.set(false);
        state.detach_after_capture.set(false);
        detach_if_needed(state);
        let generation = work.context.generation();
        drop(work);
        if let Err(CaptureError::Failed(error)) = outcome {
            settle_capture_failure(state, generation, error.clone());
        } else {
            complete_ready(state);
        }
        return;
    }
    if state.pending_dynamic_range.borrow_mut().take().is_some() {
        state.render_in_flight.set(false);
        let generation = work.context.generation();
        drop(work);
        if let Err(CaptureError::Failed(error)) = outcome {
            // The failed gate sets before the rebind re-initialises and
            // reschedules: `settle_capture_failure` resolves the ready
            // waiters this arm would otherwise leave hanging, and
            // `update_link_demand` sees the gate while the watch arms.
            settle_capture_failure(state, generation, error.clone());
        }
        detach_if_needed(state);
        initialize_gpu(state);
        request_render(state);
        return;
    }
    match outcome {
        Err(CaptureError::Failed(error)) => {
            // The frame's lease releases unpresented, readiness resolves
            // so waiters never hang, and scheduling stays dead on the
            // failed gate until the context watch publishes a rebind.
            state.render_in_flight.set(false);
            let generation = work.context.generation();
            drop(work);
            settle_capture_failure(state, generation, error.clone());
            return;
        }
        Err(CaptureError::Deferred) => {
            state.render_in_flight.set(false);
            // The deferred frame produced no pixels, so it stays owed —
            // `needs_render` was already consumed when the capture started.
            // A lost submitting context replays only from the next
            // publication; any other deferral is a child's readiness, which
            // reaches this leaf as a redraw request. A child may signal
            // before its deferral settles this frame — a GPU surface
            // requests its redraw ahead of answering `Deferred` — and that
            // request found the link paused by the flight, so it is
            // answered here; otherwise the link stays paused until it comes.
            let redraw_requested = state.needs_render.replace(true);
            let generation = work.context.generation();
            let lost = work.context.device_lost_reason().is_some();
            drop(work);
            if lost {
                arm_filtered_context_watch(state, generation);
            } else if redraw_requested {
                update_link_demand(state);
            }
            return;
        }
        Ok(()) => {}
    }
    finish_prepared_frame(state, work);
}

/// The whole terminal settle for a [`CaptureError::Failed`] — record the
/// failure, resolve readiness so waiters never hang, and recompute
/// scheduling against the failed gate. Every arm that ends on a `Failed`
/// outcome runs it after releasing its frame work — the detach arm, the
/// dynamic-range arm (before the rebind re-initialises), and the plain
/// `Failed` arm.
fn settle_capture_failure(
    state: &Rc<FilteredState>,
    generation: u64,
    error: Arc<dyn std::error::Error + Send + Sync>,
) {
    note_render_failure(state, generation, error);
    complete_ready(state);
    update_link_demand(state);
}

/// Settles a terminal [`CaptureError::Failed`] the content capture
/// answered, with the GPU surface's failure contract: the typed error
/// logs once per context generation, the `render_failed` gate stops
/// scheduling — `update_link_demand`, `participates_in_first_paint_ready`
/// and the external render answers all consult it — and the context
/// watch arms so the next published generation rebinds the view. There
/// is no retry on the failed generation: a redraw the failed context
/// never issues cannot produce the frame.
fn note_render_failure(
    state: &Rc<FilteredState>,
    generation: u64,
    error: Arc<dyn std::error::Error + Send + Sync>,
) {
    if state
        .render_failed
        .borrow()
        .as_ref()
        .is_some_and(|(settled, _)| *settled == generation)
    {
        // One failure record and one log per generation — a second Failed
        // completion on the same generation settles nothing further.
        return;
    }
    tracing::error!(
        "filtered rendering failed; the view stops scheduling until a new context generation rebinds it: {error}"
    );
    *state.render_failed.borrow_mut() = Some((generation, error));
    // The failed frame's pixels never landed — the view owes its first
    // paint to the rebind, not to this generation.
    state.first_paint_owed.set(false);
    arm_filtered_context_watch(state, generation);
}

/// Submits an external render's `encoder` and answers `completion` once
/// on the main queue: `Ok(())` once its work completed on the live queue,
/// or `Deferred` with the publication watch armed when the fence settled
/// on a dead queue.
fn submit_external(
    state: &Rc<FilteredState>,
    context: &Arc<SharedGpuContext>,
    encoder: wgpu::CommandEncoder,
    completion: SurfaceCaptureCompletion,
) {
    let weak = Sendable(Rc::downgrade(state));
    let completion = std::sync::Mutex::new(Some(completion));
    let submitted_context = context.clone();
    crate::gpu_completion::submit_with_completion(
        cocoa_ui::MainThreadMarker::new()
            .expect("FilteredView external renders complete on the main thread"),
        encoder,
        context,
        move |result| {
            cocoa_ui::main_queue::enqueue(move |_mtm| {
                let completion = completion.lock().expect("external completion lock").take();
                let Some(completion) = completion else {
                    return;
                };
                match result {
                    Err(_) => {
                        // The fence settled on a dead queue: no
                        // usable pixels to composite — deferred,
                        // and the publication watch re-arms the
                        // redraw.
                        if let Some(state) = weak.get().upgrade() {
                            arm_filtered_context_watch(&state, submitted_context.generation());
                        }
                        completion(Err(CaptureError::Deferred));
                    }
                    // Offscreen capture completion is not a
                    // `PresentedFrame` receipt —
                    // productive-generation accounting consumes
                    // on-screen receipts only.
                    Ok(()) => completion(Ok(())),
                }
            });
        },
    );
}

/// A collapsed view's external answer: its producer texture cleared to
/// transparent, which the parent composites source-over — the view
/// contributes nothing to its region, as the screen shows a view with no
/// area.
fn render_transparent(
    state: &Rc<FilteredState>,
    context: &Arc<SharedGpuContext>,
    output: &wgpu::Texture,
    completion: SurfaceCaptureCompletion,
) {
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("FilteredView collapsed output"),
        });
    let view = output.create_view(&wgpu::TextureViewDescriptor::default());
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("FilteredView collapsed clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    }));
    submit_external(state, context, encoder, completion);
}

/// The per-frame effect encode shared by the screen drawable and the
/// external-capture destination: `input` feeds effect 0, intermediates
/// chain the rest, the last pass lands in `output`. All textures are
/// `width`×`height` `PRESENTATION_FORMAT`.
fn encode_effects(
    state: &FilteredState,
    context: &SharedGpuContext,
    input_texture: &wgpu::Texture,
    output_texture: &wgpu::Texture,
    width: u32,
    height: u32,
    timing: EffectFrameTiming,
) -> (bool, wgpu::CommandEncoder) {
    let device = context.device();
    let queue = context.queue();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("filtered content encoder"),
    });
    let mut needs_redraw = false;
    let mut effects = state.effects.borrow_mut();
    let effects = effects
        .as_mut()
        .expect("FilteredView ready state is missing its effects");
    // An effect after the first renders the previous one's output:
    // `effects.len() - 1` private intermediates chain the passes.
    let intermediates: Vec<wgpu::Texture> = (0..effects.len().saturating_sub(1))
        .map(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("FilteredView Intermediate"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PRESENTATION_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        })
        .collect();
    let effect_count = effects.len();
    for (index, effect) in effects.iter_mut().enumerate() {
        let last = index + 1 == effect_count;
        let input_source = if index == 0 {
            input_texture
        } else {
            &intermediates[index - 1]
        };
        let output_target = if last {
            output_texture
        } else {
            &intermediates[index]
        };
        let input = EffectInput {
            device,
            queue,
            texture: input_source,
            view: input_source.create_view(&wgpu::TextureViewDescriptor {
                label: Some("FilteredView Input View"),
                ..Default::default()
            }),
            format: PRESENTATION_FORMAT,
            width,
            height,
            timing,
            shape: ShapeTextures::default(),
        };
        let output = EffectOutput {
            device,
            queue,
            texture: output_target,
            view: output_target.create_view(&wgpu::TextureViewDescriptor {
                label: Some("FilteredView Output View"),
                format: Some(PRESENTATION_FORMAT),
                ..Default::default()
            }),
            format: PRESENTATION_FORMAT,
            width,
            height,
        };
        needs_redraw |= effect
            .encode_render(&input, &output, &mut encoder)
            .unwrap_or_else(|error| panic!("filtered render: {error}"))
            || effect.redraw_hint();
    }
    (needs_redraw, encoder)
}

/// The imported wgpu view of the live frame's capture texture — the
/// effects' input, cached per (size, generation).
fn imported_input(
    state: &FilteredState,
    context: &SharedGpuContext,
    texture: Retained<MetalTexture>,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    let mut imported = state.imported_texture.borrow_mut();
    match imported.as_ref() {
        Some(texture) if texture.width() == width && texture.height() == height => texture.clone(),
        _ => {
            // SAFETY: `texture` is retained by the caller for the import's
            // lifetime; the format and size describe that same texture.
            let texture = unsafe {
                cocoa_ui::metal::import_texture(
                    context.device(),
                    texture,
                    PRESENTATION_FORMAT,
                    width,
                    height,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    wgpu::TextureUses::COLOR_TARGET,
                    "FilteredView Imported Input Texture",
                )
            };
            *imported = Some(texture.clone());
            texture
        }
    }
}

/// `renderCapturedFrame` — `waterui_applied_filter_render_to_metal_texture`:
/// encode the effect chain into one command buffer and present on its
/// fence. The drawable lease stays owned from the link update through this
/// completion.
#[allow(clippy::needless_pass_by_value)]
fn finish_prepared_frame(state: &Rc<FilteredState>, work: PresentWork) {
    // Encode, import, and submit on the exact context the capture was
    // prepared under — a fresh fetch could name a generation this frame's
    // native texture was never prepared for.
    let context = work.context.clone();
    if context.device_lost_reason().is_some() {
        // The prepared context died during the capture: the frame's native
        // texture must not mix into another generation's resources. Drop
        // it before any reset/import/encode and park for publication.
        state.render_in_flight.set(false);
        arm_filtered_context_watch(state, context.generation());
        return;
    }
    if state.runtime.context().generation() != context.generation() {
        // A newer generation published during the capture — this frame's
        // native texture predates it. Drop the frame before any
        // reset/import/encode; the next frame runs on the new context.
        state.render_in_flight.set(false);
        state.needs_render.set(true);
        update_link_demand(state);
        return;
    }
    ensure_filtered_generation(state, &context);
    if !effects_ready(state) {
        // The ready bundle belongs to a different generation — re-setup
        // is in flight and its landing re-requests the frame.
        state.render_in_flight.set(false);
        state.needs_render.set(true);
        return;
    }
    let (_output_width, _output_height) = (work.width, work.height);
    let input_texture =
        imported_input(state, &context, work.input.clone(), work.width, work.height);
    state.input_size.set((work.width, work.height));

    // The drawable's own texture is the output — imported
    // `RENDER_ATTACHMENT` only: the framebuffer-only drawable is rendered
    // into, never sampled or copied.
    // SAFETY: `frame.texture()` is the drawable's live texture — the lease
    // inside `work` outlives the import; the format and the drawable's
    // actual texture dimensions describe it.
    let output_wgpu_texture = unsafe {
        cocoa_ui::metal::import_texture(
            context.device(),
            work.frame.texture(),
            PRESENTATION_FORMAT,
            work.width,
            work.height,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            wgpu::TextureUses::COLOR_TARGET,
            "FilteredView Presentation Drawable",
        )
    };
    let timing = {
        let mut frame_clock = state.frame_clock.borrow_mut();
        if state.timeline_parked.replace(false) {
            // Discard the stale-gap sample: the resumed frame's delta then
            // measures the restart latency instead of the park duration, while
            // the clock's origin and monotonic sequence carry on untouched.
            let _ = frame_clock.tick();
        }
        frame_clock.tick()
    };
    let (needs_redraw, encoder) = encode_effects(
        state,
        &context,
        &input_texture,
        &output_wgpu_texture,
        work.width,
        work.height,
        timing,
    );
    drop(input_texture);
    drop(output_wgpu_texture);

    // `observeGpuCaptureFence`: the frame — and its drawable lease — stay
    // in flight until the fence.
    state.frame_presentation_in_flight.set(true);
    let weak = Sendable(Rc::downgrade(state));
    // The `take` call on the slot captures the `Sendable` as a whole
    // binding — a field projection would unwrap it before the `Send`
    // contract can apply.
    let mut work_slot = Some(Sendable(work));
    let submitted_context = context.clone();
    crate::gpu_completion::submit_with_completion(
        cocoa_ui::MainThreadMarker::new().expect("FilteredView frames render on the main thread"),
        encoder,
        &context,
        move |result| {
            // `on_submitted_work_done` closures can run on any thread that
            // maintains the device — hop to the main queue before touching
            // the weak state handle.
            cocoa_ui::main_queue::enqueue(move |_mtm| {
                let Some(state) = weak.get().upgrade() else {
                    // The state is gone — `work` drops here and settles its
                    // lease unpresented.
                    return;
                };
                let Sendable(work) = work_slot.take().expect("the fence fires once");
                state.frame_presentation_in_flight.set(false);
                state.render_in_flight.set(false);
                match result {
                    Err(_) => {
                        // The submitted generation died in flight — the
                        // fence settled on a dead queue and the drawable
                        // holds no ready pixels. Park until the rebuilt
                        // context publishes instead of revealing it.
                        state.needs_render.set(true);
                        arm_filtered_context_watch(&state, submitted_context.generation());
                    }
                    Ok(()) => {
                        finish_presented_frame(&state, work, needs_redraw, &submitted_context);
                    }
                }
            });
        },
    );
}

/// The fence continuation — the present gate hands the lease to
/// `DrawableFrame::present`, which returns a `PresentedFrame` only when the
/// frame actually hit `[CAMetalDrawable present]`; anything else settles
/// the lease unpresented and the frame stays owed.
fn finish_presented_frame(
    state: &Rc<FilteredState>,
    work: PresentWork,
    needs_redraw: bool,
    submitted_context: &SharedGpuContext,
) {
    if state.detach_after_capture.get() {
        state.detach_after_capture.set(false);
        detach_if_needed(state);
        complete_ready(state);
        return;
    }
    if state.pending_dynamic_range.borrow_mut().take().is_some() {
        detach_if_needed(state);
        initialize_gpu(state);
        request_render(state);
        return;
    }
    // The live GPU context already checked above; the remaining gates are
    // the presenter's own — generation and drawable size — plus the
    // visibility contract: attached, windowed, unsuppressed and not
    // externally captured.
    let presentable = state.attached.get()
        && cocoa_ui::view::window(&state.view).is_some()
        && state.capture_suppression.get() == 0
        && state.external_count.get() == 0
        && !presentation_occluded(&state.view);
    // `present` consumes the frame's lease; `None` — stale generation or
    // size — settles it without presenting.
    let presented = if presentable {
        work.frame.present()
    } else {
        None
    };
    let Some(_receipt) = presented else {
        // Stale, resized or unpresentable: the lease settles unpresented
        // and the frame stays owed.
        state.needs_render.set(true);
        update_link_demand(state);
        return;
    };
    // A presented frame makes this generation productive — the runtime's
    // unproductive-loss detector keys on it.
    submitted_context.note_frame_presented();
    reveal_output(state);
    crate::invalidation::invalidate_rendered_content(&state.view);
    state
        .needs_render
        .set(state.needs_render.get() || needs_redraw);
    if state.content_changed_since_capture.get() {
        // What this frame shows is already out of date; readiness waits for
        // the one that captures the change.
        state.needs_render.set(true);
    } else {
        complete_ready(state);
    }
    update_link_demand(state);
}

/// `revealFilteredOutput` / `hideFilteredOutput`.
fn reveal_output(state: &FilteredState) {
    if state.output_revealed.get() {
        return;
    }
    state.first_paint_owed.set(false);
    state.output_revealed.set(true);
    cocoa_ui::view::set_hidden(&state.output_view, false);
}

fn hide_output(state: &FilteredState) {
    state.output_revealed.set(false);
    cocoa_ui::view::set_hidden(&state.output_view, true);
}

/// `completeReady` — wakes every waiter queued since the last frame.
fn complete_ready(state: &FilteredState) {
    for waker in state.ready_waiters.borrow_mut().drain(..) {
        waker.wake();
    }
}

/// `waterui_applied_filter_setup` — run every effect's `setup` on the
/// UI-local executor, retrying on device loss until it lands on the
/// still-current context.
fn start_setup(state: &Rc<FilteredState>) {
    if effects_ready(state) {
        return;
    }
    if state.effects.borrow().is_none() {
        // Setup is already in flight — it owns the effects until it lands.
        return;
    }
    spawn_setup(state);
}

/// `spawn_applied_filter_setup`.
fn spawn_setup(state: &Rc<FilteredState>) {
    let mut effects = state
        .effects
        .borrow_mut()
        .take()
        .expect("FilteredView effects are unavailable before setup starts");
    let effects_slot = Rc::downgrade(&state.effects);
    let setup_generation = Rc::clone(&state.setup_generation);
    let weak = Sendable(Rc::downgrade(state));
    let runtime = state.runtime.clone();
    let fire_redraw: EffectRedrawCallback = Arc::new(redraw_callback_for(state));
    *state.setup_task.borrow_mut() = Some(spawn_local(async move {
        // Setup retries until it completes on the context that is still the
        // runtime's current one; a device loss mid-setup leaves corpses that
        // must not be installed, and a panic from inside `wgpu`'s purged
        // storage is loss fallout — not an effect bug — so it retries too.
        // A lost context parks on the publication wait rather than retrying
        // against the dead device.
        let context = loop {
            let context = runtime.context();
            if context.device_lost_reason().is_some() {
                let _published = runtime.context_after(context.generation()).await;
                continue;
            }
            let device = context.device();
            let queue = context.queue();
            let outcome = {
                let ctx = EffectContext {
                    device,
                    queue,
                    input_format: PRESENTATION_FORMAT,
                    output_format: PRESENTATION_FORMAT,
                };
                std::panic::AssertUnwindSafe(async {
                    for effect in &mut effects {
                        effect
                            .setup(&ctx)
                            .await
                            .unwrap_or_else(|error| panic!("filtered setup failed: {error}"));
                    }
                })
                .catch_unwind()
                .await
            };
            if let Err(payload) = outcome {
                if context.device_lost_reason().is_none() {
                    std::panic::resume_unwind(payload);
                }
                continue;
            }
            if context.device_lost_reason().is_none()
                && runtime.context().generation() == context.generation()
            {
                break context;
            }
        };
        let Some(slot) = effects_slot.upgrade() else {
            return;
        };
        slot.borrow_mut().replace(effects);
        setup_generation.set(Some(context.generation()));
        if let Some(state) = weak.get().upgrade() {
            update_link_demand(&state);
        }
        fire_redraw();
    }));
}

/// The callback every effect fires when external state becomes dirty —
/// `installRedrawCallback`'s target. The callback runs on arbitrary
/// effect-owned threads and its last drop can land there too, so the
/// weak travels in a `MainQueueOwned`: its clone and upgrade happen
/// only inside the main-queue work item, and an off-main release
/// enqueues the payload's drop rather than blocking on `exec_sync`.
fn redraw_callback_for(state: &Rc<FilteredState>) -> impl Fn() + Send + Sync + 'static {
    let mtm = cocoa_ui::MainThreadMarker::new()
        .expect("FilteredView callbacks install on the main thread");
    let weak = crate::main_queue_owned::shared(Rc::downgrade(state), mtm);
    move || {
        let weak = Arc::clone(&weak);
        cocoa_ui::main_queue::enqueue(move |mtm| {
            if let Some(state) = weak.get(mtm).upgrade() {
                handle_redraw(&state);
            }
        });
    }
}

/// `handleRendererRedraw` — the semantic redraw wake.
fn handle_redraw(state: &Rc<FilteredState>) {
    request_render(state);
}

/// `handleWindowChange` — leaving the window defers teardown to whichever
/// half of the frame is still in flight.
fn handle_window_change(state: &Rc<FilteredState>) {
    if cocoa_ui::view::window(&state.view).is_none() {
        state.timeline_parked.set(true);
        state.needs_render.set(false);
        state.pending_dynamic_range.borrow_mut().take();
        complete_ready(state);
        if state.render_in_flight.get() || state.frame_presentation_in_flight.get() {
            state.detach_after_capture.set(true);
        } else {
            detach_if_needed(state);
        }
        update_link_demand(state);
        return;
    }
    state.detach_after_capture.set(false);
    update_window_observers(state);
    initialize_gpu(state);
    request_render(state);
}

/// `WuiWindowOcclusionObserver` — occlusion/activation changes re-run
/// attach+demand. On iOS the attach a launch-time `.inactive` state
/// deferred is retaken from `didBecomeActive`; without these observers a
/// filter mounted before activation presents nothing forever.
fn update_window_observers(state: &Rc<FilteredState>) {
    let mut observers = state.observers.borrow_mut();
    observers.clear();
    let Some(window) = cocoa_ui::view::window(&state.view) else {
        return;
    };
    #[cfg(target_os = "ios")]
    let _ = &window;
    let mtm = cocoa_ui::MainThreadMarker::new().expect("main thread");
    let fire = {
        let weak = Rc::downgrade(state);
        move || {
            if let Some(state) = weak.upgrade() {
                initialize_gpu(&state);
                update_link_demand(&state);
            }
        }
    };
    #[cfg(target_os = "macos")]
    observers.push(cocoa_ui::appkit::watch_occlusion(mtm, &window, move || {
        fire();
    }));
    #[cfg(target_os = "ios")]
    for notification in [
        // SAFETY: the notification names are system constants.
        unsafe { cocoa_ui::objc2_ui_kit::UIApplicationDidBecomeActiveNotification },
        // SAFETY: the notification names are system constants.
        unsafe { cocoa_ui::objc2_ui_kit::UIApplicationWillResignActiveNotification },
    ] {
        observers.push(cocoa_ui::notification::observe(
            mtm,
            &cocoa_ui::notification::NotificationName::framework(notification),
            {
                let fire = fire.clone();
                move || fire()
            },
        ));
    }
}

/// `layoutSubviews`/`layout`: frame the hidden content with the output,
/// lay the content out, then ensure GPU state and a pending frame.
fn on_layout(state: &Rc<FilteredState>) {
    update_output_frame(state);
    cocoa_ui::view::layout_immediately(&state.content_view);
    initialize_gpu(state);
    request_render_if_geometry_changed(state);
}

/// The layout face: measurement delegates to the hidden child —
/// `sizeThatFits`/`measure`/`layoutPriority`/`setPlacementProposal`.
struct FilteredSubView {
    state: Rc<FilteredState>,
}

impl fmt::Debug for FilteredSubView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilteredSubView").finish_non_exhaustive()
    }
}

impl SubView for FilteredSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.mounted.borrow().as_ref().map_or_else(
            || ViewDimensions::new(Size::new(0.0, 0.0)),
            |m| m.layout().measure(proposal),
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state
            .mounted
            .borrow()
            .as_ref()
            .map_or(StretchAxis::None, |m| m.layout().stretch_axis())
    }

    fn priority(&self) -> i32 {
        self.state
            .mounted
            .borrow()
            .as_ref()
            .map_or(0, |m| m.layout().priority())
    }
}

// MARK: - Capturable surface and first-paint readiness

/// `participatesInFirstPaintReady` — a filter whose window cannot present
/// has no first frame to wait for. Hidden, zero-alpha, clipped or
/// invisible-ancestry views do not participate and never owe a frame.
fn participates_in_first_paint_ready(state: &FilteredState) -> bool {
    let bounds = cocoa_ui::view::bounds(&state.view);
    cocoa_ui::view::window(&state.view).is_some()
        && state.render_failed.borrow().is_none()
        && !cocoa_ui::view::is_hidden(&state.view)
        && cocoa_ui::view::alpha(&state.view) > 0.01
        && bounds.size.width > 0.5
        && bounds.size.height > 0.5
        && cocoa_ui::view::has_visible_ancestry(&state.view)
        && can_attach_now(&state.view)
}

/// `requestReadyFrame` — prepare, then register `waker` against the first
/// presented output.
fn request_ready_frame(state: &Rc<FilteredState>, waker: std::task::Waker) {
    if state.output_revealed.get() {
        waker.wake();
        return;
    }
    // `prepareForReady`.
    cocoa_ui::view::layout_immediately(&state.view);
    initialize_gpu(state);
    request_render(state);
    if !state.attached.get() {
        complete_ready(state);
        return;
    }
    // The waiter is a first-paint participant — owe the reveal frame so
    // the link answers it even while an alpha-0 reveal window is ordered.
    state
        .first_paint_owed
        .set(participates_in_first_paint_ready(state));
    state.ready_waiters.borrow_mut().push(waker);
    update_link_demand(state);
}

/// The filtered output's `CapturableSurface` face — owns only the weak
/// state, so a dropped leaf is never captured. The hidden content's own
/// nested surfaces resolve through the slots on their views inside
/// `ViewCapture`, which is what keeps a filtered subtree — and a
/// filter inside a filter — nonblank.
struct FilteredCapturable {
    state: Weak<FilteredState>,
}

impl fmt::Debug for FilteredCapturable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilteredCapturable").finish_non_exhaustive()
    }
}

impl CapturableSurface for FilteredCapturable {
    fn capture_pixel_format(&self) -> objc2_metal::MTLPixelFormat {
        output_pixel_format()
    }

    fn content_bounds(&self, relative_to: &PlatformView) -> cocoa_ui::Rect {
        let Some(state) = self.state.upgrade() else {
            return cocoa_ui::Rect::ZERO;
        };
        cocoa_ui::view::convert_rect(
            &state.output_view,
            cocoa_ui::view::bounds(&state.output_view),
            Some(relative_to),
        )
    }

    /// Native-pass suppression: direct `hidden` mutation on the
    /// presentation layer only — no transaction work of ours. The outer
    /// `ViewCapture` owns the one enclosing disabled-actions transaction
    /// and commits it only after every suppression scope has closed; the
    /// on-screen tree never flickers.
    fn begin_capture_suppression(&self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let count = state.capture_suppression.get() + 1;
        state.capture_suppression.set(count);
        if count == 1 {
            state.presentation_layer.setHidden(true);
        }
        update_link_demand(&state);
    }

    fn end_capture_suppression(&self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let count = state.capture_suppression.get();
        assert!(
            count > 0,
            "FilteredView capture suppression scopes are unbalanced"
        );
        state.capture_suppression.set(count - 1);
        if count == 1 {
            state.presentation_layer.setHidden(false);
        }
        update_link_demand(&state);
    }

    /// Redirects redraw requests to the capture's hook and suspends
    /// autonomous presentation; `end` restores demand from current
    /// visibility.
    fn begin_external_rendering(&self, on_redraw: Rc<dyn Fn()>) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        if state.external_count.get() == 0 {
            *state.external_redraw.borrow_mut() = Some(on_redraw);
        }
        state.external_count.set(state.external_count.get() + 1);
        update_link_demand(&state);
    }

    fn end_external_rendering(&self, resume: bool) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let count = state.external_count.get();
        assert!(
            count > 0,
            "FilteredView external rendering scopes are unbalanced"
        );
        state.external_count.set(count - 1);
        if count == 1 {
            *state.external_redraw.borrow_mut() = None;
            *state.external_render.borrow_mut() = None;
            if resume {
                state.needs_render.set(true);
            }
        }
        update_link_demand(&state);
    }

    /// Imports the compositor-owned target as this frame's output and
    /// reports whether the producer is ready: a live context plus an
    /// effect bundle set up on it. `false` defers the frame — the
    /// readiness/redraw contract re-arms it, never a per-interval retry.
    fn prepare_external_render(
        &self,
        texture: &objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>,
    ) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        // SAFETY: `texture` is the compositor's live retained target for
        // this capture; `retain` takes our own reference on it.
        let retained =
            unsafe { Retained::<MetalTexture>::retain(std::ptr::from_ref(texture).cast_mut()) }
                .expect("FilteredView external render received a null texture");
        assert_eq!(
            retained.pixelFormat(),
            output_pixel_format(),
            "FilteredView external target must be in the presentation format"
        );
        if state.render_failed.borrow().is_some() {
            // Prepared without a target: `render_prepared_external_texture`
            // answers the settled terminal failure itself, matching the
            // GPU surface's hold contract — a failed view never defers a
            // parent capture into a wait its context cannot end.
            return true;
        }
        let context = state.runtime.context();
        if context.device_lost_reason().is_some() {
            return false;
        }
        ensure_filtered_generation(&state, &context);
        // A collapsed view's answer needs no effect input, so it never
        // waits on setup.
        let collapsed = state.collapsed.get();
        if !collapsed && !effects_ready(&state) {
            start_setup(&state);
            return false;
        }
        let width = u32::try_from(retained.width()).unwrap_or(u32::MAX);
        let height = u32::try_from(retained.height()).unwrap_or(u32::MAX);
        // SAFETY: `retained` is the compositor's target, alive through the
        // render it was prepared for; format and size describe that
        // texture. The target is a render destination only — no sampled or
        // copy usage.
        let output = unsafe {
            cocoa_ui::metal::import_texture(
                context.device(),
                retained.clone(),
                PRESENTATION_FORMAT,
                width,
                height,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                wgpu::TextureUses::COLOR_TARGET,
                "FilteredView External Output",
            )
        };
        let target = ExternalTarget {
            texture: retained,
            output,
            generation: context.generation(),
        };
        *state.external_render.borrow_mut() = Some(if collapsed {
            ExternalRender::Transparent(target)
        } else {
            ExternalRender::Effects(target)
        });
        true
    }

    /// Captures the hidden child and encodes the effect chain into the
    /// prepared compositor target — or, for a view collapsed when it was
    /// prepared, clears that target to transparent — and answers the completion exactly once
    /// on the main thread — `Ok(())` for usable pixels,
    /// `Err(CaptureError::Deferred)` for a lost context, a stale preparation or a
    /// frame that never submitted, `Err(CaptureError::Failed)` for the
    /// leaf's own settled failure or a failed child's typed carrier. No
    /// screen drawable is ever borrowed for this offscreen render.
    #[expect(
        clippy::too_many_lines,
        reason = "the offscreen render's fence and deferred-settle states live in one pass"
    )]
    fn render_prepared_external_texture(
        &self,
        texture: &objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>,
        width: u32,
        height: u32,
        completion: SurfaceCaptureCompletion,
    ) {
        let Some(state) = self.state.upgrade() else {
            completion(Err(CaptureError::Deferred));
            return;
        };
        if let Some((_, error)) = &*state.render_failed.borrow() {
            // A settled terminal failure answers the capture with its own
            // typed carrier — deferred would wait on a redraw the failed
            // generation never issues.
            completion(Err(CaptureError::Failed(error.clone())));
            return;
        }
        let context = state.runtime.context();
        if context.device_lost_reason().is_some() {
            completion(Err(CaptureError::Deferred));
            arm_filtered_context_watch(&state, context.generation());
            return;
        }
        // The generation check runs before any slot is read: `ensure` can
        // retire the prepared pair, and a slot fetched ahead of it would
        // encode on the new context with old-generation resources.
        ensure_filtered_generation(&state, &context);
        let prepared = state.external_render.borrow().clone();
        let Some(prepared) = prepared else {
            completion(Err(CaptureError::Deferred));
            return;
        };
        if !std::ptr::eq(texture, Retained::as_ptr(&prepared.target().texture))
            || prepared.target().generation != context.generation()
        {
            // A render for a texture `prepare_external_render` never saw,
            // or a preparation from a superseded generation — stale, not a
            // hard failure.
            completion(Err(CaptureError::Deferred));
            return;
        }
        let output = match prepared {
            ExternalRender::Transparent(target) => {
                render_transparent(&state, &context, &target.output, completion);
                return;
            }
            ExternalRender::Effects(target) => target.output,
        };
        if !effects_ready(&state) {
            completion(Err(CaptureError::Deferred));
            return;
        }
        let input_native = ensure_capture_texture(&state, &context, width, height);
        let input = imported_input(&state, &context, input_native.clone(), width, height);
        let completion = std::sync::Mutex::new(Some(completion));
        let weak = Sendable(Rc::downgrade(&state));
        // The capture completion is `Fn` — the output and context cross it
        // inside the same slots the screen path uses.
        let output_slot = std::sync::Mutex::new(Some(output));
        let input_slot = std::sync::Mutex::new(Some(input));
        let submitted = std::sync::Mutex::new(Some(context.clone()));
        state
            .capture
            .capture(&input_native, context.generation(), move |outcome| {
                let completion = completion.lock().expect("capture completion lock").take();
                let Some(completion) = completion else {
                    return;
                };
                let Some(state) = weak.get().upgrade() else {
                    completion(Err(CaptureError::Deferred));
                    return;
                };
                let output = output_slot
                    .lock()
                    .expect("external render fires once")
                    .take();
                let input = input_slot
                    .lock()
                    .expect("external render fires once")
                    .take();
                let context = submitted.lock().expect("external render fires once").take();
                let (Some(output), Some(input), Some(context)) = (output, input, context) else {
                    completion(Err(CaptureError::Deferred));
                    return;
                };
                match outcome {
                    Err(CaptureError::Failed(error)) => {
                        // A failed child settles this capture with its
                        // typed failure — the outcome is terminal on the
                        // child's context: the parent answers it once and
                        // does not keep deferring for a redraw the failed
                        // context never issues. The leaf settles failed
                        // with it — one failure record, onscreen and
                        // external scheduling alike gate on it, and the
                        // shared settle resolves readiness so a first-paint
                        // waiter on this path is woken too.
                        settle_capture_failure(&state, context.generation(), error.clone());
                        completion(Err(CaptureError::Failed(error)));
                        return;
                    }
                    Err(CaptureError::Deferred) => {
                        // The deferred frame produced no pixels — the fence
                        // reports it once; the child's readiness replays through
                        // the redraw contract, not a retry.
                        completion(Err(CaptureError::Deferred));
                        if context.device_lost_reason().is_some() {
                            arm_filtered_context_watch(&state, context.generation());
                        }
                        return;
                    }
                    Ok(()) => {}
                }
                if context.device_lost_reason().is_some()
                    || state.runtime.context().generation() != context.generation()
                    || !effects_ready(&state)
                {
                    // Lost mid-capture or superseded — nothing encodes onto a
                    // generation this target was never prepared for.
                    completion(Err(CaptureError::Deferred));
                    if context.device_lost_reason().is_some() {
                        arm_filtered_context_watch(&state, context.generation());
                    }
                    return;
                }
                let timing = {
                    let mut frame_clock = state.frame_clock.borrow_mut();
                    if state.timeline_parked.replace(false) {
                        let _ = frame_clock.tick();
                    }
                    frame_clock.tick()
                };
                let (needs_redraw, encoder) =
                    encode_effects(&state, &context, &input, &output, width, height, timing);
                drop(input);
                drop(output);
                if needs_redraw {
                    request_render(&state);
                }
                submit_external(&state, &context, encoder, completion);
            });
    }

    /// The readiness-participation query the first-frame collect walk
    /// consults.
    /// Never answered as a fake presented receipt for an excluded view.
    fn participates_in_first_paint(&self) -> bool {
        self.state
            .upgrade()
            .is_some_and(|state| participates_in_first_paint_ready(&state))
    }

    /// `true` only after a frame actually presented on screen — offscreen
    /// completion never mints readiness.
    fn has_presented_frame(&self) -> bool {
        self.state
            .upgrade()
            .is_some_and(|state| state.output_revealed.get())
    }

    /// `requestReadyFrame` through the trait: layout, GPU init, a demand
    /// request and the waker push — the waker answers on the first
    /// presented output or wakes on lifecycle end.
    fn register_ready_waiter(&self, waker: std::task::Waker) {
        let Some(state) = self.state.upgrade() else {
            waker.wake();
            return;
        };
        request_ready_frame(&state, waker);
    }

    /// `true` once the filter's first frame can never land — the settled
    /// terminal failure the capture wait resolves on.
    fn presentation_failed(&self) -> bool {
        self.state
            .upgrade()
            .is_none_or(|state| state.render_failed.borrow().is_some())
    }
}

/// Dropping clears the filter's registrations and shuts the capture and
/// render state down — `deinit`.
struct FilteredGuard {
    view: Retained<HostView>,
    state: Rc<FilteredState>,
}

impl fmt::Debug for FilteredGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilteredGuard").finish_non_exhaustive()
    }
}

impl Drop for FilteredGuard {
    fn drop(&mut self) {
        crate::invalidation::unregister_sink(&self.view);
        self.view.capturable_slot().clear();
        self.state.capture.shutdown();
        detach_if_needed(&self.state);
    }
}

/// `fuseEnclosedFilters` — folds the filters this one directly encloses
/// into the chain this leaf renders, returning the view the chain captures.
///
/// The resolve walk expands `body()` on every composable view; when it
/// lands on another
/// `Native<FilteredView>` its effect joins the chain behind this one's —
/// one capture, one presentation target and one submission instead of two.
fn fuse_enclosed_filters(
    filtered: FilteredView,
    ctx: &RenderContext<'_>,
) -> (AnyView, Vec<(AnyEffect, ParamGuards)>) {
    let mut effects = Vec::new();
    let FilteredView {
        mut content,
        effect,
        guards,
    } = filtered;
    effects.push((effect, guards));
    loop {
        while !is_native_boundary(&content) {
            content = AnyView::new(content.body(ctx.env()));
        }
        match content.downcast::<waterui_core::Native<FilteredView>>() {
            Ok(inner) => {
                let inner = (*inner).into_inner();
                content = inner.content;
                effects.push((inner.effect, inner.guards));
            }
            Err(content) => return (content, effects),
        }
    }
}

/// Installs the `filtered` handler — `applied_filter` and `view_effect`
/// collapse into one leaf under the cutover's `Native<FilteredView>`.
#[allow(clippy::too_many_lines)]
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<FilteredView>(|filtered, ctx| {
        let mtm = ctx.mtm();
        let runtime = crate::gpu_runtime::runtime(ctx.env());
        let (content, chained) = fuse_enclosed_filters(filtered, ctx);
        let (sources, guard_list): (Vec<AnyEffect>, Vec<ParamGuards>) = chained.into_iter().unzip();
        // The chain presents outermost-last: effects render content-adjacent
        // first, so reverse the collection order.
        let effects: Vec<Box<dyn ErasedEffect>> =
            sources.into_iter().rev().map(AnyEffect::build).collect();

        let view = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        #[cfg(target_os = "macos")]
        cocoa_ui::view::ensure_layer_backed(&view);

        let mounted = ctx.render(content).mount(&view);
        let child_view = cocoa_ui::view::retain_base(mounted.view());
        build_filtered_parts(
            mtm,
            runtime,
            &view,
            &child_view,
            Some(mounted),
            effects,
            guard_list,
        )
        .leaf
    });
}

/// The built filtered leaf and the owned pieces its settlement paths keep —
/// `install` keeps the leaf; the native-test fixture drives the state and
/// capturable handles directly.
struct FilteredParts {
    /// The mounted leaf the dispatcher returns.
    leaf: NativeLeaf,
    /// The leaf's render state — read by the native-test fixture.
    #[cfg_attr(
        any(not(feature = "native-test"), not(target_os = "macos")),
        expect(dead_code, reason = "only the macOS native-test fixture reads it")
    )]
    state: Rc<FilteredState>,
    /// The host view the leaf presents through — read by the fixture.
    #[cfg_attr(
        any(not(feature = "native-test"), not(target_os = "macos")),
        expect(dead_code, reason = "only the macOS native-test fixture reads it")
    )]
    view: Retained<HostView>,
    /// The output's `CapturableSurface` face — read by the fixture.
    #[cfg_attr(
        any(not(feature = "native-test"), not(target_os = "macos")),
        expect(dead_code, reason = "only the macOS native-test fixture reads it")
    )]
    capturable: Rc<dyn CapturableSurface>,
}

/// The mount construction `install`'s handler performs once the child is
/// mounted into the host view: hidden-content wiring, the output view and
/// presentation layer, the render state, the handlers, the capturable
/// slot and the keep-alive assembly.
#[expect(
    clippy::too_many_lines,
    reason = "the mount's wiring is one linear construction sequence"
)]
fn build_filtered_parts(
    mtm: MainThreadMarker,
    runtime: GpuRuntime,
    view: &Retained<HostView>,
    child_view: &PlatformView,
    mounted: Option<Mounted>,
    effects: Vec<Box<dyn ErasedEffect>>,
    guard_list: Vec<ParamGuards>,
) -> FilteredParts {
    // `setupContentView`/`hideUnfilteredContent`: the unfiltered child
    // sits underneath and hidden — hidden as a *view*, because
    // `cacheDisplay` walks the view tree and ignores a hidden backing
    // layer.
    crate::primary_content::forward(view, child_view);
    let child_view = cocoa_ui::view::retain_base(child_view);
    #[cfg(target_os = "macos")]
    cocoa_ui::view::ensure_layer_backed(&child_view);
    cocoa_ui::view::set_hidden(&child_view, true);

    // `setupOutputView`: a layer-backed sibling drawn last. Its
    // presentation is the `CAMetalLayer` sublayer the
    // `CAMetalDisplayLink` presenter fills — explicit capture, never
    // `CALayer.contents`.
    let output_view = cocoa_ui::PlatformView::new(mtm);
    #[cfg(target_os = "macos")]
    cocoa_ui::view::ensure_layer_backed(&output_view);
    cocoa_ui::view::set_hidden(&output_view, true);
    if let Some(layer) = cocoa_ui::view::layer(&output_view) {
        layer.setOpaque(false);
        cocoa_ui::core_animation::set_contents_gravity_resize(&layer);
        cocoa_ui::core_animation::set_background_clear(&layer);
    }
    if let Some(host_layer) = cocoa_ui::view::layer(view) {
        cocoa_ui::core_animation::set_background_clear(&host_layer);
    }

    // The presentation `CAMetalLayer` — `MetalPresenter::new` applies
    // the constructor configuration (framebuffer-only, two drawables,
    // nontransactional presents, resize gravity); this leaf sets the
    // output pixel format and owns frame/hidden/drawableSize
    // bookkeeping.
    let presentation_layer = CAMetalLayer::new();
    presentation_layer.setPixelFormat(output_pixel_format());
    presentation_layer.setOpaque(false);
    cocoa_ui::core_animation::set_background_clear(&presentation_layer);
    if let Some(output_layer) = cocoa_ui::view::layer(&output_view) {
        output_layer.addSublayer(&presentation_layer);
    }
    cocoa_ui::view::add_subview(view, &output_view);

    let gpu_context = runtime.context();
    // SAFETY: `raw_device` is the `MTLDevice` the runtime created and
    // still owns; `retain` takes our own reference on it.
    let device = unsafe {
        Retained::<MetalDevice>::retain(
            Retained::as_ptr(
                gpu_context
                    .device()
                    .as_hal::<wgpu_hal::api::Metal>()
                    .expect("the Apple runtime's device is Metal")
                    .raw_device(),
            )
            .cast_mut(),
        )
        .expect("the Metal device is non-null")
    };

    let state = Rc::new_cyclic(|weak| {
        let frame_sink: Rc<dyn Fn(DrawableFrame)> = Rc::new({
            let weak = weak.clone();
            move |frame| {
                if let Some(state) = weak.upgrade() {
                    render_frame(&state, frame);
                }
            }
        });
        let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(mtm, child_view.clone()));
        FilteredState {
            view: view.clone(),
            output_view: output_view.clone(),
            content_view: child_view.clone(),
            presentation_layer: presentation_layer.clone(),
            device: RefCell::new(device),
            effects: Rc::new(RefCell::new(None)),
            runtime,
            frame_clock: RefCell::new(EffectFrameClock::new()),
            timeline_parked: Cell::new(true),
            setup_generation: Rc::new(Cell::new(None)),
            imported_texture: RefCell::new(None),
            input_size: Cell::new((0, 0)),
            attached: Cell::new(false),
            presenter: RefCell::new(None),
            frame_sink,
            capture_texture: RefCell::new(None),
            frame_presentation_in_flight: Cell::new(false),
            render_in_flight: Cell::new(false),
            detach_after_capture: Cell::new(false),
            pending_dynamic_range: RefCell::new(None),
            configured_range: Cell::new(None),
            needs_render: Cell::new(false),
            first_paint_owed: Cell::new(false),
            output_revealed: Cell::new(false),
            render_failed: RefCell::new(None),
            current_scale: Cell::new(1.0),
            laid_out_geometry: RefCell::new(None),
            content_changed_since_capture: Cell::new(false),
            ready_waiters: RefCell::new(Vec::new()),
            mounted: RefCell::new(mounted),
            capture,
            capture_suppression: Cell::new(0),
            external_count: Cell::new(0),
            external_redraw: RefCell::new(None),
            external_render: RefCell::new(None),
            // Unframed: the output view has no area until
            // `update_output_frame` frames it from the host bounds.
            collapsed: Cell::new(true),
            observers: RefCell::new(Vec::new()),
            gpu_generation: Cell::new(Some(gpu_context.generation())),
            context_watch: RefCell::new(None),
            setup_task: RefCell::new(None),
        }
    });
    *state.effects.borrow_mut() = Some(effects);

    // `installRedrawCallback` — every effect's redraw callback lands on
    // the main queue.
    let callback: EffectRedrawCallback = Arc::new(redraw_callback_for(&state));
    for effect in state
        .effects
        .borrow_mut()
        .as_mut()
        .expect("effects installed above")
    {
        effect.set_redraw_callback(Arc::clone(&callback));
    }

    // `capturePipeline.onRedraw`.
    {
        let weak = Rc::downgrade(&state);
        state.capture.set_on_redraw(move || {
            if let Some(state) = weak.upgrade() {
                request_render(&state);
            }
        });
    }

    // The view retains its handlers and the state retains the view —
    // the `Rc<FilteredState> → HostView → handler → Rc<FilteredState>`
    // cycle (WaterUI #1567) is broken by the leaf's `Drop`, which
    // clears every handler slot when the leaf is released.
    {
        let state = Rc::clone(&state);
        view.set_layout_handler(move |_| {
            on_layout(&state);
        });
    }
    {
        let state = Rc::clone(&state);
        view.set_window_handler(move |_| {
            handle_window_change(&state);
        });
    }
    #[cfg(target_os = "macos")]
    {
        let state = Rc::clone(&state);
        view.set_backing_changed_handler(move |_| {
            if cocoa_ui::view::window(&state.view).is_none() {
                return;
            }
            initialize_gpu(&state);
            request_render(&state);
        });
    }
    #[cfg(target_os = "ios")]
    {
        // `registerForTraitChanges(UITraitDisplayScale)` — the kit
        // surfaces scale changes through layout/window transitions.
    }

    // `WuiRenderedContentInvalidationSink` — an invalidated descendant
    // redraws the capture and invalidates upward.
    let sink_callback: Rc<dyn Fn()> = Rc::new({
        let weak = Rc::downgrade(&state);
        move || {
            if let Some(state) = weak.upgrade() {
                state.content_changed_since_capture.set(true);
                request_render(&state);
                crate::invalidation::invalidate_rendered_content(&state.view);
            }
        }
    });
    crate::invalidation::register_sink(view, sink_callback);

    // This output is itself a capturable surface — stored on its own
    // view and removed by the mount guard.
    let capturable: Rc<dyn CapturableSurface> = Rc::new(FilteredCapturable {
        state: Rc::downgrade(&state),
    });
    view.capturable_slot().install(&capturable);

    let filter_guard = FilteredGuard {
        view: view.clone(),
        state: state.clone(),
    };
    let mut leaf = NativeLeaf::new(
        view,
        FilteredSubView {
            state: state.clone(),
        },
    );
    leaf.keep(view.clone());
    leaf.keep(state.clone());
    leaf.keep(capturable.clone());
    leaf.keep(filter_guard);
    for guards in guard_list {
        leaf.keep(guards);
    }
    FilteredParts {
        leaf,
        state,
        view: view.clone(),
        capturable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The presentation format is the extended-range half-float target the
    /// `CAMetalLayer` path always rendered in, and it maps to Metal.
    #[test]
    fn presentation_format_maps_to_metal() {
        assert_eq!(PRESENTATION_FORMAT, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            cocoa_ui::metal::wgpu_to_metal_format(PRESENTATION_FORMAT),
            objc2_metal::MTLPixelFormat::RGBA16Float
        );
    }
}

/// The fixture the native trials mount: a filtered leaf through the real
/// `build_filtered_parts` construction over a real `SceneView` GPU-surface
/// child on the parent's runtime, plus the probe handles each settle
/// contract assertion needs. Compiled only with `native-test` on macOS;
/// never in a production build, never on iOS.
#[cfg(all(feature = "native-test", target_os = "macos"))]
pub mod native_test {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::task::{Wake, Waker};

    use std::future::Future;
    use std::pin::Pin;

    use futures::channel::oneshot;
    use waterui_graphics::filtrate::{
        EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderResult,
        EffectSetupResult,
    };

    use objc2_metal::{
        MTLDevice as _, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode,
        MTLTexture as _, MTLTextureDescriptor, MTLTextureUsage,
    };

    use super::{
        CapturableSurface, CaptureError, ErasedEffect, FilteredParts, FilteredState, GpuRuntime,
        HostView, MetalTexture, NativeLeaf, PlatformView, Rc, RefCell, Retained,
        build_filtered_parts, fmt, settle_capture_failure, wgpu,
    };
    use crate::components::gpu_surface::native_test::{MountedSceneSurface, fixture_env};
    use cocoa_ui::MainThreadMarker;
    use std::sync::Arc;

    /// A filtered leaf mounted through the production
    /// [`build_filtered_parts`] construction.
    ///
    /// The child is a real `MountedSceneSurface` mounted into the host's
    /// hidden content subtree — `child` stays owned beside the leaf so a
    /// trial settles the child first and feeds the carrier the child's
    /// capturable answers back into the filtered settle.
    #[must_use]
    pub struct MountedFilteredSurface {
        /// The mounted filtered leaf — dropping it unmounts the leaf.
        pub leaf: NativeLeaf,
        /// The filtered leaf's render state.
        state: Rc<FilteredState>,
        /// The host view the leaf presents through.
        view: Retained<HostView>,
        /// The output's `CapturableSurface` face.
        capturable: Rc<dyn CapturableSurface>,
        /// A filtered leaf nested between this leaf and `child` — its host
        /// is this leaf's hidden content.
        nested: Option<FilteredParts>,
        /// The view holding the host inside the fixture window: `AppKit` owns
        /// the window's content view, the trials size this one.
        container: Retained<PlatformView>,
        /// The GPU-surface child mounted inside the hidden content subtree.
        pub child: MountedSceneSurface,
        /// The shared GPU runtime.
        pub runtime: GpuRuntime,
        /// The real window that makes the leaf presentable.
        fixture_window: RefCell<Option<cocoa_ui::appkit::Window>>,
    }

    impl fmt::Debug for MountedFilteredSurface {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("MountedFilteredSurface")
                .field("attached", &self.state.attached.get())
                .finish_non_exhaustive()
        }
    }

    /// A readiness waiter registered through the real
    /// `CapturableSurface::register_ready_waiter` — `wakes` counts how
    /// many times `complete_ready` drained it.
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

    /// The colour [`FillEffect`] clears its output to — opaque, so a
    /// readback tells a filled region from a transparent one.
    pub const FILL: wgpu::Color = wgpu::Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    /// A fixture effect: every frame clears its output to [`FILL`]. A
    /// gated setup lands only once the trial releases its [`SetupGate`].
    struct FillEffect {
        gate: Option<oneshot::Receiver<()>>,
    }

    impl ErasedEffect for FillEffect {
        fn set_redraw_callback(&mut self, _callback: EffectRedrawCallback) {}

        fn setup<'a>(
            &'a mut self,
            _ctx: &'a EffectContext<'a>,
        ) -> Pin<Box<dyn Future<Output = EffectSetupResult> + 'a>> {
            let gate = self.gate.take();
            Box::pin(async move {
                if let Some(gate) = gate {
                    // A gate dropped unreleased ends with its trial; the
                    // setup has nothing left to wait for.
                    let _released = gate.await;
                }
                Ok(())
            })
        }

        fn encode_render(
            &mut self,
            _input: &EffectInput<'_>,
            output: &EffectOutput<'_>,
            encoder: &mut wgpu::CommandEncoder,
        ) -> EffectRenderResult {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("filtered fixture fill"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(FILL),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            Ok(false)
        }

        fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
            (input_width, input_height)
        }

        fn redraw_hint(&self) -> bool {
            false
        }
    }

    /// Holds a nested [`FillEffect`]'s setup until [`Self::release`].
    pub struct SetupGate(RefCell<Option<oneshot::Sender<()>>>);

    impl fmt::Debug for SetupGate {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("SetupGate")
                .field("released", &self.0.borrow().is_none())
                .finish()
        }
    }

    impl SetupGate {
        /// Lets the gated setup land.
        ///
        /// # Panics
        ///
        /// When the gate was already released or its setup is gone.
        pub fn release(&self) {
            self.0
                .borrow_mut()
                .take()
                .expect("the gate is released once")
                .send(())
                .expect("the gated setup is still waiting");
        }
    }

    /// A filtered leaf over `child_view` through the production
    /// [`build_filtered_parts`] construction. The host view adds the child
    /// itself — `build_filtered_parts` takes the same child-view reference
    /// the production `mounted.view()` hands it, and `mounted: None` is
    /// already a state every reader tolerates.
    fn filtered_parts(
        mtm: MainThreadMarker,
        runtime: &GpuRuntime,
        child_view: &PlatformView,
        effects: Vec<Box<dyn ErasedEffect>>,
    ) -> FilteredParts {
        let view = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 64.0, 64.0));
        cocoa_ui::view::ensure_layer_backed(&view);
        cocoa_ui::view::add_subview(&view, child_view);
        build_filtered_parts(
            mtm,
            runtime.clone(),
            &view,
            child_view,
            None,
            effects,
            Vec::new(),
        )
    }

    /// A parent `ViewCapture` over a fixture container, rendering into a
    /// shared `RGBA8Unorm` target a readback can read.
    pub struct ParentCapture {
        capture: Rc<cocoa_ui::capture::ViewCapture>,
        target: Retained<MetalTexture>,
        generation: u64,
    }

    impl fmt::Debug for ParentCapture {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("ParentCapture")
                .field("generation", &self.generation)
                .finish_non_exhaustive()
        }
    }

    /// The slot a [`ParentCapture`]'s completion lands in.
    #[derive(Debug, Clone, Default)]
    pub struct ParentOutcome(Arc<std::sync::Mutex<Option<Result<(), CaptureError>>>>);

    impl ParentOutcome {
        /// Whether the capture's completion has run.
        pub fn landed(&self) -> bool {
            self.0.lock().expect("outcome lock").is_some()
        }

        /// The capture's outcome, once landed.
        pub fn take(&self) -> Option<Result<(), CaptureError>> {
            self.0.lock().expect("outcome lock").take()
        }
    }

    impl ParentCapture {
        /// Starts a capture: its prepare snapshots the subtree now, and
        /// its submit follows from the main queue.
        pub fn start(&self) -> ParentOutcome {
            let outcome = ParentOutcome::default();
            let slot = outcome.0.clone();
            self.capture
                .capture(&self.target, self.generation, move |result| {
                    *slot.lock().expect("outcome lock") = Some(result);
                });
            outcome
        }

        /// The target's pixels, RGBA8 row-major.
        pub fn pixels(&self) -> Vec<[u8; 4]> {
            let (width, height) = (self.target.width(), self.target.height());
            let mut bytes = vec![0u8; width * height * 4];
            // SAFETY: `bytes` holds `width*4` bytes per row for `height`
            // rows and the target is `.shared` — the region stays in bounds.
            unsafe {
                self.target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                    std::ptr::NonNull::new(bytes.as_mut_ptr().cast())
                        .expect("the texel buffer is non-null"),
                    width * 4,
                    MTLRegion {
                        origin: MTLOrigin { x: 0, y: 0, z: 0 },
                        size: MTLSize {
                            width,
                            height,
                            depth: 1,
                        },
                    },
                    0,
                );
            }
            bytes.as_chunks::<4>().0.to_vec()
        }
    }

    impl Drop for ParentCapture {
        fn drop(&mut self) {
            self.capture.shutdown();
        }
    }

    impl MountedFilteredSurface {
        /// Mounts the `SceneView` child through `build_surface_parts` and
        /// builds the filtered leaf over it with an empty effect chain —
        /// the settle contract under trial is the failure's, not an
        /// effect's. Async because adapter/device creation is; the trial
        /// drives the future with its own executor (`pollster` is a
        /// dev-dep — unavailable in-crate).
        ///
        /// # Errors
        ///
        /// When the runner has no GPU adapter.
        #[expect(
            clippy::future_not_send,
            reason = "the mount is a main-thread fixture: Rc<FilteredState>, the MainThreadMarker and the Objective-C views stay on the trial's main thread"
        )]
        pub async fn mount(mtm: MainThreadMarker) -> Result<Self, String> {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            Self::mount_on(mtm, runtime, Vec::new())
        }

        /// [`Self::mount`] with one [`FillEffect`] whose setup lands at
        /// once, so every frame fills the output with [`FILL`].
        ///
        /// # Errors
        ///
        /// When the runner has no GPU adapter.
        #[expect(
            clippy::future_not_send,
            reason = "the mount is a main-thread fixture: Rc<FilteredState>, the MainThreadMarker and the Objective-C views stay on the trial's main thread"
        )]
        pub async fn mount_filling(mtm: MainThreadMarker) -> Result<Self, String> {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            Self::mount_on(mtm, runtime, vec![Box::new(FillEffect { gate: None })])
        }

        /// An empty-chain filtered leaf whose hidden content is a nested
        /// filtered leaf over the `SceneView` child; the nested leaf's
        /// [`FillEffect`] setup waits on the returned gate, so the outer
        /// leaf's captures defer on a live context until it is released.
        ///
        /// # Errors
        ///
        /// When the runner has no GPU adapter.
        #[expect(
            clippy::future_not_send,
            reason = "the mount is a main-thread fixture: Rc<FilteredState>, the MainThreadMarker and the Objective-C views stay on the trial's main thread"
        )]
        pub async fn mount_over_pending_filter(
            mtm: MainThreadMarker,
        ) -> Result<(Self, SetupGate), String> {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            let child = MountedSceneSurface::mount_in(mtm, &fixture_env(runtime.clone()))?;
            let (release, gate) = oneshot::channel();
            let nested = filtered_parts(
                mtm,
                &runtime,
                &child.platform_view(),
                vec![Box::new(FillEffect { gate: Some(gate) })],
            );
            let outer = filtered_parts(
                mtm,
                &runtime,
                &cocoa_ui::view::retain_base(&nested.view),
                Vec::new(),
            );
            Ok((
                Self::assemble(mtm, outer, Some(nested), child, runtime),
                SetupGate(RefCell::new(Some(release))),
            ))
        }

        fn mount_on(
            mtm: MainThreadMarker,
            runtime: GpuRuntime,
            effects: Vec<Box<dyn ErasedEffect>>,
        ) -> Result<Self, String> {
            let child = MountedSceneSurface::mount_in(mtm, &fixture_env(runtime.clone()))?;
            let parts = filtered_parts(mtm, &runtime, &child.platform_view(), effects);
            Ok(Self::assemble(mtm, parts, None, child, runtime))
        }

        fn assemble(
            mtm: MainThreadMarker,
            parts: FilteredParts,
            nested: Option<FilteredParts>,
            child: MountedSceneSurface,
            runtime: GpuRuntime,
        ) -> Self {
            let container = PlatformView::new(mtm);
            cocoa_ui::view::ensure_layer_backed(&container);
            cocoa_ui::view::set_frame(&container, cocoa_ui::view::bounds(&parts.view));
            cocoa_ui::view::add_subview(&container, &cocoa_ui::view::retain_base(&parts.view));
            Self {
                leaf: parts.leaf,
                state: parts.state,
                view: parts.view,
                capturable: parts.capturable,
                nested,
                container,
                child,
                runtime,
                fixture_window: RefCell::new(None),
            }
        }

        /// Uses a real `AppKit`/`UIKit` window — the attach door's only
        /// host requirement — then pumps the main queue until the leaf's
        /// own setup lands and the link attaches.
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
            let content = PlatformView::new(mtm);
            cocoa_ui::view::add_subview(&content, &self.container);
            window.set_content_view(&content);
            window.make_key_and_order_front();
            *self.fixture_window.borrow_mut() = Some(window);
            let platform = cocoa_ui::view::retain_base(&self.view);
            let deadline = NSDate::dateWithTimeIntervalSinceNow(5.0);
            while !cocoa_ui::view::window(&platform)
                .is_some_and(|window| cocoa_ui::appkit::is_visible(&window))
                && deadline.timeIntervalSinceNow() > 0.0
            {
                NSRunLoop::currentRunLoop()
                    .runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
            }
            assert!(
                cocoa_ui::view::window(&platform)
                    .is_some_and(|window| cocoa_ui::appkit::is_visible(&window)),
                "the fixture window reports visible"
            );
        }

        /// Attaches through the production door: the fixture window makes
        /// the leaf presentable, `initialize_gpu` builds the real
        /// `MetalPresenter`, and the main-queue pump lets the (empty)
        /// effect setup land so `effects_ready` opens the link.
        ///
        /// # Panics
        ///
        /// When the attach door does not install the epoch in time.
        pub fn ensure_attached(&self) {
            self.ensure_fixture_window();
            super::initialize_gpu(&self.state);
            assert!(
                crate::native_test_support::pump_main_until(5.0, || self.state.attached.get()),
                "the attach door installs the Attached epoch"
            );
        }

        /// Whether a frame actually presented on screen — the production
        /// `has_presented_frame` an enclosing capture reads.
        pub fn frame_presented(&self) -> bool {
            self.capturable.has_presented_frame()
        }

        /// The host view's bounds size — what `update_output_frame` frames
        /// the hidden content with.
        pub fn host_bounds(&self) -> cocoa_ui::geometry::Size {
            cocoa_ui::view::bounds(&self.view).size
        }

        /// Sets the host's frame, then runs its layout pass — the
        /// production `on_layout` path a parent's layout drives.
        pub fn set_host_frame(&self, width: f64, height: f64) {
            let frame = cocoa_ui::Rect::new(0.0, 0.0, width, height);
            cocoa_ui::view::set_frame(&self.container, frame);
            cocoa_ui::view::set_frame(&self.view, frame);
            cocoa_ui::view::layout_immediately(&self.view);
        }

        /// A parent capture over the fixture container: the filtered leaf
        /// joins it as an external surface.
        ///
        /// # Panics
        ///
        /// Off the main thread, or when the device refuses the target.
        pub fn parent_capture(&self) -> ParentCapture {
            let mtm = MainThreadMarker::new().expect("the fixture runs on the main thread");
            let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(
                mtm,
                self.container.clone(),
            ));
            capture.set_on_redraw(|| {});
            let context = self.runtime.context();
            let device = crate::gpu_runtime::raw_metal_device(&context);
            let bounds = cocoa_ui::view::bounds(&self.container);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the fixture container's pixel dimensions are small and positive"
            )]
            let (width, height) = (
                (bounds.size.width * self.backing_scale()).ceil() as usize,
                (bounds.size.height * self.backing_scale()).ceil() as usize,
            );
            // SAFETY: a 2D descriptor is always valid to construct.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::RGBA8Unorm,
                    width,
                    height,
                    false,
                )
            };
            descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
            descriptor.setStorageMode(MTLStorageMode::Shared);
            let target = device
                .newTextureWithDescriptor(&descriptor)
                .expect("the parent capture target allocates");
            ParentCapture {
                capture,
                target,
                generation: context.generation(),
            }
        }

        /// Whether the nested filtered leaf joined this leaf's capture —
        /// its external-render scope is open.
        pub fn nested_joined_capture(&self) -> bool {
            self.nested
                .as_ref()
                .is_some_and(|nested| nested.state.external_count.get() > 0)
        }

        /// The attach-time capture size in device pixels.
        pub fn input_size(&self) -> (u32, u32) {
            self.state.input_size.get()
        }

        /// The backing scale the leaf converts points to pixels with.
        pub fn backing_scale(&self) -> f64 {
            self.state.current_scale.get()
        }

        /// Whether the leaf holds its presentation epoch.
        pub fn attached(&self) -> bool {
            self.state.attached.get()
        }

        /// Whether a presenter — and with it a display link — exists.
        pub fn has_presenter(&self) -> bool {
            self.state.presenter.borrow().is_some()
        }

        /// The shared runtime's current context generation.
        pub fn current_generation(&self) -> u64 {
            self.runtime.context().generation()
        }

        /// Registers a waiter through the real
        /// `CapturableSurface::register_ready_waiter` — the same slot an
        /// enclosing capture's first-paint collect queues into —
        /// returning the probe.
        pub fn readiness_probe(&self) -> WakeProbe {
            let count = Arc::new(AtomicU32::new(0));
            self.capturable
                .register_ready_waiter(Waker::from(Arc::new(ProbeWake(count.clone()))));
            WakeProbe(count)
        }

        /// Whether the leaf still offers its first frame to an enclosing
        /// capture — the same observable the capture walk reads.
        pub fn first_paint_participation(&self) -> bool {
            self.capturable.participates_in_first_paint()
        }

        /// Whether the link presenter is paused — the observable
        /// `update_link_demand` drives: paused means no frame issues, so
        /// no further renders follow.
        pub fn link_paused(&self) -> bool {
            self.state
                .presenter
                .borrow()
                .as_ref()
                .is_some_and(cocoa_ui::metal_presenter::MetalPresenter::is_paused)
        }

        /// Whether a render is in flight — after a settle, `false` and
        /// stays so.
        pub fn render_in_flight(&self) -> bool {
            self.state.render_in_flight.get()
        }

        /// Records redraw demand through the production `request_render`
        /// — the answer `update_link_demand` must still gate while the
        /// failure stands.
        pub fn request_render(&self) {
            super::request_render(&self.state);
        }

        /// Drives the settle the `finish_captured_frame` `Failed` arm
        /// performs minus the `PresentWork` teardown the trial cannot
        /// build without a drawable — the production
        /// `settle_capture_failure` itself.
        pub fn settle_failed_capture(
            &self,
            generation: u64,
            error: Arc<dyn std::error::Error + Send + Sync>,
        ) {
            self.state.render_in_flight.set(false);
            settle_capture_failure(&self.state, generation, error);
        }

        /// The outcome a parent capture's `render_prepared_external_texture`
        /// call reads — answered against a real allocated target exactly as
        /// the compositor's, so the settled leaf's terminal contract is
        /// observable, not the state's.
        ///
        /// # Errors
        ///
        /// The `CaptureError` the external render contract answers —
        /// `Failed` carrying the settled typed error, `Deferred` for a
        /// parked or unwritten frame.
        ///
        /// # Panics
        ///
        /// When the target texture allocation, `prepare_external_render`,
        /// or the once-answered completion contract fails.
        pub fn capture_outcome(&self) -> Result<(), cocoa_ui::capture::CaptureError> {
            use objc2_metal::MTLDevice;
            let context = self.runtime.context();
            let device = crate::gpu_runtime::raw_metal_device(&context);
            // SAFETY: a 2D descriptor is always valid to construct, and the
            // allocation matches the declared extent.
            let descriptor = unsafe {
                objc2_metal::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    self.capturable.capture_pixel_format(),
                    64,
                    64,
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
                "the settled leaf still accepts the external render target"
            );
            let outcome = std::sync::Arc::new(std::sync::Mutex::new(None));
            let slot = outcome.clone();
            self.capturable.render_prepared_external_texture(
                &texture,
                64,
                64,
                Box::new(move |result| *slot.lock().expect("outcome slot") = Some(result)),
            );
            outcome
                .lock()
                .expect("outcome slot")
                .take()
                .expect("the external render answers its completion once")
        }
    }
}
