//! FFI bindings for the AppliedFilter metadata.
//!
//! This module provides the FFI interface for applying GPU filters to captured
//! view content using wgpu.
//!
//! The native backend is responsible for:
//! 1. Creating a capture layer for the child view
//! 2. Creating an output layer for the filter result
//! 3. Calling `waterui_applied_filter_init` with the output layer
//! 4. Calling `waterui_applied_filter_setup` with a callback
//! 5. Waiting for the callback before rendering
//! 6. Calling `waterui_applied_filter_render` each frame (with width/height)
//! 7. Calling `waterui_applied_filter_drop` when the view is destroyed

use core::ffi::c_void;
use std::sync::Arc;

use alloc::boxed::Box;
use alloc::vec;

use waterui_graphics::filter_view::{AppliedFilter, FilterContext, FilterInput, FilterOutput};
use waterui_graphics::shared_context::shared_context;

use crate::{IntoFFI, WuiAnyView};

/// Callback type for async completion notifications.
pub type WuiCallback = unsafe extern "C" fn(user_data: *mut c_void);

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
    /// Capture texture format
    capture_format: wgpu::TextureFormat,
    /// The filter
    filter: AppliedFilter,
    /// Whether setup() has been called
    initialized: bool,
    /// Current input dimensions (from child view)
    input_width: u32,
    input_height: u32,
    /// Current output dimensions
    output_width: u32,
    output_height: u32,
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
    let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if filter_ffi.is_null() || output_layer.is_null() || input_width == 0 || input_height == 0 {
            tracing::error!(
                "[AppliedFilter] init failed: invalid parameters (filter={:?}, layer={:?}, {}x{})",
                filter_ffi,
                output_layer,
                input_width,
                input_height
            );
            return core::ptr::null_mut();
        }

        let wui_filter = unsafe { &mut *filter_ffi };

        // Recover the filter
        if wui_filter.filter.is_null() {
            tracing::error!("[AppliedFilter] init failed: filter pointer is null");
            return core::ptr::null_mut();
        }
        let filter: AppliedFilter =
            unsafe { *Box::from_raw(wui_filter.filter as *mut AppliedFilter) };

        // Null out to prevent double-free
        wui_filter.filter = core::ptr::null_mut();

        // Output size matches input for filters
        let output_width = input_width;
        let output_height = input_height;

        // Initialize shared context if needed
        if !waterui_graphics::shared_context::is_initialized() {
            tracing::info!("[AppliedFilter] Shared context not initialized, initializing now...");
            if let Err(e) = waterui_graphics::shared_context::init_shared_context() {
                tracing::error!("[AppliedFilter] Init failed: {}", e);
                return core::ptr::null_mut();
            }
        }
        let ctx = shared_context();
        let guard = ctx.read();

        let instance = &guard.instance;
        let adapter = &guard.adapter;
        let device = guard.device.clone();
        let queue = guard.queue.clone();
        let pipeline_cache = guard.pipeline_cache.clone();

        // Create output surface
        let Some(output_surface) = super::gpu_surface::create_surface_from_layer(instance, output_layer) else {
            tracing::error!("[AppliedFilter] Failed to create output surface from layer");
            return core::ptr::null_mut();
        };

        // Configure output surface
        let surface_caps = output_surface.get_capabilities(adapter);
        if surface_caps.formats.is_empty() {
            tracing::error!("[AppliedFilter] Shared adapter cannot present to output surface!");
            return core::ptr::null_mut();
        }

        let preferred = waterui_graphics::gpu_surface::preferred_surface_format(&surface_caps);
        let format = if surface_caps.formats.contains(&preferred) {
            preferred
        } else {
            tracing::warn!("[AppliedFilter] Preferred format {:?} not supported, using {:?}", preferred, surface_caps.formats[0]);
            surface_caps.formats[0]
        };

        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
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

        tracing::info!(
            "[AppliedFilter] Configuring output: {}x{} {:?}",
            output_width, output_height, format
        );

        if !try_configure_surface(&output_surface, &device, &output_config) {
            tracing::error!("[AppliedFilter] Output surface configuration failed!");
            return core::ptr::null_mut();
        }

        // Create capture texture
        let capture_format = format;
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
            capture_format,
            filter,
            initialized: false,
            input_width,
            input_height,
            output_width,
            output_height,
        });

        Box::into_raw(state)
    }));

    match init_result {
        Ok(ptr) => ptr,
        Err(_) => {
            tracing::error!("[AppliedFilter] init panicked");
            core::ptr::null_mut()
        }
    }
}

