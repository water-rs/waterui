//! In-process `AppKit` preview runner for `water preview --platform macos`.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cocoa_ui::appkit::{ActivationPolicy, Application, HostView};
use cocoa_ui::bitmap::{HdrRaster, capture_window, set_window_color_scheme};
#[cfg(feature = "gpu_surface")]
use cocoa_ui::capture::collect_capturables;
use cocoa_ui::core_animation::flush_transaction;
use cocoa_ui::dynamic_range::{DynamicRange, apply_to_view};
use cocoa_ui::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow};
use cocoa_ui::objc2_core_foundation::{CFRunLoop, CGPoint};
use cocoa_ui::view::{display_immediately, layout_immediately};
use cocoa_ui::{ColorScheme, MainThreadMarker, Rect, Retained, Size};
use executor_core::spawn_local;
use objc2_core_graphics::{CGEvent, CGScrollEventUnit};
use waterui_backend_core::Environment;
#[cfg(feature = "inspector")]
use waterui_backend_core::inspector::{InspectorRuntime, install as install_inspector};
use waterui_backend_core::task::{sleep, yield_now};
use waterui_capture::ContentLedger;
use waterui_capture::view_renderer::RenderResult;
use waterui_core::AnyView;
use waterui_preview_protocol::run::{
    Alpha, PreviewColorScheme, PreviewRunConfig, PreviewRunMode, RawFrameStreamWriter,
    ScenarioEvent, ScenarioEventKind, ScenarioPointerButton, VideoCaptureConfig,
    plan_scenario_timeline, write_hdr_png, write_png,
};
use waterui_resources::ResourceContext;
use waterui_text::install_system_font_collection;

#[cfg(feature = "gpu_surface")]
use crate::capture_image::CaptureDrive;
use crate::content::await_content;
use crate::contract::{KeepAlive, Mounted};
use crate::dispatch::install as install_dispatch;
use crate::embedding::mount_content;
use crate::first_paint::{FirstPaint, mark as mark_first_paint};
use crate::fonts::FontRegistrations;
#[cfg(feature = "gpu_surface")]
use crate::gpu_runtime::prepare as prepare_gpu;
use crate::locale::install as install_locale;
use crate::preview::{
    CaptureFailure, PreviewError, RunTask, write_scenario_hdr_sample, write_scenario_sdr_sample,
};
use crate::reduce_motion::install as install_reduce_motion;
use crate::resources::install_application;
use crate::startup::initialize_for_preview;
use crate::{capture, theme};

pub(super) fn drive_hdr(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    width: u32,
    height: u32,
    color_scheme: Option<PreviewColorScheme>,
    content_deadline: Duration,
) -> Result<HdrRaster, PreviewError> {
    let mtm = MainThreadMarker::new().ok_or(PreviewError::NotMainThread)?;
    #[cfg(feature = "inspector")]
    let inspector = initialize_for_preview();
    #[cfg(not(feature = "inspector"))]
    initialize_for_preview();

    let color_scheme = color_scheme.map(|scheme| match scheme {
        PreviewColorScheme::Light => ColorScheme::Light,
        PreviewColorScheme::Dark => ColorScheme::Dark,
    });
    let size = Size::new(f64::from(width), f64::from(height));

    let mut keepers = KeepAlive::default();
    let mut env = prepare(
        mtm,
        resources,
        color_scheme,
        &mut keepers,
        #[cfg(feature = "inspector")]
        inspector,
    )?;

    let env_ptr = &raw mut env;
    let outcome = Rc::new(RefCell::new(None::<Result<HdrRaster, PreviewError>>));
    let launch = {
        let outcome = Rc::clone(&outcome);
        move || {
            // SAFETY: `env_ptr` stays live until the capture completes and
            // stops `CFRunLoop::run` below.
            let env = unsafe { &mut *env_ptr };
            let env = compose(env.clone());
            spawn_local(async move {
                *outcome.borrow_mut() =
                    Some(render_hdr(&env, mtm, view, size, color_scheme, content_deadline).await);
                CFRunLoop::current()
                    .expect("the main executor drives this task from the main thread's run loop")
                    .stop();
            })
            .detach();
        }
    };
    #[cfg(feature = "gpu_surface")]
    // SAFETY: `env` stays live until the completion stops `CFRunLoop::run`.
    unsafe {
        prepare_gpu(env_ptr, launch);
    }
    #[cfg(not(feature = "gpu_surface"))]
    launch();
    CFRunLoop::run();
    let result = outcome
        .borrow_mut()
        .take()
        .ok_or(PreviewError::RunLoopExited)?;
    drop(env);
    drop(keepers);
    result
}

