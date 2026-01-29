//! FFI bindings for the GpuSurface raw view.
//!
//! This module provides the FFI interface for high-performance GPU rendering
//! using wgpu. Uses a shared GPU context for efficient multi-view rendering.
//!
//! The native backend is responsible for:
//! 1. Creating a native surface layer (CAMetalLayer on Apple, SurfaceView on Android)
//! 2. Calling `waterui_gpu_surface_init` with the layer pointer
//! 3. Calling `waterui_gpu_surface_render` each frame from a display-sync callback
//! 4. Calling `waterui_gpu_surface_drop` when the view is destroyed

use core::ffi::c_void;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::vec;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    metal::MTLTextureType, metal::foreign_types::ForeignTypeRef, wgpu_hal::Api,
    wgpu_hal::api::Metal as MetalApi,
};

use waterui_graphics::gpu_surface::{
    GestureState, GpuContext, GpuFrame, GpuSurface, GpuSurfaceRenderMode, PointerState,
};
use waterui_graphics::shared_context::shared_context;

use crate::IntoFFI;

fn gpu_capture_poll_timeout() -> Option<Duration> {
    const DEFAULT_MS: u64 = 2_000;
    let ms = std::env::var("WATERUI_GPU_CAPTURE_POLL_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MS);
    if ms == 0 {
        None
    } else {
        Some(Duration::from_millis(ms))
    }
}

fn gpu_await_ready_timeout() -> Duration {
    const DEFAULT_MS: u64 = 500;
    let ms = std::env::var("WATERUI_GPU_AWAIT_READY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MS);
    Duration::from_millis(ms.max(1))
}

fn poll_capture_completion(device: &wgpu::Device) -> bool {
    let poll_type = wgpu::PollType::Wait {
        submission_index: None,
        timeout: gpu_capture_poll_timeout(),
    };

    match device.poll(poll_type) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("[GpuSurface] capture poll timed out/failed: {e}");
            false
        }
    }
}

/// FFI representation of a GpuSurface view.
///
/// This struct is passed to the native backend when rendering the view tree.
/// The native backend should call `waterui_gpu_surface_init` to initialize
/// the GPU resources, then `waterui_gpu_surface_render` each frame.
#[repr(C)]
pub struct WuiGpuSurface {
    /// Opaque pointer to the boxed GpuSurface.
    /// This is consumed during init and should not be used after.
    pub surface: *mut c_void,
    /// Render mode for the surface (see `GpuSurfaceRenderMode`).
    pub render_mode: u32,
}

impl IntoFFI for GpuSurface {
    type FFI = WuiGpuSurface;

    fn into_ffi(self) -> Self::FFI {
        let render_mode = match self.get_render_mode() {
            GpuSurfaceRenderMode::Continuous => 0,
            GpuSurfaceRenderMode::OnDemand => 1,
        };
        // Box the GpuSurface for FFI transfer.
        let boxed = Box::new(self);
        let ptr = Box::into_raw(boxed) as *mut c_void;
        WuiGpuSurface {
            surface: ptr,
            render_mode,
        }
    }
}

// Generate waterui_gpu_surface_id() and waterui_force_as_gpu_surface()
ffi_view!(GpuSurface, WuiGpuSurface, gpu_surface);

/// Opaque state held by the native backend after initialization.
///
/// Uses shared device/queue from `SharedGpuContext` for efficiency.
/// Only the Surface is created per-view.
pub struct WuiGpuSurfaceState {
    /// Adapter used to create the shared device/queue.
    adapter: wgpu::Adapter,
    /// Shared device (Arc reference from SharedGpuContext)
    device: Arc<wgpu::Device>,
    /// Shared queue (Arc reference from SharedGpuContext)
    queue: Arc<wgpu::Queue>,
    /// Optional shared pipeline cache
    pipeline_cache: Option<wgpu::PipelineCache>,
    /// Per-view wgpu surface
    wgpu_surface: wgpu::Surface<'static>,
    /// Surface configuration
    config: wgpu::SurfaceConfiguration,
    /// The format the user's renderer is currently configured for (via `GpuContext.surface_format`).
    ///
    /// This can differ from `config.format` when rendering into external textures.
    renderer_format: wgpu::TextureFormat,
    /// User's GpuSurface (contains the renderer)
    gpu_surface: GpuSurface,
    /// Whether setup() has been called
    initialized: bool,
    /// Current width from layout
    current_width: u32,
    /// Current height from layout
    current_height: u32,
    /// Current pointer/cursor state
    pointer_state: PointerState,
    /// Current gesture state (pinch, pan, double-tap)
    gesture_state: GestureState,
}

