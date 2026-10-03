//! The filtered-content leaf: `Native<FilteredView>` — the
//! `WuiAppliedFilter`/`WuiViewEffect` port. The hidden content view is
//! captured offscreen through the kit's `ViewCapture`; each erased effect in
//! the chain encodes into one shared command buffer — innermost first,
//! chained through private intermediate targets — and the last pass lands in
//! an `IOSurface` pair presented through a plain layer-backed output view
//! composited on top.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cocoa_ui::PlatformView;
use cocoa_ui::Retained;
use executor_core::spawn_local;
use futures::FutureExt;
use objc2_metal::{MTLDevice as _, MTLTexture as _};
use waterui_backend_core::{AnyView, View};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_graphics::filter_view::{AnyEffect, ErasedEffect, FilteredView, ParamGuards};
use waterui_graphics::filtrate::{
    EffectContext, EffectFrameClock, EffectInput, EffectOutput, EffectRedrawCallback, ShapeTextures,
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

/// A captured content frame — `WuiAppliedFilterCaptureFrame`. Carries the
/// exact context its capture was prepared under so `finish` encodes,
/// imports, and submits on that generation — never a re-fetched one.
struct CaptureFrame {
    texture: Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>>,
    width: u32,
    height: u32,
    context: Arc<SharedGpuContext>,
}

impl fmt::Debug for CaptureFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
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

/// The filter host's shared state — `WuiAppliedFilterRenderState` plus the
/// view's frame bookkeeping.
pub struct FilteredState {
    /// The host view.
    view: Retained<HostView>,
    /// The output presentation view.
    output_view: Retained<PlatformView>,
    /// The shared Metal device — `metalDevice`. Recreated when the
    /// runtime publishes a new context generation.
    device: RefCell<Retained<MetalDevice>>,
    /// The erased effect chain, innermost (content-adjacent) first; `None`
    /// while asynchronous setup owns it — the ffi state's effect slot.
    effects: EffectSlot,
    /// The GPU runtime.
    runtime: GpuRuntime,
    /// The host-owned effect clock — `frame_clock` on the ffi state.
    frame_clock: RefCell<EffectFrameClock>,
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
    /// The `IOSurface` presenter on the output layer — `presenter`.
    presenter: RefCell<Option<cocoa_ui::metal::SurfaceBuffers>>,
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
    /// `outputRevealed`/`filteredOutputRevealed`.
    output_revealed: Cell<bool>,
    /// `currentScaleFactor`.
    current_scale: Cell<f64>,
    /// `laidOutGeometry`: a layout pass only requests a frame when the
    /// geometry it produced is new — captures provoke layout passes of
    /// their own, so arming the clock on every pass makes nested filters
    /// drive each other forever.
    laid_out_geometry: RefCell<Option<cocoa_ui::Rect>>,
    /// `contentChangedSinceCapture` — a filter is only ready once it has
    /// shown a frame of the content as it actually stands (#521).
    content_changed_since_capture: Cell<bool>,
    /// First-paint waiters — `readyCompletions` in waker form.
    ready_waiters: RefCell<Vec<std::task::Waker>>,
    /// The hidden content leaf — `contentView`.
    mounted: RefCell<Option<Mounted>>,
    /// The `ViewCapture` pipeline — `capturePipeline`.
    capture: Rc<cocoa_ui::capture::ViewCapture>,
    /// The frame clock — `frameDriver`.
    clock: cocoa_ui::display_link::FrameClock,
    /// Window observers — `occlusionObserver`/app-activation watchers.
    observers: RefCell<Vec<cocoa_ui::notification::NotificationObserver>>,
    /// The context generation `device`/`presenter`/`capture_texture` were
    /// built under — all are recreated when the runtime publishes a new
    /// context.
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

/// The host bounds in physical pixels.
fn pixel_size(view: &PlatformView, scale: f64) -> (u32, u32) {
    let bounds = cocoa_ui::view::bounds(view);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (w, h) = (
        (bounds.size.width * scale).max(0.0) as u32,
        (bounds.size.height * scale).max(0.0) as u32,
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

/// `configureDynamicRange`.
fn configure_dynamic_range(state: &FilteredState, mode: cocoa_ui::dynamic_range::DynamicRange) {
    assert!(
        !state.attached.get(),
        "FilteredView dynamic range cannot change while attached"
    );
    cocoa_ui::dynamic_range::apply_to_view(mode, &state.view);
    if let Some(presenter) = state.presenter.borrow_mut().as_mut() {
        presenter.release();
    }
    state.capture_texture.borrow_mut().take();
    hide_output(state);
    state.configured_range.set(Some(mode));
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
/// `presenter`, `capture_texture`, `imported_texture`, the set-up effects)
/// predates a newly published context and must not be reused, even when
/// the underlying `MTLDevice` is unchanged.
fn ensure_filtered_generation(state: &Rc<FilteredState>, context: &SharedGpuContext) {
    if state.gpu_generation.get() == Some(context.generation()) {
        return;
    }
    let device = crate::gpu_runtime::raw_metal_device(context);
    *state.device.borrow_mut() = device.clone();
    *state.presenter.borrow_mut() = Some(cocoa_ui::metal::SurfaceBuffers::new(
        device,
        cocoa_ui::view::layer(&state.output_view).expect("output view is layer-backed"),
    ));
    *state.capture_texture.borrow_mut() = None;
    *state.imported_texture.borrow_mut() = None;
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
            // A park taken before attach re-runs the GPU initialization so
            // `attach_if_needed` retakes on the published context.
            initialize_gpu(&state);
            state.needs_render.set(true);
            schedule_frame_if_needed(&state);
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
    state.attached.set(true);
    if effects_ready(state) {
        request_render(state);
    } else {
        start_setup(state);
    }
}

/// `detachIfNeeded` — `waterui_applied_filter_detach`.
fn detach_if_needed(state: &FilteredState) {
    if !state.attached.get() {
        return;
    }
    state.attached.set(false);
    state.imported_texture.borrow_mut().take();
    state.capture_texture.borrow_mut().take();
    state.input_size.set((0, 0));
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

/// `initializeGpuIfNeeded`.
fn initialize_gpu(state: &Rc<FilteredState>) {
    let bounds = cocoa_ui::view::bounds(&state.view);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return;
    }
    let Some(window) = cocoa_ui::view::window(&state.view) else {
        return;
    };
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
    // once `schedule_frame_if_needed` could arm the clock (#576).
    if !can_attach_now(&state.view) {
        return;
    }
    // One explicit live context for everything below: on loss the whole
    // initialization parks before attach or any native allocation, and the
    // publication watch re-runs `initialize_gpu` on the rebuilt context.
    let context = state.runtime.context();
    if context.device_lost_reason().is_some() {
        arm_filtered_context_watch(state, context.generation());
        return;
    }
    // Device-bound state always binds to this exact generation — including
    // when the view is already attached — so nothing later allocates on a
    // stale device.
    ensure_filtered_generation(state, &context);
    let (width, height) = pixel_size(&state.view, scale);
    attach_if_needed(state, &context, width, height);
    let _ = ensure_capture_texture(state, &context, width, height);
}

/// `updateOutputLayerFrame`.
fn update_output_frame(state: &FilteredState) {
    let bounds = cocoa_ui::view::bounds(&state.view);
    cocoa_ui::core_animation::without_animation(|| {
        cocoa_ui::view::set_frame(&state.output_view, bounds);
        if let Some(layer) = cocoa_ui::view::layer(&state.output_view) {
            cocoa_ui::core_animation::set_frame(&layer, bounds);
            cocoa_ui::core_animation::set_contents_scale(&layer, state.current_scale.get());
        }
    });
}

/// `scheduleFrameIfNeeded`.
fn schedule_frame_if_needed(state: &Rc<FilteredState>) {
    if state.attached.get()
        && effects_ready(state)
        && cocoa_ui::view::window(&state.view).is_some()
        && state.needs_render.get()
        && !state.render_in_flight.get()
        && !state.frame_presentation_in_flight.get()
        && !presentation_occluded(&state.view)
    {
        state.clock.start(&state.view);
    } else {
        state.clock.stop();
    }
}

/// `requestRenderIfNeeded`.
fn request_render(state: &Rc<FilteredState>) {
    state.needs_render.set(true);
    schedule_frame_if_needed(state);
}

/// `requestRenderIfGeometryChanged` — only a pass that produced new
/// geometry arms the frame clock; see `laid_out_geometry`.
fn request_render_if_geometry_changed(state: &Rc<FilteredState>) {
    let geometry = cocoa_ui::view::bounds(&state.view);
    if state
        .laid_out_geometry
        .borrow()
        .is_some_and(|g| g == geometry)
    {
        schedule_frame_if_needed(state);
        return;
    }
    *state.laid_out_geometry.borrow_mut() = Some(geometry);
    request_render(state);
}

/// `renderFrame` — capture the hidden content into the input-size texture.
fn render_frame(state: &Rc<FilteredState>) {
    let context = state.runtime.context();
    if context.device_lost_reason().is_some() {
        // Nothing prepares, captures, or submits on a dead device — park
        // until the rebuilt context is published.
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
        schedule_frame_if_needed(state);
        return;
    }
    let (width, height) = pixel_size(&state.view, state.current_scale.get());
    if width == 0 || height == 0 {
        return;
    }
    state.needs_render.set(false);
    state.render_in_flight.set(true);
    state.clock.stop();
    let frame = CaptureFrame {
        texture: ensure_capture_texture(state, &context, width, height),
        width,
        height,
        context,
    };
    let weak = Sendable(Rc::downgrade(state));
    let frame_texture = frame.texture.clone();
    // The capture completion is `Fn` — the frame crosses it inside a slot.
    let frame = std::sync::Mutex::new(Some(Sendable(frame)));
    state.capture.capture(&frame_texture, move |captured| {
        if let Some(state) = weak.get().upgrade() {
            let frame = frame.lock().expect("capture fires once").take();
            if let Some(frame) = frame {
                finish_captured_frame(&state, frame.0, captured);
            }
        }
    });
}

/// `finishCapturedFrame`.
fn finish_captured_frame(state: &Rc<FilteredState>, frame: CaptureFrame, captured: bool) {
    if state.detach_after_capture.get() {
        state.render_in_flight.set(false);
        state.detach_after_capture.set(false);
        detach_if_needed(state);
        if let Some(presenter) = state.presenter.borrow_mut().as_mut() {
            presenter.release();
        }
        complete_ready(state, false);
        return;
    }
    if state.pending_dynamic_range.borrow_mut().take().is_some() {
        state.render_in_flight.set(false);
        detach_if_needed(state);
        initialize_gpu(state);
        request_render(state);
        return;
    }
    if !captured {
        state.render_in_flight.set(false);
        // The deferred frame produced no pixels, so it stays owed —
        // `needs_render` was already consumed when the capture started.
        // A lost submitting context replays only from the next
        // publication; any other deferral is the child's readiness, which
        // the capture contract reports through its redraw notification —
        // re-arming the frame clock here would poll `prepare_external_render`
        // every tick while the child is still setting up.
        state.needs_render.set(true);
        if frame.context.device_lost_reason().is_some() {
            arm_filtered_context_watch(state, frame.context.generation());
        }
        return;
    }
    finish_prepared_frame(state, frame);
}

/// `renderCapturedFrame` — `waterui_applied_filter_render_to_metal_texture`:
/// encode the effect chain into one command buffer and present on its fence.
#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::too_many_lines)]
fn finish_prepared_frame(state: &Rc<FilteredState>, frame: CaptureFrame) {
    // Encode, import, and submit on the exact context the capture was
    // prepared under — a fresh fetch could name a generation this frame's
    // native texture was never prepared for.
    let context = frame.context.clone();
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
        schedule_frame_if_needed(state);
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
    let (output_width, output_height) = (frame.width, frame.height);
    let pixel_format = output_pixel_format();
    {
        let mut presenter = state.presenter.borrow_mut();
        presenter
            .as_mut()
            .expect("FilteredView presenter released while rendering")
            .configure(output_width, output_height, pixel_format);
    }
    let pending = state
        .presenter
        .borrow()
        .as_ref()
        .and_then(cocoa_ui::metal::SurfaceBuffers::next_frame)
        .expect("FilteredView presenter has no texture to render into");
    let input_texture = {
        let mut imported = state.imported_texture.borrow_mut();
        match imported.as_ref() {
            Some(texture) if texture.width() == frame.width && texture.height() == frame.height => {
                texture.clone()
            }
            _ => {
                // SAFETY: `frame.texture` is retained in `frame`, which
                // outlives the import; the format and size describe that
                // same texture.
                let texture = unsafe {
                    cocoa_ui::metal::import_texture(
                        context.device(),
                        frame.texture.clone(),
                        PRESENTATION_FORMAT,
                        frame.width,
                        frame.height,
                        wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        wgpu::TextureUses::COLOR_TARGET,
                        "FilteredView Imported Input Texture",
                    )
                };
                *imported = Some(texture.clone());
                texture
            }
        }
    };
    state.input_size.set((frame.width, frame.height));

    // SAFETY: `pending.texture` is the retained texture the presenter handed
    // us for this frame; the format and size describe that texture.
    let output_wgpu_texture = unsafe {
        cocoa_ui::metal::import_texture(
            context.device(),
            pending.texture.clone(),
            PRESENTATION_FORMAT,
            output_width,
            output_height,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            wgpu::TextureUses::COLOR_TARGET,
            "FilteredView Host Presentation Texture",
        )
    };
    let timing = state.frame_clock.borrow_mut().tick();
    let (needs_redraw, encoder) = {
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
                        width: frame.width,
                        height: frame.height,
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
                &input_texture
            } else {
                &intermediates[index - 1]
            };
            let output_target = if last {
                &output_wgpu_texture
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
                width: frame.width,
                height: frame.height,
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
                width: output_width,
                height: output_height,
            };
            needs_redraw |= effect
                .encode_render(&input, &output, &mut encoder)
                .unwrap_or_else(|error| panic!("filtered render: {error}"))
                || effect.redraw_hint();
        }
        (needs_redraw, encoder)
    };
    drop(input_texture);
    drop(output_wgpu_texture);

    // `observeGpuCaptureFence`: the frame stays in flight until its fence.
    state.frame_presentation_in_flight.set(true);
    let weak = Sendable(Rc::downgrade(state));
    let pending = Sendable(pending);
    let submitted_context = context.clone();
    crate::gpu_completion::submit_with_completion(
        cocoa_ui::MainThreadMarker::new().expect("FilteredView frames render on the main thread"),
        encoder,
        &context,
        move || {
            // `on_submitted_work_done` closures can run on any thread that
            // maintains the device — hop to the main queue before touching
            // the weak state handle.
            cocoa_ui::main_queue::enqueue(move |_mtm| {
                if submitted_context.device_lost_reason().is_some() {
                    // The submitted generation died in flight — the fence
                    // settled on a dead queue and the ring slot holds no
                    // ready pixels. Park until the rebuilt context
                    // publishes instead of revealing it.
                    if let Some(state) = weak.get().upgrade() {
                        state.frame_presentation_in_flight.set(false);
                        state.render_in_flight.set(false);
                        state.needs_render.set(true);
                        arm_filtered_context_watch(&state, submitted_context.generation());
                    }
                    return;
                }
                let Some(state) = weak.get().upgrade() else {
                    return;
                };
                state.frame_presentation_in_flight.set(false);
                finish_presented_frame(&state, pending.get(), needs_redraw, &submitted_context);
            });
        },
    );
}

/// The fence continuation — `present`/`revealFilteredOutput`/`completeReady`.
fn finish_presented_frame(
    state: &Rc<FilteredState>,
    pending: &cocoa_ui::metal::PendingFrame,
    needs_redraw: bool,
    submitted_context: &SharedGpuContext,
) {
    if state.detach_after_capture.get() {
        state.detach_after_capture.set(false);
        detach_if_needed(state);
        if let Some(presenter) = state.presenter.borrow_mut().as_mut() {
            presenter.release();
        }
        complete_ready(state, false);
        return;
    }
    if state.pending_dynamic_range.borrow_mut().take().is_some() {
        detach_if_needed(state);
        initialize_gpu(state);
        request_render(state);
        return;
    }
    if let Some(presenter) = state.presenter.borrow_mut().as_mut() {
        presenter.present(pending);
    }
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
        // the one that captures the change (#521).
        state.needs_render.set(true);
    } else {
        complete_ready(state, true);
    }
    schedule_frame_if_needed(state);
}

/// `revealFilteredOutput` / `hideFilteredOutput`.
fn reveal_output(state: &FilteredState) {
    if state.output_revealed.get() {
        return;
    }
    state.output_revealed.set(true);
    cocoa_ui::view::set_hidden(&state.output_view, false);
}

fn hide_output(state: &FilteredState) {
    state.output_revealed.set(false);
    cocoa_ui::view::set_hidden(&state.output_view, true);
}

/// `completeReady` — `result` narrows to the waiters' next poll.
fn complete_ready(state: &FilteredState, _result: bool) {
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
            schedule_frame_if_needed(&state);
        }
        fire_redraw();
    }));
}