/// Setup the filter synchronously, call callback when ready.
///
/// This function runs setup to completion using `pollster::block_on`
/// and calls the callback when setup completes.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state from `waterui_applied_filter_init`
/// * `callback` - Function to call when setup is complete
/// * `user_data` - Opaque pointer passed to callback
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_applied_filter_init`
/// - `callback` must be a valid function pointer
/// - `user_data` must remain valid until callback is invoked
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_setup(
    state: *mut WuiAppliedFilterState,
    callback: WuiCallback,
    user_data: *mut c_void,
) {
    if state.is_null() {
        tracing::error!("[AppliedFilter] setup: null state");
        unsafe { callback(user_data) };
        return;
    }

    let state = unsafe { &mut *state };

    if state.initialized {
        // Already set up, call callback immediately
        unsafe { callback(user_data) };
        return;
    }

    // Build FilterContext
    let ctx = FilterContext {
        device: &state.device,
        queue: &state.queue,
        input_format: state.capture_format,
        output_format: state.output_config.format,
        pipeline_cache: state.pipeline_cache.as_ref(),
    };

    // Run setup synchronously using pollster::block_on
    let setup_future = state.filter.setup(&ctx);
    pollster::block_on(setup_future);

    state.initialized = true;

    tracing::debug!("[AppliedFilter] setup complete");

    // Call completion callback
    unsafe { callback(user_data) };
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
/// - `waterui_applied_filter_setup` must have completed (callback was called)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_render(
    state: *mut WuiAppliedFilterState,
    width: u32,
    height: u32,
) -> WuiAppliedFilterRenderResult {
    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.is_null() || width == 0 || height == 0 {
            return WuiAppliedFilterRenderResult { success: false, needs_redraw: false };
        }

        let state = unsafe { &mut *state };

        // Verify setup was called
        if !state.initialized {
            tracing::error!("[AppliedFilter] render called before setup completed");
            return WuiAppliedFilterRenderResult { success: false, needs_redraw: false };
        }

        // Handle resize if needed
        if width != state.input_width || height != state.input_height {
            state.input_width = width;
            state.input_height = height;
            state.output_width = width;
            state.output_height = height;
            state.output_config.width = width;
            state.output_config.height = height;

            // Recreate capture texture
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

            // Reconfigure output surface
            if !try_configure_surface(&state.output_surface, &state.device, &state.output_config) {
                tracing::warn!("[AppliedFilter] resize reconfigure failed ({width}x{height})");
                return WuiAppliedFilterRenderResult { success: false, needs_redraw: false };
            }
        }

        // Get output texture
        let output = match state.output_surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                if !try_configure_surface(&state.output_surface, &state.device, &state.output_config) {
                    return WuiAppliedFilterRenderResult { success: false, needs_redraw: false };
                }
                match state.output_surface.get_current_texture() {
                    Ok(o) => o,
                    Err(_) => return WuiAppliedFilterRenderResult { success: false, needs_redraw: false },
                }
            }
            Err(wgpu::SurfaceError::Timeout) => {
                // Skip frame but success
                return WuiAppliedFilterRenderResult { success: true, needs_redraw: false };
            }
            Err(e) => {
                tracing::error!("[AppliedFilter] render failed: {e}");
                return WuiAppliedFilterRenderResult { success: false, needs_redraw: false };
            }
        };

        // Get input texture
        let input_texture: &wgpu::Texture = if let Some(ref imported) = state.imported_texture {
            imported
        } else if let Some(ref capture) = state.capture_texture {
            capture
        } else {
            tracing::error!("[AppliedFilter] no input texture available");
            return WuiAppliedFilterRenderResult { success: false, needs_redraw: false };
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
        let input = FilterInput {
            device: &state.device,
            queue: &state.queue,
            texture: input_texture,
            view: input_view,
            format: state.capture_format,
            width: state.input_width,
            height: state.input_height,
        };

        let filter_output = FilterOutput {
            device: &state.device,
            queue: &state.queue,
            texture: &output.texture,
            view: output_view,
            format: state.output_config.format,
            width: state.output_width,
            height: state.output_height,
        };

        // Call filter render - returns true if animation needs more frames
        let needs_redraw = state.filter.render(&input, &filter_output);

        // Present
        output.present();

        WuiAppliedFilterRenderResult { success: true, needs_redraw }
    }));

    match render_result {
        Ok(result) => result,
        Err(_) => {
            tracing::error!("[AppliedFilter] render panicked");
            WuiAppliedFilterRenderResult { success: false, needs_redraw: false }
        }
    }
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
    if state.is_null() {
        return core::ptr::null();
    }

    let state = unsafe { &*state };

    match &state.capture_texture {
        Some(texture) => texture as *const wgpu::Texture as *const c_void,
        None => core::ptr::null(),
    }
}

/// Clean up AppliedFilter resources.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_applied_filter_init`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_drop(state: *mut WuiAppliedFilterState) {
    if !state.is_null() {
        unsafe {
            let _ = Box::from_raw(state);
        }
    }
}

fn try_configure_surface(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> bool {
    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Internal);
    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let configure_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        surface.configure(device, config);
    }))
    .is_err();

    let validation_err = pollster::block_on(device.pop_error_scope());
    let internal_err = pollster::block_on(device.pop_error_scope());
    let oom_err = pollster::block_on(device.pop_error_scope());

    if configure_panicked {
        tracing::warn!("[AppliedFilter] Surface::configure panicked");
        return false;
    }

    if let Some(err) = validation_err.or(internal_err).or(oom_err) {
        tracing::warn!("[AppliedFilter] Surface::configure failed: {err}");
        return false;
    }

    true
}
