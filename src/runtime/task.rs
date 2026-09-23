//! Task utilities and runtime guardrails for async execution.
//!
//! # When the executors exist
//!
//! Each backend installs the thread-local executor inside its run loop at the
//! moment it mounts the app's view — after `main` has run and after stores and
//! the view tree have been constructed. [`spawn_local`] called earlier, while
//! the app or its stores are still being built, panics with "Local executor
//! not set". Run startup async work from a point that executes after mount:
//! `.task(..)` on a view, `.on_appear`, or a handler body.

pub use executor_core::spawn;
/// Spawns a future on the thread-local executor the backend installed.
///
/// The local executor exists from the moment the backend mounts the app's
/// view; it is not available while the app or its stores are still being
/// constructed (`fn main`, a `Store::new`), and calling this then panics with
/// "Local executor not set". Move startup async work into `.task(..)`,
/// `.on_appear`, or a handler body instead.
///
/// Unlike [`spawn`], the future does not need to be `Send`.
///
/// # Panics
///
/// Panics if the local executor has not been installed on this thread yet.
pub use executor_core::spawn_local;
pub use native_executor::sleep;

mod runtime_guard;

pub use runtime_guard::{
    MainThreadStallProbeConfig, MonitoredLocalExecutor, RefreshRate, RuntimeProbe, TaskPollSample,
    monitored_local_executor, monitored_local_executor_with_config,
    monitored_local_executor_with_probes, outstanding_local_tasks,
};