/// Initialize a GpuSurface with a native layer.
///
/// This function creates wgpu resources (Instance, Adapter, Device, Queue, Surface)
/// from the provided native layer and calls the user's `setup()` method.
///
/// # Arguments
///
/// * `surface` - Pointer to the WuiGpuSurface FFI struct (consumed)
/// * `layer` - Platform-specific layer pointer:
///   - Apple: `CAMetalLayer*`
///   - Android: `ANativeWindow*`
/// * `width` - Initial surface width in pixels
/// * `height` - Initial surface height in pixels
///
/// # Returns
///
/// Opaque pointer to the initialized state, or null on failure.
///
/// # Safety
///
/// - `surface` must be a valid pointer obtained from `waterui_force_as_gpu_surface`
/// - `layer` must be a valid platform-specific layer pointer
/// - The layer must remain valid for the lifetime of the returned state
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_init(
    surface: *mut WuiGpuSurface,
    layer: *mut c_void,
    width: u32,
    height: u32,
) -> *mut WuiGpuSurfaceState {
    let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if surface.is_null() || layer.is_null() || width == 0 || height == 0 {
            tracing::error!(
                "[GpuSurface] init failed: invalid parameters (surface={:?}, layer={:?}, width={}, height={})",
                surface,
                layer,
                width,
                height
            );
            return core::ptr::null_mut();
        }

        let wui_surface = unsafe { &mut *surface };

        if wui_surface.surface.is_null() {
            tracing::error!("[GpuSurface] init failed: surface pointer is null");
            return core::ptr::null_mut();
        }
        let gpu_surface: GpuSurface =
            unsafe { *Box::from_raw(wui_surface.surface as *mut GpuSurface) };

        // Null out the pointer to prevent double-free
        wui_surface.surface = core::ptr::null_mut();

        // 1. Get/Init Shared Context
        // This ensures we have a valid Instance, Adapter, Device, and Queue
        if !waterui_graphics::shared_context::is_initialized() {
            tracing::info!("[GpuSurface] Shared context not initialized, initializing now...");
            if let Err(e) = waterui_graphics::shared_context::init_shared_context() {
                tracing::error!("[GpuSurface] Init failed: {}", e);
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

        // 2. Create Surface
        // We use the shared instance to create a surface for this specific window/layer.
        // NOTE: The shared instance was created with support for all backends (on desktop)
        // or Vulkan+GLES (on Android), so it should be compatible.
        let Some(wgpu_surface) = create_surface_from_layer(instance, layer) else {
            tracing::error!("[GpuSurface] Failed to create wgpu Surface from layer");
            return core::ptr::null_mut();
        };

        // 3. Configure Surface
        // We need to find a format supported by both the Adapter and Surface.
        let surface_caps = wgpu_surface.get_capabilities(adapter);

        // Validation: Ensure adapter can present to this surface
        if surface_caps.formats.is_empty() {
            tracing::error!("[GpuSurface] Shared adapter cannot present to this surface!");
            // In a perfect world, we might fallback to re-creating the shared context
            // with a different adapter, but that's complex since other views might be using it.
            // For now, this is a fatal error for this view.
            return core::ptr::null_mut();
        }

        let preferred = waterui_graphics::gpu_surface::preferred_surface_format(&surface_caps);

        // Select format (preferring what we calculated, but ensuring it's in caps)
        let format = if surface_caps.formats.contains(&preferred) {
            preferred
        } else {
            tracing::warn!(
                "[GpuSurface] Preferred format {:?} not supported, falling back to {:?}",
                preferred,
                surface_caps.formats[0]
            );
            surface_caps.formats[0]
        };

        // Select presentation mode (VSync)
        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo
        } else {
            surface_caps.present_modes[0]
        };

        // Select alpha mode
        let alpha_mode = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
            wgpu::CompositeAlphaMode::Opaque,
        ]
        .into_iter()
        .find(|mode| surface_caps.alpha_modes.contains(mode))
        .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        tracing::info!(
            "[GpuSurface] Configuring surface: {}x{} {:?} {:?}",
            width,
            height,
            format,
            present_mode
        );

        if !try_configure_surface(&wgpu_surface, &device, &config) {
            tracing::error!("[GpuSurface] Surface configuration failed!");
            return core::ptr::null_mut();
        }

        // 4. Create State
        // Store Arc<Device> and Arc<Queue> which are cheap to clone
        let state = Box::new(WuiGpuSurfaceState {
            adapter: adapter.clone(),
            device,
            queue,
            pipeline_cache,
            wgpu_surface,
            renderer_format: config.format,
            config,
            gpu_surface,
            initialized: false,
            current_width: width,
            current_height: height,
            pointer_state: PointerState::default(),
            gesture_state: GestureState::default(),
        });

        Box::into_raw(state)
    }));

    match init_result {
        Ok(ptr) => ptr,
        Err(_) => {
            tracing::error!("[GpuSurface] init panicked");
            core::ptr::null_mut()
        }
    }
}

