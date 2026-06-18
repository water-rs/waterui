//! FFI bindings for the AppliedFilter metadata.
//!
//! This module provides the FFI interface for applying GPU filters to captured
//! view content using wgpu.
//!
//! The native backend is responsible for:
//! 1. Creating a capture layer for the child view
//! 2. Creating an output layer for the filter result
//! 3. Calling `waterui_applied_filter_init` with the output layer
//! 4. Calling `waterui_applied_filter_setup` and checking the returned success flag
//! 5. Resolving output size after `waterui_applied_filter_sync_targets`
//! 6. Calling `waterui_applied_filter_render` when rendering is scheduled (with width/height)
//! 7. Calling `waterui_applied_filter_drop` when the view is destroyed

use core::ffi::c_void;
use std::sync::Arc;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::vec;

// Platform-specific imports for Metal HAL texture import
#[cfg(any(target_os = "macos", target_os = "ios"))]
use {metal::MTLTextureType, metal::foreign_types::ForeignType, wgpu_hal::api::Metal as MetalApi};

use waterui_graphics::filter_view::{AppliedFilter, EffectContext, EffectInput, EffectOutput};
use waterui_graphics::shared_context::shared_context;

#[cfg(target_os = "android")]
use crate::components::android_ahb;
use crate::components::pixel_upload::prepare_rgba8_upload;
use crate::components::view_effect::WuiExternalDropFn;
use crate::components::view_effect::WuiInputType;
use crate::{IntoFFI, WuiAnyView};

#[cold]
#[inline(never)]
fn abort_on_panic(scope: &'static str) -> ! {
    tracing::error!("{scope} panicked; aborting process");
    std::process::abort();
}

/// FFI representation of a Metadata<AppliedFilter>.
#[repr(C)]
pub struct WuiAppliedFilter {
    /// The child view to capture (pointer to WuiAnyView).
    pub content: *mut WuiAnyView,
    /// Opaque pointer to the boxed AppliedFilter.
    /// This is consumed during init and should not be used after.
    pub filter: *mut c_void,
}

impl IntoFFI for waterui_core::Metadata<AppliedFilter> {
    type FFI = WuiAppliedFilter;

    fn into_ffi(self) -> Self::FFI {
        // Take the child view and convert to FFI
        let content = self.content.into_ffi();

        // Box the AppliedFilter for FFI transfer
        let filter_ptr = Box::into_raw(Box::new(self.value)) as *mut c_void;

        WuiAppliedFilter {
            content,
            filter: filter_ptr,
        }
    }
}

// Generate waterui_metadata_applied_filter_id() and waterui_force_as_metadata_applied_filter()
ffi_metadata!(AppliedFilter, WuiAppliedFilter, applied_filter);

