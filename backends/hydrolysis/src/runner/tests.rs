use super::headless::HeadlessPlatformWindow;
use super::{
    RenderDiagnosticsConfig, RuntimeWindow, advance_runtime, schedule_redraw_or_refresh,
    schedule_scroll_refresh,
};
use crate::platform::PlatformWindow as _;
use crate::renderer::HydrolysisRenderer;
use core::time::Duration;
use std::time::Instant;
use waterui::window::{Window, WindowState};
use waterui_backend_core::widget::TextCaretMotion;
use waterui_core::{Environment, binding};

#[test]
fn changed_redraw_only_input_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();

    schedule_redraw_or_refresh(&mut runtime, true);

    assert!(!runtime.mode.is_pending());
    assert!(runtime.renderer.take_redraw_request());
    assert!(runtime.platform.take_redraw_request());
}

#[test]
fn changed_rebuild_input_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();
    runtime.renderer.request_rebuild();

    schedule_redraw_or_refresh(&mut runtime, true);

    assert!(runtime.mode.is_pending());
    assert!(
        runtime.platform.take_redraw_request(),
        "rebuild input must wake the platform event loop for the next frame"
    );
}

#[test]
fn changed_scroll_input_schedules_frame_and_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();

    // Scrolling re-composites the retained frame via a refresh (the first pump
    // builds the tree if it does not exist yet).
    schedule_scroll_refresh(&mut runtime, true);

    assert!(runtime.mode.is_pending());
    assert!(
        runtime.platform.take_redraw_request(),
        "scroll input must wake the platform event loop for the next frame"
    );
}

#[test]
fn text_caret_tick_wakes_redraw_without_layout_rebuild() {
    let mut runtime = test_runtime_window();
    let now = Instant::now();
    let motion = TextCaretMotion {
        fade_cycle_duration: Duration::from_millis(1_000),
        frame_interval: Duration::from_millis(16),
        min_opacity: 0.2,
    };
    runtime.renderer.set_frame_instant(now);
    runtime.renderer.set_text_caret_motion(motion);
    assert!(runtime.renderer.set_focused_text_input(Some(0)));
    assert!(runtime.renderer.take_patch_request());
    assert!(!runtime.renderer.take_rebuild_request());
    runtime.clear_frame_mode();
    assert!(!runtime.platform.take_redraw_request());

    let deadline = now
        .checked_add(motion.frame_interval)
        .expect("test caret deadline overflow");
    let env = Environment::new();

    assert!(advance_runtime(&mut runtime, &env, deadline).is_some());
    assert!(!runtime.mode.is_pending());
    assert!(runtime.renderer.take_redraw_request());
    assert!(runtime.platform.take_redraw_request());
}

/// The window's effective size limits reach the platform: the content's
/// measured minimum is the default resize floor, and an explicit
/// `Window::min_size`/`max_size` overrides it.
#[test]
fn window_size_limits_reach_the_platform_window() {
    use waterui_core::layout::Size;
    use waterui_layout::frame::Frame;

    // Content with a hard 200x100 minimum: the derived window minimum must be
    // at least that big (an ideal-sized frame compresses at a zero proposal,
    // so only min constraints establish a resize floor).
    let content = || Frame::new(()).min_width(200.0).min_height(100.0);
    let window = Window::new("", binding(WindowState::Normal), content);
    let mut runtime = runtime_window_for(window);
    let env = Environment::new();
    let _ = super::pump_window_semantics(&mut runtime, &env);
    let (min, max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    let min = min.expect("content-derived minimum must exist after a build");
    assert!(
        min.width >= 200.0 && min.height >= 100.0,
        "derived minimum {min:?} must cover the content's fixed frame"
    );
    assert_eq!(max, None, "no explicit maximum leaves the window unbounded");

    // Explicit limits override the derived minimum.
    let window = Window::new("", binding(WindowState::Normal), content)
        .min_size(Size::new(300.0, 150.0))
        .max_size(Size::new(640.0, 480.0));
    let mut runtime = runtime_window_for(window);
    let _ = super::pump_window_semantics(&mut runtime, &env);
    let (min, max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    assert_eq!(min, Some(Size::new(300.0, 150.0)));
    assert_eq!(max, Some(Size::new(640.0, 480.0)));
}

fn runtime_window_for(window: Window) -> RuntimeWindow<HeadlessPlatformWindow> {
    let mut platform =
        HeadlessPlatformWindow::new_for_tests(16, 16, wgpu::TextureFormat::Rgba8Unorm);
    platform.apply_properties(&window);
    let renderer = {
        let surface = platform.surface();
        HydrolysisRenderer::new(surface.device())
    };
    RuntimeWindow::new(
        window,
        platform,
        renderer,
        RenderDiagnosticsConfig {
            enabled: false,
            interval: Duration::from_secs(1),
            slow_frame_threshold_override: None,
        },
    )
}

fn test_runtime_window() -> RuntimeWindow<HeadlessPlatformWindow> {
    let window = Window::new("", binding(WindowState::Normal), || ());
    let mut platform =
        HeadlessPlatformWindow::new_for_tests(16, 16, wgpu::TextureFormat::Rgba8Unorm);
    platform.apply_properties(&window);
    let renderer = {
        let surface = platform.surface();
        HydrolysisRenderer::new(surface.device())
    };
    RuntimeWindow::new(
        window,
        platform,
        renderer,
        RenderDiagnosticsConfig {
            enabled: false,
            interval: Duration::from_secs(1),
            slow_frame_threshold_override: None,
        },
    )
}
