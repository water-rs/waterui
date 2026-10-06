//! Hydrolysis preview runtime for {{ ctx.app_display_name }}.

use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use crate::preview_symbol;
use hydrolysis::{FontFamilyResolution, HeadlessRuntime, InputEvent, PointerButton, PointerKind};
use waterui_core::handler::AnyViewBuilder;
use waterui_preview_protocol::run::{
    Alpha, PreviewRunConfig, PreviewRunMode, ScenarioEvent, ScenarioEventKind,
    ScenarioPointerButton, write_png,
};

pub(crate) fn run() {
    let config = PreviewRunConfig::load_from_env()
        .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
    match config.mode {
        PreviewRunMode::Image { ref output } => {
            run_image(output, config.width, config.height);
        }
        PreviewRunMode::Scenario {
            ref output_dir,
            ref captures_ms,
            ref events,
        } => run_scenario(output_dir, captures_ms, events, config.width, config.height),
        PreviewRunMode::Semantic => panic!(
            "hydrolysis preview: semantic runs require the preview test binary (waterui-preview-test-mode)"
        ),
    }
}

fn new_runtime(width: f32, height: f32) -> HeadlessRuntime {
    // The environment is the application's own composition root: `app(env)`
    // installs the realizations and options the application configures
    // (`waterui_map_gpu::install`, provider options, fonts), and the preview
    // style is constructed alongside exactly as `main` hands its style to
    // `hydrolysis::run`.
    let env = preview_symbol::app_environment();
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

/// Pumps until the frame stops changing and no `spawn_local` work is parked
/// on a wall-clock wake.
///
/// A `GpuView`'s `setup` is an async future spawned onto the local executor, so
/// its pipelines do not exist during the first frame. Capturing immediately
/// yields a snapshot of the surface before any GPU content was drawn — the
/// window background and nothing else. `rebuilt` is the real readiness signal
/// here, so pump on it rather than waiting a fixed amount of time.
///
/// Quiescence alone is not the whole story: a `spawn_local` task parked on a
/// timer or in-flight I/O — a `Photo` fetch, an `avatar` image — holds no
/// queued runnable, so the runtime cannot see it and reports the frame
/// settled while the placeholder is still on screen. While
/// [`waterui::task::outstanding_local_tasks`] reports such work, settling
/// paces real time — each pump runs whatever the last wake re-queued — until
/// the task publishes its result or the wall-clock cap elapses.
///
/// The virtual instant never moves while pacing: the scene is quiescent, so
/// nothing it scheduled needs a frame, and advancing the clock would carry
/// every animation past the phase the capture asked for. Once a paced pump
/// changes the tree, settling returns to the rebuild loop so whatever the
/// change scheduled can run before pacing resumes.
fn settle(runtime: &mut HeadlessRuntime, at: Instant) {
    /// Pump budget for a frame that never stops rebuilding — a perpetual
    /// animation; bounds pump work, not wall clock.
    const MAX_PUMPS: usize = 64;
    /// One frame of wall-clock time: the pacing step while local tasks are
    /// parked.
    const FRAME: Duration = Duration::from_millis(16);
    /// Wall-clock budget for work parked on real I/O — mirrors
    /// `waterui-testing`'s settle cap: long enough for a remote fetch on a
    /// slow link, bounded so a permanently parked task cannot hang the
    /// preview.
    const SETTLE_WALL_CAP: Duration = Duration::from_secs(5);

    let wall_deadline = Instant::now() + SETTLE_WALL_CAP;
    let mut pumps = 0usize;
    loop {
        if runtime.pump_at(false, at).rebuilt {
            pumps += 1;
            assert!(
                pumps < MAX_PUMPS,
                "hydrolysis preview: frame never settled after {MAX_PUMPS} pumps"
            );
            continue;
        }
        // Quiescent but a local task may be parked on a wall-clock wake: give
        // it real time, then run what it re-queued at the same instant.
        loop {
            if waterui::task::outstanding_local_tasks() == 0 || Instant::now() >= wall_deadline {
                return;
            }
            std::thread::sleep(FRAME);
            if runtime.pump_at(false, at).rebuilt {
                break;
            }
        }
    }
}

fn run_image(output_path: &Path, width: f32, height: f32) {
    let mut runtime = new_runtime(width, height);
    let frame_at = Instant::now();
    let _ = runtime.pump_at(false, frame_at);
    settle(&mut runtime, frame_at);
    let result = runtime.pump_at(true, frame_at);
    let snapshot = result
        .snapshot
        .unwrap_or_else(|| panic!("hydrolysis preview: static capture produced no snapshot"));
    write_snapshot_png(snapshot, output_path);
}

fn run_scenario(
    output_dir: &Path,
    captures_ms: &[u64],
    events: &[ScenarioEvent],
    width: f32,
    height: f32,
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
    let mut runtime = new_runtime(width, height);
    let started_at = Instant::now();
    let _ = runtime.pump_at(false, started_at);
    let mut event_index = 0usize;
    for capture_ms in captures_ms {
        while event_index < events.len() && events[event_index].at_ms <= *capture_ms {
            let event_at = started_at + Duration::from_millis(events[event_index].at_ms);
            runtime.push_input_event(input_event(events[event_index]));
            let _ = runtime.pump_at(false, event_at);
            event_index += 1;
        }
        let capture_at = started_at + Duration::from_millis(*capture_ms);
        // Work an interaction spawned — a tap that kicks off a fetch — parks
        // on wall-clock I/O exactly like mount-time work does, so a capture
        // settles by the same rule: the tree quiet *and* no parked task
        // waiting to publish.
        settle(&mut runtime, capture_at);
        let result = runtime.pump_at(true, capture_at);
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
    // `readback_rgba8` presents into the offscreen target with
    // `OutputAlpha::Premultiplied` — the convention the writer declares
    // for this capture.
    write_png(
        path,
        snapshot.width,
        snapshot.height,
        snapshot.rgba8,
        Alpha::Premultiplied,
    )
    .unwrap_or_else(|error| panic!("hydrolysis preview: {error}"));
}

fn input_event(event: ScenarioEvent) -> InputEvent {
    let button = match event.button {
        ScenarioPointerButton::Primary => PointerButton::Primary,
        ScenarioPointerButton::Secondary => PointerButton::Secondary,
        ScenarioPointerButton::Middle => PointerButton::Middle,
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
    }
}

fn dimension_to_u32(value: f32) -> u32 {
    assert!(
        value.is_finite() && value > 0.0,
        "hydrolysis preview dimension must be finite and positive"
    );
    value.round() as u32
}