/// Opaque state held by the native backend after initialization.
pub struct WuiAppliedFilterState {
    /// Shared device (Arc reference from SharedGpuContext)
    device: Arc<wgpu::Device>,
    /// Shared queue (Arc reference from SharedGpuContext)
    queue: Arc<wgpu::Queue>,
    /// Optional shared pipeline cache
    pipeline_cache: Option<wgpu::PipelineCache>,
    /// Per-view wgpu surface for output
    output_surface: wgpu::Surface<'static>,
    /// Output surface configuration
    output_config: wgpu::SurfaceConfiguration,
    /// Capture texture (for capturing child view output)
    capture_texture: Option<wgpu::Texture>,
    /// Imported texture from external source (IOSurface/AHardwareBuffer)
    imported_texture: Option<wgpu::Texture>,
    /// Format of the imported texture (if any)
    imported_format: Option<wgpu::TextureFormat>,
    /// Retained Metal texture when using the Metal import path (keeps it alive for wgpu)
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    imported_metal_texture: Option<metal::Texture>,
    /// Capture texture format
    capture_format: wgpu::TextureFormat,
    /// The filter
    filter: AppliedFilter,
    /// Whether setup() has been called
    initialized: bool,
    /// Input format used for the most recent successful setup.
    setup_input_format: Option<wgpu::TextureFormat>,
    /// Output format used for the most recent successful setup.
    setup_output_format: Option<wgpu::TextureFormat>,
    /// Current input dimensions (from child view)
    input_width: u32,
    input_height: u32,
    /// Current output dimensions
    output_width: u32,
    output_height: u32,
    /// Latest output dimensions resolved from snapped filter state.
    resolved_output_width: u32,
    resolved_output_height: u32,
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

fn output_dimensions_for_input(
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
    let (output_width, output_height) = output_dimensions_for_input(state, width, height);
    let input_resized = width != state.input_width || height != state.input_height;
    let output_resized = output_width != state.output_width || output_height != state.output_height;

    if input_resized {
        state.input_width = width;
        state.input_height = height;
    }

    if output_resized {
        state.output_width = output_width;
        state.output_height = output_height;
        state.output_config.width = output_width;
        state.output_config.height = output_height;
    }

    if input_resized || state.capture_texture.is_none() {
        state.capture_texture = Some(state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("AppliedFilter Capture Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: state.capture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        }));
    }

    if output_resized {
        try_configure_surface(&state.output_surface, &state.device, &state.output_config);
    }
}

fn resolve_output_size(
    state: &mut WuiAppliedFilterState,
    input_width: u32,
    input_height: u32,
) -> WuiAppliedFilterOutputSize {
    let (output_width, output_height) = state.filter.output_size(input_width, input_height);
    assert!(
        output_width > 0 && output_height > 0,
        "waterui_applied_filter_resolve_output_size: filter produced invalid output size {}x{} for input {}x{}",
        output_width,
        output_height,
        input_width,
        input_height
    );
    state.resolved_output_width = output_width;
    state.resolved_output_height = output_height;
    WuiAppliedFilterOutputSize {
        width: output_width,
        height: output_height,
    }
}

/// Initialize an AppliedFilter with native layers.
///
/// This function creates wgpu resources for the filter rendering pipeline.
///
/// # Arguments
///
/// * `filter_ffi` - Pointer to the WuiAppliedFilter FFI struct (consumed)
/// * `output_layer` - Platform-specific layer for filter output:
///   - Apple: `CAMetalLayer*`
///   - Android: `ANativeWindow*`
/// * `input_width` - Width of the captured view in pixels
/// * `input_height` - Height of the captured view in pixels
///
/// # Returns
///
/// Opaque pointer to the initialized state, or null on failure.
///
/// # Safety
///
/// - `filter_ffi` must be a valid pointer obtained from `waterui_force_as_metadata_applied_filter`
/// - `output_layer` must be a valid platform-specific layer pointer
/// - The layer must remain valid for the lifetime of the returned state
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_init(
    filter_ffi: *mut WuiAppliedFilter,
    output_layer: *mut c_void,
    input_width: u32,
    input_height: u32,
) -> *mut WuiAppliedFilterState {
    unsafe {
        crate::expect_non_null_mut(filter_ffi, "waterui_applied_filter_init", "filter_ffi");
        crate::expect_non_null(output_layer, "waterui_applied_filter_init", "output_layer");
    }

    let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let wui_filter = unsafe { &mut *filter_ffi };

        // Recover the filter
        let filter: AppliedFilter =
            unsafe { *Box::from_raw(wui_filter.filter as *mut AppliedFilter) };

        // Null out to prevent double-free
        wui_filter.filter = core::ptr::null_mut();

        let output_width = input_width;
        let output_height = input_height;

        // Initialize shared context if needed
        if !waterui_graphics::shared_context::is_initialized() {
            tracing::debug!("[AppliedFilter] Shared context not initialized, initializing now...");
            waterui_graphics::shared_context::init_shared_context()
                .expect("waterui_applied_filter_init: shared GPU context init failed");
        }
        let ctx = shared_context();
        let guard = ctx.read();

        let instance = &guard.instance;
        let adapter = &guard.adapter;
        let device = guard.device.clone();
        let queue = guard.queue.clone();
        let pipeline_cache = guard.pipeline_cache.clone();

        // Create output surface
        let Some(output_surface) =
            crate::components::gpu_surface::create_surface_from_layer(instance, output_layer)
        else {
            panic!("waterui_applied_filter_init: failed to create output surface from layer");
        };

        // Configure output surface
        let surface_caps = output_surface.get_capabilities(adapter);
        assert!(
            !(surface_caps.formats.is_empty()),
            "waterui_applied_filter_init: adapter cannot present to output surface"
        );

        let preferred = waterui_graphics::gpu_surface::preferred_surface_format(&surface_caps);
        let format = if surface_caps.formats.contains(&preferred) {
            preferred
        } else {
            panic!(
                "waterui_applied_filter_init: preferred format {:?} is not supported by surface capabilities {:?}",
                preferred, surface_caps.formats
            );
        };

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo
        } else {
            surface_caps.present_modes[0]
        };

        let alpha_mode = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
            wgpu::CompositeAlphaMode::Opaque,
        ]
        .into_iter()
        .find(|mode| surface_caps.alpha_modes.contains(mode))
        .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

        let output_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: output_width,
            height: output_height,
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        tracing::debug!(
            "[AppliedFilter] Configuring output: {}x{} {:?}",
            output_width,
            output_height,
            format
        );

        try_configure_surface(&output_surface, &device, &output_config);

        // Create capture texture. Prefer the output format so GpuSurface capture
        // can render into it without format mismatches (common on SDR displays).
        let required = wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST;
        let capture_format = if adapter
            .get_texture_format_features(format)
            .allowed_usages
            .contains(required)
        {
            format
        } else {
            let hdr = wgpu::TextureFormat::Rgba16Float;
            let features = adapter.get_texture_format_features(hdr);
            if features.allowed_usages.contains(required) {
                hdr
            } else {
                format
            }
        };
        if capture_format != format {
            tracing::debug!(
                "[AppliedFilter] Using capture format {:?} (output {:?})",
                capture_format,
                format
            );
        }
        let capture_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("AppliedFilter Capture Texture"),
            size: wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: capture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let state = Box::new(WuiAppliedFilterState {
            device,
            queue,
            pipeline_cache,
            output_surface,
            output_config,
            capture_texture: Some(capture_texture),
            imported_texture: None,
            imported_format: None,
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            imported_metal_texture: None,
            capture_format,
            filter,
            initialized: false,
            setup_input_format: None,
            setup_output_format: None,
            input_width,
            input_height,
            output_width,
            output_height,
            resolved_output_width: output_width,
            resolved_output_height: output_height,
        });

        Box::into_raw(state)
    }));

    match init_result {
        Ok(ptr) => ptr,
        Err(_) => abort_on_panic("waterui_applied_filter_init"),
    }
}

