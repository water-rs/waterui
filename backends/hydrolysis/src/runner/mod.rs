//! The hydrolysis runner: per-platform event loops around a shared
//! frame-driving core.
//!
//! - [`window`]: `FrameMode`/`RuntimeWindow` frame pump shared by all loops
//! - [`headless`]: pump-based runtime for tests and offscreen rendering
//! - [`winit_runner`] / [`web_runner`]: desktop and browser event loops
//! - [`fonts`]: resource font registration and CJK fallbacks
//! - [`diagnostics`]: opt-in frame timing reports

#[cfg(all(feature = "accessibility", hydrolysis_gpu_painter))]
use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
#[cfg(hydrolysis_gpu_painter)]
use executor_core::try_init_local_executor;
use nami::Signal as _;
#[cfg(not(target_arch = "wasm32"))]
use std::cell::Cell;
#[cfg(hydrolysis_gpu_painter)]
use std::collections::VecDeque;
use std::time::Duration;
use std::{cell::RefCell, rc::Rc};
#[cfg(hydrolysis_run)]
use waterui::app::App;
#[cfg(all(
    not(target_arch = "wasm32"),
    not(hydrolysis_winit),
    not(target_os = "android")
))]
use waterui::app::AppParts;
use waterui::component::table::TableConfig;
use waterui::window::Window;
use waterui::window::WindowManager;
#[cfg(hydrolysis_gpu_painter)]
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::Retain;
#[cfg(hydrolysis_gpu_painter)]
use waterui_core::handler::AnyViewBuilder;
use waterui_core::key::KeyPress;
use waterui_core::view::Hook;
#[cfg(hydrolysis_gpu_painter)]
use waterui_text::FontCollection;

// The JNI host exists only where a painter does: a painter-less Android
// build ships none of the session bridge the Kotlin side calls into.
#[cfg(all(target_os = "android", any(hydrolysis_gpu_painter, hydrolysis_hwui)))]
pub mod android;
/// The Android accessibility publish protocol — consecutive-tree event
/// diffing and explore-by-touch hit testing — compiled on Android for the
/// JNI bridge and on host for its tests; dead elsewhere.
#[cfg(all(
    feature = "accessibility",
    any(target_os = "android", all(test, not(target_arch = "wasm32")))
))]
pub mod android_accessibility;
/// The `HydrolysisSession` callback table — compiled on Android for the
/// JNI bridge and on host for the Kotlin-agreement test; dead elsewhere.
#[cfg(any(target_os = "android", all(test, not(target_arch = "wasm32"))))]
pub mod android_methods;
// Bare wasm has no window pump to drive these modules' diagnostics
// and menu-bar plumbing.
/// The insets-region metrics the Android session's `set_metrics`
/// change-detects on — compiled on Android for the session and on host
/// for its tests; dead elsewhere.
#[cfg(any(target_os = "android", all(test, not(target_arch = "wasm32"))))]
pub mod android_metrics;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod content_tests;
#[cfg_attr(all(target_arch = "wasm32", not(feature = "web")), allow(dead_code))]
#[cfg_attr(
    hydrolysis_hwui,
    expect(dead_code, reason = "the GPU pump is the sole diagnostics consumer")
)]
mod diagnostics;
/// The platform range-editing protocol state machine (UTF-16 ranges,
/// composition, selection) — compiled where a host speaks it and on host for
/// its tests; dead elsewhere.
#[cfg(any(hydrolysis_range_editing, all(test, not(target_arch = "wasm32"))))]
pub mod editing;
#[cfg(hydrolysis_gpu_painter)]
mod executor;
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
mod headless;
/// The host UI-thread executor and its wake — compiled on Android for the
/// JNI bridge and, because `eventfd` is Linux-only, on a Linux host for the
/// fd tests; dead elsewhere.
#[cfg(any(target_os = "android", all(test, target_os = "linux")))]
pub mod host_executor;
pub mod ime;
#[cfg_attr(all(target_arch = "wasm32", not(feature = "web")), allow(dead_code))]
pub mod menu_bar;
#[cfg(hydrolysis_winit)]
pub mod placement;
#[cfg(hydrolysis_gpu_painter)]
mod semantic;
/// The windowed runner's SIGINT/SIGTERM/SIGHUP contract on Unix — compiled
/// where the winit runner needs it, and in test builds for its child-process
/// suite, which runs without the `winit` feature.
#[cfg(all(unix, not(target_os = "android"), any(hydrolysis_winit, test)))]
mod termination;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_accessibility;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_runner;
// Bare wasm compiles the module for its profile types (the semantic runtime
// uses them everywhere) but has no window pump to call the rest.
// `pub` within the private `runner` module: the platform surfaces name
// `GpuSurfaceFrame` from outside `runner`.
#[cfg_attr(all(target_arch = "wasm32", not(feature = "web")), allow(dead_code))]
pub mod window;
#[cfg(hydrolysis_winit)]
mod winit_runner;
#[cfg(hydrolysis_wayland_platform)]
mod x11_state_watch;

