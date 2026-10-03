//! Process startup: the work `waterui_init` used to do, minus the FFI.
//!
//! Runs once, on the main thread, before the environment exists: ignore
//! `SIGPIPE`, attach the inspector when the environment asks for one, route
//! panics and records through `tracing`, install the global and the
//! main-queue local executors, then start the system-locale listener (its
//! mailbox pump needs the executor that was just installed).

use alloc::boxed::Box;

use waterui::inspector::InspectorRuntime;

/// The environment variable a launcher sets to name the level the application
/// logs at — `water run --logs <level>` writes it.
const LOG_LEVEL_ENV: &str = "WATERUI_LOG";

/// One-time process startup. Returns the inspector runtime when the
/// environment asked for one, for [`crate::entry`] to install.
pub fn initialize() -> Option<InspectorRuntime> {
    ignore_sigpipe();
    let inspector = waterui::inspector::maybe_init_from_env("apple");

    std::panic::set_hook(Box::new(|info| {
        tracing_panic::panic_hook(info);
    }));
    init_tracing(
        inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::tracing_layer),
    );

    executor_core::init_global_executor(native_executor::NativeExecutor::new());
    let main_executor = native_executor::NativeMainExecutor::new()
        .expect("waterui_apple_main runs on the platform main thread");
    executor_core::init_local_executor(waterui::task::monitored_local_executor_with_probes(
        main_executor,
        display_refresh_rate(),
        inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe),
    ));

    // The listener's mailbox pump needs the executor installed above.
    waterui_locale::start_system_locale_listener();

    inspector
}

/// Rust's own `lang_start` ignores `SIGPIPE`; a cdylib loaded by a foreign
/// main never runs it, so a process that pipes this output and exits first
/// would kill it. A failed write must surface as `EPIPE`.
fn ignore_sigpipe() {
    // SAFETY: `signal` only swaps the process-wide SIGPIPE disposition for
    // `SIG_IGN`; no handler runs Rust code, and this runs once at startup.
    unsafe {
        let previous = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        assert_ne!(
            previous,
            libc::SIG_ERR,
            "libc::signal(SIGPIPE, SIG_IGN) failed"
        );
    }
}

/// The refresh rate of the displays this host drives, for the executor's
/// frame budget.
fn display_refresh_rate() -> waterui::task::RefreshRate {
    use waterui::task::RefreshRate;

    // Metadata the platform does not expose is not a fault of the app: the
    // budget only scales stall diagnostics, so it takes the nominal rate.
    max_frames_per_second().map_or_else(
        || {
            tracing::info!(
                target: "waterui::runtime_guard",
                "display refresh rate is unavailable; budgeting frames at the nominal rate"
            );
            RefreshRate::HEADLESS
        },
        RefreshRate::from_millihertz,
    )
}

/// The fastest attached screen's refresh rate in millihertz.
///
/// `maximumFramesPerSecond` is the `CADisplayLink` ceiling every renderer
/// already respects; the budget takes the maximum across screens because it
/// bounds stall diagnostics for the whole process, not one window. `None`
/// when the platform reports no usable rate.
#[cfg(target_os = "macos")]
fn max_frames_per_second() -> Option<core::num::NonZeroU32> {
    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::objc2_app_kit::NSScreen;

    let mtm = MainThreadMarker::new().expect("startup runs on the main thread");
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| screen.maximumFramesPerSecond())
        .max()
        .filter(|fps| *fps > 0)
        .map(|fps| {
            // `maximumFramesPerSecond` is bounded to the hardware's few
            // hundred Hz, so the millihertz value is nonzero and fits a `u32`.
            core::num::NonZeroU32::new(
                u32::try_from(fps).expect("the filtered positive rate fits a `u32`") * 1000,
            )
            .expect("a refresh rate of at least 1 Hz")
        })
}

/// The iOS analogue of [`max_frames_per_second`].
#[cfg(not(target_os = "macos"))]
fn max_frames_per_second() -> Option<core::num::NonZeroU32> {
    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::objc2_ui_kit::UIScreen;

    let mtm = MainThreadMarker::new().expect("startup runs on the main thread");
    // `screens` is deprecated in favour of scene-session discovery, but at
    // startup no scene session exists yet — this is the only API that can
    // answer inside this window.
    #[expect(deprecated, reason = "no scene session exists at process startup")]
    UIScreen::screens(mtm)
        .iter()
        .map(|screen| screen.maximumFramesPerSecond())
        .max()
        .filter(|fps| *fps > 0)
        .map(|fps| {
            core::num::NonZeroU32::new(
                u32::try_from(fps).expect("the filtered positive rate fits a `u32`") * 1000,
            )
            .expect("a refresh rate of at least 1 Hz")
        })
}

/// The `tracing` filter this process runs with: `RUST_LOG` wins outright,
/// otherwise `WATERUI_LOG` names the level and the graphics stack stays at
/// `error`.
fn env_filter() -> tracing_subscriber::EnvFilter {
    use tracing_subscriber::EnvFilter;

    if let Ok(filter) = EnvFilter::try_from_default_env() {
        return filter;
    }
    let level = std::env::var(LOG_LEVEL_ENV).unwrap_or_else(|_| String::from("error"));
    EnvFilter::try_new(format!(
        "{level},wgpu_core=error,wgpu_hal=error,naga=error,metal=error"
    ))
    .unwrap_or_else(|error| panic!("{LOG_LEVEL_ENV}={level:?} is not a tracing level: {error}"))
}

/// Sends `tracing` records to `os_log`, plus stderr when `WATERUI_LOG` asked
/// for a level — a physical iOS device's unified log is unreachable from the
/// host, and `devicectl --console` only carries stderr.
fn init_tracing(inspector: Option<waterui::inspector::InspectorLayer>) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let console_layer = std::env::var_os(LOG_LEVEL_ENV).map(|_| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .without_time()
    });
    let native_layer = tracing_subscriber::fmt::layer()
        .with_writer(crate::native_log::NativeLog)
        .without_time()
        .with_ansi(false);
    tracing_subscriber::registry()
        .with(env_filter())
        .with(native_layer)
        .with(console_layer)
        .with(inspector)
        .init();
}