#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across awaits are not Send"
)]
async fn render_hdr(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> AnyView,
    size: Size,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
) -> Result<HdrRaster, PreviewError> {
    let ledger = ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let stage = Stage::mount(
        &content_env,
        mtm,
        view,
        size,
        color_scheme,
        DynamicRange::High,
    );
    await_content(&ledger, &stage.root, content_deadline).await?;
    let result = stage.capture_hdr().await?;
    drop(stage);
    Ok(result)
}

pub(super) fn drive(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    config: PreviewRunConfig,
) -> Result<(), PreviewError> {
    let load_started = Instant::now();
    let mtm = MainThreadMarker::new().ok_or(PreviewError::NotMainThread)?;
    #[cfg(feature = "inspector")]
    let inspector = initialize_for_preview();
    #[cfg(not(feature = "inspector"))]
    initialize_for_preview();

    let content_deadline = config.content_deadline();
    let hdr = config.hdr();
    let color_scheme = config.color_scheme().map(|scheme| match scheme {
        PreviewColorScheme::Light => ColorScheme::Light,
        PreviewColorScheme::Dark => ColorScheme::Dark,
    });
    let size = Size::new(f64::from(config.width()), f64::from(config.height()));
    let task = match config.into_mode() {
        PreviewRunMode::Image { output } => RunTask::Image { output },
        PreviewRunMode::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        } => RunTask::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        },
        PreviewRunMode::Semantic { .. } => return Err(PreviewError::UnsupportedMode("semantic")),
    };

    let mut keepers = KeepAlive::default();
    let mut env = prepare(
        mtm,
        resources,
        color_scheme,
        &mut keepers,
        #[cfg(feature = "inspector")]
        inspector,
    )?;

    let env_ptr = &raw mut env;
    let outcome = Rc::new(RefCell::new(None::<Result<(), PreviewError>>));
    let launch = {
        let outcome = Rc::clone(&outcome);
        move || {
            // SAFETY: `env_ptr` stays live until the capture completes.
            let env = unsafe { &mut *env_ptr };
            let env = compose(env.clone());
            tracing::info!(
                phase = "load",
                elapsed_ms = load_started.elapsed().as_millis(),
                "Apple preview phase"
            );
            spawn_local(async move {
                *outcome.borrow_mut() = Some(
                    execute_run_task(
                        &env,
                        mtm,
                        view,
                        size,
                        color_scheme,
                        content_deadline,
                        hdr,
                        task,
                    )
                    .await,
                );
                CFRunLoop::current()
                    .expect("the main executor drives this task from the main thread's run loop")
                    .stop();
            })
            .detach();
        }
    };
    #[cfg(feature = "gpu_surface")]
    // SAFETY: `env` stays live until the completion stops `CFRunLoop::run`.
    unsafe {
        prepare_gpu(env_ptr, launch);
    }
    #[cfg(not(feature = "gpu_surface"))]
    launch();
    CFRunLoop::run();
    outcome
        .borrow_mut()
        .take()
        .ok_or(PreviewError::RunLoopExited)?
}