/// The callback every effect fires when external state becomes dirty —
/// `installRedrawCallback`'s target. The callback runs on arbitrary
/// effect-owned threads, so the weak travels in a `MainThreadBound`: its
/// clone and upgrade happen only inside the main-queue work item.
fn redraw_callback_for(state: &Rc<FilteredState>) -> impl Fn() + Send + Sync + 'static {
    let mtm = cocoa_ui::MainThreadMarker::new()
        .expect("FilteredView callbacks install on the main thread");
    let weak = Arc::new(dispatch2::MainThreadBound::new(Rc::downgrade(state), mtm));
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
        state.clock.stop();
        state.needs_render.set(false);
        state.pending_dynamic_range.borrow_mut().take();
        complete_ready(state, false);
        if state.render_in_flight.get() || state.frame_presentation_in_flight.get() {
            state.detach_after_capture.set(true);
        } else {
            detach_if_needed(state);
            if let Some(presenter) = state.presenter.borrow_mut().as_mut() {
                presenter.release();
            }
        }
        return;
    }
    state.detach_after_capture.set(false);
    update_window_observers(state);
    initialize_gpu(state);
    request_render(state);
}

/// `WuiWindowOcclusionObserver` — occlusion/activation changes re-run
/// attach+schedule. On iOS the attach a launch-time `.inactive` state
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
                schedule_frame_if_needed(&state);
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

