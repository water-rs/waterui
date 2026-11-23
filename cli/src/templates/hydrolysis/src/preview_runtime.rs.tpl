//! Hydrolysis preview runtime for {{ ctx.app_display_name }}.

use std::{
    fs,
    future::Future as _,
    path::Path,
    pin::pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use crate::preview_symbol;
use hydrolysis::{
    FontFamilyResolution, HeadlessColorFormat, HeadlessHdrSnapshot, HeadlessRuntime,
    HeadlessSnapshot, InputEvent, OutputAlpha, PointerButton, PointerKind,
};
use waterui::ContentLedger;
use waterui::content_ledger::{ContentError, ContentFailure};
use waterui_core::handler::AnyViewBuilder;
use waterui_preview_protocol::run::{
    Alpha, PreviewColorScheme, PreviewRunConfig, PreviewRunMode, RawFrameStreamWriter,
    ScenarioEvent, ScenarioEventKind, ScenarioPointerButton, VideoCaptureConfig,
    plan_scenario_timeline, write_hdr_png, write_png,
};

pub(crate) fn run() {
    // `app(env)` runs inside `app_environment` below; the subscriber must be
    // installed first or events the application emits while building its
    // `App` are dropped (water-rs/waterui#2513).
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    hydrolysis::init_tracing();
    let config = PreviewRunConfig::load_from_env()
        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
    match config.mode() {
        PreviewRunMode::Image { output } => {
            run_image(output, &config);
        }
        PreviewRunMode::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        } => run_scenario(
            output_dir.as_deref(),
            captures_ms,
            events,
            video.as_ref(),
            &config,
        ),
        PreviewRunMode::Semantic { .. } => panic!(
            "hydrolysis preview: semantic runs require the preview test binary (waterui-preview-test-mode)"
        ),
    }
}

/// A mounted preview: the runtime, the ledger its views owe content to, and
/// how long a capture waits for that content.
struct Preview {
    runtime: HeadlessRuntime,
    ledger: ContentLedger,
    content_deadline: Duration,
}

fn mount(config: &PreviewRunConfig) -> Preview {
    let ledger = ContentLedger::new();
    Preview {
        runtime: new_runtime(config, &ledger),
        ledger,
        content_deadline: config.content_deadline(),
    }
}

fn resolve_color_scheme(config: &PreviewRunConfig) -> Option<waterui::theme::ColorScheme> {
    config.color_scheme().map(|scheme| match scheme {
        PreviewColorScheme::Light => waterui::theme::ColorScheme::Light,
        PreviewColorScheme::Dark => waterui::theme::ColorScheme::Dark,
    })
}

fn new_runtime(config: &PreviewRunConfig, ledger: &ContentLedger) -> HeadlessRuntime {
    // The environment is the application's own composition root: `app(env)`
    // installs the options and hooks the application configures, and the
    // preview style is constructed alongside exactly as `main` hands its style
    // to `hydrolysis::run`. The capture's ledger layers over that environment,
    // so every view the preview mounts owes its content to this capture.
    let mut env = preview_symbol::app_environment();
    let color_scheme = resolve_color_scheme(config);
    if let Some(scheme) = color_scheme {
        waterui::theme::install_color_scheme(&mut env, waterui::Computed::constant(scheme));
    }
    let env = env.extending(ledger.clone());
    let content = AnyViewBuilder::new(preview_symbol::load_preview_view);
    let scale_factor = f64::from(config.scale());

    let color_format = if config.hdr() {
        HeadlessColorFormat::HdrLinearDisplayP3
    } else {
        HeadlessColorFormat::SdrDisplayP3
    };

    HeadlessRuntime::new_with_format(
        env,
        content,
        dimension_to_u32(config.width()),
        dimension_to_u32(config.height()),
        preview_symbol::preview_style(color_scheme),
        // A preview host is an application: an uninstalled named family
        // resolves the way it does under `water run`.
        FontFamilyResolution::Lenient,
        color_format,
    )
    .unwrap_or_else(|error| match std::error::Error::source(&error) {
        Some(source) => panic!("hydrolysis preview: {error}: {source}"),
        None => panic!("hydrolysis preview: {error}"),
    })
    .with_scale_factor(scale_factor)
}