/// Await filter setup to completion on the synchronous FFI path.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state from `waterui_applied_filter_init`
///
/// Returns `true` when setup completes successfully.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_init`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_setup(state: *mut WuiAppliedFilterState) -> bool {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_applied_filter_setup", "state");
    }

    let setup_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };
        ensure_applied_filter_setup(state, current_applied_filter_input_format(state))
    }));

    match setup_result {
        Ok(success) => success,
        Err(_) => abort_on_panic("waterui_applied_filter_setup"),
    }
}

/// Result of a filter render operation.
#[repr(C)]
pub struct WuiAppliedFilterRenderResult {
    /// Whether rendering succeeded.
    pub success: bool,
    /// Whether another frame is needed (animation in progress).
    /// Only valid if `success` is true.
    pub needs_redraw: bool,
}

/// Render the filter.
///
/// This function applies the filter to the captured input and renders to the output.
/// Pass current width/height - resources are recreated if size changed.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state
/// * `width` - Current width in pixels
/// * `height` - Current height in pixels
///
/// # Returns
///
/// A `WuiAppliedFilterRenderResult` with:
/// - `success`: whether rendering succeeded
/// - `needs_redraw`: whether another frame is needed (for animations)
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_init`
/// - `waterui_applied_filter_setup` must have returned `true`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_render(
    state: *mut WuiAppliedFilterState,
    width: u32,
    height: u32,
) -> WuiAppliedFilterRenderResult {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_applied_filter_render", "state");
    }
    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };

        ensure_dimensions(state, width, height);

        // Get output texture
        let output = match state.output_surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                try_configure_surface(&state.output_surface, &state.device, &state.output_config);
                match state.output_surface.get_current_texture() {
                    Ok(o) => o,
                    Err(e) => {
                        panic!(
                            "waterui_applied_filter_render: acquire after reconfigure failed: {e}"
                        );
                    }
                }
            }
            Err(wgpu::SurfaceError::Timeout) => {
                panic!("waterui_applied_filter_render: surface timeout");
            }
            Err(e) => {
                panic!("waterui_applied_filter_render: get_current_texture failed: {e}");
            }
        };

        let input_format = current_applied_filter_input_format(state);

        if !ensure_applied_filter_setup(state, input_format) {
            tracing::error!("[AppliedFilter] render requested before successful setup");
            return WuiAppliedFilterRenderResult {
                success: false,
                needs_redraw: false,
            };
        }

        // Get input texture after setup so mutable borrows of `state` are finished.
        let input_texture: &wgpu::Texture = if let Some(ref imported) = state.imported_texture {
            imported
        } else if let Some(ref capture) = state.capture_texture {
            capture
        } else {
            panic!("waterui_applied_filter_render: no input texture available");
        };

        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("AppliedFilter Input View"),
            ..Default::default()
        });

        let output_view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("AppliedFilter Output View"),
            format: Some(state.output_config.format),
            ..Default::default()
        });

        // Create input/output structs
        let input = EffectInput {
            device: &state.device,
            queue: &state.queue,
            texture: input_texture,
            view: input_view,
            format: input_format,
            width: state.input_width,
            height: state.input_height,
        };

        let filter_output = EffectOutput {
            device: &state.device,
            queue: &state.queue,
            texture: &output.texture,
            view: output_view,
            format: state.output_config.format,
            width: state.output_width,
            height: state.output_height,
        };

        let needs_redraw = match state.filter.render(&input, &filter_output) {
            Ok(needs_redraw) => needs_redraw,
            Err(err) => {
                tracing::error!("[AppliedFilter] render failed fast: {err}");
                return WuiAppliedFilterRenderResult {
                    success: false,
                    needs_redraw: false,
                };
            }
        };

        // Present
        output.present();

        WuiAppliedFilterRenderResult {
            success: true,
            needs_redraw,
        }
    }));

    match render_result {
        Ok(result) => result,
        Err(_) => abort_on_panic("waterui_applied_filter_render"),
    }
}

fn current_applied_filter_input_format(state: &WuiAppliedFilterState) -> wgpu::TextureFormat {
    if state.imported_texture.is_some() {
        unsafe { state.imported_format.unwrap_unchecked() }
    } else {
        state.capture_format
    }
}

fn ensure_applied_filter_setup(
    state: &mut WuiAppliedFilterState,
    input_format: wgpu::TextureFormat,
) -> bool {
    let output_format = state.output_config.format;
    if state.initialized
        && state.setup_input_format == Some(input_format)
        && state.setup_output_format == Some(output_format)
    {
        return true;
    }

    let ctx = EffectContext {
        device: &state.device,
        queue: &state.queue,
        input_format,
        output_format,
        pipeline_cache: state.pipeline_cache.as_ref(),
    };

    let setup_future = state.filter.setup(&ctx);
    match pollster::block_on(setup_future) {
        Ok(()) => {
            state.initialized = true;
            state.setup_input_format = Some(input_format);
            state.setup_output_format = Some(output_format);
            tracing::debug!("[AppliedFilter] setup complete");
            true
        }
        Err(err) => {
            state.initialized = false;
            state.setup_input_format = None;
            state.setup_output_format = None;
            tracing::error!("[AppliedFilter] setup failed fast: {err}");
            false
        }
    }
}

/// Snapshot reactive filter targets on the caller thread.
///
/// This must be called before scheduling render work on background queues so
/// filter parameter reads stay on the UI/reactive thread.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_init`
/// - Caller must ensure no concurrent `waterui_applied_filter_render` is running
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_sync_targets(
    state: *mut WuiAppliedFilterState,
) -> bool {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_applied_filter_sync_targets", "state");
    }
    let sync_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };
        state.filter.sync_targets();
        true
    }));

    match sync_result {
        Ok(ok) => ok,
        Err(_) => abort_on_panic("waterui_applied_filter_sync_targets"),
    }
}

