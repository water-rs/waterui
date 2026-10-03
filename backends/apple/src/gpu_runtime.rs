//! The process GPU runtime: created asynchronously on the shared executor,
//! installed into the environment before the fallback's services — the Rust
//! half of what `WuiGpuRuntime.swift` + the `waterui_gpu_runtime_*` FFI did.

#[cfg(feature = "gpu_surface")]
use cocoa_ui::Retained;
use executor_core::{spawn, spawn_local};
#[cfg(feature = "gpu_surface")]
use objc2::runtime::ProtocolObject;
#[cfg(feature = "gpu_surface")]
use objc2_metal::MTLDevice;
use waterui_backend_core::Environment;
use waterui_graphics::gpu::GpuRuntime;
#[cfg(feature = "gpu_surface")]
use waterui_graphics::gpu::SharedGpuContext;
#[cfg(feature = "gpu_surface")]
use wgpu_hal::api::Metal as MetalApi;

/// Creates the runtime on the shared executor, installs it into `env` on the
/// main thread, then runs `then`. The owner retains `env` until setup and
/// the completion callback finish.
///
/// # Safety
///
/// `env` must be a valid, live `Environment` until `then` finishes and
/// `then` runs on the main thread.
pub unsafe fn prepare(env: *mut Environment, then: impl FnOnce() + 'static) {
    let (sender, receiver) = async_channel::bounded(1);
    spawn(async move {
        let runtime = GpuRuntime::new().await;
        let _ = sender.send(runtime).await;
    })
    .detach();
    spawn_local(async move {
        let runtime = receiver
            .recv()
            .await
            .expect("GPU runtime creation task ended without producing a runtime")
            .unwrap_or_else(|error| panic!("GPU runtime creation failed: {error}"));
        // SAFETY: `env` is lent for the process and this task is pinned to the
        // main executor — the same thread the launch handler runs on.
        let env = unsafe { &mut *env };
        env.insert(runtime);
        then();
    })
    .detach();
}

/// The environment's GPU runtime.
///
/// # Panics
///
/// When no runtime was installed — [`prepare`] runs before any surface or
/// effect can render, so a missing runtime is a launch error.
#[cfg(feature = "gpu_surface")]
pub fn runtime(env: &Environment) -> GpuRuntime {
    env.get::<GpuRuntime>()
        .expect("GPU runtime is not installed in the WaterUI environment")
        .clone()
}

/// The `MTLDevice` a context generation's wgpu device wraps.
///
/// # Panics
///
/// When the runtime's device is not Metal or the device pointer is null.
#[cfg(feature = "gpu_surface")]
pub fn raw_metal_device(context: &SharedGpuContext) -> Retained<ProtocolObject<dyn MTLDevice>> {
    // SAFETY: `raw_device` is the `MTLDevice` the runtime created and still
    // owns; `retain` takes our own reference on it.
    unsafe {
        Retained::retain(
            Retained::as_ptr(
                context
                    .device()
                    .as_hal::<MetalApi>()
                    .expect("the Apple runtime's device is Metal")
                    .raw_device(),
            )
            .cast_mut(),
        )
        .expect("the Metal device is non-null")
    }
}
