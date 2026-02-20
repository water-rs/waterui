//! Task utilities and runtime guardrails for async execution.

pub use executor_core::{spawn, spawn_local};
pub use native_executor::sleep;

mod runtime_guard;

pub use runtime_guard::{
    MainThreadStallProbeConfig, MonitoredLocalExecutor, RuntimeProbe, TaskPollSample,
    install_runtime_probe, monitored_local_executor, monitored_local_executor_with_config,
};
