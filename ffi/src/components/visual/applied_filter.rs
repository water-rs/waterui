//! FFI bindings for the `AppliedFilter` metadata.
//!
//! This module provides the FFI interface for applying GPU filters to captured
//! view content using wgpu.
//!
//! The native backend is responsible for:
//! 1. Creating a capture layer for the child view
//! 2. Creating an output layer for the filter result
//! 3. Calling `waterui_applied_filter_create` immediately to consume the semantic filter
//! 4. Attaching/detaching presentation targets independently of filter lifetime
//! 5. Installing `waterui_applied_filter_set_redraw_callback`
//! 6. Calling `waterui_applied_filter_setup` after attachment
//! 7. Resolving output size from the latest observed parameters
//! 8. Calling `waterui_applied_filter_render` when rendering is scheduled (with width/height)
//! 9. Calling `waterui_applied_filter_drop` when the view is destroyed

use core::ffi::c_void;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use alloc::boxed::Box;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
use alloc::vec;
use executor_core::spawn_local;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    objc2::{rc::Retained, runtime::ProtocolObject},
    objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureType},
    wgpu_hal::{Api, api::Metal as MetalApi},
};

use waterui_graphics::RedrawHandle;
use waterui_graphics::filter_view::{
    AppliedFilter, EffectContext, EffectFrameClock, EffectInput, EffectOutput, WgslModuleCache,
};
use waterui_graphics::shared_context::{GpuRuntime, reclaim_device};

use crate::{IntoFFI, WuiAnyView};

/// Native callback invoked when an idle applied-filter surface becomes dirty.
pub type WuiAppliedFilterRedrawCallback = unsafe extern "C" fn(context: *mut c_void);

/// Where a filter's finished frames go.
///
/// Apple hands the target in per frame: the host owns a pair of
/// `IOSurface`-backed textures and shows the one this filter just finished, so
/// there is no swapchain here and nothing to present. That is not a detail of
/// how the pixels arrive — a `CAMetalLayer`'s drawable can only be read by the
/// pipeline that presented it, so a filter that owned one was invisible to
/// `CARenderer`, which is what the preview snapshot and every enclosing filter
/// capture with. An `IOSurface` on `CALayer.contents` is read by both (#519).
///
/// Every other platform still presents into a surface of its own.
enum FilterOutput {
    /// A swapchain this filter presents into.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// A texture the host hands in with every frame.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    HostTexture { format: wgpu::TextureFormat },
}

impl FilterOutput {
    /// The format frames are drawn in, which the capture texture must match.
    const fn format(&self) -> wgpu::TextureFormat {
        match self {
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::Surface { config, .. } => config.format,
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::HostTexture { format } => *format,
        }
    }

    /// Follows an output resize.
    ///
    /// Only a swapchain has one to follow, which is why this exists nowhere
    /// else: a host texture pair is made by the host, which hands the first of
    /// the new pair in with the next frame.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let Self::Surface { surface, config } = self;
        config.width = width;
        config.height = height;
        surface.configure(device, config);
    }
}

struct ForeignRedrawTarget {
    context: usize,
    wake: WuiAppliedFilterRedrawCallback,
    drop: WuiAppliedFilterRedrawCallback,
}

// SAFETY: native installs callbacks whose context is explicitly documented as
// callable and releasable from any thread. The target owns that context until
// its `drop` callback runs.
unsafe impl Send for ForeignRedrawTarget {}
// SAFETY: see the `Send` implementation. The wake callback must be thread-safe.
unsafe impl Sync for ForeignRedrawTarget {}

impl ForeignRedrawTarget {
    fn wake(&self) {
        // SAFETY: `wake` and `context` were registered together by the backend, and
        // the context outlives this waker.
        unsafe {
            (self.wake)(self.context as *mut c_void);
        }
    }
}

impl Drop for ForeignRedrawTarget {
    fn drop(&mut self) {
        // SAFETY: `drop` and `context` are one registration, and `Drop` runs once.
        unsafe {
            (self.drop)(self.context as *mut c_void);
        }
    }
}

/// FFI representation of a `Metadata<AppliedFilter>`.
#[repr(C)]
#[derive(Debug)]
pub struct WuiAppliedFilter {
    /// The child view to capture (pointer to `WuiAnyView`).
    pub content: *mut WuiAnyView,
    /// Opaque pointer to the boxed `AppliedFilter`.
    /// This is consumed during create and should not be used after.
    pub filter: *mut c_void,
}

impl IntoFFI for waterui_core::Metadata<AppliedFilter> {
    type FFI = WuiAppliedFilter;

    fn into_ffi(self) -> Self::FFI {
        // Take the child view and convert to FFI
        let content = self.content.into_ffi();

        // Box the AppliedFilter for FFI transfer
        let filter_ptr = Box::into_raw(Box::new(self.value)).cast::<c_void>();

        WuiAppliedFilter {
            content,
            filter: filter_ptr,
        }
    }
}

// Generate waterui_metadata_applied_filter_id() and waterui_force_as_metadata_applied_filter()
ffi_metadata!(AppliedFilter, WuiAppliedFilter, applied_filter);

