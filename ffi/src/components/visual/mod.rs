// Gated on `gpu` alone, like `gpu_surface`: Android drives both of these through
// the JNI bindings, which are compiled with `android-jni` and without `c-api`.
#[cfg(feature = "gpu")]
pub mod applied_filter;
#[cfg(all(target_os = "android", feature = "gpu"))]
pub mod capture_composite;
#[cfg(feature = "gpu")]
pub mod capture_format;
#[cfg(feature = "gpu")]
pub mod gpu_runtime;
#[cfg(feature = "gpu")]
pub mod gpu_surface;
#[cfg(feature = "gpu")]
pub mod gpu_surface_input;
#[cfg(all(target_os = "android", feature = "gpu"))]
pub mod hardware_buffer;
pub mod picture;
#[cfg(feature = "gpu")]
pub mod view_effect;
pub mod view_renderer;

/// Acquires the next texture of a configured surface, reconfiguring once when
/// the swapchain is lost or outdated.
///
/// `None` means the frame was skipped because the surface is occluded: nothing
/// was drawn, and the caller must report the frame as still pending so the host
/// comes back for it — a view whose only clock is its own render loop has no
/// other way to be woken.
#[cfg(all(feature = "gpu", not(any(target_os = "macos", target_os = "ios"))))]
fn acquire_surface_texture(
    surface: &wgpu::Surface<'_>,
    gpu: &waterui_graphics::shared_context::SharedGpuContext,
    config: &wgpu::SurfaceConfiguration,
    context: &'static str,
) -> Option<wgpu::SurfaceTexture> {
    let device = &gpu.device;
    match checked_surface_acquire(surface, device) {
        Ok(
            wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output),
        ) => Some(output),
        Ok(wgpu::CurrentSurfaceTexture::Occluded) => {
            tracing::debug!(context, "surface is occluded; skipping frame");
            None
        }
        Ok(wgpu::CurrentSurfaceTexture::Timeout) => panic!("{context}: surface timeout"),
        // `Lost`/`Outdated` name the stale swapchain; `Validation` is what a
        // dead device reports, and `Err` carries whatever the error scope
        // caught. Every one of them earns one reconfigure + retry — on a lost
        // device the configure fails and names the real cause.
        Ok(
            status @ (wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Validation),
        ) => retry_surface_acquire(surface, gpu, config, context, format_args!("{status:?}")),
        Err(error) => retry_surface_acquire(surface, gpu, config, context, format_args!("{error}")),
    }
}

/// One reconfigure-and-retry for a failed swapchain acquire, after ruling out
/// a dead device: when the device-lost callback has already fired, surface
/// state is unrecoverable and the panic must name that cause instead of the
/// generic acquire status.
#[cfg(all(feature = "gpu", not(any(target_os = "macos", target_os = "ios"))))]
fn retry_surface_acquire(
    surface: &wgpu::Surface<'_>,
    gpu: &waterui_graphics::shared_context::SharedGpuContext,
    config: &wgpu::SurfaceConfiguration,
    context: &'static str,
    first_failure: std::fmt::Arguments<'_>,
) -> Option<wgpu::SurfaceTexture> {
    if let Some(reason) = gpu.device_lost_reason() {
        // The surface is bound to a dead device and cannot be reconfigured.
        // Skipping the frame keeps it pending; the next render call arrives
        // on the runtime's rebuilt context, which recreates the surface from
        // its retained window handle.
        tracing::debug!(context, reason, "GPU device lost; skipping the frame");
        return None;
    }
    let device = &gpu.device;
    tracing::debug!(
        context,
        "surface acquire failed ({first_failure}); reconfiguring and retrying"
    );
    checked_surface_configure(surface, device, config, context);
    match checked_surface_acquire(surface, device) {
        Ok(
            wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output),
        ) => Some(output),
        Ok(wgpu::CurrentSurfaceTexture::Occluded) => {
            tracing::debug!(context, "surface is occluded; skipping frame");
            None
        }
        Ok(status) => {
            panic!("{context}: acquire after reconfigure failed: {status:?} with {config:?}")
        }
        Err(error) => {
            panic!("{context}: acquire after reconfigure failed: {error} with {config:?}")
        }
    }
}

/// Runs one frame of device-bound work, recovering when the driver kills the
/// device in the middle of it.
///
/// Device loss is discovered lazily — inside whatever `wgpu` call first
/// notices the dead driver — and `wgpu-core` purges its resource storage as it
/// marks the loss. Any later call in the same frame that resolves one of the
/// dead device's handles then panics inside `wgpu` rather than erroring; the
/// semantic renderer (`vello` uploads, `queue.write_buffer`, submits) cannot
/// be taught to bail, so the frame body runs inside `catch_unwind` here.
///
/// A caught panic is only swallowed when the context confirms the device was
/// actually lost — that is the recoverable fallout this exists for, and it
/// returns `None`. Anything else is a real bug and is re-raised unchanged.
#[cfg(all(feature = "gpu", not(any(target_os = "macos", target_os = "ios"))))]
pub fn run_gpu_frame<T>(
    gpu: &waterui_graphics::shared_context::SharedGpuContext,
    scope: &'static str,
    frame: impl FnOnce() -> T,
) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(frame)) {
        Ok(done) => Some(done),
        Err(payload) => {
            if gpu.device_lost_reason().is_some() {
                tracing::warn!(
                    scope,
                    "GPU device was lost mid-frame; dropping it and rebuilding on the next render"
                );
                None
            } else {
                std::panic::resume_unwind(payload)
            }
        }
    }
}

/// Acquires the next swapchain texture with the real failure attached.
///
/// `get_current_texture` collapses every acquisition error into a bare
/// [`wgpu::CurrentSurfaceTexture::Validation`] status and routes the underlying
/// [`wgpu::Error`] to the device's error-scope stack — where any leaked
/// validation scope on this thread would swallow it silently. Wrapping the call
/// in our own innermost scope claims that error back so failures can name their
/// cause. The pop future is ready immediately on native (error scopes are
/// thread-local bookkeeping, not GPU work), so blocking on it never waits.
#[cfg(all(feature = "gpu", not(any(target_os = "macos", target_os = "ios"))))]
fn checked_surface_acquire(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
) -> Result<wgpu::CurrentSurfaceTexture, wgpu::Error> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let status = surface.get_current_texture();
    pollster::block_on(scope.pop()).map_or_else(|| Ok(status), Err)
}

/// Configures the surface with validation failures surfaced instead of dropped.
///
/// A failed `configure` leaves the surface unconfigured, which only shows up
/// later as a bare `Validation` status on the next acquire — with the actual
/// error gone. Claiming it here names the real cause at the point of failure.
///
/// # Panics
/// Panics with the underlying validation error when the configuration is
/// rejected by the device.
#[cfg(all(feature = "gpu", not(any(target_os = "macos", target_os = "ios"))))]
pub fn checked_surface_configure(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    context: &'static str,
) {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    surface.configure(device, config);
    if let Some(error) = pollster::block_on(scope.pop()) {
        panic!("{context}: surface configure failed: {error} with {config:?}");
    }
}
