//! FFI bindings for the GpuSurface raw view.
//!
//! This module provides the FFI interface for high-performance GPU rendering
//! using wgpu. Uses a shared GPU context for efficient multi-view rendering.
//!
//! The native backend is responsible for:
//! 1. Creating a native surface layer (CAMetalLayer on Apple, SurfaceView on Android)
//! 2. Calling `waterui_gpu_surface_init` with the layer pointer
//! 3. Calling `waterui_gpu_surface_render` whenever the surface is dirty
//! 4. Calling `waterui_gpu_surface_drop` when the view is destroyed

use core::ffi::c_void;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::vec;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    metal::MTLTextureType,
    metal::foreign_types::{ForeignType, ForeignTypeRef},
    wgpu_hal::Api,
    wgpu_hal::api::Metal as MetalApi,
};

use waterui_graphics::gpu_surface::{
    GestureState, GpuContext, GpuFrame, GpuSurface, KeyCode, KeyEvent, KeyModifiers, NamedKey,
    PointerState, RedrawHandle,
};
use waterui_graphics::shared_context::shared_context;

use crate::IntoFFI;

#[cold]
#[inline(never)]
fn abort_on_panic(scope: &'static str) -> ! {
    tracing::error!("{scope} panicked; aborting process");
    std::process::abort();
}

struct GpuSurfaceDiagnostics {
    enabled: bool,
    interval_ms: u64,
    start: Instant,
    next_surface_id: AtomicU64,
    active_surfaces: AtomicU64,
    total_inits: AtomicU64,
    total_drops: AtomicU64,
    total_renders: AtomicU64,
    total_redraw_requests: AtomicU64,
    total_failures: AtomicU64,
    last_report_ms: AtomicU64,
    last_report_renders: AtomicU64,
}

impl GpuSurfaceDiagnostics {
    fn disabled() -> Self {
        Self {
            enabled: false,
            interval_ms: 1_000,
            start: Instant::now(),
            next_surface_id: AtomicU64::new(1),
            active_surfaces: AtomicU64::new(0),
            total_inits: AtomicU64::new(0),
            total_drops: AtomicU64::new(0),
            total_renders: AtomicU64::new(0),
            total_redraw_requests: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            last_report_ms: AtomicU64::new(0),
            last_report_renders: AtomicU64::new(0),
        }
    }

    fn new_from_env() -> Self {
        let enabled = std::env::var("WATERUI_GPU_SURFACE_DIAG")
            .ok()
            .is_some_and(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "yes" | "on")
            });
        if !enabled {
            return Self::disabled();
        }

        let interval_ms = std::env::var("WATERUI_GPU_SURFACE_DIAG_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|ms| ms.max(100))
            .unwrap_or(1_000);

        Self {
            enabled,
            interval_ms,
            start: Instant::now(),
            next_surface_id: AtomicU64::new(1),
            active_surfaces: AtomicU64::new(0),
            total_inits: AtomicU64::new(0),
            total_drops: AtomicU64::new(0),
            total_renders: AtomicU64::new(0),
            total_redraw_requests: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            last_report_ms: AtomicU64::new(0),
            last_report_renders: AtomicU64::new(0),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn on_surface_init(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        present_mode: wgpu::PresentMode,
    ) -> u64 {
        let active = self.active_surfaces.fetch_add(1, Ordering::Relaxed) + 1;
        if !self.enabled {
            return 0;
        }

        let id = self.next_surface_id.fetch_add(1, Ordering::Relaxed);
        let total = self.total_inits.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::debug!(
            "[GpuSurfaceDiag] surface#{id} init {}x{} format={:?} present={:?} active={} total_inits={}",
            width,
            height,
            format,
            present_mode,
            active,
            total
        );
        id
    }

    fn on_surface_drop(&self, id: u64, renders: u64) -> u64 {
        let active = self
            .active_surfaces
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        if !self.enabled {
            return active;
        }
        let dropped = self.total_drops.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::debug!(
            "[GpuSurfaceDiag] surface#{id} drop renders={} active={} total_drops={}",
            renders,
            active,
            dropped
        );
        active
    }

    fn on_render_result(&self, ok: bool, needs_redraw: bool) {
        if !self.enabled {
            return;
        }
        self.total_renders.fetch_add(1, Ordering::Relaxed);
        if !ok {
            self.total_failures.fetch_add(1, Ordering::Relaxed);
        }
        if needs_redraw {
            self.total_redraw_requests.fetch_add(1, Ordering::Relaxed);
        }
        self.maybe_emit_report();
    }

    fn maybe_emit_report(&self) {
        if !self.enabled {
            return;
        }

        let now_ms = self.elapsed_ms();
        let last_ms = self.last_report_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) < self.interval_ms {
            return;
        }

        if self
            .last_report_ms
            .compare_exchange(last_ms, now_ms, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        let total_renders = self.total_renders.load(Ordering::Relaxed);
        let prev_renders = self
            .last_report_renders
            .swap(total_renders, Ordering::AcqRel);
        let delta = total_renders.saturating_sub(prev_renders);
        let elapsed = now_ms.saturating_sub(last_ms).max(1);
        let fps = delta as f64 * 1_000.0 / elapsed as f64;

        tracing::debug!(
            "[GpuSurfaceDiag] report active={} renders_total={} renders_delta={} approx_fps={:.1} redraw_total={} failures_total={}",
            self.active_surfaces.load(Ordering::Relaxed),
            total_renders,
            delta,
            fps,
            self.total_redraw_requests.load(Ordering::Relaxed),
            self.total_failures.load(Ordering::Relaxed)
        );
    }
}

fn gpu_surface_diagnostics() -> &'static GpuSurfaceDiagnostics {
    static DIAGNOSTICS: OnceLock<GpuSurfaceDiagnostics> = OnceLock::new();
    DIAGNOSTICS.get_or_init(GpuSurfaceDiagnostics::new_from_env)
}

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

