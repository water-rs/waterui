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
use alloc::vec;
use executor_core::spawn_local;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use wgpu_hal::api::Metal as MetalApi;

use waterui_graphics::RedrawHandle;
use waterui_graphics::filter_view::{
    AppliedFilter, EffectContext, EffectFrameClock, EffectInput, EffectOutput,
};
use waterui_graphics::shared_context::GpuRuntime;

use crate::{IntoFFI, WuiAnyView};

/// Native callback invoked when an idle applied-filter surface becomes dirty.
pub type WuiAppliedFilterRedrawCallback = unsafe extern "C" fn(context: *mut c_void);

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
        unsafe {
            (self.wake)(self.context as *mut c_void);
        }
    }
}

impl Drop for ForeignRedrawTarget {
    fn drop(&mut self) {
        unsafe {
            (self.drop)(self.context as *mut c_void);
        }
    }
}

/// FFI representation of a Metadata<AppliedFilter>.
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
ffi_metadata!(
    AppliedFilter,
    WuiAppliedFilter,
    applied_filter,
    all(),
    any()
);

/// Opaque state held by the native backend for one semantic applied filter.
pub struct WuiAppliedFilterState {
    /// Explicit environment-owned GPU runtime used by this semantic filter.
    runtime: GpuRuntime,
    /// Currently attached presentation surface.
    output_surface: Option<wgpu::Surface<'static>>,
    /// Configuration for the currently attached presentation surface.
    output_config: Option<wgpu::SurfaceConfiguration>,
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
        let config = state
            .output_config
            .as_mut()
            .expect("AppliedFilter resize requires an attached output configuration");
        config.width = output_width;
        config.height = output_height;
        let surface = state
            .output_surface
            .as_ref()
            .expect("AppliedFilter resize requires an attached output surface");
        surface.configure(&state.runtime.context().device, config);
    }

    if input_resized || state.capture_texture.is_none() {
        state.capture_texture = Some(create_capture_texture(
            &state.runtime.context().device,
            capture_format,
            width,
            height,
        ));
    }
}

fn create_capture_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
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
    })
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
    let wui_filter = unsafe { &mut *filter_ffi };
    assert!(
        !wui_filter.filter.is_null(),
        "waterui_applied_filter_create: descriptor was already consumed"
    );
    let filter: AppliedFilter =
        unsafe { *Box::from_raw(wui_filter.filter.cast::<AppliedFilter>()) };
    wui_filter.filter = core::ptr::null_mut();

    let runtime = super::gpu_runtime::gpu_runtime(&unsafe { &*env }.0);
    let redraw_handle = filter.redraw_handle();
    Box::into_raw(Box::new(WuiAppliedFilterState {
        runtime,
        output_surface: None,
        output_config: None,
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
        frame_clock: EffectFrameClock::new(),
    }))
}

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
    let capture_usages = wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::COPY_DST;
    assert!(
        gpu.adapter
            .get_texture_format_features(format)
            .allowed_usages
            .contains(capture_usages),
        "waterui_applied_filter_attach: output format {format:?} cannot be used for capture"
    );
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_attach(
    state: *mut WuiAppliedFilterState,
    output_layer: *mut c_void,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.output_surface.is_none(),
        "waterui_applied_filter_attach: output surface is already attached"
    );
    let (surface, config, capture_format) =
        create_configured_surface(state, output_layer, input_width, input_height, prefers_hdr);
    if let Some((_, setup_output_format)) = state.setup_formats.get() {
        assert_eq!(
            setup_output_format, config.format,
            "AppliedFilter output format changed after setup"
        );
    }
    let capture_texture = create_capture_texture(
        &state.runtime.context().device,
        capture_format,
        input_width,
        input_height,
    );
    state.input_width = input_width;
    state.input_height = input_height;
    state.output_width = config.width;
    state.output_height = config.height;
    state.capture_format = Some(capture_format);
    state.capture_texture = Some(capture_texture);
    state.output_config = Some(config);
    state.output_surface = Some(surface);
    let _ = state.redraw_handle.take_dirty();
    if state.setup_ready.get() {
        state.redraw_handle.request_redraw();
    }
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
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let surface = state
        .output_surface
        .take()
        .expect("waterui_applied_filter_detach: output surface is already detached");
    state.output_config = None;
    state.capture_texture = None;
    state.capture_format = None;
    state.input_width = 0;
    state.input_height = 0;
    state.output_width = 0;
    state.output_height = 0;
    state.resolved_output_width = 0;
    state.resolved_output_height = 0;
    drop(surface);
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
    let state = unsafe { crate::borrow_ffi(state) };
    state.setup_ready.get()
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_render(
    state: *mut WuiAppliedFilterState,
    width: u32,
    height: u32,
) -> bool {
    let state = unsafe { crate::borrow_ffi_mut(state) };

    ensure_dimensions(state, width, height);
    let input_format = current_applied_filter_input_format(state);
    assert!(
        state.setup_ready.get(),
        "waterui_applied_filter_render called before asynchronous setup completed"
    );
    assert_setup_input_format(state, input_format);
    let output_surface = state
        .output_surface
        .as_ref()
        .expect("waterui_applied_filter_render: presentation target is detached");
    let output_config = state
        .output_config
        .as_ref()
        .expect("waterui_applied_filter_render: output configuration is detached");

    // Get output texture
    let Some(output) = super::acquire_surface_texture(
        output_surface,
        &state.runtime.context().device,
        output_config,
        "waterui_applied_filter_render",
    ) else {
        return false;
    };

    // Get input texture after setup so mutable borrows of `state` are finished.
    let input_texture = state
        .capture_texture
        .as_ref()
        .expect("waterui_applied_filter_render: presentation target is detached");

    let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("AppliedFilter Input View"),
        ..Default::default()
    });

    let output_view = output.texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("AppliedFilter Output View"),
        format: Some(output_config.format),
        ..Default::default()
    });

    // Create input/output structs
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
        texture: &output.texture,
        view: output_view,
        format: output_config.format,
        width: state.output_width,
        height: state.output_height,
    };

    let needs_redraw = state
        .filter
        .borrow_mut()
        .as_mut()
        .expect("AppliedFilter ready state is missing its semantic filter")
        .render(&input, &filter_output)
        .unwrap_or_else(|err| panic!("waterui_applied_filter_render: {err}"));

    // Present
    output.present();

    needs_redraw
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
        .output_config
        .as_ref()
        .expect("AppliedFilter setup requires an attached presentation target")
        .format;
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
        let ctx = EffectContext {
            device: &gpu.device,
            queue: &gpu.queue,
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
    let state = unsafe { crate::borrow_ffi_mut(state) };

    ensure_dimensions(state, width, height);
    let capture_format = state
        .capture_format
        .expect("waterui_applied_filter_prepare_capture: presentation target is detached");
    assert_setup_input_format(state, capture_format);
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
    let state = unsafe { crate::borrow_ffi(state) };
    let texture = state.capture_texture.as_ref().expect(
        "waterui_applied_filter_get_capture_metal_texture: presentation target is detached",
    );
    let hal_texture = unsafe { texture.as_hal::<MetalApi>().unwrap_unchecked() };

    let raw = hal_texture.raw_handle();
    core::ptr::from_ref(raw).cast_mut().cast::<c_void>()
}

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
    unsafe {
        let _ = Box::from_raw(state);
    }
}
