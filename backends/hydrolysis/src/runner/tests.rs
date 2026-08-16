use super::headless::HeadlessPlatformWindow;
use super::{
    RenderDiagnosticsConfig, RuntimeWindow, acquire_surface_frame, advance_runtime,
    handle_input_events, pump_window_semantics, render_window, schedule_animation_update,
    schedule_redraw_or_refresh, surface_error_requires_reconfigure,
};
use crate::platform::{
    InputEvent, OffscreenSurface, PlatformWindow as _, SurfaceError, SurfaceFrame, SurfaceProvider,
};
use crate::renderer::{HydrolysisRenderer, InteractionKey};
use core::time::Duration;
use std::rc::Rc;
use std::time::Instant;
use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::window::{Window, WindowState};
use waterui_backend_core::widget::TextCaretMotion;
use waterui_core::id::SelfId;
use waterui_core::{AnyView, Environment, binding};
use waterui_layout::scroll::ScrollController;

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
fn changed_reactive_input_refreshes_retained_tree() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();
    runtime.renderer.request_refresh();

    schedule_redraw_or_refresh(&mut runtime, true);

    assert!(
        runtime.mode.needs_layout(),
        "a pending reactive patch must be sampled before presenting the next frame"
    );
    assert!(
        runtime.platform.take_redraw_request(),
        "reactive input must wake the platform event loop"
    );
}

#[test]
fn changed_scroll_input_schedules_a_full_frame_and_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();

    // A consumed scroll schedules the one per-frame pass — patch, layout,
    // re-encode — like every other content change.
    schedule_redraw_or_refresh(&mut runtime, true);

    assert!(runtime.mode.is_pending());
    assert!(
        runtime.mode.needs_layout(),
        "every awake frame runs the full pass, layout included"
    );
    assert!(
        runtime.platform.take_redraw_request(),
        "scroll input must wake the platform event loop for the next frame"
    );
}

#[test]
fn animation_ticks_schedule_full_frames() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();

    schedule_animation_update(&mut runtime, true);

    assert!(runtime.mode.is_pending());
    assert!(
        runtime.mode.needs_layout(),
        "an animation tick schedules the same full frame as any content change"
    );
}

#[test]
fn only_invalid_swap_chains_require_surface_reconfiguration() {
    assert!(surface_error_requires_reconfigure(SurfaceError::Lost));
    assert!(surface_error_requires_reconfigure(SurfaceError::Outdated));
    assert!(!surface_error_requires_reconfigure(SurfaceError::Timeout));
    assert!(!surface_error_requires_reconfigure(SurfaceError::Occluded));
}

#[test]
fn invalid_swap_chain_is_reconfigured_and_retried_in_the_same_frame() {
    let mut surface = RecoveringSurface::new(SurfaceError::Outdated);

    let frame = acquire_surface_frame(&mut surface)
        .expect("surface acquisition must recover after reconfiguration");

    assert_eq!(surface.acquire_count, 2);
    assert_eq!(surface.resize_count, 1);
    surface.present(frame);
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
    // Focus by identity: the caret animates off the focused field's identity, and
    // this runtime emits no text-input targets, so there is no position to name.
    let focused_field = Rc::new(());
    assert!(
        runtime
            .renderer
            .set_focused_text_input_key(Some(InteractionKey::for_rc(&focused_field, 0)))
    );
    assert!(runtime.renderer.take_patch_request());
    assert!(!runtime.renderer.take_rebuild_request());
    runtime.clear_frame_mode();
    assert!(!runtime.platform.take_redraw_request());

    let deadline = now
        .checked_add(motion.frame_interval)
        .expect("test caret deadline overflow");
    let env = Environment::new();

    assert!(advance_runtime(&mut runtime, &env, deadline).is_some());
    assert!(
        !runtime.mode.is_pending(),
        "the caret repaints through the transient text-input overlay on present; \
         a tick must not schedule retained-scene work, or an idle window could \
         never stay idle"
    );
    assert!(runtime.renderer.take_redraw_request());
    assert!(runtime.platform.take_redraw_request());
}