fn prepare(
    mtm: MainThreadMarker,
    resources: ResourceContext,
    color_scheme: Option<ColorScheme>,
    keepers: &mut KeepAlive,
    #[cfg(feature = "inspector")] inspector: Option<InspectorRuntime>,
) -> Result<Environment, PreviewError> {
    let mut env = Environment::new();
    #[cfg(feature = "inspector")]
    install_inspector(&mut env, inspector);
    install_system_font_collection(&mut env);
    let mut fonts = FontRegistrations::default();
    env.insert(resources);
    install_application(&mut env, &mut fonts);
    install_dispatch(&mut env);
    env.insert(FirstPaint::default());

    let application = Application::shared(mtm);
    if application.activation_policy() != ActivationPolicy::Prohibited
        && !application.set_activation_policy(ActivationPolicy::Prohibited)
    {
        return Err(PreviewError::ActivationPolicyRefused);
    }
    let initial = color_scheme.unwrap_or_else(|| application.color_scheme());
    let installed_theme = Rc::new(theme::install(&mut env, initial));
    if color_scheme.is_none() {
        let appearance = application.observe_color_scheme({
            let installed_theme = Rc::clone(&installed_theme);
            move |scheme| theme::refresh(&installed_theme, scheme)
        });
        keepers.keep(appearance);
    }
    keepers.keep(installed_theme);

    let locale = install_locale(&mut env, mtm);
    keepers.keep(locale);
    keepers.keep(install_reduce_motion(&mut env, mtm));
    crate::accessibility_announcer::install(&mut env);
    keepers.keep(fonts);
    Ok(env)
}

struct Capture<'a> {
    size: Size,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
    hdr: bool,
    output: &'a Path,
}

#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
)]
async fn render_and_write(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> AnyView,
    capture_cfg: Capture<'_>,
) -> Result<(), PreviewError> {
    let render_started = Instant::now();
    let ledger = ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let dynamic_range = if capture_cfg.hdr {
        DynamicRange::High
    } else {
        DynamicRange::Standard
    };
    let stage = Stage::mount(
        &content_env,
        mtm,
        view,
        capture_cfg.size,
        capture_cfg.color_scheme,
        dynamic_range,
    );
    await_content(&ledger, &stage.root, capture_cfg.content_deadline).await?;
    if capture_cfg.hdr {
        let raster = stage.capture_hdr().await?;
        drop(stage);
        tracing::info!(
            phase = "render",
            elapsed_ms = render_started.elapsed().as_millis(),
            "Apple preview phase"
        );
        let encode_started = Instant::now();
        write_hdr_png(
            capture_cfg.output,
            raster.width,
            raster.height,
            raster.pixels,
            Alpha::Premultiplied,
        )?;
        tracing::info!(
            phase = "encode",
            elapsed_ms = encode_started.elapsed().as_millis(),
            "Apple preview phase"
        );
    } else {
        let result = stage.capture().await?;
        drop(stage);
        tracing::info!(
            phase = "render",
            elapsed_ms = render_started.elapsed().as_millis(),
            "Apple preview phase"
        );
        let encode_started = Instant::now();
        let (rgba_data, width, height) = result.into_parts();
        write_png(
            capture_cfg.output,
            width,
            height,
            rgba_data,
            Alpha::Premultiplied,
        )?;
        tracing::info!(
            phase = "encode",
            elapsed_ms = encode_started.elapsed().as_millis(),
            "Apple preview phase"
        );
    }
    Ok(())
}

struct ScenarioRun<'a> {
    size: Size,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
    hdr: bool,
    output_dir: Option<&'a Path>,
    captures_ms: &'a [u64],
    events: &'a [ScenarioEvent],
    video: Option<&'a VideoCaptureConfig>,
}

#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across awaits are not Send"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper bundling drive's locals before dispatch"
)]
async fn execute_run_task(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> AnyView,
    size: Size,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
    hdr: bool,
    task: RunTask,
) -> Result<(), PreviewError> {
    match task {
        RunTask::Image { output } => {
            render_and_write(
                env,
                mtm,
                view,
                Capture {
                    size,
                    color_scheme,
                    content_deadline,
                    hdr,
                    output: &output,
                },
            )
            .await
        }
        RunTask::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        } => {
            render_scenario(
                env,
                mtm,
                view,
                ScenarioRun {
                    size,
                    color_scheme,
                    content_deadline,
                    hdr,
                    output_dir: output_dir.as_deref(),
                    captures_ms: &captures_ms,
                    events: &events,
                    video: video.as_ref(),
                },
            )
            .await
        }
    }
}