/// Render a single frame.
///
/// This function should be called from a display-sync callback (CADisplayLink on Apple,
/// Choreographer on Android) to render at the display's refresh rate.
///
/// # Arguments
///
/// * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
/// * `width` - Current surface width in pixels (from layout)
/// * `height` - Current surface height in pixels (from layout)
///
/// # Returns
///
/// `true` if rendering succeeded, `false` on error.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_render(
    state: *mut WuiGpuSurfaceState,
    width: u32,
    height: u32,
) -> bool {
    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.is_null() || width == 0 || height == 0 {
            return false;
        }

        let state = unsafe { &mut *state };

        // Handle resize if needed
        if width != state.current_width || height != state.current_height {
            // Ensure the queue is idle before reconfiguring the surface.
            let _ = state.device.poll(wgpu::PollType::wait_indefinitely());

            state.config.width = width;
            state.config.height = height;

            if !try_configure_surface(&state.wgpu_surface, &state.device, &state.config) {
                tracing::warn!("[GpuSurface] resize reconfigure failed ({width}x{height})");
                return false;
            }
            state.current_width = width;
            state.current_height = height;

            // Call user's resize callback
            state.gpu_surface.resize(width, height);
        }

        // Call setup on first render (await the future synchronously)
        if !state.initialized || state.renderer_format != state.config.format {
            let format = state.config.format;
            let ctx = GpuContext {
                adapter: Some(&state.adapter),
                device: &state.device,
                queue: &state.queue,
                surface_format: format,
                msaa_samples: waterui_graphics::gpu_surface::preferred_msaa_samples(
                    &state.adapter,
                    format,
                    4,
                ),
                pipeline_cache: state.pipeline_cache.as_ref(),
            };
            let setup_future = state.gpu_surface.setup(&ctx);
            pollster::block_on(setup_future);
            state.initialized = true;
            state.renderer_format = format;
        }

        // Get next frame texture (guard against wgpu panics so we don't abort across the FFI boundary).
        let output = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.wgpu_surface.get_current_texture()
        })) {
            Ok(Ok(o)) => o,
            Ok(Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated)) => {
                tracing::debug!("[GpuSurface] surface lost/outdated, reconfiguring");
                if !try_configure_surface(&state.wgpu_surface, &state.device, &state.config) {
                    tracing::warn!("[GpuSurface] reconfigure failed after surface lost/outdated");
                    return false;
                }
                match state.wgpu_surface.get_current_texture() {
                    Ok(o) => o,
                    Err(wgpu::SurfaceError::Timeout) => {
                        // Surface isn't ready yet (common during window move/resize); skip this frame.
                        return true;
                    }
                    Err(e) => {
                        tracing::error!(
                            "[GpuSurface] render failed: could not get texture after reconfigure: {e}"
                        );
                        return false;
                    }
                }
            }
            Ok(Err(wgpu::SurfaceError::Timeout)) => {
                // Surface isn't ready yet (common during window move/resize); skip this frame.
                return true;
            }
            Ok(Err(e)) => {
                tracing::error!("[GpuSurface] render failed: could not get current texture: {e}");
                return false;
            }
            Err(_) => {
                tracing::error!("[GpuSurface] render panicked while acquiring swapchain texture");
                return false;
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface Frame View"),
            format: Some(state.config.format),
            ..Default::default()
        });

        // Create frame data
        let frame = GpuFrame {
            device: &state.device,
            queue: &state.queue,
            texture: &output.texture,
            view,
            format: state.config.format,
            width,
            height,
            pointer: state.pointer_state,
            gesture: state.gesture_state,
        };

        // Call user's render callback
        state.gpu_surface.render(&frame);

        // Present
        output.present();

        true
    }));

    match render_result {
        Ok(ok) => ok,
        Err(_) => {
            tracing::error!("[GpuSurface] render panicked");
            false
        }
    }
}