/// `layoutSubviews`/`layout`: frame the hidden child, refresh geometry,
/// then ensure GPU state and a pending frame.
fn on_layout(state: &Rc<FilteredState>) {
    let bounds = cocoa_ui::view::bounds(&state.view);
    if let Some(mounted) = state.mounted.borrow().as_ref() {
        cocoa_ui::view::set_frame(mounted.view(), bounds);
        cocoa_ui::view::layout_immediately(mounted.view());
    }
    update_output_frame(state);
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

// MARK: - First-paint readiness (WuiFirstPaintReadyParticipant)

/// Every live filtered host view → its state, so a first-paint walk finds
/// unrevealed filters anywhere in the tree.
static FILTERS: std::sync::Mutex<
    Option<std::collections::HashMap<usize, Sendable<Weak<FilteredState>>>>,
> = std::sync::Mutex::new(None);

fn filter_key(view: &PlatformView) -> usize {
    core::ptr::from_ref(view).cast::<u8>() as usize
}

/// `participatesInFirstPaintReady` — a filter whose window cannot present
/// has no first frame to wait for.
fn participates_in_first_paint_ready(state: &FilteredState) -> bool {
    let bounds = cocoa_ui::view::bounds(&state.view);
    cocoa_ui::view::window(&state.view).is_some()
        && !cocoa_ui::view::is_hidden(&state.view)
        && cocoa_ui::view::alpha(&state.view) > 0.01
        && bounds.size.width > 0.5
        && bounds.size.height > 0.5
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
        complete_ready(state, false);
        return;
    }
    state.ready_waiters.borrow_mut().push(waker);
    schedule_frame_if_needed(state);
}