#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across awaits are not Send"
)]
async fn render_scenario(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> AnyView,
    scenario: ScenarioRun<'_>,
) -> Result<(), PreviewError> {
    let samples = plan_scenario_timeline(scenario.captures_ms, scenario.video, scenario.events)?;
    let mut video_writer = match scenario.video {
        Some(video) if scenario.hdr => Some(RawFrameStreamWriter::create_hdr(
            video.output(),
            video.fps(),
        )?),
        Some(video) => Some(RawFrameStreamWriter::create(video.output(), video.fps())?),
        None => None,
    };

    let ledger = ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let dynamic_range = if scenario.hdr {
        DynamicRange::High
    } else {
        DynamicRange::Standard
    };
    let stage = Stage::mount(
        &content_env,
        mtm,
        view,
        scenario.size,
        scenario.color_scheme,
        dynamic_range,
    );
    await_content(&ledger, &stage.root, scenario.content_deadline).await?;

    let mut input_state = ScenarioInputState::default();
    let mut event_index = 0usize;
    let started_at = Instant::now();

    for sample in samples {
        while event_index < scenario.events.len() {
            let event = scenario.events[event_index];
            let event_offset = Duration::from_millis(event.at_ms());
            if event_offset > sample.offset() {
                break;
            }
            if let Some(wait) = event_offset.checked_sub(started_at.elapsed())
                && !wait.is_zero()
            {
                sleep(wait).await;
            }
            dispatch_scenario_event(&stage, &mut input_state, event);
            flush_stage(&stage);
            yield_now().await;
            event_index += 1;
        }

        if let Some(wait) = sample.offset().checked_sub(started_at.elapsed())
            && !wait.is_zero()
        {
            sleep(wait).await;
        }
        await_content(&ledger, &stage.root, scenario.content_deadline).await?;
        if scenario.hdr {
            let raster = stage.capture_hdr().await?;
            write_scenario_hdr_sample(
                raster,
                sample.png_capture_ms(),
                sample.record_video(),
                sample.offset(),
                scenario.output_dir,
                video_writer.as_mut(),
            )?;
        } else {
            let result = stage.capture().await?;
            write_scenario_sdr_sample(
                result,
                sample.png_capture_ms(),
                sample.record_video(),
                sample.offset(),
                scenario.output_dir,
                video_writer.as_mut(),
            )?;
        }
    }

    drop(stage);
    if let Some(writer) = video_writer {
        let _ = writer.finish()?;
    }
    Ok(())
}

fn flush_stage(stage: &Stage) {
    layout_immediately(&stage.root);
    display_immediately(&stage.root);
    flush_transaction();
}

#[derive(Default)]
struct ScenarioInputState {
    pressed_button: Option<ScenarioPointerButton>,
    pressed_view: Option<Retained<NSView>>,
}

fn make_mouse_event(
    event_type: NSEventType,
    window_point: CGPoint,
    timestamp: f64,
    window_number: isize,
    click_count: isize,
) -> Option<Retained<NSEvent>> {
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        event_type,
        window_point,
        NSEventModifierFlags::empty(),
        timestamp,
        window_number,
        None,
        0,
        click_count,
        if click_count > 0 { 1.0 } else { 0.0 },
    )
}

#[derive(Clone, Copy)]
enum MouseButtonPhase {
    Down,
    Dragged,
    Up,
}

