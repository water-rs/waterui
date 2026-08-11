//! The hydrolysis runner: per-platform event loops around a shared
//! frame-driving core.
//!
//! - [`window`]: `FrameMode`/`RuntimeWindow` frame pump shared by all loops
//! - [`headless`]: pump-based runtime for tests and offscreen rendering
//! - [`winit_runner`] / [`web_runner`]: desktop and browser event loops
//! - [`fonts`]: resource font registration and CJK fallbacks
//! - [`diagnostics`]: opt-in frame timing reports

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;
#[cfg(feature = "winit")]
#[cfg(target_os = "linux")]
use std::{process::Command, str};

#[cfg(feature = "accessibility")]
use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
use executor_core::{
    LocalExecutor,
    async_task::{AsyncTask, Runnable},
    try_init_local_executor,
};
use nami::Signal as _;
use waterui::app::App;
use waterui::component::table::TableConfig;
use waterui::graphics::Color;
use waterui::theme;
use waterui::window::WindowManager;
use waterui::window::{Window, WindowBackground};
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::view::Hook;
use waterui_map::MapConfig;

mod diagnostics;
mod fonts;
#[cfg(not(target_arch = "wasm32"))]
mod headless;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_runner;
mod window;
#[cfg(feature = "winit")]
mod winit_runner;

use diagnostics::*;
#[cfg(not(target_arch = "wasm32"))]
use fonts::*;
#[cfg(not(target_arch = "wasm32"))]
pub use headless::{HeadlessPumpResult, HeadlessRuntime};
use window::*;
mod inspector;

pub use window::{FrameCounters, FramePhases, FrameProfile, HeadlessSnapshot};

use crate::env::{parse_bool_env, parse_positive_u64_env};
use crate::platform::OffscreenWindow;
use crate::platform::{InputEvent, KeyState, PlatformWindow};
#[cfg(not(target_arch = "wasm32"))]
use crate::readback::readback_texture_rgba8;
use crate::renderer::{
    HydrolysisRenderer, HydrolysisTextContextMenuMode, HydrolysisWindowOrigin, PopupWindowManager,
};
use crate::time::Instant;

fn init_main_thread_executors() -> Option<waterui::inspector::InspectorRuntime> {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    waterui::inspector::maybe_init_from_env()
}

/// Physical pixels per logical pixel for offscreen rendering, from
/// `WATERUI_HYDROLYSIS_OFFSCREEN_SCALE`.
///
/// Offscreen output is usually viewed on a HiDPI display (a preview image in
/// docs, a snapshot opened on a laptop), where rendering one physical pixel per
/// logical pixel looks soft. Defaults to 2.
#[cfg(all(not(target_arch = "wasm32"), not(feature = "winit")))]
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

fn install_native_component_hooks(env: &mut Environment) {
    waterui_video_gpu::install(env);
    #[cfg(any(test, feature = "testing"))]
    let semantic_native_map = env.get::<crate::testing::SemanticNativeMap>().is_some();
    #[cfg(not(any(test, feature = "testing")))]
    let semantic_native_map = false;
    if env.get::<Hook<MapConfig>>().is_none() && !semantic_native_map {
        waterui_map_gpu::install(env);
    }
    #[cfg(all(hydrolysis_webview, not(hydrolysis_cef_webview)))]
    crate::widgets::platform::webview::install_controller(env);
    #[cfg(all(any(hydrolysis_cef_webview, feature = "chromium"), not(test)))]
    crate::widgets::platform::browser_cef::install_runtime(env);
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}

#[cfg(not(target_arch = "wasm32"))]
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
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "winit")))]
pub fn run(app: App) {
    let inspector = init_main_thread_executors();
    let inspector_probe = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::runtime_probe);
    // This path renders each window once offscreen and returns; it owns no event
    // loop, so it supplies the headless executor rather than a platform one.
    // Keep a handle on the executor: the render loop below has to drive it, or
    // any async work a view starts (a `GpuView`'s `setup`, above all) never
    // completes and the frame is rendered against uninitialized state.
    let local_executor = headless::HeadlessMainThreadExecutor::default();
    let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
        local_executor.clone(),
        inspector_probe,
    ));
    let (windows, _menu_bar, env) = app.into_parts();
    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    waterui::inspector::install(&mut env, inspector);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    install_native_component_hooks(&mut env);
    install_headless_window_managers(&mut env, Rc::clone(&pending_window_queue));
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::default(),
    ));
    let mut pending_windows = VecDeque::from(windows);
    while let Some(window) = pending_windows.pop_front() {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm)
            .with_scale_factor(offscreen_scale_factor());
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_native_resource_fonts(&mut renderer);
        let mut runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
        render_window(&mut runtime, &env, &mut || local_executor.drain());
        pending_windows.extend(pending_window_queue.borrow_mut().drain(..));
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn run(app: App) {
    web_runner::run(app, init_main_thread_executors());
}

#[cfg(all(not(target_arch = "wasm32"), feature = "winit"))]
pub fn run(app: App) {
    #[cfg(all(target_os = "macos", any(hydrolysis_cef_webview, feature = "chromium")))]
    waterui_browser_cef::initialize_macos_application();
    initialize_tracing_from_env();
    winit_runner::run(app, init_main_thread_executors());
}

#[cfg(all(not(target_arch = "wasm32"), feature = "winit"))]
fn initialize_tracing_from_env() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}