/// Opaque state held by the native backend for one semantic applied filter.
pub struct WuiAppliedFilterState {
    /// Explicit environment-owned GPU runtime used by this semantic filter.
    runtime: GpuRuntime,
    /// Currently attached presentation target.
    output: Option<FilterOutput>,
    /// Capture texture (for capturing child view output)
    capture_texture: Option<wgpu::Texture>,
    /// Capture texture format for the currently attached presentation target.
    capture_format: Option<wgpu::TextureFormat>,
    /// The semantic filter. Setup temporarily moves it into its local future.
    filter: Rc<RefCell<Option<AppliedFilter>>>,
    /// Handle shared with the effect's reactive redraw callback.
    redraw_handle: RedrawHandle,
    /// Immutable input/output formats captured by the one-time setup.
    setup_formats: Cell<Option<(wgpu::TextureFormat, wgpu::TextureFormat)>>,
    /// Becomes true only after asynchronous setup completes.
    setup_ready: Rc<Cell<bool>>,
    /// Current input dimensions (from child view)
    input_width: u32,
    input_height: u32,
    /// Current output dimensions
    output_width: u32,
    output_height: u32,
    /// Latest output dimensions resolved from snapped filter state.
    resolved_output_width: u32,
    resolved_output_height: u32,
    /// Vulkan imports of the Android capture buffers handed to this filter.
    #[cfg(target_os = "android")]
    hardware_buffer_imports: super::hardware_buffer::HardwareBufferImports,
    /// Pipelines for drawing surfaces nested in the captured subtree back into it.
    #[cfg(target_os = "android")]
    capture_compositor: super::capture_composite::CaptureCompositor,
    /// Host-owned effect clock; media effects may bypass this API with explicit timing.
    frame_clock: EffectFrameClock,
}

impl core::fmt::Debug for WuiAppliedFilterState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WuiAppliedFilterState")
            .field("input_width", &self.input_width)
            .field("input_height", &self.input_height)
            .field("output_width", &self.output_width)
            .field("output_height", &self.output_height)
            .finish_non_exhaustive()
    }
}

/// Resolved output size returned to native before render scheduling.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiAppliedFilterOutputSize {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
}

const fn output_dimensions_for_input(
    state: &WuiAppliedFilterState,
    input_width: u32,
    input_height: u32,
) -> (u32, u32) {
    let output_width = if state.resolved_output_width == 0 {
        input_width
    } else {
        state.resolved_output_width
    };
    let output_height = if state.resolved_output_height == 0 {
        input_height
    } else {
        state.resolved_output_height
    };
    (output_width, output_height)
}

fn ensure_dimensions(state: &mut WuiAppliedFilterState, width: u32, height: u32) {
    assert!(
        width > 0 && height > 0,
        "AppliedFilter input dimensions must be non-zero, got {width}x{height}"
    );
    let capture_format = state
        .capture_format
        .expect("AppliedFilter input update requires an attached presentation target");
    let (output_width, output_height) = output_dimensions_for_input(state, width, height);
    assert!(
        output_width > 0 && output_height > 0,
        "AppliedFilter output dimensions must be non-zero, got {output_width}x{output_height}"
    );
    let input_resized = width != state.input_width || height != state.input_height;
    let output_resized = output_width != state.output_width || output_height != state.output_height;

    if input_resized {
        state.input_width = width;
        state.input_height = height;
    }

    if output_resized {
        state.output_width = output_width;
        state.output_height = output_height;
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            let device = state.runtime.context().device.clone();
            state
                .output
                .as_mut()
                .expect("AppliedFilter resize requires an attached presentation target")
                .resize(&device, output_width, output_height);
        }
    }

    if input_resized || state.capture_texture.is_none() {
        state.capture_texture = Some(create_capture_texture(
            &state.runtime.context().device,
            &state.runtime.context().queue,
            capture_format,
            width,
            height,
        ));
    }
}

/// Creates the texture the native backend captures the filtered subtree into.
///
/// The host writes this texture from outside wgpu — Metal's `CARenderer` and
/// the capture compositor on Apple — and wgpu only ever samples it. wgpu tracks
/// memory initialisation per texture, and a texture it has never written is
/// zero-cleared the first time it is bound for sampling, which would wipe the
/// host's first capture and hand the filter a transparent input. Clearing it
/// here, through wgpu, marks it initialised once and for all, so every later
/// external write is read back as written. The clear is an empty render pass
/// rather than `clear_texture`, which needs the `CLEAR_TEXTURE` device feature.
fn create_capture_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("AppliedFilter Capture Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("AppliedFilter Capture Texture Init"),
    });
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("AppliedFilter Capture Texture Init"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    queue.submit([encoder.finish()]);
    texture
}

fn resolve_output_size(
    state: &mut WuiAppliedFilterState,
    input_width: u32,
    input_height: u32,
) -> WuiAppliedFilterOutputSize {
    let (output_width, output_height) = state
        .filter
        .borrow()
        .as_ref()
        .expect("AppliedFilter output size requested while asynchronous setup is pending")
        .output_size(input_width, input_height);
    assert!(
        output_width > 0 && output_height > 0,
        "waterui_applied_filter_resolve_output_size: filter produced invalid output size {output_width}x{output_height} for input {input_width}x{input_height}"
    );
    state.resolved_output_width = output_width;
    state.resolved_output_height = output_height;
    WuiAppliedFilterOutputSize {
        width: output_width,
        height: output_height,
    }
}