/// The window's effective size limits reach the platform: the content's
/// measured minimum and maximum are the defaults, and explicit limits override
/// them.
#[test]
fn window_size_limits_reach_the_platform_window() {
    use waterui_core::layout::Size;
    use waterui_layout::frame::Frame;

    // Content with hard limits must pass both ends of the layout negotiation to
    // the platform window.
    let content = || {
        Frame::new(())
            .min_width(200.0)
            .min_height(100.0)
            .max_width(640.0)
            .max_height(480.0)
    };
    let window = Window::new("", binding(WindowState::Normal), content);
    let mut runtime = runtime_window_for(window);
    let env = Environment::new();
    let _ = super::pump_window_semantics(&mut runtime, &env);
    let (min, max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    assert_eq!(min, Some(Size::new(200.0, 100.0)));
    assert_eq!(max, Some(Size::new(640.0, 480.0)));

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

#[test]
fn zero_layout_minimum_is_not_replaced_by_ideal_size() {
    use waterui_core::layout::{Layout, ProposalSize, Rect, Size, SubView};
    use waterui_layout::container::FixedContainer;

    #[derive(Debug)]
    struct IdealOnlyLayout;

    impl Layout for IdealOnlyLayout {
        fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
            Size::new(
                proposal.width.unwrap_or(500.0),
                proposal.height.unwrap_or(400.0),
            )
        }

        fn place(&self, _bounds: Rect, _children: &[&dyn SubView]) -> Vec<Rect> {
            Vec::new()
        }
    }

    let window = Window::new("", binding(WindowState::Normal), || {
        FixedContainer::new(IdealOnlyLayout, ())
    });
    let mut runtime = runtime_window_for(window);
    let _ = super::pump_window_semantics(&mut runtime, &Environment::new());
    let (min, _) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");

    assert_eq!(
        min.expect("content-derived minimum must exist").width,
        0.0,
        "a valid zero minimum must not fall back to the content's ideal width"
    );
}

#[test]
fn rapid_resize_events_keep_the_retained_tree_at_the_latest_size() {
    use std::{cell::Cell, rc::Rc};

    let build_count = Rc::new(Cell::new(0));
    let window = Window::new("", binding(WindowState::Normal), {
        let build_count = Rc::clone(&build_count);
        move || build_count.set(build_count.get() + 1)
    });
    let mut runtime = runtime_window_for(window);
    let env = Environment::new();
    let _ = pump_window_semantics(&mut runtime, &env);
    assert_eq!(build_count.get(), 1);

    runtime.platform.push_event(InputEvent::Resize {
        width: 960,
        height: 720,
    });
    runtime.platform.push_event(InputEvent::Resize {
        width: 320,
        height: 240,
    });
    runtime.platform.push_event(InputEvent::Resize {
        width: 640,
        height: 480,
    });
    let _ = handle_input_events(&mut runtime, &env);
    let _ = pump_window_semantics(&mut runtime, &env);

    assert_eq!(
        build_count.get(),
        1,
        "resize must retain the existing view tree"
    );
    assert_eq!(runtime.platform.surface().size(), (640, 480));
    assert_eq!(runtime.window.frame.get().width(), 640.0);
    assert_eq!(runtime.window.frame.get().height(), 480.0);
}

#[test]
fn pending_lazy_height_refresh_precedes_scroll_input() {
    use waterui::ViewExt as _;
    use waterui_core::dynamic::watch;
    use waterui_core::id::SelfId;
    use waterui_layout::scroll::scroll;
    use waterui_layout::stack::VStack;

    let expanded = binding(false);
    let expanded_for_view = expanded.clone();
    let rows = vec![SelfId::new(0usize)];
    let window = Window::new("", binding(WindowState::Normal), move || {
        let expanded = expanded_for_view.clone();
        scroll(VStack::for_each(rows.clone(), move |_| {
            let expanded = expanded.clone();
            watch(expanded, |expanded| {
                ().size(800.0, if expanded { 1_200.0 } else { 200.0 })
            })
        }))
    });
    let mut runtime = runtime_window_for(window);
    let env = crate::renderer::tests::test_environment();
    let _ = pump_window_semantics(&mut runtime, &env);

    expanded.set(true);
    runtime.platform.push_event(InputEvent::Scroll {
        x: 400.0,
        y: 300.0,
        dx: 0.0,
        dy: -4.0,
        is_line_delta: false,
    });
    let _ = handle_input_events(&mut runtime, &env);

    let metrics = runtime
        .renderer
        .scroll_metrics_at(400.0, 300.0)
        .expect("scroll target must remain registered after the input geometry refresh");
    assert!(
        metrics.max_y > 8.0,
        "expanded lazy item must update the scroll extent before input dispatch: {metrics:?}"
    );
    assert!(
        runtime
            .renderer
            .handle_scroll(400.0, 300.0, 0.0, -4.0, false),
        "scroll input must see a Dynamic lazy item's latest height before the pending frame is presented"
    );
}

fn runtime_window_for(window: Window) -> RuntimeWindow<HeadlessPlatformWindow> {
    runtime_window_sized(window, 16, 16)
}

fn runtime_window_sized(
    window: Window,
    width: u32,
    height: u32,
) -> RuntimeWindow<HeadlessPlatformWindow> {
    let mut platform =
        HeadlessPlatformWindow::new_for_tests(width, height, wgpu::TextureFormat::Rgba8Unorm);
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

/// An animated `List` jump must keep the window awake until it settles.
///
/// This drives the runtime the way the winit loop does — a frame runs only while
/// the runtime is still asking to be woken — so a jump that fails to schedule
/// its own animation frames shows up as the loop going idle almost immediately.
/// Pumping frames unconditionally cannot see that, because it supplies the very
/// frames the bug withholds.
#[test]
fn an_animated_list_jump_keeps_the_window_awake_until_it_settles() {
    const ROWS: usize = 200;
    const TARGET_ROW: usize = 40;
    /// Hard stop so a runaway loop fails loudly instead of hanging.
    const MAX_FRAMES: usize = 400;

    let controller = ScrollController::new(0usize);
    let controller_for_view = controller.clone();
    let window = Window::new("", binding(WindowState::Normal), move || {
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        AnyView::new(
            List::for_each(rows, |_row| {
                ListItem::new(().size(160.0, ROW_HEIGHT_FOR_JUMP_TEST))
            })
            .scroll_controller(&controller_for_view),
        )
    });
    let mut runtime = runtime_window_sized(window, 160, 320);
    let env = crate::renderer::tests::test_environment();
    let mut now = Instant::now();

    // Settle the initial layout, then let the window go idle.
    let idle_frames = drive_until_idle(&mut runtime, &env, &mut now, MAX_FRAMES);
    assert!(
        idle_frames < MAX_FRAMES,
        "the window never went idle before the jump"
    );

    controller.scroll_to(TARGET_ROW);
    // The frame the button press itself produces: the loop is already running an
    // iteration for that input, and this is where the jump gets armed.
    now += Duration::from_millis(16);
    let _ = advance_runtime(&mut runtime, &env, now);
    render_window(&mut runtime, &env, &mut || false);

    // Everything after this point has to be self-sustaining.
    let animation_frames = drive_until_idle(&mut runtime, &env, &mut now, MAX_FRAMES);

    assert!(
        animation_frames < MAX_FRAMES,
        "the jump never settled: the window stayed awake for {animation_frames} frames"
    );
    // The approach eases with a ~180ms time constant, so a real animation spans
    // many frames. A jump that teleported, or one whose frames were never
    // scheduled, would idle again almost at once.
    assert!(
        animation_frames >= 8,
        "an animated jump should span many frames; the window went idle after \
         {animation_frames}, so the jump landed without animating"
    );
}

/// Runs frames while the runtime is still asking to be woken, returning how many
/// it took to go idle. This is the winit loop's rule: no wake request, no frame.
fn drive_until_idle(
    runtime: &mut RuntimeWindow<HeadlessPlatformWindow>,
    env: &Environment,
    now: &mut Instant,
    max_frames: usize,
) -> usize {
    for frame in 0..max_frames {
        let wake = runtime.mode.is_pending() | runtime.platform.take_redraw_request();
        if !wake {
            return frame;
        }
        *now += Duration::from_millis(16);
        let _ = advance_runtime(runtime, env, *now);
        render_window(runtime, env, &mut || false);
    }
    max_frames
}

/// Row height used by the jump test; any fixed height works, it only has to be
/// stable so the target sits a predictable distance away.
const ROW_HEIGHT_FOR_JUMP_TEST: f32 = 24.0;

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

struct RecoveringSurface {
    inner: OffscreenSurface,
    first_error: Option<SurfaceError>,
    acquire_count: usize,
    resize_count: usize,
}

impl RecoveringSurface {
    fn new(first_error: SurfaceError) -> Self {
        Self {
            inner: pollster::block_on(OffscreenSurface::new_for_tests(
                16,
                16,
                wgpu::TextureFormat::Rgba8Unorm,
            )),
            first_error: Some(first_error),
            acquire_count: 0,
            resize_count: 0,
        }
    }
}

impl SurfaceProvider for RecoveringSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        self.inner.adapter()
    }

    fn device(&self) -> &wgpu::Device {
        self.inner.device()
    }

    fn queue(&self) -> &wgpu::Queue {
        self.inner.queue()
    }

    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        self.acquire_count += 1;
        self.first_error
            .take()
            .map_or_else(|| self.inner.acquire(), Err)
    }

    fn present(&mut self, frame: SurfaceFrame) {
        self.inner.present(frame);
    }

    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.inner.format()
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.resize_count += 1;
        self.inner.resize(width, height);
    }
}