/// Walks `view`'s subtree calling `f` on every registered filter — the
/// filter half of `collectFirstPaintReadyParticipants`.
pub fn collect_filters(view: &PlatformView, f: &mut impl FnMut(&Rc<FilteredState>)) {
    let filters = FILTERS.lock().expect("filtered registry");
    if let Some(state) = filters
        .as_ref()
        .and_then(|filters| filters.get(&filter_key(view)))
        .and_then(|weak| weak.get().upgrade())
    {
        f(&state);
    }
    drop(filters);
    for subview in cocoa_ui::view::subviews(view) {
        collect_filters(&subview, f);
    }
}

/// The state's own `wait`, used by [`collect_filters`] callers.
pub fn filter_needs_frame(state: &Rc<FilteredState>, waker: std::task::Waker) -> bool {
    if !participates_in_first_paint_ready(state) || state.output_revealed.get() {
        return false;
    }
    request_ready_frame(state, waker);
    !state.output_revealed.get()
}

/// Dropping clears the filter's registrations and shuts the capture and
/// render state down — `deinit`.
struct FilteredGuard {
    view: Retained<PlatformView>,
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
        if let Some(filters) = FILTERS.lock().expect("filtered registry").as_mut() {
            filters.remove(&filter_key(&self.view));
        }
        self.state.clock.stop();
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
/// one capture, one presentation target and one submission instead of two
/// (#521).
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

