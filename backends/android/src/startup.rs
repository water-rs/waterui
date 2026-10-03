//! Process startup: the work `waterui_init` used to do, minus the FFI.
//!
//! Runs once, on the main thread, before the environment exists: route
//! panics and records through `tracing`, install the global executor and the
//! looper-bound local executor, then start the system-locale listener (its
//! mailbox pump needs the executor that was just installed).
//!
//! Nothing here is guarded by a `Once`: mount owns this call, the way the
//! Apple backend's `initialize` runs unconditionally at mount. The process
//! hosts one runtime per activity, so a second `nativeCreate` reaching here
//! is already off contract — the `try_init_*` installs only keep that
//! contract's breach a warning instead of a panic.

#[cfg(target_os = "android")]
use alloc::boxed::Box;

use waterui::inspector::InspectorRuntime;

use crate::jvm::Platform;

/// The environment variable a launcher sets to name the level the application
/// logs at — `water run --logs <level>` writes it.
const LOG_LEVEL_ENV: &str = "WATERUI_LOG";

/// One-time process startup. Returns the inspector runtime when the
/// environment asked for one, for [`crate::entry`] to install.
#[cfg(target_os = "android")]
pub fn initialize(env: &mut jni::Env, platform: &Platform) -> Option<InspectorRuntime> {
    let inspector = waterui::inspector::maybe_init_from_env("android");

    std::panic::set_hook(Box::new(|info| {
        tracing_panic::panic_hook(info);
    }));
    init_tracing(
        inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::tracing_layer),
    );

    // `try_init` because a runtime may follow an already-initialized one:
    // the second `nativeCreate` must not panic for wanting what exists.
    if let Err(_already_installed) =
        executor_core::try_init_global_executor(native_executor::NativeExecutor::new())
    {
        tracing::warn!(
            target: "waterui::executor",
            "a global executor is already installed"
        );
    }
    // The local executor is the main `Looper`'s own pump — never
    // `native_executor::NativeExecutor`, whose tasks would run on a
    // worker thread that must never touch views.
    let main_executor = crate::executor::install();
    let monitored = waterui::task::monitored_local_executor_with_probes(
        main_executor,
        display_refresh_rate(env, platform),
        inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe),
    );
    if let Err(_already_installed) = executor_core::try_init_local_executor(monitored) {
        tracing::warn!(
            target: "waterui::executor",
            "a local executor is already installed on the main thread"
        );
    }

    // The listener's mailbox pump needs the executor installed above.
    waterui_locale::start_system_locale_listener();

    inspector
}

/// A host-compilation stand-in: startup is a device path, but the crate must
/// still type-check off target.
#[cfg(not(target_os = "android"))]
#[allow(clippy::missing_const_for_fn, reason = "parallel to the android fn")]
pub fn initialize(_env: &mut jni::Env, _platform: &Platform) -> Option<InspectorRuntime> {
    None
}

/// The refresh rate of the display this host drives, for the executor's
/// frame budget — the `maximumFramesPerSecond` counterpart. Falls back to
/// the nominal rate when the platform reports nothing; the budget only
/// scales stall diagnostics, so an absent rate is not a fault of the app.
#[cfg(target_os = "android")]
fn display_refresh_rate(env: &mut jni::Env, platform: &Platform) -> waterui::task::RefreshRate {
    use waterui::task::RefreshRate;

    platform.refresh_rate_hz(env).ok().flatten().map_or_else(
        || {
            tracing::info!(
                target: "waterui::runtime_guard",
                "display refresh rate is unavailable; budgeting frames at the nominal rate"
            );
            RefreshRate::HEADLESS
        },
        |hz| {
            // `getRefreshRate` answers whole Hz for every shipping
            // display; the float's fractional tail is reporting noise.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the rate is clamped to a positive whole Hz"
            )]
            let whole_hz = hz.max(1.0) as u32;
            RefreshRate::from_millihertz(
                core::num::NonZeroU32::new(whole_hz.saturating_mul(1000))
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

/// Sends `tracing` records to `__android_log_write`. A second install — a
/// repeated `nativeCreate` — keeps the first subscriber and warns.
#[cfg(target_os = "android")]
fn init_tracing(inspector: Option<waterui::inspector::InspectorLayer>) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let native_layer = tracing_subscriber::fmt::layer()
        .with_writer(crate::android_log::AndroidLog)
        .without_time()
        .with_ansi(false);
    if tracing_subscriber::registry()
        .with(env_filter())
        .with(native_layer)
        .with(inspector)
        .try_init()
        .is_err()
    {
        tracing::warn!(target: "waterui::runtime", "a tracing subscriber is already installed");
    }
}
