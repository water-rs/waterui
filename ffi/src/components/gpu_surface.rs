//! FFI bindings for the GpuSurface raw view.
//!
//! This module provides the FFI interface for high-performance GPU rendering
//! using wgpu. The native backend is responsible for:
//! 1. Creating a native surface layer (CAMetalLayer on Apple, SurfaceView on Android)
//! 2. Calling `waterui_gpu_surface_init` with the layer pointer
//! 3. Calling `waterui_gpu_surface_render` each frame from a display-sync callback
//! 4. Calling `waterui_gpu_surface_drop` when the view is destroyed

use core::ffi::c_void;

use alloc::boxed::Box;
use alloc::vec;

use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

use crate::IntoFFI;

/// FFI representation of a GpuSurface view.
///
/// This struct is passed to the native backend when rendering the view tree.
/// The native backend should call `waterui_gpu_surface_init` to initialize
/// the GPU resources, then `waterui_gpu_surface_render` each frame.
#[repr(C)]
pub struct WuiGpuSurface {
    /// Opaque pointer to the boxed GpuRenderer trait object.
    /// This is consumed during init and should not be used after.
    pub renderer: *mut c_void,
}

impl IntoFFI for GpuSurface {
    type FFI = WuiGpuSurface;

    fn into_ffi(self) -> Self::FFI {
        // Double-box the renderer to get a thin pointer for FFI.
        // Box<dyn GpuRenderer> is a fat pointer (data + vtable), which can't be
        // passed through C FFI. By boxing it again, we get Box<Box<dyn GpuRenderer>>
        // where Box::into_raw returns a thin *mut Box<dyn GpuRenderer>.
        let boxed_renderer: Box<Box<dyn GpuRenderer>> = Box::new(self.renderer);
        let renderer_ptr = Box::into_raw(boxed_renderer) as *mut c_void;
        WuiGpuSurface {
            renderer: renderer_ptr,
        }
    }
}

// Generate waterui_gpu_surface_id() and waterui_force_as_gpu_surface()
ffi_view!(GpuSurface, WuiGpuSurface, gpu_surface);

/// Opaque state held by the native backend after initialization.
///
/// This struct owns all wgpu resources and the user's renderer.
/// It is created by `waterui_gpu_surface_init` and destroyed by
/// `waterui_gpu_surface_drop`.
pub struct WuiGpuSurfaceState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Box<dyn GpuRenderer>,
    initialized: bool,
    current_width: u32,
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
    if surface.is_null() || layer.is_null() || width == 0 || height == 0 {
        tracing::error!("[GpuSurface] init failed: invalid parameters (surface={:?}, layer={:?}, width={}, height={})",
            surface, layer, width, height);
        return core::ptr::null_mut();
    }

    let wui_surface = unsafe { &mut *surface };

    // Take ownership of the renderer (it's a Box<Box<dyn GpuRenderer>> pointer)
    // The double-boxing in into_ffi allows us to pass a thin pointer through FFI.
    if wui_surface.renderer.is_null() {
        tracing::error!("[GpuSurface] init failed: renderer pointer is null");
        return core::ptr::null_mut();
    }
    let renderer: Box<dyn GpuRenderer> =
        unsafe { *Box::from_raw(wui_surface.renderer as *mut Box<dyn GpuRenderer>) };

    // Null out the pointer to prevent double-free
    wui_surface.renderer = core::ptr::null_mut();

    // Create wgpu instance
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    // Create surface from native layer
    let wgpu_surface = match create_surface_from_layer(&instance, layer) {
        Some(s) => s,
        None => {
            tracing::error!("[GpuSurface] init failed: could not create wgpu surface from native layer");
            return core::ptr::null_mut();
        }
    };

    // Request adapter
    let adapter: wgpu::Adapter =
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&wgpu_surface),
            force_fallback_adapter: false,
        })) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("[GpuSurface] init failed: could not request GPU adapter: {e}");
                return core::ptr::null_mut();
            }
        };

    // Request device and queue with custom error handler to avoid panic on validation errors
    let (device, queue) =
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("WaterUI GpuSurface Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            trace: wgpu::Trace::default(),
        })) {
            Ok((d, q)) => (d, q),
            Err(e) => {
                tracing::error!("[GpuSurface] init failed: could not request GPU device: {e}");
                return core::ptr::null_mut();
            }
        };

    // Set custom error handler to log validation errors via tracing
    device.on_uncaptured_error(alloc::sync::Arc::new(|error: wgpu::Error| {
        tracing::error!("[wgpu] Validation error: {error}");
    }));

    // Get surface capabilities and configure
    // Use Bgra8Unorm for compatibility (must match CAMetalLayer.pixelFormat)
    let surface_caps = wgpu_surface.get_capabilities(&adapter);
    let _ = &surface_caps; // suppress unused warning
    let surface_format = wgpu::TextureFormat::Bgra8Unorm;


    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width,
        height,
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };

    wgpu_surface.configure(&device, &config);

    // Create state
    let state = Box::new(WuiGpuSurfaceState {
        device,
        queue,
        surface: wgpu_surface,
        config,
        renderer,
        initialized: false,
        current_width: width,
        current_height: height,
    });

    Box::into_raw(state)
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
    if state.is_null() || width == 0 || height == 0 {
        return false;
    }

    let state = unsafe { &mut *state };

    // Handle resize if needed
    if width != state.current_width || height != state.current_height {
        state.config.width = width;
        state.config.height = height;
        state.surface.configure(&state.device, &state.config);
        state.current_width = width;
        state.current_height = height;

        // Call user's resize callback
        state.renderer.resize(width, height);
    }

    // Call setup on first render
    if !state.initialized {
        let ctx = GpuContext {
            device: &state.device,
            queue: &state.queue,
            surface_format: state.config.format,
        };
        state.renderer.setup(&ctx);
        state.initialized = true;
    }

    // Get next frame texture
    let output = match state.surface.get_current_texture() {
        Ok(o) => o,
        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
            tracing::warn!("[GpuSurface] surface lost/outdated, reconfiguring");
            // Reconfigure and try again
            state.surface.configure(&state.device, &state.config);
            match state.surface.get_current_texture() {
                Ok(o) => o,
                Err(e) => {
                    tracing::error!("[GpuSurface] render failed: could not get texture after reconfigure: {e}");
                    return false;
                }
            }
        }
        Err(e) => {
            tracing::error!("[GpuSurface] render failed: could not get current texture: {e}");
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
    state.renderer.render(&frame);

    // Present
    output.present();

    true
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
fn create_surface_from_layer(
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
fn create_surface_from_layer(
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
fn create_surface_from_layer(
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
fn create_surface_from_layer(
    _instance: &wgpu::Instance,
    _layer: *mut c_void,
) -> Option<wgpu::Surface<'static>> {
    // Unsupported platform
    None
}