/// Combines a filter with the filter it encloses, into one.
///
/// A component that filters its own body, filtered again by its caller, is two
/// filters over one subtree, and the `impl View` boundary between them hides
/// the first from the second's type — so nothing at the authoring layer fuses
/// them the way a chain written in one expression is fused. Run as written they
/// cost two captures of the same content, two presentation targets and two
/// full-size intermediates (#521).
///
/// A backend reaches this when the walk it already runs to resolve a view — id
/// against its component registry, `waterui_view_body` when the id is not
/// registered — starts at `outer`'s content and lands on another filter. That
/// it landed there is the proof there was nothing realizable in between: every
/// view that could draw is a registered component that would have stopped the
/// walk first.
///
/// That walk consumes the views it steps through, `outer`'s content among them,
/// so `outer.content` must already be null when this is called: the caller nulls
/// it as it walks, and a non-null one here would mean a handle the backend still
/// believes it owns.
///
/// Both descriptors are consumed. The returned descriptor carries `inner`'s
/// content and a filter that runs `inner`'s filters and then `outer`'s, and the
/// caller repeats until the content no longer resolves to a filter.
///
/// # Safety
///
/// - `inner` and `outer` must be valid descriptors whose filters have not been
///   consumed by a previous call to this function or to
///   [`waterui_applied_filter_create`].
/// - `inner`'s content must be an owning handle from the matching FFI
///   constructor; it becomes the returned descriptor's content.
///
/// # Panics
///
/// Panics if either descriptor's filter was already consumed, or if `outer`
/// still holds a content handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_chain(
    inner: *mut WuiAppliedFilter,
    outer: *mut WuiAppliedFilter,
) -> WuiAppliedFilter {
    // SAFETY: the caller contract requires both descriptors to be valid and not
    // otherwise borrowed for this call.
    let (inner, outer) = unsafe { (&mut *inner, &mut *outer) };
    // SAFETY: `take_filter` asserts the descriptor still owns its filter and nulls
    // the field, so each is reclaimed exactly once.
    let (inner_filter, outer_filter) =
        unsafe { (take_filter(inner, "inner"), take_filter(outer, "outer")) };

    // The chain captures what the inner filter captured, and the view that led
    // from one to the other was consumed by the walk that found it.
    assert!(
        outer.content.is_null(),
        "waterui_applied_filter_chain: the outer descriptor still holds a content handle, so the walk that reached the inner filter did not run on it"
    );

    let content = core::mem::replace(&mut inner.content, core::ptr::null_mut());
    WuiAppliedFilter {
        content,
        filter: Box::into_raw(Box::new(AppliedFilter::chained(inner_filter, outer_filter)))
            .cast::<c_void>(),
    }
}

/// Reclaims a descriptor's filter, leaving the descriptor consumed.
///
/// # Safety
///
/// The descriptor must still own its filter.
unsafe fn take_filter(descriptor: &mut WuiAppliedFilter, which: &str) -> AppliedFilter {
    assert!(
        !descriptor.filter.is_null(),
        "waterui_applied_filter_chain: the {which} descriptor was already consumed"
    );
    // SAFETY: the assert above proves the descriptor still owns its filter, and the
    // field is nulled immediately after, so it is reclaimed once.
    let filter = unsafe { *Box::from_raw(descriptor.filter.cast::<AppliedFilter>()) };
    descriptor.filter = core::ptr::null_mut();
    filter
}

/// Creates persistent state and immediately consumes the semantic filter.
///
/// Presentation targets are attached later with [`waterui_applied_filter_attach`],
/// so a view destroyed before layout still has one clear Rust owner.
///
/// # Safety
///
/// `filter_ffi` must be a valid, unconsumed descriptor returned by
/// `waterui_force_as_metadata_applied_filter`.
/// `env` must contain an installed GPU runtime.
///
/// # Panics
///
/// Panics if `filter_ffi`'s inner filter pointer is null, meaning the descriptor
/// was already consumed by a previous call to this function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_create(
    filter_ffi: *mut WuiAppliedFilter,
    env: *const crate::WuiEnv,
) -> *mut WuiAppliedFilterState {
    // SAFETY: the caller contract requires `filter_ffi` to be a valid descriptor that
    // no one else is borrowing for this call.
    let wui_filter = unsafe { &mut *filter_ffi };
    assert!(
        !wui_filter.filter.is_null(),
        "waterui_applied_filter_create: descriptor was already consumed"
    );
    let mut filter: AppliedFilter =
        // SAFETY: the assert above proves the descriptor still owns its filter, and the
        // field is nulled immediately after, so it is reclaimed once.
        unsafe { *Box::from_raw(wui_filter.filter.cast::<AppliedFilter>()) };
    wui_filter.filter = core::ptr::null_mut();

    // SAFETY: the caller contract requires `env` to be a valid handle alive for this
    // call; it is only borrowed.
    let runtime = super::gpu_runtime::gpu_runtime(&unsafe { &*env }.0);
    #[cfg(target_os = "android")]
    let hardware_buffer_imports =
        super::hardware_buffer::HardwareBufferImports::new(runtime.clone());
    let redraw_handle = filter.redraw_handle();
    Box::into_raw(Box::new(WuiAppliedFilterState {
        runtime,
        output: None,
        capture_texture: None,
        capture_format: None,
        filter: Rc::new(RefCell::new(Some(filter))),
        redraw_handle,
        setup_formats: Cell::new(None),
        setup_ready: Rc::new(Cell::new(false)),
        input_width: 0,
        input_height: 0,
        output_width: 0,
        output_height: 0,
        resolved_output_width: 0,
        resolved_output_height: 0,
        #[cfg(target_os = "android")]
        hardware_buffer_imports,
        #[cfg(target_os = "android")]
        capture_compositor: super::capture_composite::CaptureCompositor::default(),
        frame_clock: EffectFrameClock::new(),
    }))
}

/// Asserts the filter can both draw into `format` and capture its subtree in it.
///
/// The capture texture and the output share one format, so a format the output
/// accepts but capture cannot use would fail later, inside a frame, instead of
/// at attach.
fn assert_capture_usable_format(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    caller: &str,
) {
    let capture_usages = wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::COPY_DST;
    assert!(
        adapter
            .get_texture_format_features(format)
            .allowed_usages
            .contains(capture_usages),
        "{caller}: output format {format:?} cannot be used for capture"
    );
}