/// Resolve the current output size from snapped filter state.
///
/// Call this after `waterui_applied_filter_sync_targets` and before scheduling render work.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_init`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_resolve_output_size(
    state: *mut WuiAppliedFilterState,
    input_width: u32,
    input_height: u32,
) -> WuiAppliedFilterOutputSize {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_applied_filter_resolve_output_size", "state");
    }

    let resolve_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };
        resolve_output_size(state, input_width, input_height)
    }));

    match resolve_result {
        Ok(output_size) => output_size,
        Err(_) => abort_on_panic("waterui_applied_filter_resolve_output_size"),
    }
}

/// Poll whether the filter requires a new frame.
///
/// This synchronizes reactive targets and returns the filter's redraw hint.
/// Native backends use this to keep on-demand loops responsive without
/// continuously rendering when nothing changed.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_poll_redraw(
    state: *mut WuiAppliedFilterState,
) -> bool {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_applied_filter_poll_redraw", "state");
    }
    let poll_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };
        state.filter.sync_targets();
        state.filter.redraw_hint()
    }));

    match poll_result {
        Ok(should_redraw) => should_redraw,
        Err(_) => abort_on_panic("waterui_applied_filter_poll_redraw"),
    }
}

/// Provide input texture from child view.
///
/// Call this before each scheduled `waterui_applied_filter_render` to provide
/// the captured child view's texture.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state
/// * `input_type` - Type of input being provided
/// * `input_handle` - Platform-specific handle:
///   - `WgpuTexture`: Pointer to `wgpu::Texture`
///   - `MetalTexture`: `MTLTexture*` (Apple)
///   - `AHardwareBuffer`: `AHardwareBuffer*` (Android)
///   - `PixelData`: Pointer to pixel data
/// * `width` - Input width in pixels
/// * `height` - Input height in pixels
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_init`
/// - `input_handle` must be valid for the specified `input_type`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_set_input(
    state: *mut WuiAppliedFilterState,
    input_type: WuiInputType,
    input_handle: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    let state =
        unsafe { crate::expect_non_null_mut(state, "waterui_applied_filter_set_input", "state") };

    ensure_dimensions(state, width, height);

    match input_type {
        WuiInputType::WgpuTexture => {
            let imported = unsafe { &*(input_handle as *const wgpu::Texture) };
            state.imported_texture = Some(imported.clone());
            state.imported_format = Some(imported.format());
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                state.imported_metal_texture = None;
            }
            true
        }
        WuiInputType::MetalTexture => {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                import_metal_texture(state, input_handle, width, height)
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            {
                panic!(
                    "waterui_applied_filter_set_input: MetalTexture is only supported on Apple platforms"
                );
            }
        }
        WuiInputType::AHardwareBuffer => {
            #[cfg(target_os = "android")]
            {
                panic!(
                    "waterui_applied_filter_set_input: AHardwareBuffer requires a drop callback; use waterui_applied_filter_set_input_ahardwarebuffer"
                );
            }
            #[cfg(not(target_os = "android"))]
            {
                panic!(
                    "waterui_applied_filter_set_input: AHardwareBuffer is only supported on Android"
                );
            }
        }
        WuiInputType::PixelData => {
            let capture_texture = unsafe { state.capture_texture.as_ref().unwrap_unchecked() };
            let upload =
                unsafe { prepare_rgba8_upload(input_handle, width, height).unwrap_unchecked() };

            state.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: capture_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                upload.bytes(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(upload.bytes_per_row()),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            state.imported_texture = None;
            state.imported_format = None;
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                state.imported_metal_texture = None;
            }
            true
        }
    }
}

/// Set input from an AHardwareBuffer (Android-specific zero-copy path).
///
/// This requires native to pass a drop callback that releases an acquired reference to the
/// AHardwareBuffer when wgpu is done using it (after GPU work completes).
///
/// # Safety
///
/// - `state` must be a valid, exclusively-borrowable `WuiAppliedFilterState` pointer.
/// - `ahb_ptr` must be a valid `AHardwareBuffer` pointer that stays alive until `drop_fn` is invoked.
/// - `drop_fn` must safely release the buffer when called with `drop_data`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_set_input_ahardwarebuffer(
    state: *mut WuiAppliedFilterState,
    ahb_ptr: *mut c_void,
    drop_fn: WuiExternalDropFn,
    drop_data: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    let state = unsafe {
        crate::expect_non_null_mut(
            state,
            "waterui_applied_filter_set_input_ahardwarebuffer",
            "state",
        )
    };

    #[cfg(target_os = "android")]
    {
        ensure_dimensions(state, width, height);

        match android_ahb::import_ahardwarebuffer_as_wgpu_texture(
            &state.device,
            ahb_ptr,
            width,
            height,
            "AppliedFilter Imported AHardwareBuffer Texture",
            drop_fn,
            drop_data,
        ) {
            Ok((texture, format)) => {
                state.imported_texture = Some(texture);
                state.imported_format = Some(format);
                true
            }
            Err(e) => {
                panic!(
                    "waterui_applied_filter_set_input_ahardwarebuffer: AHardwareBuffer import failed: {e}"
                );
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (state, ahb_ptr, drop_fn as usize, drop_data, width, height);
        panic!("waterui_applied_filter_set_input_ahardwarebuffer: only supported on Android");
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn import_metal_texture(
    state: &mut WuiAppliedFilterState,
    mtl_texture_ptr: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    use metal::foreign_types::ForeignTypeRef;
    use wgpu_hal::Api;

    let metal_texture_ref = unsafe { metal::TextureRef::from_ptr(mtl_texture_ptr.cast()) };
    let metal_texture = metal_texture_ref.to_owned();
    let wgpu_format = match metal_texture.pixel_format() {
        metal::MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        metal::MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        metal::MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => {
            panic!(
                "applied_filter::import_metal_texture: unsupported Metal format {:?}",
                other
            );
        }
    };

    tracing::debug!(
        "[AppliedFilter] Importing Metal texture: {}x{} {:?}",
        width,
        height,
        metal_texture.pixel_format()
    );

    let hal_texture = unsafe {
        <MetalApi as Api>::Device::texture_from_raw(
            metal_texture.clone(),
            wgpu_format,
            MTLTextureType::D2,
            1,
            1,
            wgpu_hal::CopyExtent {
                width,
                height,
                depth: 1,
            },
        )
    };

    let texture_desc = wgpu::TextureDescriptor {
        label: Some("AppliedFilter Imported Metal Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    let wgpu_texture = unsafe {
        state
            .device
            .create_texture_from_hal::<MetalApi>(hal_texture, &texture_desc)
    };

    state.imported_texture = Some(wgpu_texture);
    state.imported_format = Some(wgpu_format);
    state.imported_metal_texture = Some(metal_texture);
    state.input_width = width;
    state.input_height = height;

    true
}

/// Prepare the capture texture for rendering.
///
/// Ensures the capture texture matches the requested dimensions and returns
/// a pointer to the underlying wgpu texture for zero-copy rendering paths.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_prepare_capture(
    state: *mut WuiAppliedFilterState,
    width: u32,
    height: u32,
) -> *const c_void {
    let state = unsafe {
        crate::expect_non_null_mut(state, "waterui_applied_filter_prepare_capture", "state")
    };

    ensure_dimensions(state, width, height);

    state.imported_texture = None;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        state.imported_metal_texture = None;
    }

    let texture = unsafe { state.capture_texture.as_ref().unwrap_unchecked() };
    texture as *const wgpu::Texture as *const c_void
}

/// Get a pointer to the capture texture.
///
/// The native backend should render the child view to this texture.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_get_capture_texture(
    state: *mut WuiAppliedFilterState,
) -> *const c_void {
    let state = unsafe {
        crate::expect_non_null(state, "waterui_applied_filter_get_capture_texture", "state")
    };

    let texture = unsafe { state.capture_texture.as_ref().unwrap_unchecked() };
    texture as *const wgpu::Texture as *const c_void
}

/// Get a pointer to the Metal texture backing the capture texture (Apple only).
///
/// This exposes the underlying MTLTexture so native code can render directly
/// into the wgpu capture texture without extra copies.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_init`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_get_capture_metal_texture(
    state: *mut WuiAppliedFilterState,
) -> *mut c_void {
    let state = unsafe {
        crate::expect_non_null(
            state,
            "waterui_applied_filter_get_capture_metal_texture",
            "state",
        )
    };
    let texture = unsafe { state.capture_texture.as_ref().unwrap_unchecked() };
    let hal_texture = unsafe { texture.as_hal::<MetalApi>().unwrap_unchecked() };

    let raw = unsafe { hal_texture.raw_handle() };
    raw.as_ptr().cast::<c_void>()
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_get_capture_metal_texture(
    _state: *mut WuiAppliedFilterState,
) -> *mut c_void {
    panic!("waterui_applied_filter_get_capture_metal_texture: only supported on Apple platforms");
}

/// Clean up AppliedFilter resources.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_init`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_drop(state: *mut WuiAppliedFilterState) {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_applied_filter_drop", "state");
        let _ = Box::from_raw(state);
    }
}

fn try_configure_surface(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) {
    let _ = device.poll(wgpu::PollType::Poll);

    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Internal);
    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let configure_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        surface.configure(device, config);
    }))
    .is_err();

    let validation_err =
        crate::pop_error_scope_now(device, "applied_filter::validation_error_scope");
    let internal_err = crate::pop_error_scope_now(device, "applied_filter::internal_error_scope");
    let oom_err = crate::pop_error_scope_now(device, "applied_filter::oom_error_scope");

    if configure_panicked {
        abort_on_panic("applied_filter::try_configure_surface");
    }

    let configure_err = validation_err.or(internal_err).or(oom_err);
    assert!(
        configure_err.is_none(),
        "applied_filter::try_configure_surface failed: {configure_err:?}"
    );
}
