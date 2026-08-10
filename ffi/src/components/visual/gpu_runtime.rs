//! Explicit GPU runtime ownership across native backends.

use executor_core::spawn_local;
use waterui_graphics::shared_context::GpuRuntime;

#[cfg(all(feature = "c-api", any(target_os = "macos", target_os = "ios")))]
use {objc2::rc::Retained, wgpu_hal::api::Metal as MetalApi};

#[cfg(feature = "c-api")]
use crate::bridge::closure::ForeignCallbackContext;
#[cfg(feature = "c-api")]
use crate::{IntoFFI, IntoRust, WuiEnv};

opaque!(WuiGpuRuntime, GpuRuntime, gpu_runtime);

/// Completion invoked after a GPU runtime is ready for installation.
#[cfg(feature = "c-api")]
pub type WuiGpuRuntimeCreateCallback =
    unsafe extern "C" fn(context: *mut (), runtime: *mut WuiGpuRuntime);

/// Releases the native context retained for asynchronous GPU runtime creation.
#[cfg(feature = "c-api")]
pub type WuiGpuRuntimeCreateContextDrop = unsafe extern "C" fn(context: *mut ());

pub fn create_gpu_runtime(complete: impl FnOnce(GpuRuntime) + 'static) {
    spawn_local(async move {
        let runtime = GpuRuntime::new()
            .await
            .unwrap_or_else(|error| panic!("GPU runtime creation failed: {error}"));
        complete(runtime);
    })
    .detach();
}

/// Creates a GPU runtime asynchronously.
///
/// `complete` receives ownership of the runtime. Rust releases `context`
/// through `drop_context` immediately after the completion returns.
///
/// # Safety
///
/// `context`, `complete`, and `drop_context` must remain valid until completion.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_runtime_create(
    context: *mut (),
    complete: WuiGpuRuntimeCreateCallback,
    drop_context: WuiGpuRuntimeCreateContextDrop,
) {
    // SAFETY: the caller contract requires `context` and `drop_context` to be one
    // registration from the backend.
    let context = unsafe { ForeignCallbackContext::new(context, drop_context) };
    // SAFETY: `complete` belongs to that same registration, and the context is moved
    // into the closure, so it is alive when the completion fires.
    create_gpu_runtime(move |runtime| unsafe {
        complete(context.data(), runtime.into_ffi());
    });
}

pub fn install_gpu_runtime(env: &mut waterui::Environment, runtime: GpuRuntime) {
    env.insert(runtime);
}

pub fn gpu_runtime(env: &waterui::Environment) -> GpuRuntime {
    env.get::<GpuRuntime>()
        .expect("GPU runtime is not installed in the WaterUI environment")
        .clone()
}

/// Installs a GPU runtime into an environment, consuming the runtime handle.
///
/// # Safety
///
/// `env` and `runtime` must be valid owning pointers. `runtime` is consumed.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_gpu_runtime(
    env: *mut WuiEnv,
    runtime: *mut WuiGpuRuntime,
) {
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };
    // SAFETY: the caller contract makes `runtime` an owning handle from the matching
    // FFI constructor; it is consumed here and not observed again.
    let runtime = unsafe { runtime.into_rust() };
    install_gpu_runtime(&mut env.0, runtime);
}

/// Returns the Metal device owned by the environment's GPU runtime.
///
/// The returned `MTLDevice` pointer is borrowed and remains valid while the
/// environment or one of its clones retains the installed runtime.
///
/// # Safety
///
/// `env` must be valid and contain an installed GPU runtime.
#[cfg(all(feature = "c-api", any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_runtime_metal_device(
    env: *const WuiEnv,
) -> *mut core::ffi::c_void {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    let runtime = gpu_runtime(&env.0);
    // SAFETY: this entry point is Metal-only, so the runtime's device has `MetalApi`
    // as its HAL type.
    let device = unsafe { runtime.context().device.as_hal::<MetalApi>() }
        .expect("WaterUI GPU runtime did not create a Metal device");
    Retained::as_ptr(device.raw_device()).cast_mut().cast()
}