/// The output size an attach resolves to, with its own zero check.
fn attach_output_size(
    state: &WuiAppliedFilterState,
    input_width: u32,
    input_height: u32,
) -> (u32, u32) {
    assert!(
        input_width > 0 && input_height > 0,
        "waterui_applied_filter_attach: dimensions must be non-zero, got {input_width}x{input_height}"
    );
    let (output_width, output_height) =
        output_dimensions_for_input(state, input_width, input_height);
    assert!(
        output_width > 0 && output_height > 0,
        "waterui_applied_filter_attach: output size must be non-zero, got {output_width}x{output_height}"
    );
    (output_width, output_height)
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn create_configured_surface(
    state: &WuiAppliedFilterState,
    output_layer: *mut c_void,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) -> (
    wgpu::Surface<'static>,
    wgpu::SurfaceConfiguration,
    wgpu::TextureFormat,
) {
    let (output_width, output_height) = attach_output_size(state, input_width, input_height);

    let gpu = state.runtime.context();
    let surface =
        crate::components::gpu_surface::create_surface_from_layer(&gpu.instance, output_layer);
    let capabilities = surface.get_capabilities(&gpu.adapter);
    let format = waterui_graphics::gpu_surface::preferred_surface_format_with_preference(
        &capabilities,
        prefers_hdr,
    );
    assert!(
        capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Fifo),
        "waterui_applied_filter_attach: output surface does not support FIFO presentation"
    );
    let alpha_mode = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
        wgpu::CompositeAlphaMode::Opaque,
    ]
    .into_iter()
    .find(|mode| capabilities.alpha_modes.contains(mode))
    .expect("waterui_applied_filter_attach: output surface reports no composite alpha mode");
    assert_capture_usable_format(&gpu.adapter, format, "waterui_applied_filter_attach");
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: output_width,
        height: output_height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&gpu.device, &config);
    (surface, config, format)
}

/// Records an attached presentation target and sizes everything that follows it.
fn finish_attach(
    state: &mut WuiAppliedFilterState,
    output: FilterOutput,
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
) {
    let format = output.format();
    if let Some((_, setup_output_format)) = state.setup_formats.get() {
        assert_eq!(
            setup_output_format, format,
            "AppliedFilter output format changed after setup"
        );
    }
    let capture_texture = create_capture_texture(
        &state.runtime.context().device,
        &state.runtime.context().queue,
        format,
        input_width,
        input_height,
    );
    state.input_width = input_width;
    state.input_height = input_height;
    state.output_width = output_width;
    state.output_height = output_height;
    state.capture_format = Some(format);
    state.capture_texture = Some(capture_texture);
    state.output = Some(output);
    let _ = state.redraw_handle.take_dirty();
    if state.setup_ready.get() {
        state.redraw_handle.request_redraw();
    }
}

/// Attaches a native presentation target while preserving the semantic filter.
///
/// # Safety
///
/// - `state` must come from [`waterui_applied_filter_create`].
/// - `output_layer` must remain valid until [`waterui_applied_filter_detach`].
/// - The state must currently be detached.
///
/// # Panics
///
/// Panics if `state` already has an output surface attached, or if
/// `input_width`/`input_height` is zero.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_attach(
    state: *mut WuiAppliedFilterState,
    output_layer: *mut c_void,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.output.is_none(),
        "waterui_applied_filter_attach: output surface is already attached"
    );
    let (surface, config, _) =
        create_configured_surface(state, output_layer, input_width, input_height, prefers_hdr);
    let (output_width, output_height) = (config.width, config.height);
    finish_attach(
        state,
        FilterOutput::Surface { surface, config },
        input_width,
        input_height,
        output_width,
        output_height,
    );
}

/// Attaches a presentation surface (non-Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_applied_filter_create`].
///
/// # Panics
///
/// Always panics: Apple hosts own their presentation memory and attach with
/// [`waterui_applied_filter_attach_host_textures`].
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_attach(
    _state: *mut WuiAppliedFilterState,
    _output_layer: *mut c_void,
    _input_width: u32,
    _input_height: u32,
    _prefers_hdr: bool,
) {
    panic!(
        "waterui_applied_filter_attach: Apple hosts attach with waterui_applied_filter_attach_host_textures"
    );
}

/// The format an Apple host renders a filter into, with no swapchain to ask.
///
/// A `CAMetalLayer` reported capabilities to choose from; an `IOSurface` has
/// none — it is created in whatever format it is told, so the choice moves here
/// and the host asks for it with [`waterui_applied_filter_capture_format`].
/// These are the two the layer path picked between anyway: half-float linear
/// for an extended-range presentation, sRGB-encoded 8-bit otherwise.
#[cfg(any(target_os = "macos", target_os = "ios"))]
const fn apple_presentation_format(prefers_hdr: bool) -> wgpu::TextureFormat {
    if prefers_hdr {
        wgpu::TextureFormat::Rgba16Float
    } else {
        wgpu::TextureFormat::Bgra8UnormSrgb
    }
}

/// The `MTLPixelFormat` an attached filter renders its output in (Apple only).
///
/// The host allocates its `IOSurface` pair from this. It is the raw Metal enum
/// value rather than a `WuiCaptureFormat` because the two are not the same
/// alphabet: `WuiCaptureFormat` names `AHardwareBuffer` layouts, and the
/// presentation format here is `BGRA8Unorm_sRGB`, which has no name there.
///
/// # Safety
///
/// `state` must be a valid pointer from [`waterui_applied_filter_create`] with a
/// presentation target attached.
///
/// # Panics
///
/// Panics if the filter is detached, or if its output format has no Metal
/// equivalent.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_output_metal_pixel_format(
    state: *const WuiAppliedFilterState,
) -> u32 {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    let format = state
        .output
        .as_ref()
        .expect("waterui_applied_filter_output_metal_pixel_format: presentation target is detached")
        .format();
    let metal_format = match format {
        wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => MTLPixelFormat::BGRA8Unorm_sRGB,
        wgpu::TextureFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
        other => panic!(
            "waterui_applied_filter_output_metal_pixel_format: {other:?} has no Metal equivalent"
        ),
    };
    u32::try_from(metal_format.0).expect("MTLPixelFormat values fit in a u32")
}

