//! Task utilities and runtime guardrails for async execution.

pub use executor_core::{spawn, spawn_local};
pub use native_executor::sleep;

mod runtime_guard;

pub use runtime_guard::{
    MainThreadStallProbeConfig, MonitoredLocalExecutor, RuntimeProbe, TaskPollSample,
    install_runtime_probe, monitored_local_executor, monitored_local_executor_with_config,
};

/// Initializes the WaterUI runtime instance linked into a macOS preview dylib.
///
/// The preview support app and the preview dylib are separate linkage units, so
/// each owns its own `executor-core` state. Preview exports call this on the main
/// thread before constructing a view. Repeated calls are expected because one
/// dylib can expose multiple preview functions.
#[doc(hidden)]
#[cfg(target_os = "macos")]
pub fn __initialize_preview_runtime() {
    drop(executor_core::try_init_global_executor(
        native_executor::NativeExecutor::new(),
    ));
    drop(executor_core::try_init_local_executor(
        monitored_local_executor(native_executor::NativeExecutor::new()),
    ));
}