fn poll_capture_completion(device: &wgpu::Device) -> bool {
    let poll_type = wgpu::PollType::Wait {
        submission_index: None,
        timeout: gpu_capture_poll_timeout(),
    };

    match device.poll(poll_type) {
        Ok(_) => true,
        Err(e) => {
            panic!("gpu_surface::poll_capture_completion failed: {e}");
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn gpu_capture_trace_path() -> std::path::PathBuf {
    std::env::temp_dir().join("waterui_gpu_capture_trace.log")
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn trace_metal_capture_step(step: &str) {
    use std::io::Write as _;

    let enabled = std::env::var("WATERUI_GPU_CAPTURE_TRACE")
        .ok()
        .is_some_and(|v| {
            let value = v.trim().to_ascii_lowercase();
            matches!(value.as_str(), "1" | "true" | "yes" | "on")
        });
    if !enabled {
        return;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(gpu_capture_trace_path())
    {
        let _ = writeln!(file, "{now} {step}");
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
    /// Whether this surface should register as a picture-in-picture host.
    pub has_picture_in_picture_host_id: bool,
    /// Stable picture-in-picture host id when `has_picture_in_picture_host_id` is true.
    pub picture_in_picture_host_id: u64,
}

impl IntoFFI for GpuSurface {
    type FFI = WuiGpuSurface;

    fn into_ffi(self) -> Self::FFI {
        // Box the GpuSurface for FFI transfer.
        let boxed = Box::new(self);
        let picture_in_picture_host_id = boxed.get_picture_in_picture_host_id();
        let ptr = Box::into_raw(boxed) as *mut c_void;
        WuiGpuSurface {
            surface: ptr,
            has_picture_in_picture_host_id: picture_in_picture_host_id.is_some(),
            picture_in_picture_host_id: picture_in_picture_host_id.unwrap_or_default(),
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
    /// Keyboard events for the current frame (replaced each `set_keys` call).
    keyboard_events: Vec<KeyEvent>,
    /// Animation clock start for frame timing.
    start_time: Instant,
    /// Timestamp of the previous render.
    last_frame_time: Instant,
    /// Number of render invocations handled by this state.
    render_invocations: u64,
    /// Stable id for diagnostics output.
    diagnostic_id: u64,
    /// Environment pointer for GpuView::setup().
    env: *mut crate::WuiEnv,
    /// Redraw handle for external redraw triggers.
    redraw_handle: RedrawHandle,
}

fn advance_frame_timing(state: &mut WuiGpuSurfaceState) -> (Duration, Duration) {
    let now = Instant::now();
    let elapsed = now.duration_since(state.start_time);
    let delta = now
        .duration_since(state.last_frame_time)
        .min(Duration::from_millis(100));
    state.last_frame_time = now;
    (elapsed, delta)
}

/// Result returned by a GpuSurface render invocation.
#[repr(C)]
pub struct WuiGpuSurfaceRenderResult {
    /// Whether rendering succeeded.
    pub ok: bool,
    /// Whether another frame should be scheduled immediately.
    pub needs_redraw: bool,
}

/// Renderer-driven HDR preference exported to native backends before init.
///
/// `has_preference = false` means the surface should follow backend/global policy.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WuiGpuSurfaceHdrPreference {
    /// Whether the renderer provided an explicit HDR/SDR preference.
    pub has_preference: bool,
    /// Explicit preferred dynamic range when `has_preference` is true.
    pub prefers_hdr: bool,
    /// Final dynamic range preference resolved by Rust (`GpuSurface` + env policy).
    pub resolved_prefers_hdr: bool,
}

/// Returns the renderer-driven HDR preference for a `WuiGpuSurface`.
///
/// This must be called before `waterui_gpu_surface_init` consumes the surface.
///
/// # Safety
///
/// - `surface` must be a valid pointer obtained from `waterui_force_as_gpu_surface`
/// - `surface` must not have been consumed by `waterui_gpu_surface_init`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_hdr_preference(
    surface: *const WuiGpuSurface,
) -> WuiGpuSurfaceHdrPreference {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        unsafe {
            crate::expect_non_null(surface, "waterui_gpu_surface_hdr_preference", "surface");
        }
        let wui_surface = unsafe { &*surface };
        let gpu_surface = unsafe { &*(wui_surface.surface as *const GpuSurface) };
        let explicit = gpu_surface.get_surface_prefers_hdr();
        let resolved = waterui_graphics::gpu_surface::resolve_surface_hdr_preference(explicit);
        WuiGpuSurfaceHdrPreference {
            has_preference: explicit.is_some(),
            prefers_hdr: explicit.unwrap_or(false),
            resolved_prefers_hdr: resolved,
        }
    }));

    match result {
        Ok(value) => value,
        Err(_) => abort_on_panic("waterui_gpu_surface_hdr_preference"),
    }
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
    env: *mut crate::WuiEnv,
) -> *mut WuiGpuSurfaceState {
    unsafe {
        crate::expect_non_null_mut(surface, "waterui_gpu_surface_init", "surface");
        crate::expect_non_null(layer, "waterui_gpu_surface_init", "layer");
        crate::expect_non_null_mut(env, "waterui_gpu_surface_init", "env");
    }

    let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let wui_surface = unsafe { &mut *surface };

        let gpu_surface: GpuSurface =
            unsafe { *Box::from_raw(wui_surface.surface as *mut GpuSurface) };

        // Null out the pointer to prevent double-free
        wui_surface.surface = core::ptr::null_mut();

        // 1. Get/Init Shared Context
        // This ensures we have a valid Instance, Adapter, Device, and Queue
        if !waterui_graphics::shared_context::is_initialized() {
            tracing::debug!("[GpuSurface] Shared context not initialized, initializing now...");
            waterui_graphics::shared_context::init_shared_context()
                .expect("waterui_gpu_surface_init: shared GPU context init failed");
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
            panic!("waterui_gpu_surface_init: failed to create wgpu surface from layer");
        };

        // 3. Configure Surface
        // We need to find a format supported by both the Adapter and Surface.
        let surface_caps = wgpu_surface.get_capabilities(adapter);

        // Validation: Ensure adapter can present to this surface
        assert!(
            !(surface_caps.formats.is_empty()),
            "waterui_gpu_surface_init: adapter cannot present to this surface"
        );

        let preferred = waterui_graphics::gpu_surface::preferred_surface_format_with_preference(
            &surface_caps,
            gpu_surface.get_surface_prefers_hdr(),
        );

        // Select format (preferring what we calculated, but ensuring it's in caps)
        let format = if surface_caps.formats.contains(&preferred) {
            preferred
        } else {
            panic!(
                "waterui_gpu_surface_init: preferred format {:?} is not supported by surface capabilities {:?}",
                preferred, surface_caps.formats
            );
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

        tracing::debug!(
            "[GpuSurface] Configuring surface: {}x{} {:?} {:?}",
            width,
            height,
            format,
            present_mode
        );

        try_configure_surface(&wgpu_surface, &device, &config);

        // 4. Create State
        // Store Arc<Device> and Arc<Queue> which are cheap to clone
        let now = Instant::now();
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
            keyboard_events: Vec::new(),
            start_time: now,
            last_frame_time: now - Duration::from_secs_f32(1.0 / 60.0),
            render_invocations: 0,
            diagnostic_id: gpu_surface_diagnostics().on_surface_init(
                width,
                height,
                format,
                present_mode,
            ),
            env,
            redraw_handle: RedrawHandle::new(),
        });

        Box::into_raw(state)
    }));

    match init_result {
        Ok(ptr) => ptr,
        Err(_) => abort_on_panic("waterui_gpu_surface_init"),
    }
}