/// Attaches host-owned presentation on Apple, where frames arrive per texture.
///
/// No layer is named because none is configured: the host keeps a pair of
/// `IOSurface`-backed textures, hands one to
/// [`waterui_applied_filter_render_to_metal_texture`] per frame, and shows it
/// on `CALayer.contents` once that frame's fence completes. The texture format
/// is this call's answer, read back with
/// [`waterui_applied_filter_output_metal_pixel_format`].
///
/// # Safety
///
/// - `state` must come from [`waterui_applied_filter_create`].
/// - The state must currently be detached.
///
/// # Panics
///
/// Panics if `state` already has a presentation target attached, if
/// `input_width`/`input_height` is zero, or if the chosen format cannot carry
/// the subtree capture.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_attach_host_textures(
    state: *mut WuiAppliedFilterState,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.output.is_none(),
        "waterui_applied_filter_attach: output surface is already attached"
    );
    let (output_width, output_height) = attach_output_size(state, input_width, input_height);
    let format = apple_presentation_format(prefers_hdr);
    assert_capture_usable_format(
        &state.runtime.context().adapter,
        format,
        "waterui_applied_filter_attach",
    );
    finish_attach(
        state,
        FilterOutput::HostTexture { format },
        input_width,
        input_height,
        output_width,
        output_height,
    );
}

/// Attaches host-owned presentation (Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_applied_filter_create`].
///
/// # Panics
///
/// Always panics: only Apple hosts present from their own textures.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_attach_host_textures(
    _state: *mut WuiAppliedFilterState,
    _input_width: u32,
    _input_height: u32,
    _prefers_hdr: bool,
) {
    panic!("waterui_applied_filter_attach_host_textures: only supported on Apple platforms");
}

/// The `MTLPixelFormat` an attached filter renders its output in (Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_applied_filter_create`].
///
/// # Panics
///
/// Always panics: Metal pixel formats only exist on Apple platforms.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_output_metal_pixel_format(
    _state: *const WuiAppliedFilterState,
) -> u32 {
    panic!("waterui_applied_filter_output_metal_pixel_format: only supported on Apple platforms");
}

/// Detaches the presentation target without destroying the semantic filter.
///
/// # Safety
///
/// `state` must be valid and currently attached.
///
/// # Panics
///
/// Panics if `state` does not currently have an output surface attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_detach(state: *mut WuiAppliedFilterState) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    // Held to the end of this function: a swapchain is released after the
    // capture texture and the imported buffers below, never before them.
    let _output = state
        .output
        .take()
        .expect("waterui_applied_filter_detach: output surface is already detached");
    // Before the capture texture goes: the imported buffers are raw Vulkan
    // objects wgpu does not defer the destruction of, so they are released here
    // rather than left to outlive the target they were captured for.
    #[cfg(target_os = "android")]
    state.hardware_buffer_imports.clear();
    state.capture_texture = None;
    state.capture_format = None;
    state.input_width = 0;
    state.input_height = 0;
    state.output_width = 0;
    state.output_height = 0;
    state.resolved_output_width = 0;
    state.resolved_output_height = 0;
}

/// Installs the native wake target for reactive filter redraw requests.
///
/// The wake callback may be called from any thread. `drop_callback` releases
/// `context` after the callback is replaced or the applied filter is destroyed.
///
/// # Safety
///
/// - `state` and `context` must be valid.
/// - Both callbacks must be thread-safe and use `context` only for the lifetime
///   retained by `drop_callback`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_set_redraw_callback(
    state: *mut WuiAppliedFilterState,
    context: *mut c_void,
    wake: WuiAppliedFilterRedrawCallback,
    drop_callback: WuiAppliedFilterRedrawCallback,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let target = ForeignRedrawTarget {
        context: context as usize,
        wake,
        drop: drop_callback,
    };
    state
        .redraw_handle
        .set_waker(Some(Arc::new(move || target.wake())));
}

/// Starts filter setup on the main-thread local executor.
///
/// # Arguments
///
/// * `state` - Pointer to attached state from `waterui_applied_filter_create`
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_create`
/// - A presentation target must be attached
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_setup(state: *mut WuiAppliedFilterState) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let input_format = current_applied_filter_input_format(state);
    start_applied_filter_setup(state, input_format);
}

/// Returns whether asynchronous filter setup has completed.
///
/// Completion also triggers the installed redraw callback.
///
/// # Safety
///
/// `state` must be a valid pointer returned by [`waterui_applied_filter_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_is_ready(
    state: *const WuiAppliedFilterState,
) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    state.setup_ready.get()
}

/// Runs the semantic filter once, from the capture texture into `output_texture`.
///
/// Every presentation path ends here: a swapchain's acquired frame and a host's
/// `IOSurface`-backed texture are both just a texture of the established output
/// format. Nothing is submitted or presented — the caller decides what the work
/// is ordered against and who shows the result.
///
/// # Panics
///
/// Panics if setup has not completed, or if the filter itself fails.
fn render_filter_into(
    state: &mut WuiAppliedFilterState,
    output_texture: &wgpu::Texture,
    width: u32,
    height: u32,
    caller: &str,
) -> bool {
    ensure_dimensions(state, width, height);
    let input_format = current_applied_filter_input_format(state);
    assert!(
        state.setup_ready.get(),
        "{caller} called before asynchronous setup completed"
    );
    assert_setup_input_format(state, input_format);
    let output_format = state
        .output
        .as_ref()
        .expect("AppliedFilter render requires an attached presentation target")
        .format();

    let input_texture = state
        .capture_texture
        .as_ref()
        .expect("AppliedFilter render requires an attached presentation target");

    let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("AppliedFilter Input View"),
        ..Default::default()
    });

    let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("AppliedFilter Output View"),
        format: Some(output_format),
        ..Default::default()
    });

    let timing = state.frame_clock.tick();
    let input = EffectInput {
        device: &state.runtime.context().device,
        queue: &state.runtime.context().queue,
        texture: input_texture,
        view: input_view,
        format: input_format,
        width: state.input_width,
        height: state.input_height,
        timing,
    };

    let filter_output = EffectOutput {
        device: &state.runtime.context().device,
        queue: &state.runtime.context().queue,
        texture: output_texture,
        view: output_view,
        format: output_format,
        width: state.output_width,
        height: state.output_height,
    };

    state
        .filter
        .borrow_mut()
        .as_mut()
        .expect("AppliedFilter ready state is missing its semantic filter")
        .render(&input, &filter_output)
        .unwrap_or_else(|err| panic!("{caller}: {err}"))
}

