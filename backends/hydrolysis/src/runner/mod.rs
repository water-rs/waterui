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
pub use window::{FrameCounters, FramePhases, FrameProfile, HeadlessSnapshot};
use window::*;

use crate::env::{parse_bool_env, parse_positive_u64_env};
#[cfg(not(target_arch = "wasm32"))]
use crate::readback::readback_texture_rgba8;
use crate::platform::OffscreenWindow;
use crate::platform::{InputEvent, KeyState, PlatformWindow};
use crate::renderer::{HydrolysisRenderer, HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};
use crate::time::Instant;

fn init_main_thread_executors() {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = waterui::inspector::maybe_init_from_env();
}

fn install_native_component_hooks(env: &mut Environment) {
    waterui_video::install_rust_player_hooks(env);
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "winit")))]
fn install_window_manager(env: &mut Environment, pending_windows: Rc<RefCell<Vec<Window>>>) {
    env.insert(WindowManager::new(move |window| {
        pending_windows.borrow_mut().push(window);
    }));
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "winit")))]
pub fn run(app: App) {
    init_main_thread_executors();
    let (windows, _menu_bar, env) = app.into_parts();
    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    install_native_component_hooks(&mut env);
    install_window_manager(&mut env, Rc::clone(&pending_window_queue));
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::default(),
    ));
    let mut pending_windows = VecDeque::from(windows);
    while let Some(window) = pending_windows.pop_front() {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_native_resource_fonts(&mut renderer);
        let mut runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
        render_window(&mut runtime, &env, &mut || false);
        pending_windows.extend(pending_window_queue.borrow_mut().drain(..));
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn run(app: App) {
    init_main_thread_executors();
    web_runner::run(app);
}

#[cfg(all(not(target_arch = "wasm32"), feature = "winit"))]
pub fn run(app: App) {
    initialize_tracing_from_env();
    init_main_thread_executors();
    winit_runner::run(app);
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