use diagnostics::{RenderDiagnostics, RenderDiagnosticsConfig};
#[cfg(hydrolysis_gpu_painter)]
use diagnostics::{RenderPhaseSample, elapsed_or_zero};
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
use executor::{DrainExecutorOnDrop, HeadlessMainThreadExecutor};
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
pub use headless::{HeadlessPumpResult, HeadlessRuntime};
#[cfg(hydrolysis_gpu_painter)]
pub use semantic::{SemanticPumpResult, SemanticRuntime};
// Bare wasm has no window model until `web` compiles the browser runner.
#[cfg(all(
    any(not(target_arch = "wasm32"), feature = "web"),
    hydrolysis_gpu_painter
))]
use window::RuntimeWindow;
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
use window::{advance_runtime, handle_input_events};
// One frame's entry is shared by the winit pump, the headless `run`, the
// `#[cfg(test)]` suite, and the browser runner; the Android host reaches it
// through `window::` itself, so it stays out of this re-export.
#[cfg(any(
    hydrolysis_winit,
    all(test, not(target_arch = "wasm32")),
    all(not(target_arch = "wasm32"), not(target_os = "android")),
    all(target_arch = "wasm32", feature = "web")
))]
use window::render_window;
// Only the native headless/capture paths read frames back; the browser surface presents directly.
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
use window::{FrameReader, render_window_with_capture};
// `runtime_window_origin` is reached by the winit runner and by headless's
// accessibility-action path.
#[cfg(any(
    hydrolysis_winit,
    all(
        not(target_arch = "wasm32"),
        feature = "accessibility",
        hydrolysis_gpu_painter
    )
))]
use window::runtime_window_origin;
// The winit runner pumps events and window semantics through these.
#[cfg(hydrolysis_winit)]
use window::handle_input_events_with;
#[cfg(any(test, all(not(target_arch = "wasm32"), hydrolysis_winit)))]
use window::pump_window_semantics;
// Names the `#[cfg(test)]` suite pulls through `super::`; kept out of the
// unconditional import so non-test builds report no unused names.
#[cfg(all(test, any(not(target_arch = "wasm32"), feature = "web")))]
use window::{
    FrameMode, acquire_surface_frame, axes_whose_limits_changed, clamp_window_size,
    reports_ui_idle, schedule_animation_update, schedule_redraw_or_refresh,
    surface_error_requires_reconfigure,
};
// Frame and tree profiles are published to the inspector endpoint, which exists
// only where `waterui::inspector` does.
#[cfg(not(target_arch = "wasm32"))]
#[cfg(hydrolysis_gpu_painter)]
mod inspector;

#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
pub use window::HeadlessSnapshot;
#[cfg(hydrolysis_winit)]
pub use window::window_requires_transparency;
pub use window::{FrameCounters, FramePhases, FrameProfile};

use crate::env::{parse_bool_env, parse_optional_positive_u64_env, parse_positive_u64_env};
use crate::platform::{InputEvent, KeyState, PlatformWindow};
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
use crate::platform::{OffscreenGpuContext, OffscreenWindow};
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
use crate::readback::readback_surface_texture_rgba8;
#[cfg(hydrolysis_gpu_painter)]
use crate::renderer::FontFamilyResolution;
#[cfg(hydrolysis_gpu_painter)]
use crate::renderer::HydrolysisTextContextMenuMode;
use crate::renderer::{
    HydrolysisRenderer, HydrolysisWindowOrigin, KeyDelivery, KeyPressOutcome, SemanticCore,
};
use crate::renderer::{MenuShortcutRegistry, PopupWindowManager};
#[cfg(hydrolysis_gpu_painter)]
use crate::text::ParleyEngine;
use crate::time::Instant;

