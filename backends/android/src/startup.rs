//! Process startup: the work `waterui_init` used to do, minus the FFI.
//!
//! Runs once, on the main thread, before the environment exists: route
//! panics and records through `tracing`, install the global executor and the
//! looper-bound local executor, then start the system-locale listener (its
//! mailbox pump needs the executor that was just installed).

use alloc::boxed::Box;
use std::sync::Mutex;

use waterui::inspector::InspectorRuntime;

/// The environment variable a launcher sets to name the level the application
/// logs at — `water run --logs <level>` writes it.
const LOG_LEVEL_ENV: &str = "WATERUI_LOG";

/// One-time process startup. Returns the inspector runtime when the
/// environment asked for one, for [`crate::entry`] to install.
#[cfg(target_os = "android")]
pub(crate) fn initialize(env: &mut jni::Env) -> Option<InspectorRuntime> {
    // The inspector runtime is a process-global endpoint: the first mount
    // takes it, later mounts find `None` and install nothing.
    static INSPECTOR: Mutex<Option<InspectorRuntime>> = Mutex::new(None);
    static INIT: std::sync::Once = std::sync::Once::new();

    INIT.call_once(|| {
        let inspector = waterui::inspector::maybe_init_from_env("android");

        std::panic::set_hook(Box::new(|info| {
            tracing_panic::panic_hook(info);
        }));
        init_tracing(
            inspector
                .as_ref()
                .map(waterui::inspector::InspectorRuntime::tracing_layer),
        );

        executor_core::init_global_executor(native_executor::NativeExecutor::new());
        // The local executor is the main `Looper`'s own pump — never
        // `native_executor::NativeExecutor`, whose tasks would run on a
        // worker thread that must never touch views.
        let main_executor = crate::executor::install();
        let monitored = waterui::task::monitored_local_executor_with_probes(
            main_executor,
            display_refresh_rate(env),
            inspector
                .as_ref()
                .map(waterui::inspector::InspectorRuntime::runtime_probe),
        );
        // `try_init` because a runtime may follow an already-initialized one:
        // the second `nativeCreate` must not panic for wanting what exists.
        if let Err(_already_installed) = executor_core::try_init_local_executor(monitored) {
            tracing::warn!(
                target: "waterui::executor",
                "a local executor is already installed on the main thread"
            );
        }

        // The listener's mailbox pump needs the executor installed above.
        waterui_locale::start_system_locale_listener();

        if let Some(inspector) = inspector {
            INSPECTOR
                .lock()
                .expect("the inspector slot is not poisoned")
                .replace(inspector);
        }
    });

    INSPECTOR
        .lock()
        .expect("the inspector slot is not poisoned")
        .take()
}

/// A host-compilation stand-in: startup is a device path, but the crate must
/// still type-check off target.
#[cfg(not(target_os = "android"))]
#[allow(clippy::missing_const_for_fn, reason = "parallel to the android fn")]
pub(crate) fn initialize(_env: &mut jni::Env) -> Option<InspectorRuntime> {
    None
}

/// The refresh rate of the display this host drives, for the executor's
/// frame budget — the `maximumFramesPerSecond` counterpart. Falls back to
/// the nominal rate when the platform reports nothing; the budget only
/// scales stall diagnostics, so an absent rate is not a fault of the app.
#[cfg(target_os = "android")]
fn display_refresh_rate(env: &mut jni::Env) -> waterui::task::RefreshRate {
    use waterui::task::RefreshRate;

    crate::jvm::globals()
        .bindings()
        .refresh_rate_hz(env)
        .ok()
        .flatten()
        .map_or_else(
            || {
                tracing::info!(
                    target: "waterui::runtime_guard",
                    "display refresh rate is unavailable; budgeting frames at the nominal rate"
                );
                RefreshRate::HEADLESS
            },
            |hz| {
                RefreshRate::from_millihertz(
                    core::num::NonZeroU32::new(
                        // `getRefreshRate` answers whole Hz for every
                        // shipping display; the float's fractional tail is
                        // reporting noise.
                        u32::try_from(hz.max(1.0) as u32)
                            .expect("the positive rate fits a `u32`")
                            .saturating_mul(1000),
                    )
                    .expect("a refresh rate of at least 1 Hz"),
                )
            },
        )
}

/// The `tracing` filter this process runs with: `RUST_LOG` wins outright,
/// otherwise `WATERUI_LOG` names the level and the JNI layer stays at
/// `error`.
#[cfg(target_os = "android")]
fn env_filter() -> tracing_subscriber::EnvFilter {
    use tracing_subscriber::EnvFilter;

    if let Ok(filter) = EnvFilter::try_from_default_env() {
        return filter;
    }
    let level = std::env::var(LOG_LEVEL_ENV).unwrap_or_else(|_| String::from("error"));
    EnvFilter::try_new(format!("{level},jni=error"))
        .unwrap_or_else(|error| panic!("{LOG_LEVEL_ENV}={level:?} is not a tracing level: {error}"))
}

/// Sends `tracing` records to `__android_log_write`.
#[cfg(target_os = "android")]
fn init_tracing(inspector: Option<waterui::inspector::InspectorLayer>) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let native_layer = tracing_subscriber::fmt::layer()
        .with_writer(crate::android_log::AndroidLog)
        .without_time()
        .with_ansi(false);
    tracing_subscriber::registry()
        .with(env_filter())
        .with(native_layer)
        .with(inspector)
        .init();
}
