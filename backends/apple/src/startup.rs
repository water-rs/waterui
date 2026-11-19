//! Process startup: the work `waterui_init` used to do, minus the FFI.
//!
//! Runs once, on the main thread, before the environment exists: ignore
//! `SIGPIPE`, attach the inspector when the environment asks for one, route
//! panics and records through `tracing`, install the global and the
//! main-queue local executors, then start the system-locale listener (its
//! mailbox pump needs the executor that was just installed).

use alloc::boxed::Box;

#[cfg(feature = "inspector")]
use waterui::inspector::InspectorRuntime;

#[cfg(feature = "inspector")]
use waterui_backend_core::log::{LOG_LEVEL_ENV, env_filter};

/// One-time process startup. Returns the inspector runtime when the
/// environment asked for one, for [`crate::entry`] to install.
#[cfg(feature = "inspector")]
pub fn initialize() -> Option<InspectorRuntime> {
    initialize_inner(false)
}

/// One-time process startup without inspector instrumentation.
#[cfg(not(feature = "inspector"))]
pub fn initialize() {
    initialize_inner();
}

/// [`initialize`] for the preview entry: identical bring-up, plus the
/// stderr console layer `WATERUI_LOG` would have asked for —
/// [`crate::preview`]'s contract is that a panic message and any error
/// reach stderr without it.
///
/// Exactly once per process: a `preview::run` performs the whole
/// bring-up and returns the inspector runtime; a second call in one
/// process is a bug, and the panic hook, subscriber and executor
/// installs inside make it fail the same loud way a repeated
/// [`initialize`] does.
#[cfg(all(preview_entry, feature = "inspector"))]
pub fn initialize_for_preview() -> Option<InspectorRuntime> {
    initialize_inner(true)
}

/// [`initialize`] for the preview entry without inspector instrumentation.
#[cfg(all(preview_entry, not(feature = "inspector")))]
pub fn initialize_for_preview() {
    initialize_inner();
}

fn install_process_base() {
    ignore_sigpipe();
    std::panic::set_hook(Box::new(|info| {
        tracing_panic::panic_hook(info);
    }));
}

fn install_executors(#[cfg(feature = "inspector")] inspector: Option<&InspectorRuntime>) {
    executor_core::init_global_executor(native_executor::NativeExecutor::new());
    // Without the main executor nothing the app declares can run, so the
    // app cannot launch: the failure is logged and ends the process.
    let Some(main_executor) = native_executor::NativeMainExecutor::new() else {
        tracing::error!(
            "the main-thread executor is unavailable off the platform main thread; \
             the app cannot launch"
        );
        std::process::exit(1);
    };
    #[cfg(feature = "inspector")]
    let local_executor = waterui::task::monitored_local_executor_with_probes(
        main_executor,
        display_refresh_rate(),
        inspector.map(InspectorRuntime::runtime_probe),
    );
    #[cfg(not(feature = "inspector"))]
    let local_executor =
        waterui::task::monitored_local_executor(main_executor, display_refresh_rate());
    executor_core::init_local_executor(local_executor);

    // The listener's mailbox pump needs the executor installed above.
    waterui_locale::start_system_locale_listener();
}

/// One-time process startup. Returns the inspector runtime when the
/// environment asked for one, for [`crate::entry`] to install.
#[cfg(feature = "inspector")]
fn initialize_inner(stderr: bool) -> Option<InspectorRuntime> {
    install_process_base();
    let inspector = waterui::inspector::maybe_init_from_env("apple");
    init_tracing(
        inspector.as_ref().map(InspectorRuntime::tracing_layer),
        stderr,
    );
    install_executors(inspector.as_ref());
    inspector
}

#[cfg(not(feature = "inspector"))]
fn initialize_inner() {
    install_process_base();
    install_executors();
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
            "`signal(SIGPIPE, SIG_IGN)` cannot fail: SIGPIPE is a valid, catchable signal"
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

    let mtm = MainThreadMarker::new().expect(
        "`initialize` runs after the main-thread executor was installed, on the main thread",
    );
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

    let mtm = MainThreadMarker::new().expect(
        "`initialize` runs after the main-thread executor was installed, on the main thread",
    );
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

/// Sends `tracing` records to `os_log`, plus stderr when `WATERUI_LOG` asked
/// for a level — a physical iOS device's unified log is unreachable from the
/// host, and `devicectl --console` only carries stderr. `stderr` forces the
/// console layer on regardless — the preview entry's contract.
#[cfg(feature = "inspector")]
fn init_tracing(inspector: Option<waterui::inspector::InspectorLayer>, stderr: bool) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let console_layer = (stderr || std::env::var_os(LOG_LEVEL_ENV).is_some()).then(|| {
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