/// Subscribes every reactive input of a window declaration once, for the
/// window's whole lifetime: `title`, `frame`, `state`, `style`,
/// `background`, `level`, `attention`, `icon`, and `resize_increments`,
/// `min_size` and `max_size` when present.
///
/// Each subscription requests a refresh through
/// [`SemanticCore::refresh_watch`] — the same `FrameSignals::request_refresh`
/// wake every reactive read uses — so a binding an app flips while the
/// window is parked reaches the next pump, where `apply_properties`,
/// `apply_window_size_limits` and the accessibility root label re-read the
/// values. A per-frame watch cannot serve this: the frame watch registry
/// keeps a subscription for one unread frame, and an idle window runs no
/// frame to re-register it (water-rs/waterui#2131).
///
/// `background` is watched resolved, not raw: the resolved signal covers a
/// write to the declaration and everything the resolution reads — the theme
/// colour an `Opaque` or `Color` background resolves through — so a theme
/// switch repaints the clear colour of a window whose content reads no
/// theme token. `env`'s installed colour scheme is watched alongside: a
/// material background resolves to a constant, while its clear colour is
/// tinted in the scheme (water-rs/waterui#3548).
///
/// Both window kinds hold the returned guards as their last field so they
/// drop after the core — whose teardown owns every frame-scoped
/// subscription — in the outermost scope, the same tail position the
/// frame-level `lifecycle` teardown takes inside a flush (water-rs/waterui#1213).
///
/// The runtime's own writes land on the same flag — a resize writes
/// `frame`, so that write is one bounded extra frame, not a loop: the
/// frame the flag arms performs no further write to the declaration.
///
/// `env` must carry the installed theme — colour scheme and colour slots —
/// the resolved background reads; every runner installs it before mounting
/// a window.
fn subscribe_window_declaration_signals<E: crate::text::TextEngine>(
    window: &Window,
    env: &Environment,
    core: &SemanticCore<E>,
) -> Vec<Retain> {
    let mut watches = vec![
        core.refresh_watch(&window.title),
        core.refresh_watch(&window.frame),
        core.refresh_watch(&window.state),
        core.refresh_watch(&window.style),
        core.refresh_watch(&window.resolved_background(env)),
        core.refresh_watch(&waterui::theme::current_color_scheme(env)),
        core.refresh_watch(&window.level),
        core.refresh_watch(&window.attention),
        core.refresh_watch(&window.icon),
    ];
    for signal in [
        &window.resize_increments,
        &window.min_size,
        &window.max_size,
    ]
    .into_iter()
    .flatten()
    {
        watches.push(core.refresh_watch(signal));
    }
    watches
}

/// The global executor every runner installs before anything can spawn.
fn init_global_executor() {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
}

/// Installs the global executor and prepares inspection.
///
/// A browser page has no transport for the inspector endpoint, so the wasm
/// runner installs the executor alone.
#[cfg(not(target_arch = "wasm32"))]
fn init_main_thread_executors() -> Option<waterui::inspector::InspectorRuntime> {
    init_global_executor();
    waterui::inspector::maybe_init_from_env("hydrolysis")
}

/// Physical pixels per logical pixel for offscreen rendering, from
/// `WATERUI_HYDROLYSIS_OFFSCREEN_SCALE`.
///
/// Offscreen output is usually viewed on a `HiDPI` display (a preview image in
/// docs, a snapshot opened on a laptop), where rendering one physical pixel per
/// logical pixel looks soft. Defaults to 2.
#[cfg(all(
    not(target_arch = "wasm32"),
    not(hydrolysis_winit),
    not(target_os = "android")
))]
fn offscreen_scale_factor() -> f64 {
    const VARIABLE: &str = "WATERUI_HYDROLYSIS_OFFSCREEN_SCALE";
    const DEFAULT: f64 = 2.0;

    let Some(raw) = std::env::var_os(VARIABLE) else {
        return DEFAULT;
    };
    let raw = raw
        .to_str()
        .unwrap_or_else(|| panic!("{VARIABLE} must be valid UTF-8"));
    let parsed: f64 = raw
        .parse()
        .unwrap_or_else(|error| panic!("{VARIABLE} must be a number, got {raw:?}: {error}"));
    assert!(
        parsed.is_finite() && parsed > 0.0,
        "{VARIABLE} must be finite and positive, got {parsed}"
    );
    parsed
}