fn dispatch_mouse_button_event(
    target: &NSView,
    button: ScenarioPointerButton,
    phase: MouseButtonPhase,
    window_point: CGPoint,
    timestamp: f64,
    window_number: isize,
) {
    let event_type = match (phase, button) {
        (MouseButtonPhase::Down, ScenarioPointerButton::Secondary) => NSEventType::RightMouseDown,
        (MouseButtonPhase::Down, ScenarioPointerButton::Middle) => NSEventType::OtherMouseDown,
        (MouseButtonPhase::Down, _) => NSEventType::LeftMouseDown,
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Secondary) => {
            NSEventType::RightMouseDragged
        }
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Middle) => {
            NSEventType::OtherMouseDragged
        }
        (MouseButtonPhase::Dragged, _) => NSEventType::LeftMouseDragged,
        (MouseButtonPhase::Up, ScenarioPointerButton::Secondary) => NSEventType::RightMouseUp,
        (MouseButtonPhase::Up, ScenarioPointerButton::Middle) => NSEventType::OtherMouseUp,
        (MouseButtonPhase::Up, _) => NSEventType::LeftMouseUp,
    };
    let Some(ns_event) = make_mouse_event(event_type, window_point, timestamp, window_number, 1)
    else {
        return;
    };
    match (phase, button) {
        (MouseButtonPhase::Down, ScenarioPointerButton::Secondary) => {
            target.rightMouseDown(&ns_event);
        }
        (MouseButtonPhase::Down, ScenarioPointerButton::Middle) => target.otherMouseDown(&ns_event),
        (MouseButtonPhase::Down, _) => target.mouseDown(&ns_event),
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Secondary) => {
            target.rightMouseDragged(&ns_event);
        }
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Middle) => {
            target.otherMouseDragged(&ns_event);
        }
        (MouseButtonPhase::Dragged, _) => target.mouseDragged(&ns_event),
        (MouseButtonPhase::Up, ScenarioPointerButton::Secondary) => target.rightMouseUp(&ns_event),
        (MouseButtonPhase::Up, ScenarioPointerButton::Middle) => target.otherMouseUp(&ns_event),
        (MouseButtonPhase::Up, _) => target.mouseUp(&ns_event),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "CGEvent scroll wheel deltas are integer pixels or lines"
)]
fn dispatch_scenario_event(stage: &Stage, input: &mut ScenarioInputState, event: ScenarioEvent) {
    let root: &NSView = &stage.root;
    let local_point = CGPoint::new(f64::from(event.x()), f64::from(event.y()));
    let window_point = root.convertPoint_toView(local_point, None);
    // SAFETY: `root` is owned by the live capture window on the main thread;
    // reading its superview only retains the parent view for coordinate conversion.
    let superview = unsafe { root.superview() };
    let superview_point = root.convertPoint_toView(local_point, superview.as_deref());
    let window_number = stage.window.windowNumber();
    let timestamp = Duration::from_millis(event.at_ms()).as_secs_f64();

    match event.kind() {
        ScenarioEventKind::PointerDown => {
            let hit = root.hitTest(superview_point);
            let target = hit.as_deref().unwrap_or(root);
            dispatch_mouse_button_event(
                target,
                event.button(),
                MouseButtonPhase::Down,
                window_point,
                timestamp,
                window_number,
            );
            input.pressed_button = Some(event.button());
            input.pressed_view = hit;
        }
        ScenarioEventKind::PointerMove => {
            if let Some(button) = input.pressed_button {
                let target = input.pressed_view.as_deref().unwrap_or(root);
                dispatch_mouse_button_event(
                    target,
                    button,
                    MouseButtonPhase::Dragged,
                    window_point,
                    timestamp,
                    window_number,
                );
            } else {
                let hit = root.hitTest(superview_point);
                let target = hit.as_deref().unwrap_or(root);
                if let Some(ns_event) = make_mouse_event(
                    NSEventType::MouseMoved,
                    window_point,
                    timestamp,
                    window_number,
                    0,
                ) {
                    target.mouseMoved(&ns_event);
                }
            }
        }
        ScenarioEventKind::PointerUp => {
            let button = input
                .pressed_button
                .take()
                .unwrap_or_else(|| event.button());
            let target = input
                .pressed_view
                .take()
                .or_else(|| root.hitTest(superview_point));
            let target_ref = target.as_deref().unwrap_or(root);
            dispatch_mouse_button_event(
                target_ref,
                button,
                MouseButtonPhase::Up,
                window_point,
                timestamp,
                window_number,
            );
        }
        ScenarioEventKind::PointerCancel => {
            input.pressed_button = None;
            input.pressed_view = None;
        }
        ScenarioEventKind::Scroll => {
            let unit = if event.is_line_delta() {
                CGScrollEventUnit::Line
            } else {
                CGScrollEventUnit::Pixel
            };
            let wheel_y = event.dy().round() as i32;
            let wheel_x = event.dx().round() as i32;
            if let Some(cg_event) =
                CGEvent::new_scroll_wheel_event2(None, unit, 2, wheel_y, wheel_x, 0)
                && let Some(ns_event) = NSEvent::eventWithCGEvent(&cg_event)
            {
                let hit = root.hitTest(superview_point);
                let target = hit.as_deref().unwrap_or(root);
                target.scrollWheel(&ns_event);
            }
        }
        _ => {}
    }
}