/// Render a single frame.
///
/// This function should be called when the surface is dirty (size/input/state changed)
/// and backend should schedule another frame when `needs_redraw` is true.
///
/// # Arguments
///
/// * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
/// * `width` - Current surface width in pixels (from layout)
/// * `height` - Current surface height in pixels (from layout)
///
/// # Returns
///
/// Render result containing success + redraw intent.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_render(
    state: *mut WuiGpuSurfaceState,
    width: u32,
    height: u32,
) -> WuiGpuSurfaceRenderResult {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_gpu_surface_render", "state");
    }

    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };

        state.render_invocations = state.render_invocations.saturating_add(1);
        let frame_no = state.render_invocations;
        let trace_frame = frame_no <= 3;
        if trace_frame {
            tracing::debug!(
                "[GpuSurface] render#{frame_no}: begin ({}x{})",
                width,
                height
            );
        }

        // Handle resize if needed
        if width != state.current_width || height != state.current_height {
            // Avoid blocking the UI thread during live resize; perform a non-blocking poll
            // and keep retrying on transient reconfigure failures.
            let _ = state.device.poll(wgpu::PollType::Poll);

            state.config.width = width;
            state.config.height = height;

            try_configure_surface(&state.wgpu_surface, &state.device, &state.config);
            state.current_width = width;
            state.current_height = height;
        }

        // Call setup on first render (await the future synchronously)
        if !state.initialized || state.renderer_format != state.config.format {
            if trace_frame {
                tracing::debug!("[GpuSurface] render#{frame_no}: setup begin");
            }
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
                redraw_handle: state.redraw_handle.clone(),
            };
            let setup_future = state
                .gpu_surface
                .setup(&ctx, &mut unsafe { &mut *state.env }.0);
            pollster::block_on(setup_future);
            state.initialized = true;
            state.renderer_format = format;
            if trace_frame {
                tracing::debug!("[GpuSurface] render#{frame_no}: setup done");
            }
        }

        // Get next frame texture (guard against wgpu panics so we don't abort across the FFI boundary).
        let output = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.wgpu_surface.get_current_texture()
        })) {
            Ok(Ok(o)) => o,
            Ok(Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated)) => {
                tracing::debug!("[GpuSurface] surface lost/outdated, reconfiguring");
                try_configure_surface(&state.wgpu_surface, &state.device, &state.config);
                match state.wgpu_surface.get_current_texture() {
                    Ok(o) => o,
                    Err(wgpu::SurfaceError::Timeout) => {
                        panic!(
                            "waterui_gpu_surface_render: surface timeout after reconfigure for \
                             surface#{} frame#{}",
                            state.diagnostic_id, frame_no
                        );
                    }
                    Err(e) => {
                        panic!("waterui_gpu_surface_render: acquire after reconfigure failed: {e}");
                    }
                }
            }
            Ok(Err(wgpu::SurfaceError::Timeout)) => {
                panic!(
                    "waterui_gpu_surface_render: surface timeout for surface#{} frame#{}",
                    state.diagnostic_id, frame_no
                );
            }
            Ok(Err(e)) => {
                panic!("waterui_gpu_surface_render: get_current_texture failed: {e}");
            }
            Err(_) => {
                abort_on_panic("waterui_gpu_surface_render::acquire_swapchain_texture");
            }
        };
        if trace_frame {
            tracing::debug!("[GpuSurface] render#{frame_no}: acquired current texture");
        }

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface Frame View"),
            format: Some(state.config.format),
            ..Default::default()
        });

        let (elapsed, delta) = advance_frame_timing(state);

        // Create frame data
        let mut frame = GpuFrame::new(
            &state.device,
            &state.queue,
            &output.texture,
            view,
            state.config.format,
            width,
            height,
            state.pointer_state,
            state.gesture_state,
            elapsed,
            delta,
        );

        // Call user's render callback
        frame.keyboard = &state.keyboard_events;
        state.gpu_surface.render(&mut frame);
        let needs_redraw = frame.was_redraw_requested() || state.redraw_handle.take_dirty();
        if trace_frame {
            tracing::debug!("[GpuSurface] render#{frame_no}: user render callback complete");
        }

        // Present
        output.present();
        if trace_frame {
            tracing::debug!("[GpuSurface] render#{frame_no}: present complete");
        }

        if frame_no == 1 {
            tracing::debug!("[GpuSurface] first frame presented successfully");
        }

        WuiGpuSurfaceRenderResult {
            ok: true,
            needs_redraw,
        }
    }));

    let result = match render_result {
        Ok(result) => result,
        Err(_) => abort_on_panic("waterui_gpu_surface_render"),
    };

    gpu_surface_diagnostics().on_render_result(result.ok, result.needs_redraw);
    result
}