/// Hooks for the platform services this renderer itself provides.
///
/// The self-drawn realizations of semantic components — the GPU video player,
/// the vector map — are not among them: which realization draws a component is
/// the application's choice, installed by `waterui::app::App` from the
/// `video-gpu` / `map-gpu` features. What this renderer installs are the
/// platform's own primitives it bridges: the system web view on macOS and the
/// `<video>` element on the web.
fn install_native_component_hooks(env: &mut Environment) {
    crate::localization::install(env);
    // The only web engine this backend knows about is the platform's own: a
    // browser engine an application links installs its realization itself.
    #[cfg(hydrolysis_system_webview)]
    crate::widgets::platform::webview::install_controller(env);
    // The page's own `<video>` element is the web's native player.
    #[cfg(all(target_arch = "wasm32", feature = "web", feature = "video"))]
    crate::widgets::platform::video::install(env);
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}

fn install_headless_window_managers<E: crate::text::TextEngine>(
    env: &mut Environment,
    pending_windows: Rc<RefCell<Vec<Window>>>,
) {
    env.insert(WindowManager::new({
        let pending_windows = Rc::clone(&pending_windows);
        move |window| {
            pending_windows.borrow_mut().push(window);
        }
    }));
    env.insert(PopupWindowManager::new(move |window| {
        pending_windows.borrow_mut().push(window);
    }));
    let _ = env.get_or_insert_with::<MenuShortcutRegistry<E>, _>(MenuShortcutRegistry::default);
}

// The headless offscreen `run` never exists on Android: `hydrolysis::run`
// must not silently dispatch an app to the one-shot offscreen pump there.
// The Android host's entry point is `runner::android` — a missing `run` is a
// compile error naming the real boundary, which is the plan's "no accidental
// Android-to-headless dispatch" made mechanical.
#[cfg(all(
    not(target_arch = "wasm32"),
    not(hydrolysis_winit),
    not(target_os = "android")
))]
/// Runs `app` once offscreen and returns — the headless one-shot `run` that
/// exists only where no platform runner can claim the entry point.
///
/// `app` is a factory, not a built value: the subscriber and executors are
/// installed before it runs, so events the application emits while building
/// its `App` reach the log (water-rs/waterui#2513).
///
/// # Panics
/// Panics when the seeded environment lost its `MenuShortcutRegistry`.
pub fn run(app: impl FnOnce(Environment) -> App, env: Environment, style: impl crate::Style) {
    init_tracing();
    let inspector = init_main_thread_executors();
    let app = app(env);
    let inspector_probe = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::runtime_probe);
    // This path renders each window once offscreen and returns; it owns no event
    // loop, so it supplies the headless executor rather than a platform one.
    // Keep a handle on the executor: the render loop below has to drive it, or
    // any async work a view starts (a `GpuView`'s `setup`, above all) never
    // completes and the frame is rendered against uninitialized state.
    let local_executor = executor::HeadlessMainThreadExecutor::thread_shared();
    // Declared before the runtime state below so it drops after it: closing
    // and draining the shared queue while this thread's locals are intact is
    // what keeps a still-queued task out of thread-local teardown (#332).
    let _executor_teardown = executor::DrainExecutorOnDrop::new(local_executor.clone());
    // This host paces frames itself rather than vsyncing against a panel, so
    // the executor budgets at the headless rate.
    let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
        local_executor.clone(),
        waterui::task::RefreshRate::HEADLESS,
        inspector_probe,
    ));

    // Locale changes reach views through a mailbox, whose pump needs the
    // executor installed just above.
    waterui_locale::start_system_locale_listener();
    // This host renders each window once and returns, so no window's closing
    // ends it and the last-window policy has nothing to decide. Its lifetime
    // is the pump's, not the app's, so the termination machine is never
    // started.
    let AppParts {
        windows,
        menu_bar,
        env,
        last_window: _,
        termination: _,
    } = app.into_parts();
    let mut env = env.extending(waterui_graphics::scene_view::SceneViewMergeToParent);
    waterui_core::install_application_resources(&mut env);
    waterui::inspector::install(&mut env, inspector);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    install_native_component_hooks(&mut env);
    install_headless_window_managers::<crate::text::ParleyEngine>(
        &mut env,
        Rc::clone(&pending_window_queue),
    );
    // App-level menu bar: arms its chords on the shared registry. The
    // headless host renders no chrome, so there is no surface to draw.
    menu_bar::register_menu_bar::<crate::text::ParleyEngine>(&menu_bar, &env);
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    // A headless host has no platform appearance: the scheme is Light
    // unless the application installs its own.
    crate::theme::install_theme_tokens(&mut env, Some(&style), waterui::theme::ColorScheme::Light);
    let theme: Rc<dyn crate::theme::WidgetTheme> = Rc::new(style);
    // The application's fonts, discovered once. Every window's renderer is
    // seeded from this collection, and a self-drawn component that typesets
    // text itself reads it out of the environment instead of enumerating the
    // system's fonts for itself.
    let fonts = crate::text::fonts::native_collection(&env);
    fonts.clone().install(&mut env);
    // The capture service mounts its subtree under a snapshot of the
    // assembled environment — the theme tokens, hooks and font collection a
    // window's subtree resolves — so it must be installed last
    // (water-rs/waterui#2703).
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::new(
            Rc::clone(&theme),
            env.clone(),
            FontFamilyResolution::Lenient,
        ),
    ));
    let shortcuts = env
        .get::<MenuShortcutRegistry<crate::text::ParleyEngine>>()
        .expect("install_headless_window_managers seeds MenuShortcutRegistry")
        .clone();
    let mut pending_windows = VecDeque::from(windows);
    while let Some(window) = pending_windows.pop_front() {
        let frame = crate::platform::validated_window_frame(window.frame.snapshot());
        let width = crate::num_cast::f32_as_u32(frame.width().max(1.0));
        let height = crate::num_cast::f32_as_u32(frame.height().max(1.0));
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm)
            .with_scale_factor(offscreen_scale_factor());
        platform.apply_properties(&window);
        let mut renderer = HydrolysisRenderer::with_engine(
            Rc::clone(&theme),
            ParleyEngine::from_collection(&fonts, FontFamilyResolution::Lenient),
        );
        renderer.set_window_id(shortcuts.mint_window_id());
        renderer.set_window_closable(window.closable);
        let mut runtime =
            RuntimeWindow::new_gpu(window, platform, renderer, render_diagnostics_config, &env);
        render_window(&mut runtime, &env, &mut || local_executor.drain());
        pending_windows.extend(pending_window_queue.borrow_mut().drain(..));
    }
}