        // `setupContentView`/`hideUnfilteredContent`: the unfiltered child
        // sits underneath and hidden — hidden as a *view*, because
        // `cacheDisplay` walks the view tree and ignores a hidden backing
        // layer.
        let mounted = ctx.render(content).mount(&view);
        crate::primary_content::forward(&view, mounted.view());
        let child_view = cocoa_ui::view::retain_base(mounted.view());
        #[cfg(target_os = "macos")]
        cocoa_ui::view::ensure_layer_backed(&child_view);
        cocoa_ui::view::set_hidden(&child_view, true);

        // `setupOutputView`: a layer-backed sibling drawn last, its plain
        // `CALayer` presenting `IOSurface` contents every capture path can
        // read (#519).
        let output_view = cocoa_ui::PlatformView::new(mtm);
        #[cfg(target_os = "macos")]
        cocoa_ui::view::ensure_layer_backed(&output_view);
        cocoa_ui::view::set_hidden(&output_view, true);
        if let Some(layer) = cocoa_ui::view::layer(&output_view) {
            layer.setOpaque(false);
            cocoa_ui::core_animation::set_contents_gravity_resize(&layer);
            cocoa_ui::core_animation::set_background_clear(&layer);
        }
        if let Some(host_layer) = cocoa_ui::view::layer(&view) {
            cocoa_ui::core_animation::set_background_clear(&host_layer);
        }
        cocoa_ui::view::add_subview(&view, &output_view);
        let output_layer =
            cocoa_ui::view::layer(&output_view).expect("output view is layer-backed");

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
            let weak = weak.clone();
            let clock = cocoa_ui::display_link::FrameClock::new(mtm, move || {
                if let Some(state) = weak.upgrade() {
                    render_frame(&state);
                }
            });
            let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(
                mtm,
                cocoa_ui::view::retain_base(&child_view),
                crate::components::gpu_surface::capturable_resolver(),
            ));
            let presenter = cocoa_ui::metal::SurfaceBuffers::new(device.clone(), output_layer);
            FilteredState {
                view: view.clone(),
                output_view: output_view.clone(),
                device: RefCell::new(device),
                effects: Rc::new(RefCell::new(None)),
                runtime,
                frame_clock: RefCell::new(EffectFrameClock::new()),
                setup_generation: Rc::new(Cell::new(None)),
                imported_texture: RefCell::new(None),
                input_size: Cell::new((0, 0)),
                attached: Cell::new(false),
                presenter: RefCell::new(Some(presenter)),
                capture_texture: RefCell::new(None),
                frame_presentation_in_flight: Cell::new(false),
                render_in_flight: Cell::new(false),
                detach_after_capture: Cell::new(false),
                pending_dynamic_range: RefCell::new(None),
                configured_range: Cell::new(None),
                needs_render: Cell::new(false),
                output_revealed: Cell::new(false),
                current_scale: Cell::new(1.0),
                laid_out_geometry: RefCell::new(None),
                content_changed_since_capture: Cell::new(false),
                ready_waiters: RefCell::new(Vec::new()),
                mounted: RefCell::new(Some(mounted)),
                capture,
                clock,
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

        {
            let state = state.clone();
            view.set_layout_handler(move |_| on_layout(&state));
        }
        {
            let state = state.clone();
            view.set_window_handler(move |_| handle_window_change(&state));
        }
        #[cfg(target_os = "macos")]
        {
            let state = state.clone();
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
        crate::invalidation::register_sink(&view, sink_callback);
        FILTERS
            .lock()
            .expect("filtered registry")
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(filter_key(&view), Sendable(Rc::downgrade(&state)));

        let filter_guard = FilteredGuard {
            view: cocoa_ui::view::retain_base(&view),
            state: state.clone(),
        };
        let mut leaf = NativeLeaf::new(
            &view,
            FilteredSubView {
                state: state.clone(),
            },
        );
        leaf.keep(view);
        leaf.keep(state);
        leaf.keep(filter_guard);
        for guards in guard_list {
            leaf.keep(guards);
        }
        leaf
    });
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