/// Render the filter.
///
/// This function applies the filter to the captured input and renders to the output.
/// Pass current width/height - resources are recreated if size changed.
///
/// # Arguments
///
/// * `state` - Pointer to attached persistent state
/// * `width` - Current width in pixels
/// * `height` - Current height in pixels
///
/// # Returns
///
/// Whether another frame is needed for animation or an effect callback that
/// arrived while this frame was rendering.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_create`
/// - A presentation target must be attached
/// - `waterui_applied_filter_setup` must have completed
///
/// # Panics
///
/// Panics if the asynchronous setup started by `waterui_applied_filter_setup`
/// has not completed yet.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_render(
    state: *mut WuiAppliedFilterState,
    width: u32,
    height: u32,
) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };

    ensure_dimensions(state, width, height);
    let FilterOutput::Surface { surface, config } = state
        .output
        .as_ref()
        .expect("waterui_applied_filter_render: presentation target is detached");

    // Get output texture
    let Some(output) = super::acquire_surface_texture(
        surface,
        &state.runtime.context().device,
        config,
        "waterui_applied_filter_render",
    ) else {
        // Nothing was drawn, so the frame this call was asked for is still
        // pending: the host must come back for it once the surface can be
        // acquired again. Reporting it done here would strand a view whose only
        // clock is its own render loop.
        return true;
    };

    let needs_redraw = render_filter_into(
        state,
        &output.texture,
        width,
        height,
        "waterui_applied_filter_render",
    );

    // Present
    output.present();
    reclaim_device(&state.runtime.context().device);

    needs_redraw
}

/// Render the filter into a host-owned Metal texture (Apple only).
///
/// The host keeps a pair of `IOSurface`-backed textures and hands in the one it
/// is not currently showing. The returned fence is that frame's: the host shows
/// the texture on `CALayer.contents` when it completes, never before, so a
/// half-drawn frame is never composited.
///
/// `needs_redraw` is reported through `out_needs_redraw` because the return
/// value carries the fence.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_create` with
///   a presentation target attached.
/// - `texture` must point to a live `MTLTexture` of the attached format, at
///   least `width` by `height`.
/// - `out_needs_redraw` must be writable.
///
/// # Panics
///
/// Panics if `texture` is null, if its format is not the one established at
/// attach, or if setup has not completed.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_render_to_metal_texture(
    state: *mut WuiAppliedFilterState,
    texture: *mut c_void,
    width: u32,
    height: u32,
    out_needs_redraw: *mut bool,
) -> *mut super::gpu_surface::WuiGpuCaptureFence {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    // SAFETY: the caller contract requires `texture` to be a live `MTLTexture`;
    // `retain` takes its own reference, so it outlives the render below.
    let metal_texture = unsafe {
        Retained::<ProtocolObject<dyn MTLTexture>>::retain(texture.cast())
            .expect("waterui_applied_filter_render_to_metal_texture received a null texture")
    };

    ensure_dimensions(state, width, height);
    let output_format = state
        .output
        .as_ref()
        .expect("waterui_applied_filter_render_to_metal_texture: presentation target is detached")
        .format();
    let texture_format = match metal_texture.pixelFormat() {
        MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => panic!(
            "waterui_applied_filter_render_to_metal_texture: unsupported Metal format {other:?}"
        ),
    };
    assert_eq!(
        texture_format, output_format,
        "waterui_applied_filter_render_to_metal_texture: host texture format does not match the attached output format"
    );

    let (output_width, output_height) = (state.output_width, state.output_height);
    // SAFETY: `metal_texture` is the retained texture above, and the format and
    // size passed alongside it are read from that same texture and the output
    // size it was created for, so the HAL description matches the real resource.
    let hal_texture = unsafe {
        <MetalApi as Api>::Device::texture_from_raw(
            metal_texture,
            output_format,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu_hal::CopyExtent {
                width: output_width,
                height: output_height,
                depth: 1,
            },
        )
    };
    // SAFETY: the HAL texture above was created from this runtime's device, which
    // is the device the wgpu texture is created on.
    let wgpu_texture = unsafe {
        state
            .runtime
            .context()
            .device
            .create_texture_from_hal::<MetalApi>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("AppliedFilter Host Presentation Texture"),
                    size: wgpu::Extent3d {
                        width: output_width,
                        height: output_height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: output_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
            )
    };

    let needs_redraw = render_filter_into(
        state,
        &wgpu_texture,
        width,
        height,
        "waterui_applied_filter_render_to_metal_texture",
    );
    // SAFETY: the caller contract requires `out_needs_redraw` to be writable.
    unsafe { out_needs_redraw.write(needs_redraw) };

    let submission = state.runtime.context().queue.submit([]);
    let fence = super::gpu_surface::WuiGpuCaptureFence::new(
        state.runtime.context().submission_completion_driver(),
        submission,
    );
    reclaim_device(&state.runtime.context().device);
    Box::into_raw(Box::new(fence))
}