/// Query whether the renderer currently requests another frame.
///
/// This is a lightweight probe used by strict on-demand backends before they
/// schedule a render. It must not render or mutate GPU resources.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_needs_redraw(state: *mut WuiGpuSurfaceState) -> bool {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_gpu_surface_needs_redraw", "state");
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };
        state.redraw_handle.take_dirty()
    }));

    match result {
        Ok(value) => value,
        Err(_) => abort_on_panic("waterui_gpu_surface_needs_redraw"),
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
    unsafe {
        crate::expect_non_null_mut(state, "waterui_gpu_surface_render_to_texture", "state");
        crate::expect_non_null(texture, "waterui_gpu_surface_render_to_texture", "texture");
    }

    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *state };
        let texture = unsafe { &*(texture as *const wgpu::Texture) };

        let target_format = texture.format();

        if width != state.current_width || height != state.current_height {
            state.current_width = width;
            state.current_height = height;
            state.config.width = width;
            state.config.height = height;
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
                redraw_handle: state.redraw_handle.clone(),
            };
            let setup_future = state
                .gpu_surface
                .setup(&ctx, &mut unsafe { &mut *state.env }.0);
            pollster::block_on(setup_future);
            state.initialized = true;
            state.renderer_format = target_format;
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface External Frame View"),
            format: Some(target_format),
            ..Default::default()
        });
        let (elapsed, delta) = advance_frame_timing(state);

        let mut frame = GpuFrame::new(
            &state.device,
            &state.queue,
            texture,
            view,
            target_format,
            width,
            height,
            state.pointer_state,
            state.gesture_state,
            elapsed,
            delta,
        );

        frame.keyboard = &state.keyboard_events;
        state.gpu_surface.render(&mut frame);
        // Ensure external renderers see completed writes before returning.
        poll_capture_completion(&state.device)
    }));

    match render_result {
        Ok(ok) => ok,
        Err(_) => abort_on_panic("waterui_gpu_surface_render_to_texture"),
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
    unsafe {
        crate::expect_non_null_mut(
            state,
            "waterui_gpu_surface_render_to_metal_texture",
            "state",
        );
        crate::expect_non_null(
            texture,
            "waterui_gpu_surface_render_to_metal_texture",
            "texture",
        );
    }
    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        trace_metal_capture_step("render_to_metal_texture: begin");
        trace_metal_capture_step("render_to_metal_texture: deref state");
        let state = unsafe { &mut *state };
        trace_metal_capture_step("render_to_metal_texture: from_ptr");
        let metal_texture_ref = unsafe { metal::TextureRef::from_ptr(texture.cast()) };
        trace_metal_capture_step("render_to_metal_texture: to_owned");
        let metal_texture = metal_texture_ref.to_owned();
        let hal_device = unsafe { state.device.as_hal::<MetalApi>().unwrap_unchecked() };
        let wgpu_device_ptr = {
            let raw_device = hal_device.raw_device().lock();
            raw_device.as_ptr().cast::<c_void>()
        };
        let texture_device_ptr = metal_texture.device().as_ptr().cast::<c_void>();
        trace_metal_capture_step(&format!(
            "render_to_metal_texture: device_ptrs wgpu={wgpu_device_ptr:p} texture={texture_device_ptr:p}"
        ));
        let _ = texture_device_ptr;
        trace_metal_capture_step("render_to_metal_texture: pixel_format query");
        let metal_pixel_format = metal_texture.pixel_format();
        trace_metal_capture_step(&format!(
            "render_to_metal_texture: pixel_format={metal_pixel_format:?}"
        ));

        let target_format = match metal_pixel_format {
            metal::MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
            metal::MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
            metal::MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
            other => {
                panic!(
                    "[GpuSurface] render_to_metal_texture: unsupported format {:?}",
                    other
                );
            }
        };

        trace_metal_capture_step("render_to_metal_texture: texture_from_raw");
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
        trace_metal_capture_step("render_to_metal_texture: create_texture_from_hal");

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
        trace_metal_capture_step("render_to_metal_texture: imported texture ready");

        if width != state.current_width || height != state.current_height {
            trace_metal_capture_step("render_to_metal_texture: resize");
            state.current_width = width;
            state.current_height = height;
            state.config.width = width;
            state.config.height = height;
        }

        trace_metal_capture_step(&format!(
            "render_to_metal_texture: pre_setup_check initialized={} renderer_format={:?} target_format={:?}",
            state.initialized, state.renderer_format, target_format
        ));
        if !state.initialized || state.renderer_format != target_format {
            trace_metal_capture_step(&format!(
                "render_to_metal_texture: setup state initialized={} renderer_format={:?} config_format={:?} target_format={:?}",
                state.initialized, state.renderer_format, state.config.format, target_format
            ));
            trace_metal_capture_step("render_to_metal_texture: setup");
            trace_metal_capture_step("render_to_metal_texture: setup ctx begin");
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
                redraw_handle: state.redraw_handle.clone(),
            };
            trace_metal_capture_step("render_to_metal_texture: setup future begin");
            let setup_future = state
                .gpu_surface
                .setup(&ctx, &mut unsafe { &mut *state.env }.0);
            trace_metal_capture_step("render_to_metal_texture: setup future created");
            trace_metal_capture_step("render_to_metal_texture: setup immediate-poll begin");
            pollster::block_on(setup_future);
            trace_metal_capture_step("render_to_metal_texture: setup immediate-poll done");
            state.initialized = true;
            state.renderer_format = target_format;
        }
        trace_metal_capture_step("render_to_metal_texture: create view");

        let view = wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface Metal Frame View"),
            format: Some(target_format),
            ..Default::default()
        });
        let (elapsed, delta) = advance_frame_timing(state);

        let mut frame = GpuFrame::new(
            &state.device,
            &state.queue,
            &wgpu_texture,
            view,
            target_format,
            width,
            height,
            state.pointer_state,
            state.gesture_state,
            elapsed,
            delta,
        );

        trace_metal_capture_step("render_to_metal_texture: render");
        frame.keyboard = &state.keyboard_events;
        state.gpu_surface.render(&mut frame);
        trace_metal_capture_step("render_to_metal_texture: poll");
        poll_capture_completion(&state.device)
    }));

    match render_result {
        Ok(ok) => ok,
        Err(_) => abort_on_panic("waterui_gpu_surface_render_to_metal_texture"),
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
    panic!("waterui_gpu_surface_render_to_metal_texture: only supported on Apple platforms");
}

