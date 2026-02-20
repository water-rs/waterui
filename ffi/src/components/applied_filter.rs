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

use waterui_graphics::filter_view::{AppliedFilter, FilterContext, FilterInput, FilterOutput};
use waterui_graphics::shared_context::shared_context;

use super::view_effect::WuiExternalDropFn;
use super::view_effect::WuiInputType;
#[cfg(target_os = "android")]
use crate::components::android_ahb;
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
    /// Current input dimensions (from child view)
    input_width: u32,
    input_height: u32,
    /// Current output dimensions
    output_width: u32,
    output_height: u32,
}

fn ensure_dimensions(state: &mut WuiAppliedFilterState, width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }

    let needs_resize = width != state.input_width || height != state.input_height;

    if needs_resize {
        state.input_width = width;
        state.input_height = height;
        state.output_width = width;
        state.output_height = height;
        state.output_config.width = width;
        state.output_config.height = height;
    }

    if needs_resize || state.capture_texture.is_none() {
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

    if needs_resize
        && !try_configure_surface(&state.output_surface, &state.device, &state.output_config)
    {
        tracing::warn!("[AppliedFilter] resize reconfigure failed ({width}x{height})");
        return false;
    }

    true
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
            tracing::debug!("[AppliedFilter] Shared context not initialized, initializing now...");
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
        let Some(output_surface) =
            super::gpu_surface::create_surface_from_layer(instance, output_layer)
        else {
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
            tracing::warn!(
                "[AppliedFilter] Preferred format {:?} not supported, using {:?}",
                preferred,
                surface_caps.formats[0]
            );
            surface_caps.formats[0]
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

        if !try_configure_surface(&output_surface, &device, &output_config) {
            tracing::error!("[AppliedFilter] Output surface configuration failed!");
            return core::ptr::null_mut();
        }

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
            return WuiAppliedFilterRenderResult {
                success: false,
                needs_redraw: false,
            };
        }

        let state = unsafe { &mut *state };

        // Verify setup was called
        if !state.initialized {
            tracing::error!("[AppliedFilter] render called before setup completed");
            return WuiAppliedFilterRenderResult {
                success: false,
                needs_redraw: false,
            };
        }

        if !ensure_dimensions(state, width, height) {
            return WuiAppliedFilterRenderResult {
                success: false,
                needs_redraw: false,
            };
        }

        // Get output texture
        let output = match state.output_surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                if !try_configure_surface(
                    &state.output_surface,
                    &state.device,
                    &state.output_config,
                ) {
                    return WuiAppliedFilterRenderResult {
                        success: false,
                        needs_redraw: false,
                    };
                }
                match state.output_surface.get_current_texture() {
                    Ok(o) => o,
                    Err(_) => {
                        return WuiAppliedFilterRenderResult {
                            success: false,
                            needs_redraw: false,
                        };
                    }
                }
            }
            Err(wgpu::SurfaceError::Timeout) => {
                // Skip frame but success
                return WuiAppliedFilterRenderResult {
                    success: true,
                    needs_redraw: false,
                };
            }
            Err(e) => {
                tracing::error!("[AppliedFilter] render failed: {e}");
                return WuiAppliedFilterRenderResult {
                    success: false,
                    needs_redraw: false,
                };
            }
        };

        // Get input texture
        let input_texture: &wgpu::Texture = if let Some(ref imported) = state.imported_texture {
            imported
        } else if let Some(ref capture) = state.capture_texture {
            capture
        } else {
            tracing::error!("[AppliedFilter] no input texture available");
            return WuiAppliedFilterRenderResult {
                success: false,
                needs_redraw: false,
            };
        };

        let input_format = if state.imported_texture.is_some() {
            state.imported_format.unwrap_or(state.capture_format)
        } else {
            state.capture_format
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
            format: input_format,
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

        WuiAppliedFilterRenderResult {
            success: true,
            needs_redraw,
        }
    }));

    match render_result {
        Ok(result) => result,
        Err(_) => {
            tracing::error!("[AppliedFilter] render panicked");
            WuiAppliedFilterRenderResult {
                success: false,
                needs_redraw: false,
            }
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
    let sync_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.is_null() {
            return false;
        }

        let state = unsafe { &mut *state };
        if !state.initialized {
            return true;
        }

        state.filter.sync_targets();
        true
    }));

    match sync_result {
        Ok(ok) => ok,
        Err(_) => {
            tracing::error!("[AppliedFilter] sync_targets panicked");
            false
        }
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
    let poll_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.is_null() {
            return false;
        }

        let state = unsafe { &mut *state };
        if !state.initialized {
            return false;
        }

        state.filter.sync_targets();
        state.filter.redraw_hint()
    }));

    match poll_result {
        Ok(should_redraw) => should_redraw,
        Err(_) => {
            tracing::error!("[AppliedFilter] poll_redraw panicked");
            false
        }
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
    if state.is_null() || input_handle.is_null() || width == 0 || height == 0 {
        return false;
    }

    let state = unsafe { &mut *state };

    if !ensure_dimensions(state, width, height) {
        return false;
    }

    match input_type {
        WuiInputType::WgpuTexture => {
            state.imported_texture = None;
            state.imported_format = None;
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
                tracing::error!("[AppliedFilter] MetalTexture not supported on this platform");
                false
            }
        }
        WuiInputType::AHardwareBuffer => {
            #[cfg(target_os = "android")]
            {
                tracing::error!(
                    "[AppliedFilter] AHardwareBuffer import requires a drop callback; use waterui_applied_filter_set_input_ahardwarebuffer"
                );
                false
            }
            #[cfg(not(target_os = "android"))]
            {
                tracing::error!("[AppliedFilter] AHardwareBuffer not supported on this platform");
                false
            }
        }
        WuiInputType::PixelData => {
            let Some(ref capture_texture) = state.capture_texture else {
                return false;
            };

            let bytes_per_row = width * 4;
            let data = unsafe {
                core::slice::from_raw_parts(
                    input_handle as *const u8,
                    (bytes_per_row * height) as usize,
                )
            };

            state.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: capture_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_set_input_ahardwarebuffer(
    state: *mut WuiAppliedFilterState,
    ahb_ptr: *mut c_void,
    drop_fn: WuiExternalDropFn,
    drop_data: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    if state.is_null() || ahb_ptr.is_null() || drop_fn as usize == 0 || width == 0 || height == 0 {
        return false;
    }

    #[cfg(target_os = "android")]
    {
        let state = unsafe { &mut *state };

        if !ensure_dimensions(state, width, height) {
            return false;
        }

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
                tracing::error!("[AppliedFilter] AHardwareBuffer import failed: {e}");
                false
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = state;
        let _ = drop_fn as usize;
        let _ = drop_data;
        tracing::error!("[AppliedFilter] AHardwareBuffer import only supported on Android");
        false
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

    if mtl_texture_ptr.is_null() {
        tracing::error!("[AppliedFilter] MTLTexture pointer is null");
        return false;
    }

    let metal_texture_ref = unsafe { metal::TextureRef::from_ptr(mtl_texture_ptr.cast()) };
    let metal_texture = metal_texture_ref.to_owned();
    let wgpu_format = match metal_texture.pixel_format() {
        metal::MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        metal::MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        metal::MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => {
            tracing::error!(
                "[AppliedFilter] Unsupported Metal texture format {:?}",
                other
            );
            return false;
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
    if state.is_null() {
        return core::ptr::null();
    }

    let state = unsafe { &mut *state };

    if !ensure_dimensions(state, width, height) {
        return core::ptr::null();
    }

    state.imported_texture = None;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        state.imported_metal_texture = None;
    }

    match &state.capture_texture {
        Some(texture) => texture as *const wgpu::Texture as *const c_void,
        None => core::ptr::null(),
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
    if state.is_null() {
        return core::ptr::null_mut();
    }

    let state = unsafe { &*state };
    let Some(texture) = state.capture_texture.as_ref() else {
        return core::ptr::null_mut();
    };

    let Some(hal_texture) = (unsafe { texture.as_hal::<MetalApi>() }) else {
        tracing::error!("[AppliedFilter] capture texture is not a Metal texture");
        return core::ptr::null_mut();
    };

    let raw = unsafe { hal_texture.raw_handle() };
    raw.as_ptr().cast::<c_void>()
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_applied_filter_get_capture_metal_texture(
    _state: *mut WuiAppliedFilterState,
) -> *mut c_void {
    core::ptr::null_mut()
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

    let oom_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal_scope = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let configure_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        surface.configure(device, config);
    }))
    .is_err();

    let validation_err = pollster::block_on(validation_scope.pop());
    let internal_err = pollster::block_on(internal_scope.pop());
    let oom_err = pollster::block_on(oom_scope.pop());

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