/// Runs the application in the browser, drawing onto the page's canvas.
///
/// Available only when compiling hydrolysis for wasm32 with the `web` feature.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
/// Runs `app` on the web runner for this wasm build.
///
/// `app` is a factory, not a built value: the panic hook and executor are
/// installed before it runs, matching every other `run` (see
/// water-rs/waterui#2513).
///
/// # Panics
/// Propagates panics from `web_runner::run`.
pub fn run(app: impl FnOnce(Environment) -> App, env: Environment, style: impl crate::Style) {
    // The page has no stderr, so the default hook's report goes nowhere;
    // this one sends the message and location to `console.error`, and shows
    // it on the page: a panic traps the wasm instance, so the app is gone and
    // the page says why instead of freezing on its last frame.
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        crate::platform::report_fatal_error(&info.to_string());
    }));
    init_global_executor();
    let app = app(env);
    web_runner::run(app, style);
}

// The plan of record's Android host is the Kotlin `HydrolysisHostView`, not
// winit (no NativeActivity/GameActivity): `run` is absent on Android under
// every feature combination, so a winit-enabled Android build cannot
// silently dispatch to a windowing model the host does not have.
/// Runs `app` on the winit runner.
///
/// `app` is a factory, not a built value: the tracing subscriber is
/// installed before it runs, so events the application emits while building
/// its `App` reach the log (water-rs/waterui#2513).
///
/// # Panics
/// Propagates panics from `winit_runner::run` and tracing initialization.
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_winit))]
pub fn run(app: impl FnOnce(Environment) -> App, env: Environment, style: impl crate::Style) {
    init_tracing();
    let executors = init_main_thread_executors();
    let app = app(env);
    winit_runner::run(app, style, executors);
}

/// Installs the process-wide tracing subscriber when `RUST_LOG` is set:
/// its directives, to stderr.
///
/// Entry points that run application code outside [`run`] — preview,
/// preview-test and MCP runtimes mount the app through `waterui-testing`
/// hosts — call it before that code executes. Idempotent: the first install
/// wins, so a caller that already set a subscriber (or a `run` that follows
/// an earlier `init_tracing`) is not disturbed. Android installs its logcat
/// writer at `nativeInit` instead, so this function does not exist there.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn init_tracing() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
}