struct Stage {
    #[cfg(feature = "gpu_surface")]
    drive: Rc<CaptureDrive>,
    _content: Rc<Mounted>,
    _keepalive: KeepAlive,
    root: Retained<HostView>,
    window: Retained<NSWindow>,
}

impl Stage {
    fn mount(
        env: &Environment,
        mtm: MainThreadMarker,
        view: impl FnOnce() -> AnyView,
        size: Size,
        color_scheme: Option<ColorScheme>,
        dynamic_range: DynamicRange,
    ) -> Self {
        let window = capture_window(mtm, size);
        if let Some(scheme) = color_scheme {
            set_window_color_scheme(&window, scheme);
        }
        let root = HostView::new(mtm, Rect::new(0.0, 0.0, size.width, size.height));
        window.setContentView(Some(&*root));
        apply_to_view(dynamic_range, &root);
        let mut keepalive = KeepAlive::default();
        let content = mount_content(&root, view(), env, &mut keepalive, mtm);
        mark_first_paint(&root, env);
        #[cfg(feature = "gpu_surface")]
        let drive = CaptureDrive::begin(&root, env, mtm);
        Self {
            #[cfg(feature = "gpu_surface")]
            drive,
            _content: content,
            _keepalive: keepalive,
            root,
            window,
        }
    }

    #[cfg(feature = "gpu_surface")]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture(&self) -> Result<RenderResult, CaptureFailure> {
        self.window.displayIfNeeded();
        let mut has_capturable = false;
        collect_capturables(&self.root, &mut |_| has_capturable = true);
        if !has_capturable {
            return capture::capture_presented(&self.root)
                .await
                .map_err(CaptureFailure::NativeCapture);
        }
        let result = self.drive.capture_display_p3_rgba().await?;
        Ok(RenderResult::new(
            result.pixels,
            result.width,
            result.height,
        ))
    }

    #[cfg(feature = "gpu_surface")]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture_hdr(&self) -> Result<HdrRaster, CaptureFailure> {
        self.window.displayIfNeeded();
        let mut has_capturable = false;
        collect_capturables(&self.root, &mut |_| has_capturable = true);
        if !has_capturable {
            return capture::capture_presented_hdr(&self.root)
                .await
                .map_err(CaptureFailure::NativeCapture);
        }
        self.drive.capture_hdr().await
    }

    #[cfg(not(feature = "gpu_surface"))]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture(&self) -> Result<RenderResult, CaptureFailure> {
        capture::capture_presented(&self.root).await
    }

    #[cfg(not(feature = "gpu_surface"))]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture_hdr(&self) -> Result<HdrRaster, CaptureFailure> {
        capture::capture_presented_hdr(&self.root).await
    }
}