/// Render a single frame into an external texture.
///
/// This is used for GPU-based view captures (e.g., filter pipelines) so a
/// GpuSurface can render directly into a provided texture.
///
/// # Arguments
///
/// * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
/// * `texture` - Pointer to a `wgpu::Texture` to render into
/// * `width` - Target width in pixels
/// * `height` - Target height in pixels
///
/// # Returns
///
/// `true` if rendering succeeded, `false` on error.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`.
/// `texture` must be a valid pointer to a `wgpu::Texture` with RENDER_ATTACHMENT usage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_render_to_texture(
    state: *mut WuiGpuSurfaceState,
    texture: *mut core::ffi::c_void,
    width: u32,
    height: u32,
) -> bool {
    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.is_null() || texture.is_null() || width == 0 || height == 0 {
            return false;
        }

        let state = unsafe { &mut *state };
        let texture = unsafe { &*(texture as *const wgpu::Texture) };

        if !texture
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            tracing::error!(
                "[GpuSurface] render_to_texture: texture missing RENDER_ATTACHMENT usage"
            );
            return false;
        }

        let target_format = texture.format();

        if width != state.current_width || height != state.current_height {
            state.current_width = width;
            state.current_height = height;
            state.config.width = width;
            state.config.height = height;
            state.gpu_surface.resize(width, height);
        }

        if !state.initialized || state.renderer_format != target_format {
            let ctx = GpuContext {
                adapter: Some(&state.adapter),
                device: &state.device,
                queue: &state.queue,
                surface_format: target_format,
                msaa_samples: waterui_graphics::gpu_surface::preferred_msaa_samples(
                    &state.adapter,
                    target_format,
                    4,
                ),
                pipeline_cache: state.pipeline_cache.as_ref(),
            };
            let setup_future = state.gpu_surface.setup(&ctx);
            pollster::block_on(setup_future);
            state.initialized = true;
            state.renderer_format = target_format;
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface External Frame View"),
            format: Some(target_format),
            ..Default::default()
        });

        let frame = GpuFrame {
            device: &state.device,
            queue: &state.queue,
            texture,
            view,
            format: target_format,
            width,
            height,
            pointer: state.pointer_state,
            gesture: state.gesture_state,
        };

        state.gpu_surface.render(&frame);
        // Ensure external renderers see completed writes before returning.
        poll_capture_completion(&state.device)
    }));

    match render_result {
        Ok(ok) => ok,
        Err(_) => {
            tracing::error!("[GpuSurface] render_to_texture panicked");
            false
        }
    }
}