/// Pumps until the frame stops changing and the views owe the capture no
/// content.
///
/// A `GpuContent`'s `setup` is an async future spawned onto the local executor, so
/// its pipelines do not exist during the first frame. Capturing immediately
/// yields a snapshot of the surface before any GPU content was drawn — the
/// window background and nothing else. `rebuilt` is the real readiness signal
/// here, so pump on it rather than waiting a fixed amount of time.
///
/// Quiescence alone is not the whole story: a view whose content arrives
/// through a load — a `Photo` fetch or decode, an `avatar` image, a
/// `Suspense` future — holds no queued runnable while the load is in flight,
/// so the runtime reports the frame settled while the placeholder is still on
/// screen. While the capture's [`ContentLedger`] reports content owed,
/// settling blocks on the local executor's wake edge — the load re-queues its
/// task when it completes — and pumps what arrived. A load that fails, or
/// content still owed at the run's content deadline, fails the preview naming
/// the view: a capture of the placeholder would be a wrong preview.
///
/// The virtual instant never moves while waiting: the scene is quiescent, so
/// nothing it scheduled needs a frame, and advancing the clock would carry
/// every animation past the phase the capture asked for. Once a pump changes
/// the tree, settling returns to the rebuild loop so whatever the change
/// scheduled can run before waiting resumes.
fn settle(preview: &mut Preview, at: Instant) {
    /// Pump budget for a frame that never stops rebuilding — a perpetual
    /// animation; bounds pump work, not wall clock.
    const MAX_PUMPS: usize = 64;

    let deadline = Instant::now() + preview.content_deadline;
    let mut pumps = 0usize;
    loop {
        if preview.runtime.pump_at(false, at).rebuilt() {
            pumps += 1;
            assert!(
                pumps < MAX_PUMPS,
                "hydrolysis preview: frame never settled after {MAX_PUMPS} pumps"
            );
            continue;
        }
        // Quiescent, but a view may still owe content its load has not
        // delivered: block until the load re-queues its task, then run it at
        // the same instant.
        loop {
            match poll_settled(&preview.ledger) {
                Poll::Ready(Ok(())) => return,
                Poll::Ready(Err(failure)) => content_failed(&ContentError::Failed(failure)),
                Poll::Pending => {}
            }
            let now = Instant::now();
            if now >= deadline {
                content_failed(&ContentError::Pending {
                    outstanding: preview.ledger.outstanding(),
                    deadline: preview.content_deadline,
                });
            }
            let _ = preview.runtime.wait_for_local_work(deadline - now);
            if preview.runtime.pump_at(false, at).rebuilt() {
                break;
            }
        }
    }
}

/// Whether the ledger has settled, polled once: settling is driven by the
/// runtime's pump, so the ledger's own wake is not needed.
fn poll_settled(ledger: &ContentLedger) -> Poll<Result<(), ContentFailure>> {
    pin!(ledger.settled()).poll(&mut Context::from_waker(Waker::noop()))
}

/// Fails the preview with `error` and the cause it carries.
fn content_failed(error: &ContentError) -> ! {
    match std::error::Error::source(error) {
        Some(source) => panic!("hydrolysis preview: {error}: {source}"),
        None => panic!("hydrolysis preview: {error}"),
    }
}

fn run_image(output_path: &Path, config: &PreviewRunConfig) {
    let mut preview = mount(config);
    let frame_at = Instant::now();
    let _ = preview.runtime.pump_at(false, frame_at);
    settle(&mut preview, frame_at);
    let result = preview.runtime.pump_at(true, frame_at);
    if config.hdr() {
        let snapshot = result
            .into_hdr_snapshot()
            .unwrap_or_else(|| panic!("hydrolysis preview: HDR capture produced no snapshot"));
        write_hdr_snapshot_png(snapshot, output_path);
    } else {
        let snapshot = result
            .into_snapshot()
            .unwrap_or_else(|| panic!("hydrolysis preview: static capture produced no snapshot"));
        write_snapshot_png(snapshot, output_path);
    }
}

