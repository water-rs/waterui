use super::headless::HeadlessPlatformWindow;
use super::{
    RenderDiagnosticsConfig, RuntimeWindow, advance_runtime, schedule_redraw_or_rebuild,
    schedule_scroll_scene_rebuild,
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

    schedule_redraw_or_rebuild(&mut runtime, true);

    assert!(!runtime.mode.is_rebuild());
    assert!(runtime.renderer.take_redraw_request());
    assert!(runtime.platform.take_redraw_request());
}

#[test]
fn changed_rebuild_input_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();
    runtime.renderer.request_rebuild();

    schedule_redraw_or_rebuild(&mut runtime, true);

    assert!(runtime.mode.is_rebuild());
    assert!(
        runtime.platform.take_redraw_request(),
        "rebuild input must wake the platform event loop for the next frame"
    );
}

#[test]
fn changed_scroll_input_schedules_frame_and_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();

    // With no retained window frame yet, a scroll schedules a structural rebuild;
    // once a frame is retained, scrolling re-composites it via a window refresh.
    schedule_scroll_scene_rebuild(&mut runtime, true);

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
    assert!(runtime.renderer.take_rebuild_request());
    runtime.clear_frame_mode();
    assert!(!runtime.platform.take_redraw_request());

    let deadline = now
        .checked_add(motion.frame_interval)
        .expect("test caret deadline overflow");
    let env = Environment::new();

    assert!(advance_runtime(&mut runtime, &env, deadline).is_some());
    assert!(!runtime.mode.is_rebuild());
    assert!(runtime.renderer.take_redraw_request());
    assert!(runtime.platform.take_redraw_request());
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