/// Render a single frame into an external Metal texture (Apple only).
///
/// # Safety
/// `state` must be valid, `texture` must point to a `MTLTexture`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_render_to_metal_texture(
    state: *mut WuiGpuSurfaceState,
    texture: *mut core::ffi::c_void,
    width: u32,
    height: u32,
) -> bool {
    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.is_null() || texture.is_null() || width == 0 || height == 0 {
            return false;
        }

        let state = unsafe { &mut *state };
        let metal_texture_ref = unsafe { metal::TextureRef::from_ptr(texture.cast()) };
        let metal_texture = metal_texture_ref.to_owned();

        let target_format = match metal_texture.pixel_format() {
            metal::MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
            metal::MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
            metal::MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
            other => {
                tracing::error!(
                    "[GpuSurface] render_to_metal_texture: unsupported format {:?}",
                    other
                );
                return false;
            }
        };

        let hal_texture = unsafe {
            <MetalApi as Api>::Device::texture_from_raw(
                metal_texture.clone(),
                target_format,
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
            label: Some("GpuSurface Imported Metal Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };

        let wgpu_texture = unsafe {
            state
                .device
                .create_texture_from_hal::<MetalApi>(hal_texture, &texture_desc)
        };

        if width != state.current_width || height != state.current_height {
            state.current_width = width;
            state.current_height = height;
            state.config.width = width;
            state.config.height = height;
            state.gpu_surface.resize(width, height);
        }

        if !state.initialized || state.renderer_format != target_format {
            let ctx = GpuContext {
                adapter: Some(&state.adapter),
                device: &state.device,
                queue: &state.queue,
                surface_format: target_format,
                msaa_samples: waterui_graphics::gpu_surface::preferred_msaa_samples(
                    &state.adapter,
                    target_format,
                    4,
                ),
                pipeline_cache: state.pipeline_cache.as_ref(),
            };
            let setup_future = state.gpu_surface.setup(&ctx);
            pollster::block_on(setup_future);
            state.initialized = true;
            state.renderer_format = target_format;
        }

        let view = wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface Metal Frame View"),
            format: Some(target_format),
            ..Default::default()
        });

        let frame = GpuFrame {
            device: &state.device,
            queue: &state.queue,
            texture: &wgpu_texture,
            view,
            format: target_format,
            width,
            height,
            pointer: state.pointer_state,
            gesture: state.gesture_state,
        };

        state.gpu_surface.render(&frame);
        poll_capture_completion(&state.device)
    }));

    match render_result {
        Ok(ok) => ok,
        Err(_) => {
            tracing::error!("[GpuSurface] render_to_metal_texture panicked");
            false
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_render_to_metal_texture(
    _state: *mut WuiGpuSurfaceState,
    _texture: *mut core::ffi::c_void,
    _width: u32,
    _height: u32,
) -> bool {
    false
}

/// Callback type for async completion notifications.
pub type WuiGpuCallback = unsafe extern "C" fn(user_data: *mut c_void);

/// Setup the GpuSurface and render the first frame, then call callback.
///
/// This function performs async setup (awaited synchronously via `block_on`),
/// then renders the first frame. Native code should call this before showing
/// the window to ensure all GpuSurfaces are ready.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state from `waterui_gpu_surface_init`
/// * `callback` - Function to call when ready
/// * `user_data` - Opaque pointer passed to callback
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_gpu_surface_init`
/// - `callback` must be a valid function pointer
/// - `user_data` must remain valid until callback is invoked
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_await_ready(
    state: *mut WuiGpuSurfaceState,
    callback: WuiGpuCallback,
    user_data: *mut c_void,
) {
    if state.is_null() {
        tracing::error!("[GpuSurface] await_ready: null state");
        unsafe { callback(user_data) };
        return;
    }

    let state = unsafe { &mut *state };

    // Call setup if not already done
    if !state.initialized || state.renderer_format != state.config.format {
        let format = state.config.format;
        let ctx = GpuContext {
            adapter: Some(&state.adapter),
            device: &state.device,
            queue: &state.queue,
            surface_format: format,
            msaa_samples: waterui_graphics::gpu_surface::preferred_msaa_samples(
                &state.adapter,
                format,
                4,
            ),
            pipeline_cache: state.pipeline_cache.as_ref(),
        };
        let setup_future = state.gpu_surface.setup(&ctx);
        pollster::block_on(setup_future);
        state.initialized = true;
        state.renderer_format = format;
    }

    // Render first frame.
    //
    // On macOS, CAMetalLayer may not produce a drawable immediately after becoming visible,
    // and wgpu can report `Timeout`. Since this function is explicitly used to prevent
    // "pop-in", retry briefly before giving up.
    let deadline = Instant::now() + gpu_await_ready_timeout();
    let output = loop {
        match state.wgpu_surface.get_current_texture() {
            Ok(o) => break o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                tracing::debug!("[GpuSurface] await_ready: surface lost/outdated, reconfiguring");
                if !try_configure_surface(&state.wgpu_surface, &state.device, &state.config) {
                    tracing::warn!("[GpuSurface] await_ready: reconfigure failed");
                    unsafe { callback(user_data) };
                    return;
                }
                continue;
            }
            Err(wgpu::SurfaceError::Timeout) => {
                if Instant::now() >= deadline {
                    tracing::warn!("[GpuSurface] await_ready: timed out waiting for drawable");
                    unsafe { callback(user_data) };
                    return;
                }
                // Small sleep to avoid busy-looping; we're on a backend render thread.
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(e) => {
                tracing::warn!("[GpuSurface] await_ready: failed to get texture: {e}");
                unsafe { callback(user_data) };
                return;
            }
        }
    };

    let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("GpuSurface Ready Frame"),
        format: Some(state.config.format),
        ..Default::default()
    });

    let frame = GpuFrame {
        device: &state.device,
        queue: &state.queue,
        texture: &output.texture,
        view,
        format: state.config.format,
        width: state.current_width,
        height: state.current_height,
        pointer: state.pointer_state,
        gesture: state.gesture_state,
    };

    state.gpu_surface.render(&frame);
    output.present();

    // Call completion callback
    unsafe { callback(user_data) };
}

/// Clean up GPU resources.
///
/// This function should be called when the GpuSurface view is destroyed.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_drop(state: *mut WuiGpuSurfaceState) {
    if !state.is_null() {
        unsafe {
            let _ = Box::from_raw(state);
        }
    }
}

/// FFI-safe pointer state for passing from native.
///
/// Native backends should update this before each render call to provide
/// current pointer/cursor information to the GPU renderer.
#[repr(C)]
pub struct WuiPointerState {
    /// Whether the pointer is currently over this surface.
    pub has_position: bool,
    /// X coordinate in surface-local pixels.
    pub x: f32,
    /// Y coordinate in surface-local pixels.
    pub y: f32,
    /// Whether there's an active hit (press/touch in progress).
    pub has_hit: bool,
    /// X coordinate where hit started.
    pub hit_x: f32,
    /// Y coordinate where hit started.
    pub hit_y: f32,
}

/// Update the pointer/cursor state for a GpuSurface.
///
/// Native backends should call this before each render to update pointer state.
/// This enables GPU renderers to implement hover effects, hit detection, and
/// interactive feedback.
///
/// # Arguments
///
/// * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
/// * `pointer` - Current pointer state
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_set_pointer(
    state: *mut WuiGpuSurfaceState,
    pointer: WuiPointerState,
) {
    if state.is_null() {
        return;
    }

    let state = unsafe { &mut *state };

    state.pointer_state = PointerState {
        position: if pointer.has_position {
            Some(waterui_core::layout::Point::new(pointer.x, pointer.y))
        } else {
            None
        },
        hit: if pointer.has_hit {
            Some(waterui_core::layout::Point::new(
                pointer.hit_x,
                pointer.hit_y,
            ))
        } else {
            None
        },
    };
}

/// FFI-safe gesture state for zoom/pan interactions.
///
/// Native backends should update this when pinch, pan, or double-tap
/// gestures are detected to enable interactive chart zoom/pan.
#[repr(C)]
pub struct WuiGestureState {
    /// Whether a gesture is currently active.
    pub active: bool,
    /// Cumulative pinch scale factor (1.0 = no scaling).
    pub pinch_scale: f32,
    /// Whether a pinch center is present.
    pub has_pinch_center: bool,
    /// X coordinate of pinch center in surface-local pixels.
    pub pinch_center_x: f32,
    /// Y coordinate of pinch center in surface-local pixels.
    pub pinch_center_y: f32,
    /// Pan offset X in pixels since gesture began.
    pub pan_offset_x: f32,
    /// Pan offset Y in pixels since gesture began.
    pub pan_offset_y: f32,
    /// Whether a double-tap was detected this frame.
    pub double_tap: bool,
}

/// Update the gesture state for a GpuSurface.
///
/// Native backends should call this when pinch/pan/double-tap gestures are
/// detected. This enables GPU renderers (like charts) to implement zoom/pan
/// interactions.
///
/// # Arguments
///
/// * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
/// * `gesture` - Current gesture state
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_set_gesture(
    state: *mut WuiGpuSurfaceState,
    gesture: WuiGestureState,
) {
    if state.is_null() {
        return;
    }

    let state = unsafe { &mut *state };

    state.gesture_state = GestureState {
        pinch_scale: gesture.pinch_scale,
        pinch_center: if gesture.has_pinch_center {
            Some(waterui_core::layout::Point::new(
                gesture.pinch_center_x,
                gesture.pinch_center_y,
            ))
        } else {
            None
        },
        pan_offset: waterui_core::layout::Point::new(gesture.pan_offset_x, gesture.pan_offset_y),
        double_tap: gesture.double_tap,
        active: gesture.active,
    };
}

/// Create a wgpu Surface from a platform-specific layer pointer.
#[cfg(target_os = "macos")]
pub(crate) fn create_surface_from_layer(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> Option<wgpu::Surface<'static>> {
    // On macOS, layer is a CAMetalLayer*. Use the CoreAnimationLayer target
    // so wgpu treats the pointer as a CA layer rather than an NSView.
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
            .ok()
    }
}

#[cfg(target_os = "ios")]
pub(crate) fn create_surface_from_layer(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> Option<wgpu::Surface<'static>> {
    // On iOS, layer is also a CAMetalLayer*; use CoreAnimationLayer here too.
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
            .ok()
    }
}

#[cfg(target_os = "android")]
pub(crate) fn create_surface_from_layer(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> Option<wgpu::Surface<'static>> {
    use raw_window_handle::{AndroidNdkWindowHandle, RawWindowHandle};
    use std::ptr::NonNull;

    // On Android, layer is an ANativeWindow*
    let window_ptr = NonNull::new(layer)?;
    let handle = AndroidNdkWindowHandle::new(window_ptr);

    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: raw_window_handle::RawDisplayHandle::Android(
                    raw_window_handle::AndroidDisplayHandle::new(),
                ),
                raw_window_handle: RawWindowHandle::AndroidNdk(handle),
            })
            .ok()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
pub(crate) fn create_surface_from_layer(
    _instance: &wgpu::Instance,
    _layer: *mut c_void,
) -> Option<wgpu::Surface<'static>> {
    // Unsupported platform
    None
}

fn try_configure_surface(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> bool {
    // Keep the device/queue idle before attempting to (re)configure.
    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    // `Surface::configure` doesn't return a `Result`, so use error scopes to detect failures.
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
        tracing::warn!("[GpuSurface] Surface::configure panicked");
        return false;
    }

    if let Some(err) = validation_err.or(internal_err).or(oom_err) {
        tracing::warn!("[GpuSurface] Surface::configure failed: {err}");
        return false;
    }

    true
}