/// Setup the GpuSurface and render the first frame.
///
/// This function performs setup in the synchronous FFI render path,
/// then renders the first frame. Native code should call this before showing
/// the window to ensure all visible GpuSurfaces are ready.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state from `waterui_gpu_surface_init`
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_gpu_surface_init`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_await_ready(state: *mut WuiGpuSurfaceState) -> bool {
    unsafe {
        crate::expect_non_null_mut(state, "waterui_gpu_surface_await_ready", "state");
    }
    let await_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                redraw_handle: state.redraw_handle.clone(),
            };
            let setup_future = state
                .gpu_surface
                .setup(&ctx, &mut unsafe { &mut *state.env }.0);
            pollster::block_on(setup_future);
            state.initialized = true;
            state.renderer_format = format;
        }

        // Render first frame with a fast, non-blocking probe.
        let output = match state.wgpu_surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                tracing::debug!("[GpuSurface] await_ready: surface lost/outdated, reconfiguring");
                try_configure_surface(&state.wgpu_surface, &state.device, &state.config);
                match state.wgpu_surface.get_current_texture() {
                    Ok(o) => o,
                    Err(e) => {
                        panic!("waterui_gpu_surface_await_ready: retry acquire failed: {e}");
                    }
                }
            }
            Err(e) => {
                panic!("waterui_gpu_surface_await_ready: acquire failed: {e}");
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface Ready Frame"),
            format: Some(state.config.format),
            ..Default::default()
        });
        let (elapsed, delta) = advance_frame_timing(state);

        let mut frame = GpuFrame::new(
            &state.device,
            &state.queue,
            &output.texture,
            view,
            state.config.format,
            state.current_width,
            state.current_height,
            state.pointer_state,
            state.gesture_state,
            elapsed,
            delta,
        );

        frame.keyboard = &state.keyboard_events;
        state.gpu_surface.render(&mut frame);
        output.present();

        true
    }));

    match await_result {
        Ok(ok) => ok,
        Err(_) => abort_on_panic("waterui_gpu_surface_await_ready"),
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
    unsafe {
        crate::expect_non_null_mut(state, "waterui_gpu_surface_drop", "state");
        let state = Box::from_raw(state);
        let remaining = gpu_surface_diagnostics()
            .on_surface_drop(state.diagnostic_id, state.render_invocations);
        if remaining == 0 {
            waterui_graphics::shared_context::save_pipeline_cache();
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

#[inline]
fn pointer_state_from_ffi(pointer: WuiPointerState) -> PointerState {
    PointerState {
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
    }
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

#[inline]
fn gesture_state_from_ffi(gesture: WuiGestureState) -> GestureState {
    GestureState {
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
    }
}

/// FFI-safe combined input state for a GpuSurface.
///
/// This keeps the native bridge minimal by forwarding pointer and gesture
/// snapshots in one call.
#[repr(C)]
pub struct WuiGpuSurfaceInput {
    /// Current pointer snapshot.
    pub pointer: WuiPointerState,
    /// Current gesture snapshot.
    pub gesture: WuiGestureState,
}

/// Update both pointer and gesture state for a GpuSurface.
///
/// Native backends should prefer this API to minimize bridge calls.
///
/// # Arguments
///
/// * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
/// * `input` - Combined pointer + gesture snapshot
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_set_input(
    state: *mut WuiGpuSurfaceState,
    input: WuiGpuSurfaceInput,
) {
    let state =
        unsafe { crate::expect_non_null_mut(state, "waterui_gpu_surface_set_input", "state") };
    state.pointer_state = pointer_state_from_ffi(input.pointer);
    state.gesture_state = gesture_state_from_ffi(input.gesture);
}

/// FFI keyboard modifier flags.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WuiKeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// FFI keyboard event.
///
/// `codepoint != 0` denotes a typed character (`KeyCode::Char`); otherwise
/// `named` selects a [`NamedKey`] via `named_key_from_ffi`. `pressed` is `true`
/// for key-down.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WuiKeyEvent {
    pub codepoint: u32,
    pub named: u32,
    pub modifiers: WuiKeyModifiers,
    pub pressed: bool,
}

fn named_key_from_ffi(named: u32) -> NamedKey {
    match named {
        0 => NamedKey::Enter,
        1 => NamedKey::Tab,
        2 => NamedKey::Backspace,
        3 => NamedKey::Delete,
        4 => NamedKey::Escape,
        5 => NamedKey::ArrowUp,
        6 => NamedKey::ArrowDown,
        7 => NamedKey::ArrowLeft,
        8 => NamedKey::ArrowRight,
        9 => NamedKey::Home,
        10 => NamedKey::End,
        11 => NamedKey::PageUp,
        12 => NamedKey::PageDown,
        13 => NamedKey::Insert,
        f if f >= 100 => NamedKey::Function((f - 100) as u8),
        _ => NamedKey::Escape,
    }
}

fn key_event_from_ffi(event: WuiKeyEvent) -> KeyEvent {
    let key = if event.codepoint != 0 {
        KeyCode::Char(char::from_u32(event.codepoint).unwrap_or(char::REPLACEMENT_CHARACTER))
    } else {
        KeyCode::Named(named_key_from_ffi(event.named))
    };
    KeyEvent {
        key,
        modifiers: KeyModifiers {
            shift: event.modifiers.shift,
            ctrl: event.modifiers.ctrl,
            alt: event.modifiers.alt,
            meta: event.modifiers.meta,
        },
        pressed: event.pressed,
    }
}

/// Replace the keyboard-event snapshot for a GpuSurface.
///
/// Native backends call this once per frame while the surface is the keyboard
/// responder, passing the key events to deliver. `len == 0` clears the snapshot.
///
/// # Safety
///
/// `state` must be valid; `events` must point to `len` `WuiKeyEvent`s (or be
/// null when `len == 0`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_set_keys(
    state: *mut WuiGpuSurfaceState,
    events: *const WuiKeyEvent,
    len: usize,
) {
    let state =
        unsafe { crate::expect_non_null_mut(state, "waterui_gpu_surface_set_keys", "state") };
    let slice = if events.is_null() || len == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(events, len) }
    };
    state.keyboard_events = slice.iter().copied().map(key_event_from_ffi).collect();
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
) {
    // Non-blocking poll so we don't stall the render thread.
    let _ = device.poll(wgpu::PollType::Poll);

    // `Surface::configure` doesn't return a `Result`, so use error scopes to detect failures.
    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Internal);
    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let configure_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        surface.configure(device, config);
    }))
    .is_err();

    let validation_err = crate::pop_error_scope_now(device, "gpu_surface::validation_error_scope");
    let internal_err = crate::pop_error_scope_now(device, "gpu_surface::internal_error_scope");
    let oom_err = crate::pop_error_scope_now(device, "gpu_surface::oom_error_scope");

    if configure_panicked {
        abort_on_panic("gpu_surface::try_configure_surface");
    }

    let configure_err = validation_err.or(internal_err).or(oom_err);
    assert!(
        configure_err.is_none(),
        "gpu_surface::try_configure_surface failed: {configure_err:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IntoFFI;
    use waterui_core::Environment;
    use waterui_graphics::{GpuView, impl_gpu_subview};

    struct DummyGpuRenderer;

    impl_gpu_subview!(DummyGpuRenderer);

    impl GpuView for DummyGpuRenderer {
        async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut Environment) {}

        fn render(&mut self, _frame: &mut GpuFrame) {}
    }

    struct SurfaceGuard {
        inner: WuiGpuSurface,
    }

    impl SurfaceGuard {
        fn new(surface: GpuSurface) -> Self {
            Self {
                inner: surface.into_ffi(),
            }
        }
    }

    impl Drop for SurfaceGuard {
        fn drop(&mut self) {
            if self.inner.surface.is_null() {
                return;
            }
            unsafe {
                drop(Box::from_raw(self.inner.surface as *mut GpuSurface));
            }
            self.inner.surface = core::ptr::null_mut();
        }
    }

    #[test]
    fn hdr_preference_defaults_to_none() {
        let guard = SurfaceGuard::new(GpuSurface::new(DummyGpuRenderer));
        let pref = unsafe { waterui_gpu_surface_hdr_preference(&guard.inner) };
        assert!(!pref.has_preference);
        assert!(!pref.prefers_hdr);
    }

    #[test]
    fn hdr_preference_reports_explicit_hdr() {
        let guard = SurfaceGuard::new(GpuSurface::new(DummyGpuRenderer).prefer_hdr_surface());
        let pref = unsafe { waterui_gpu_surface_hdr_preference(&guard.inner) };
        assert!(pref.has_preference);
        assert!(pref.prefers_hdr);
        assert!(pref.resolved_prefers_hdr);
    }

    #[test]
    fn hdr_preference_reports_explicit_sdr() {
        let guard = SurfaceGuard::new(GpuSurface::new(DummyGpuRenderer).prefer_sdr_surface());
        let pref = unsafe { waterui_gpu_surface_hdr_preference(&guard.inner) };
        assert!(pref.has_preference);
        assert!(!pref.prefers_hdr);
        assert!(!pref.resolved_prefers_hdr);
    }
}
