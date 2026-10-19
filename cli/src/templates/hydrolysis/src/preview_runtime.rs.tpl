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
use hydrolysis::{FontFamilyResolution, HeadlessRuntime, InputEvent, PointerButton, PointerKind};
use waterui_core::ContentLedger;
use waterui_core::content_ledger::{ContentError, ContentFailure};
use waterui_core::handler::AnyViewBuilder;
use waterui_preview_protocol::run::{
    Alpha, PreviewRunConfig, PreviewRunMode, ScenarioEvent, ScenarioEventKind,
    ScenarioPointerButton, write_png,
};

pub(crate) fn run() {
    // `app(env)` runs inside `app_environment` below; the subscriber must be
    // installed first or events the application emits while building its
    // `App` are dropped (water-rs/waterui#2513).
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    hydrolysis::init_tracing();
    let config = PreviewRunConfig::load_from_env()
        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
    match config.mode {
        PreviewRunMode::Image { ref output } => {
            run_image(output, &config);
        }
        PreviewRunMode::Scenario {
            ref output_dir,
            ref captures_ms,
            ref events,
        } => run_scenario(output_dir, captures_ms, events, &config),
        PreviewRunMode::Semantic => panic!(
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
        runtime: new_runtime(config.width, config.height, &ledger),
        ledger,
        content_deadline: config.content_deadline(),
    }
}

fn new_runtime(width: f32, height: f32, ledger: &ContentLedger) -> HeadlessRuntime {
    // The environment is the application's own composition root: `app(env)`
    // installs the realizations and options the application configures
    // (`waterui_map_gpu::install`, provider options, fonts), and the preview
    // style is constructed alongside exactly as `main` hands its style to
    // `hydrolysis::run`. On Apple that leaves video to AVPlayer, and this
    // host is Hydrolysis on every OS, so the self-drawn player is installed
    // here — otherwise a `VideoPlayer` stays an unbridged native view.
    // The capture's ledger layers over that environment, so every view the
    // preview mounts owes its content to this capture.
    let mut env = preview_symbol::app_environment();
    waterui::realization::install_video(&mut env);
    let env = env.extending(ledger.clone());
    let content = AnyViewBuilder::new(preview_symbol::load_preview_view);
    // Previews are read on HiDPI displays, so render at 2x: the layout stays in
    // logical units and only the captured image gets sharper.
    const PREVIEW_SCALE_FACTOR: f64 = 2.0;

    HeadlessRuntime::new(
        env,
        content,
        dimension_to_u32(width),
        dimension_to_u32(height),
        preview_symbol::preview_style(),
        // A preview host is an application: an uninstalled named family
        // resolves the way it does under `water run`.
        FontFamilyResolution::Lenient,
    )
    .with_scale_factor(PREVIEW_SCALE_FACTOR)
}

/// Pumps until the frame stops changing and the views owe the capture no
/// content.
///
/// A `GpuView`'s `setup` is an async future spawned onto the local executor, so
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
        if preview.runtime.pump_at(false, at).rebuilt {
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
            if preview.runtime.pump_at(false, at).rebuilt {
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
    let snapshot = result
        .snapshot
        .unwrap_or_else(|| panic!("hydrolysis preview: static capture produced no snapshot"));
    write_snapshot_png(snapshot, output_path);
}

fn run_scenario(
    output_dir: &Path,
    captures_ms: &[u64],
    events: &[ScenarioEvent],
    config: &PreviewRunConfig,
) {
    assert!(
        !captures_ms.is_empty(),
        "hydrolysis preview scenario requires at least one capture"
    );
    fs::create_dir_all(output_dir).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview: failed to create scenario output dir `{}`: {error}",
            output_dir.display()
        )
    });
    let mut preview = mount(config);
    let started_at = Instant::now();
    let _ = preview.runtime.pump_at(false, started_at);
    let mut event_index = 0usize;
    for capture_ms in captures_ms {
        while event_index < events.len() && events[event_index].at_ms <= *capture_ms {
            let event_at = started_at + Duration::from_millis(events[event_index].at_ms);
            preview
                .runtime
                .push_input_event(input_event(events[event_index]));
            let _ = preview.runtime.pump_at(false, event_at);
            event_index += 1;
        }
        let capture_at = started_at + Duration::from_millis(*capture_ms);
        // Content an interaction made a view owe — a tap that kicks off a
        // fetch — is owed to the same ledger mount-time content is, so a
        // capture settles by the same rule: the tree quiet *and* nothing owed.
        settle(&mut preview, capture_at);
        let result = preview.runtime.pump_at(true, capture_at);
        let Some(snapshot) = result.snapshot else {
            panic!("hydrolysis preview: scenario capture at {capture_ms}ms produced no snapshot");
        };
        write_snapshot_png(
            snapshot,
            &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
        );
    }
}

fn write_snapshot_png(snapshot: hydrolysis::HeadlessSnapshot, path: &Path) {
    // The alpha convention comes from the surface that presented the
    // capture — `Opaque` reads like `Straight` to the flattener, and
    // nothing else is produced.
    let alpha = match snapshot.output_alpha {
        hydrolysis::OutputAlpha::Premultiplied => Alpha::Premultiplied,
        hydrolysis::OutputAlpha::Opaque | hydrolysis::OutputAlpha::Straight => Alpha::Straight,
    };
    write_png(path, snapshot.width, snapshot.height, snapshot.rgba8, alpha)
        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
}

fn input_event(event: ScenarioEvent) -> InputEvent {
    let button = match event.button {
        ScenarioPointerButton::Primary => PointerButton::Primary,
        ScenarioPointerButton::Secondary => PointerButton::Secondary,
        ScenarioPointerButton::Middle => PointerButton::Middle,
        _ => panic!(
            "hydrolysis preview: unsupported pointer button {:?}",
            event.button
        ),
    };
    match event.kind {
        ScenarioEventKind::PointerMove => InputEvent::PointerMove {
            id: 0,
            kind: PointerKind::Mouse,
            x: event.x,
            y: event.y,
        },
        ScenarioEventKind::PointerDown => InputEvent::PointerDown {
            id: 0,
            kind: PointerKind::Mouse,
            x: event.x,
            y: event.y,
            button,
        },
        ScenarioEventKind::PointerUp => InputEvent::PointerUp {
            id: 0,
            kind: PointerKind::Mouse,
            x: event.x,
            y: event.y,
            button,
        },
        ScenarioEventKind::PointerCancel => InputEvent::PointerCancel {
            id: 0,
            kind: PointerKind::Mouse,
        },
        ScenarioEventKind::Scroll => InputEvent::Scroll {
            x: event.x,
            y: event.y,
            dx: event.dx,
            dy: event.dy,
            is_line_delta: event.is_line_delta,
        },
        _ => panic!(
            "hydrolysis preview: unsupported scenario event {:?}",
            event.kind
        ),
    }
}

fn dimension_to_u32(value: f32) -> u32 {
    assert!(
        value.is_finite() && value > 0.0,
        "hydrolysis preview dimension must be finite and positive"
    );
    value.round() as u32
}