fn run_scenario(
    output_dir: Option<&Path>,
    captures_ms: &[u64],
    events: &[ScenarioEvent],
    video: Option<&VideoCaptureConfig>,
    config: &PreviewRunConfig,
) {
    let samples = plan_scenario_timeline(captures_ms, video, events)
        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
    if let Some(output_dir) = output_dir
        && !captures_ms.is_empty()
    {
        fs::create_dir_all(output_dir).unwrap_or_else(|error| {
            panic!(
                "hydrolysis preview: failed to create scenario output dir `{}`: {error}",
                output_dir.display()
            )
        });
    }
    let hdr = config.hdr();
    let mut video_writer = video.map(|video| {
        let writer = if hdr {
            RawFrameStreamWriter::create_hdr(video.output(), video.fps())
        } else {
            RawFrameStreamWriter::create(video.output(), video.fps())
        };
        writer.unwrap_or_else(|error| panic!("hydrolysis preview: {error}"))
    });
    let mut preview = mount(config);
    let started_at = Instant::now();
    let _ = preview.runtime.pump_at(false, started_at);
    let mut event_index = 0usize;
    for sample in samples {
        while event_index < events.len()
            && Duration::from_millis(events[event_index].at_ms()) <= sample.offset()
        {
            let event_at = started_at + Duration::from_millis(events[event_index].at_ms());
            preview
                .runtime
                .push_input_event(input_event(events[event_index]));
            let _ = preview.runtime.pump_at(false, event_at);
            event_index += 1;
        }
        let capture_at = started_at + sample.offset();
        // Content an interaction made a view owe — a tap that kicks off a
        // fetch — is owed to the same ledger mount-time content is, so a
        // capture settles by the same rule: the tree quiet *and* nothing owed.
        settle(&mut preview, capture_at);
        let result = preview.runtime.pump_at(true, capture_at);
        if hdr {
            let Some(snapshot) = result.into_hdr_snapshot() else {
                panic!(
                    "hydrolysis preview: HDR scenario capture at {}ms produced no snapshot",
                    sample.offset().as_millis()
                );
            };
            let (width, height, rgba_f32, _headroom, output_alpha) = snapshot.into_parts();
            let alpha = snapshot_alpha(output_alpha);
            if sample.record_video()
                && let Some(writer) = &mut video_writer
            {
                writer
                    .write_hdr_frame_f32(width, height, &rgba_f32, alpha, sample.offset())
                    .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
            }
            if let (Some(capture_ms), Some(output_dir)) = (sample.png_capture_ms(), output_dir) {
                write_hdr_png(
                    &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
                    width,
                    height,
                    rgba_f32,
                    alpha,
                )
                .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
            }
        } else {
            let Some(snapshot) = result.into_snapshot() else {
                panic!(
                    "hydrolysis preview: scenario capture at {}ms produced no snapshot",
                    sample.offset().as_millis()
                );
            };
            let (width, height, rgba8, output_alpha) = snapshot.into_parts();
            let alpha = snapshot_alpha(output_alpha);
            match (sample.png_capture_ms(), sample.record_video()) {
                (Some(capture_ms), true) => {
                    if let Some(output_dir) = output_dir {
                        write_png(
                            &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
                            width,
                            height,
                            rgba8.clone(),
                            alpha,
                        )
                        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
                    }
                    if let Some(writer) = &mut video_writer {
                        writer
                            .write_frame(width, height, rgba8, alpha, sample.offset())
                            .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
                    }
                }
                (Some(capture_ms), false) => {
                    if let Some(output_dir) = output_dir {
                        write_png(
                            &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
                            width,
                            height,
                            rgba8,
                            alpha,
                        )
                        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
                    }
                }
                (None, true) => {
                    if let Some(writer) = &mut video_writer {
                        writer
                            .write_frame(width, height, rgba8, alpha, sample.offset())
                            .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
                    }
                }
                (None, false) => {}
            }
        }
    }
    if let Some(writer) = video_writer {
        let _ = writer
            .finish()
            .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
    }
}

const fn snapshot_alpha(output_alpha: OutputAlpha) -> Alpha {
    match output_alpha {
        OutputAlpha::Premultiplied => Alpha::Premultiplied,
        OutputAlpha::Opaque | OutputAlpha::Straight => Alpha::Straight,
    }
}

fn write_snapshot_png(snapshot: HeadlessSnapshot, path: &Path) {
    // The alpha convention comes from the surface that presented the
    // capture — `Opaque` reads like `Straight` to the flattener, and
    // nothing else is produced.
    let (width, height, rgba8, output_alpha) = snapshot.into_parts();
    write_png(path, width, height, rgba8, snapshot_alpha(output_alpha))
        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
}

fn write_hdr_snapshot_png(snapshot: HeadlessHdrSnapshot, path: &Path) {
    let (width, height, rgba_f32, _headroom, output_alpha) = snapshot.into_parts();
    write_hdr_png(path, width, height, rgba_f32, snapshot_alpha(output_alpha))
        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
}

fn input_event(event: ScenarioEvent) -> InputEvent {
    let button = match event.button() {
        ScenarioPointerButton::Primary => PointerButton::Primary,
        ScenarioPointerButton::Secondary => PointerButton::Secondary,
        ScenarioPointerButton::Middle => PointerButton::Middle,
        other => panic!("hydrolysis preview: unsupported pointer button {other:?}"),
    };
    match event.kind() {
        ScenarioEventKind::PointerMove => InputEvent::PointerMove {
            id: 0,
            kind: PointerKind::Mouse,
            x: event.x(),
            y: event.y(),
        },
        ScenarioEventKind::PointerDown => InputEvent::PointerDown {
            id: 0,
            kind: PointerKind::Mouse,
            x: event.x(),
            y: event.y(),
            button,
        },
        ScenarioEventKind::PointerUp => InputEvent::PointerUp {
            id: 0,
            kind: PointerKind::Mouse,
            x: event.x(),
            y: event.y(),
            button,
        },
        ScenarioEventKind::PointerCancel => InputEvent::PointerCancel {
            id: 0,
            kind: PointerKind::Mouse,
        },
        ScenarioEventKind::Scroll => InputEvent::Scroll {
            x: event.x(),
            y: event.y(),
            dx: event.dx(),
            dy: event.dy(),
            is_line_delta: event.is_line_delta(),
        },
        other => panic!("hydrolysis preview: unsupported scenario event {other:?}"),
    }
}

fn dimension_to_u32(value: f32) -> u32 {
    assert!(
        value.is_finite() && value > 0.0,
        "hydrolysis preview dimension must be finite and positive"
    );
    value.round() as u32
}
