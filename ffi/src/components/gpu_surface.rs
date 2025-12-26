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

use alloc::boxed::Box;
use alloc::vec;

use alloc::vec::Vec;

use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuSurface};
use waterui_graphics::shared_context::shared_context;

use crate::IntoFFI;

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
}

impl IntoFFI for GpuSurface {
    type FFI = WuiGpuSurface;

    fn into_ffi(self) -> Self::FFI {
        // Box the GpuSurface for FFI transfer.
        let boxed = Box::new(self);
        let ptr = Box::into_raw(boxed) as *mut c_void;
        WuiGpuSurface { surface: ptr }
    }
}

// Generate waterui_gpu_surface_id() and waterui_force_as_gpu_surface()
ffi_view!(GpuSurface, WuiGpuSurface, gpu_surface);

/// Opaque state held by the native backend after initialization.
///
/// Uses shared device/queue from `SharedGpuContext` for efficiency.
/// Only the Surface is created per-view.
pub struct WuiGpuSurfaceState {
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
    /// User's GpuSurface (contains the renderer)
    gpu_surface: GpuSurface,
    /// Whether setup() has been called
    initialized: bool,
    /// Current width from layout
    current_width: u32,
    /// Current height from layout
    current_height: u32,
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
            tracing::warn!("[GpuSurface] Preferred format {:?} not supported, falling back to {:?}", preferred, surface_caps.formats[0]);
            surface_caps.formats[0]
        };

        // Select presentation mode (VSync)
        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
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
        
        tracing::info!("[GpuSurface] Configuring surface: {}x{} {:?} {:?}", width, height, format, present_mode);

        if !try_configure_surface(&wgpu_surface, &device, &config) {
             tracing::error!("[GpuSurface] Surface configuration failed!");
             return core::ptr::null_mut();
        }

        // 4. Create State
        // Store Arc<Device> and Arc<Queue> which are cheap to clone
        let state = Box::new(WuiGpuSurfaceState {
            device,
            queue,
            pipeline_cache,
            wgpu_surface,
            config,
            gpu_surface,
            initialized: false,
            current_width: width,
            current_height: height,
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
        if !state.initialized {
            let ctx = GpuContext {
                device: &state.device,
                queue: &state.queue,
                surface_format: state.config.format,
                pipeline_cache: state.pipeline_cache.as_ref(),
            };
            let setup_future = state.gpu_surface.setup(&ctx);
            pollster::block_on(setup_future);
            state.initialized = true;
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
    if !state.initialized {
        let ctx = GpuContext {
            device: &state.device,
            queue: &state.queue,
            surface_format: state.config.format,
            pipeline_cache: state.pipeline_cache.as_ref(),
        };
        let setup_future = state.gpu_surface.setup(&ctx);
        pollster::block_on(setup_future);
        state.initialized = true;
    }

    // Render first frame
    let output = match state.wgpu_surface.get_current_texture() {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("[GpuSurface] await_ready: failed to get texture: {e}");
            unsafe { callback(user_data) };
            return;
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