/// Render the filter into a host-owned Metal texture (Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_applied_filter_create`].
///
/// # Panics
///
/// Always panics: Metal textures only exist on Apple platforms.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_render_to_metal_texture(
    _state: *mut WuiAppliedFilterState,
    _texture: *mut c_void,
    _width: u32,
    _height: u32,
    _out_needs_redraw: *mut bool,
) -> *mut super::gpu_surface::WuiGpuCaptureFence {
    panic!("waterui_applied_filter_render_to_metal_texture: only supported on Apple platforms");
}

/// Render the filter, presenting into the attached surface (non-Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_applied_filter_create`].
///
/// # Panics
///
/// Always panics: Apple hosts render with
/// [`waterui_applied_filter_render_to_metal_texture`].
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_render(
    _state: *mut WuiAppliedFilterState,
    _width: u32,
    _height: u32,
) -> bool {
    panic!(
        "waterui_applied_filter_render: Apple hosts render with waterui_applied_filter_render_to_metal_texture"
    );
}

const fn current_applied_filter_input_format(state: &WuiAppliedFilterState) -> wgpu::TextureFormat {
    state
        .capture_format
        .expect("AppliedFilter input format requires an attached presentation target")
}

fn assert_setup_input_format(state: &WuiAppliedFilterState, input_format: wgpu::TextureFormat) {
    if let Some((setup_input_format, _)) = state.setup_formats.get() {
        assert_eq!(
            setup_input_format, input_format,
            "AppliedFilter input format changed after setup"
        );
    }
}

fn start_applied_filter_setup(state: &WuiAppliedFilterState, input_format: wgpu::TextureFormat) {
    let output_format = state
        .output
        .as_ref()
        .expect("AppliedFilter setup requires an attached presentation target")
        .format();
    if let Some((setup_input_format, setup_output_format)) = state.setup_formats.get() {
        assert_eq!(
            setup_input_format, input_format,
            "AppliedFilter input format changed after setup"
        );
        assert_eq!(
            setup_output_format, output_format,
            "AppliedFilter output format changed after setup"
        );
        return;
    }

    state.setup_formats.set(Some((input_format, output_format)));
    let mut filter = state
        .filter
        .borrow_mut()
        .take()
        .expect("AppliedFilter semantic filter is unavailable before setup starts");
    let filter_slot = Rc::clone(&state.filter);
    let setup_ready = Rc::clone(&state.setup_ready);
    let runtime = state.runtime.clone();
    let redraw_handle = state.redraw_handle.clone();
    spawn_local(async move {
        let gpu = runtime.context();
        let shader_cache = WgslModuleCache::new();
        let ctx = EffectContext {
            device: &gpu.device,
            queue: &gpu.queue,
            shader_cache: &shader_cache,
            input_format,
            output_format,
        };
        filter
            .setup(&ctx)
            .await
            .unwrap_or_else(|error| panic!("AppliedFilter setup failed: {error}"));
        filter_slot.replace(Some(filter));
        setup_ready.set(true);
        redraw_handle.request_redraw();
    })
    .detach();
}

/// Resolve the current output size from the latest observed filter state.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_create`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_resolve_output_size(
    state: *mut WuiAppliedFilterState,
    input_width: u32,
    input_height: u32,
) -> WuiAppliedFilterOutputSize {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    resolve_output_size(state, input_width, input_height)
}

/// Prepare the capture texture for rendering.
///
/// Ensures the capture texture matches the requested dimensions.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_create` with an attached target.
///
/// # Panics
///
/// Panics if `state` does not currently have an output surface attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_prepare_capture(
    state: *mut WuiAppliedFilterState,
    width: u32,
    height: u32,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };

    ensure_dimensions(state, width, height);
    let capture_format = state
        .capture_format
        .expect("waterui_applied_filter_prepare_capture: presentation target is detached");
    assert_setup_input_format(state, capture_format);
}

/// The pixel layout this filter's capture buffers must be allocated with (Android only).
///
/// The Android backend reads this after attaching and allocates its
/// `ImageReader` from it, so the `AHardwareBuffer` it captures the subtree into
/// is copy-compatible with the capture texture the filter samples.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_create` with an
/// attached target.
///
/// # Panics
///
/// Panics if `state` is detached, or if its capture format has no
/// `AHardwareBuffer` layout.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_capture_format(
    state: *const WuiAppliedFilterState,
) -> super::capture_format::WuiCaptureFormat {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    let format = state
        .capture_format
        .expect("waterui_applied_filter_capture_format: presentation target is detached");
    super::capture_format::capture_buffer_format(format)
}

/// The pixel layout this filter's capture buffers must be allocated with (Android only).
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_create` with an
/// attached target.
///
/// # Panics
///
/// Always panics: `AHardwareBuffer` capture only exists on Android.
#[cfg(not(target_os = "android"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_capture_format(
    _state: *const WuiAppliedFilterState,
) -> super::capture_format::WuiCaptureFormat {
    panic!("waterui_applied_filter_capture_format: only supported on Android");
}

/// Copies a captured `AHardwareBuffer` into the capture texture (Android only).
///
/// The buffer is the one `HardwareRenderer` drew the filtered subtree into. It
/// is imported once per distinct buffer and copied on the GPU; the returned
/// fence is what tells the backend the copy is finished, so it must be consumed
/// by `waterui_gpu_capture_fence_on_complete` and the `Image` the buffer came
/// from closed only from that completion.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_create` with an
///   attached target, and `waterui_applied_filter_prepare_capture` must have run
///   for this frame's size.
/// - `buffer` must be a live `AHardwareBuffer` for the duration of this call.
///
/// # Panics
///
/// Panics if `state` is detached, or if the buffer's size or layout does not
/// match the capture texture.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_set_capture_hardware_buffer(
    state: *mut WuiAppliedFilterState,
    buffer: *mut c_void,
) -> *mut super::gpu_surface::WuiGpuCaptureFence {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let capture_texture = state.capture_texture.take().expect(
        "waterui_applied_filter_set_capture_hardware_buffer: presentation target is detached",
    );
    // SAFETY: the caller contract makes `buffer` a live `AHardwareBuffer` for this
    // call, which is when the import takes its own reference on it.
    let fence = unsafe {
        super::hardware_buffer::copy_hardware_buffer_into_texture(
            &mut state.hardware_buffer_imports,
            buffer.cast(),
            &capture_texture,
            "waterui_applied_filter_set_capture_hardware_buffer",
        )
    };
    state.capture_texture = Some(capture_texture);
    fence
}

