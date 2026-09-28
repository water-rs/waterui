//! The hydrolysis runner: per-platform event loops around a shared
//! frame-driving core.
//!
//! - [`window`]: `FrameMode`/`RuntimeWindow` frame pump shared by all loops
//! - [`headless`]: pump-based runtime for tests and offscreen rendering
//! - [`winit_runner`] / [`web_runner`]: desktop and browser event loops
//! - [`fonts`]: resource font registration and CJK fallbacks
//! - [`diagnostics`]: opt-in frame timing reports

#[cfg(feature = "accessibility")]
use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
use executor_core::try_init_local_executor;
use nami::Signal as _;
#[cfg(not(target_arch = "wasm32"))]
use std::cell::Cell;
use std::time::Duration;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
#[cfg(any(
    all(not(target_arch = "wasm32"), not(target_os = "android")),
    all(target_arch = "wasm32", feature = "web")
))]
use waterui::app::App;
#[cfg(all(not(target_arch = "wasm32"), not(feature = "winit")))]
use waterui::app::AppParts;
use waterui::component::table::TableConfig;
use waterui::graphics::Color;
use waterui::theme;
use waterui::window::WindowManager;
use waterui::window::{Window, WindowBackground};
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::key::KeyPress;
use waterui_core::view::Hook;
use waterui_text::FontCollection;

#[cfg(target_os = "android")]
pub mod android;
mod diagnostics;
mod executor;
mod fonts;
#[cfg(not(target_arch = "wasm32"))]
mod headless;
pub(crate) mod ime;
pub(crate) mod menu_bar;
mod semantic;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_accessibility;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_runner;
mod window;
#[cfg(feature = "winit")]
mod winit_runner;

use diagnostics::*;
#[cfg(not(target_arch = "wasm32"))]
use executor::*;
use fonts::*;
#[cfg(not(target_arch = "wasm32"))]
pub use headless::{HeadlessPumpResult, HeadlessRuntime};
pub use semantic::{SemanticPumpResult, SemanticRuntime};
use window::*;
// Frame and tree profiles are published to the inspector endpoint, which exists
// only where `waterui::inspector` does.
#[cfg(not(target_arch = "wasm32"))]
mod inspector;

#[cfg(not(target_arch = "wasm32"))]
pub use window::HeadlessSnapshot;
#[cfg(feature = "winit")]
pub(crate) use window::window_requires_transparency;
pub use window::{FrameCounters, FramePhases, FrameProfile};

use crate::env::{parse_bool_env, parse_optional_positive_u64_env, parse_positive_u64_env};
#[cfg(not(target_arch = "wasm32"))]
use crate::platform::GpuSurfaceWindow;
use crate::platform::{InputEvent, KeyState, PlatformWindow};
#[cfg(not(target_arch = "wasm32"))]
use crate::platform::{OffscreenGpuContext, OffscreenWindow};
#[cfg(not(target_arch = "wasm32"))]
use crate::readback::readback_texture_rgba8;
use crate::renderer::{HydrolysisRenderer, HydrolysisWindowOrigin, KeyDelivery, KeyPressOutcome};
use crate::renderer::{HydrolysisTextContextMenuMode, MenuShortcutRegistry, PopupWindowManager};
use crate::time::Instant;

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
/// Offscreen output is usually viewed on a HiDPI display (a preview image in
/// docs, a snapshot opened on a laptop), where rendering one physical pixel per
/// logical pixel looks soft. Defaults to 2.
#[cfg(all(
    not(target_arch = "wasm32"),
    not(feature = "winit"),
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
/// `video-gpu` / `map-gpu` features, so this renderer never names a component
/// crate.
fn install_native_component_hooks(env: &mut Environment) {
    crate::localization::install(env);
    // The only web engine this backend knows about is the platform's own: a
    // browser engine an application links installs its realization itself.
    #[cfg(hydrolysis_macos_system_webview)]
    crate::widgets::platform::webview::install_controller(env);
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}

fn install_headless_window_managers(
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
    let _ = env.get_or_insert_with::<MenuShortcutRegistry, _>(MenuShortcutRegistry::default);
}

// The headless offscreen `run` never exists on Android: `hydrolysis::run`
// must not silently dispatch an app to the one-shot offscreen pump there.
// The Android host's entry point is `runner::android` — a missing `run` is a
// compile error naming the real boundary, which is the plan's "no accidental
// Android-to-headless dispatch" made mechanical.
#[cfg(all(
    not(target_arch = "wasm32"),
    not(feature = "winit"),
    not(target_os = "android")
))]
pub fn run(app: App, style: impl crate::Style) {
    let inspector = init_main_thread_executors();
    let inspector_probe = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::runtime_probe);
    // This path renders each window once offscreen and returns; it owns no event
    // loop, so it supplies the headless executor rather than a platform one.
    // Keep a handle on the executor: the render loop below has to drive it, or
    // any async work a view starts (a `GpuView`'s `setup`, above all) never
    // completes and the frame is rendered against uninitialized state.
    let local_executor = executor::HeadlessMainThreadExecutor::thread_shared();
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
    // ends it and the last-window policy has nothing to decide.
    let AppParts {
        windows,
        menu_bar,
        env,
        last_window: _,
    } = app.into_parts();
    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    waterui::inspector::install(&mut env, inspector);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    install_native_component_hooks(&mut env);
    install_headless_window_managers(&mut env, Rc::clone(&pending_window_queue));
    // App-level menu bar: arms its chords on the shared registry. The
    // headless host renders no chrome, so there is no surface to draw.
    menu_bar::register_menu_bar(&menu_bar, &env);
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    crate::theme::install_theme_tokens(&mut env, Some(&style));
    let theme: Rc<dyn crate::engine::WidgetTheme> = Rc::new(style);
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::new(Rc::clone(&theme)),
    ));
    // The application's fonts, discovered once. Every window's renderer is
    // seeded from this collection, and a self-drawn component that typesets
    // text itself reads it out of the environment instead of enumerating the
    // system's fonts for itself.
    let fonts = FontCollection::new(native_resource_fonts());
    fonts.clone().install(&mut env);
    let shortcuts = env
        .get::<MenuShortcutRegistry>()
        .expect("install_headless_window_managers seeds MenuShortcutRegistry")
        .clone();
    let mut pending_windows = VecDeque::from(windows);
    while let Some(window) = pending_windows.pop_front() {
        let frame = crate::platform::validated_window_frame(window.frame.snapshot());
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm)
            .with_scale_factor(offscreen_scale_factor());
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.adapter(), surface.device(), Rc::clone(&theme))
        };
        seed_core(&mut renderer, &fonts);
        renderer.set_window_id(shortcuts.mint_window_id());
        let mut runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
        render_window(&mut runtime, &env, &mut || local_executor.drain());
        pending_windows.extend(pending_window_queue.borrow_mut().drain(..));
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn run(app: App, style: impl crate::Style) {
    init_global_executor();
    web_runner::run(app, style);
}

// The plan of record's Android host is the Kotlin `HydrolysisHostView`, not
// winit (no NativeActivity/GameActivity): `run` is absent on Android under
// every feature combination, so a winit-enabled Android build cannot
// silently dispatch to a windowing model the host does not have.
#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "winit",
    not(target_os = "android")
))]
pub fn run(app: App, style: impl crate::Style) {
    initialize_tracing_from_env();
    winit_runner::run(app, style, init_main_thread_executors());
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "winit",
    not(target_os = "android")
))]
fn initialize_tracing_from_env() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}
