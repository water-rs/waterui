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
use alloc::vec::Vec;

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

        // On Android, a Surface can only be connected to one GPU API at a time. When a wgpu
        // `Surface` is created with multiple backends enabled, wgpu-core creates per-backend
        // surfaces internally (e.g. Vulkan + GLES), which can cause the underlying
        // `ANativeWindow` to become "already connected" and make subsequent configuration fail.
        //
        // To avoid this, try one backend at a time on Android.
        let backend_attempts: Vec<wgpu::Backends> = if cfg!(target_os = "android") {
            vec![wgpu::Backends::VULKAN, wgpu::Backends::GL]
        } else {
            vec![wgpu::Backends::all()]
        };

        for backends in backend_attempts {
            tracing::info!("[GpuSurface] init: trying backends {backends:?}");

            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends,
                ..Default::default()
            });

            // Create surface from native layer
            let Some(wgpu_surface) = create_surface_from_layer(&instance, layer) else {
                tracing::warn!(
                    "[GpuSurface] init failed: could not create wgpu surface from native layer"
                );
                continue;
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
                        tracing::warn!(
                            "[GpuSurface] init: could not request GPU adapter for {backends:?}: {e}"
                        );
                        continue;
                    }
                };

            // Pick limits that are compatible with the selected adapter.
            //
            // On downlevel backends (notably GLES 3.0 / WebGL2-class), compute limits can be 0 and
            // requesting WebGPU-default limits will fail the device request.
            let adapter_limits = adapter.limits();
            let downlevel_caps = adapter.get_downlevel_capabilities();
            let required_limits = if downlevel_caps.is_webgpu_compliant() {
                wgpu::Limits::default()
            } else if downlevel_caps
                .flags
                .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
            {
                wgpu::Limits::downlevel_defaults()
            } else {
                wgpu::Limits::downlevel_webgl2_defaults()
            }
            .using_resolution(adapter_limits.clone())
            .using_alignment(adapter_limits);

            // Request device and queue with custom error handler to avoid panic on validation errors
            let (device, queue) =
                match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("WaterUI GpuSurface Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: wgpu::MemoryHints::Performance,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                    trace: wgpu::Trace::default(),
                })) {
                    Ok((d, q)) => (d, q),
                    Err(e) => {
                        tracing::warn!(
                            "[GpuSurface] init: could not request GPU device for {backends:?}: {e}"
                        );
                        continue;
                    }
                };

            // Set custom error handler to log validation errors via tracing
            device.on_uncaptured_error(alloc::sync::Arc::new(|error: wgpu::Error| {
                tracing::error!("[wgpu] Validation error: {error}");
            }));

            // Ensure the queue is idle before configuring the surface.
            // This avoids wgpu-core rejecting `Surface::configure` when there are in-flight submissions.
            let _ = device.poll(wgpu::PollType::wait_indefinitely());

            let adapter_info = adapter.get_info();
            tracing::info!("[GpuSurface] adapter: {adapter_info:?}");

            // Android emulator Vulkan is commonly backed by SwiftShader (CPU). We've observed
            // swapchain configuration/acquire crashing (SIGSEGV) for some formats on this stack.
            // Prefer falling back to the GL backend in this case.
            if cfg!(target_os = "android")
                && backends == wgpu::Backends::VULKAN
                && adapter_info.device_type == wgpu::DeviceType::Cpu
            {
                tracing::warn!(
                    "[GpuSurface] init: Vulkan adapter is CPU ({:?}); falling back to GL",
                    adapter_info.name
                );
                continue;
            }

            // Get surface capabilities and configure. Some backends (notably Android/GLES) may report
            // formats that are not actually configurable on a given device/driver, so we probe with
            // fallbacks.
            let surface_caps = wgpu_surface.get_capabilities(&adapter);
            tracing::info!(
                "[GpuSurface] surface caps: formats={:?}, present_modes={:?}, alpha_modes={:?}, usages={:?}",
                surface_caps.formats,
                surface_caps.present_modes,
                surface_caps.alpha_modes,
                surface_caps.usages
            );

            if surface_caps.formats.is_empty() {
                tracing::warn!("[GpuSurface] init: surface reported no supported formats");
                continue;
            }

            // Prefer FIFO (vsync) for broad compatibility. Some Android drivers/emulators
            // advertise Mailbox but behave poorly with it.
            let present_mode = if surface_caps
                .present_modes
                .contains(&wgpu::PresentMode::Fifo)
            {
                wgpu::PresentMode::Fifo
            } else {
                surface_caps
                    .present_modes
                    .first()
                    .copied()
                    .unwrap_or(wgpu::PresentMode::Fifo)
            };
            let alpha_mode = surface_caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

            let mut formats_to_try = Vec::<wgpu::TextureFormat>::new();
            let preferred = waterui_graphics::gpu_surface::preferred_surface_format(&surface_caps);
            for fmt in [
                preferred,
                preferred.remove_srgb_suffix(),
                preferred.add_srgb_suffix(),
            ] {
                if surface_caps.formats.contains(&fmt) && !formats_to_try.contains(&fmt) {
                    formats_to_try.push(fmt);
                }
            }
            for fmt in &surface_caps.formats {
                for candidate in [*fmt, fmt.remove_srgb_suffix(), fmt.add_srgb_suffix()] {
                    if surface_caps.formats.contains(&candidate)
                        && !formats_to_try.contains(&candidate)
                    {
                        formats_to_try.push(candidate);
                    }
                }
            }

            let mut selected_config: Option<wgpu::SurfaceConfiguration> = None;
            for format in formats_to_try {
                tracing::info!("[GpuSurface] trying surface format {format:?}");
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

                if try_configure_surface(&wgpu_surface, &device, &config) {
                    tracing::info!("[GpuSurface] configured surface with format {format:?}");
                    selected_config = Some(config);
                    break;
                }

                tracing::warn!("[GpuSurface] surface configure probe failed for format {format:?}");
            }

            let Some(config) = selected_config else {
                tracing::warn!(
                    "[GpuSurface] init: could not configure surface for presentation using {backends:?}"
                );
                continue;
            };

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

            return Box::into_raw(state);
        }

        tracing::error!("[GpuSurface] init failed: no compatible backend could configure the surface");
        core::ptr::null_mut()
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

            if !try_configure_surface(&state.surface, &state.device, &state.config) {
                tracing::warn!("[GpuSurface] resize reconfigure failed ({width}x{height})");
                return false;
            }
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

        // Get next frame texture (guard against wgpu panics so we don't abort across the FFI boundary).
        let output = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.surface.get_current_texture()
        })) {
            Ok(Ok(o)) => o,
            Ok(Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated)) => {
                tracing::debug!("[GpuSurface] surface lost/outdated, reconfiguring");
                if !try_configure_surface(&state.surface, &state.device, &state.config) {
                    tracing::warn!("[GpuSurface] reconfigure failed after surface lost/outdated");
                    return false;
                }
                match state.surface.get_current_texture() {
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
        state.renderer.render(&frame);

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