/// Copies a captured `AHardwareBuffer` into the capture texture (Android only).
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_create` with an
/// attached target, and `buffer` a live `AHardwareBuffer`.
///
/// # Panics
///
/// Always panics: `AHardwareBuffer` capture only exists on Android.
#[cfg(not(target_os = "android"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_set_capture_hardware_buffer(
    _state: *mut WuiAppliedFilterState,
    _buffer: *mut c_void,
) -> *mut super::gpu_surface::WuiGpuCaptureFence {
    panic!("waterui_applied_filter_set_capture_hardware_buffer: only supported on Android");
}

/// Draws a `GpuSurface` nested in the captured subtree into the capture (Android only).
///
/// HWUI records a `SurfaceView` as a cleared hole, so a GPU surface inside a
/// filtered subtree is missing from the buffer the subtree was captured into.
/// The backend calls this once per nested surface, between handing over the
/// captured buffer and rendering the filter, with the rectangle the surface
/// occupies inside the captured content in capture pixels. The surface renders
/// one frame of its own and it is drawn into the capture at that rectangle.
///
/// Everything runs on one queue in call order, so the capture copy, the
/// composites and the filter render need no fence between them.
///
/// # Safety
///
/// - `filter` must be a valid pointer from `waterui_applied_filter_create` with
///   an attached target, and `waterui_applied_filter_prepare_capture` must have
///   run for this frame's size.
/// - `surface` must be a valid pointer from `waterui_gpu_surface_create` that
///   has been attached at least once, so it has an established renderer format.
/// - Both must be used from the thread that created them.
///
/// # Panics
///
/// Panics if the filter is detached, if the surface has never been attached, or
/// if the placement is empty.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_composite_gpu_surface(
    filter: *mut WuiAppliedFilterState,
    surface: *mut super::gpu_surface::WuiGpuSurfaceState,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
) {
    // SAFETY: the caller contract requires `filter` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let filter = unsafe { crate::borrow_ffi_mut(filter) };
    // SAFETY: the caller contract requires `surface` to be a valid handle for a
    // different state, alive and not otherwise borrowed for this call.
    let surface = unsafe { crate::borrow_ffi_mut(surface) };
    let capture_texture = filter
        .capture_texture
        .take()
        .expect("waterui_applied_filter_composite_gpu_surface: presentation target is detached");
    super::capture_composite::composite_gpu_surface(
        &mut filter.capture_compositor,
        surface,
        &capture_texture,
        super::capture_composite::CompositePlacement {
            x,
            y,
            width,
            height,
            scale,
        },
        "waterui_applied_filter_composite_gpu_surface",
    );
    filter.capture_texture = Some(capture_texture);
}

/// Draws a `GpuSurface` nested in the captured subtree into the capture (Android only).
///
/// # Safety
///
/// `filter` and `surface` must be valid state pointers from their matching
/// constructors.
///
/// # Panics
///
/// Always panics: nested-surface compositing only exists on Android, because
/// only there does the capture arrive with the surface missing from it.
#[cfg(not(target_os = "android"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_composite_gpu_surface(
    _filter: *mut WuiAppliedFilterState,
    _surface: *mut super::gpu_surface::WuiGpuSurfaceState,
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
    _scale: f64,
) {
    panic!("waterui_applied_filter_composite_gpu_surface: only supported on Android");
}

/// Get a pointer to the Metal texture backing the capture texture (Apple only).
///
/// This exposes the underlying `MTLTexture` so native code can render directly
/// into the wgpu capture texture without extra copies.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_create` with an attached target.
///
/// # Panics
///
/// Panics if `state` does not currently have a capture texture, meaning the
/// presentation target is detached.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_get_capture_metal_texture(
    state: *mut WuiAppliedFilterState,
) -> *mut c_void {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    let texture = state.capture_texture.as_ref().expect(
        "waterui_applied_filter_get_capture_metal_texture: presentation target is detached",
    );
    // SAFETY: this path is only reached on the Metal backend, so the texture's HAL
    // type is `MetalApi` and the downcast cannot fail.
    let hal_texture = unsafe { texture.as_hal::<MetalApi>().unwrap_unchecked() };

    let raw = hal_texture.raw_handle();
    core::ptr::from_ref(raw).cast_mut().cast::<c_void>()
}

/// Get a pointer to the Metal texture backing the capture texture (Apple only).
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_create` with an attached target.
///
/// # Panics
///
/// Always panics: Metal textures only exist on Apple platforms.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_get_capture_metal_texture(
    _state: *mut WuiAppliedFilterState,
) -> *mut c_void {
    panic!("waterui_applied_filter_get_capture_metal_texture: only supported on Apple platforms");
}

/// Clean up `AppliedFilter` resources.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_create`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_drop(state: *mut WuiAppliedFilterState) {
    // SAFETY: the caller contract makes `state` an owning handle from the matching
    // constructor that has not been dropped.
    unsafe {
        let _ = Box::from_raw(state);
    }
}
