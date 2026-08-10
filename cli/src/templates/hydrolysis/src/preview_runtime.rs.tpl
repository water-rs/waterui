//! Hydrolysis preview runtime for {{ ctx.app_display_name }}.

use std::{
    fs,
    path::Path,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::preview_symbol;
use hydrolysis::{HeadlessRuntime, InputEvent, PointerButton, PointerKind};
use waterui_core::handler::AnyViewBuilder;
use waterui_preview::{RenderResult, RenderResultExt as _};
use waterui_preview_protocol::hydrolysis::{
    PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode, ScenarioEvent, ScenarioEventKind,
    ScenarioPointerButton,
};

pub(crate) fn run() {
    let config = load_run_config();
    match config.mode {
        PreviewRunMode::Image { ref output } => {
            run_image(output, config.width, config.height);
        }
        PreviewRunMode::Scenario {
            ref output_dir,
            ref captures_ms,
            ref events,
        } => run_scenario(output_dir, captures_ms, events, config.width, config.height),
        PreviewRunMode::Semantic | PreviewRunMode::Perf(_) => panic!(
            "hydrolysis preview: semantic/perf runs require the preview test binary (waterui-preview-test-mode)"
        ),
    }
}

fn load_run_config() -> PreviewRunConfig {
    let path = std::env::var_os(PREVIEW_RUN_CONFIG_ENV).unwrap_or_else(|| {
        panic!("hydrolysis preview: missing environment variable `{PREVIEW_RUN_CONFIG_ENV}`")
    });
    let raw = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview: failed to read run config `{}`: {error}",
            PathBuf::from(&path).display()
        )
    });
    serde_json::from_slice(&raw).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview: failed to parse run config `{}`: {error}",
            PathBuf::from(&path).display()
        )
    })
}

fn new_runtime(width: f32, height: f32) -> HeadlessRuntime {
    let mut env = waterui::env::Environment::new();
    preview_symbol::install_preview_theme(&mut env);
    let content = AnyViewBuilder::new(preview_symbol::load_preview_view);
    HeadlessRuntime::new(
        env,
        content,
        dimension_to_u32(width),
        dimension_to_u32(height),
    )
}

fn run_image(output_path: &Path, width: f32, height: f32) {
    let mut runtime = new_runtime(width, height);
    let frame_at = Instant::now();
    let _ = runtime.pump_semantic_at(frame_at);
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
    let _ = runtime.pump_semantic_at(started_at);
    let mut event_index = 0usize;
    for capture_ms in captures_ms {
        while event_index < events.len() && events[event_index].at_ms <= *capture_ms {
            let event_at = started_at + Duration::from_millis(events[event_index].at_ms);
            runtime.push_input_event(input_event(events[event_index]));
            let _ = runtime.pump_at(false, event_at);
            event_index += 1;
        }
        let capture_at = started_at + Duration::from_millis(*capture_ms);
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
    let mut render = RenderResult {
        width: snapshot.width,
        height: snapshot.height,
        rgba_data: snapshot.rgba8,
    };
    flatten_alpha_over_white(&mut render);
    let png_data = render
        .into_png()
        .unwrap_or_else(|error| panic!("hydrolysis preview: failed to encode PNG: {error}"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|error| {
            panic!(
                "hydrolysis preview: failed to create output directory `{}`: {error}",
                parent.display()
            )
        });
    }
    fs::write(path, png_data).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview: failed to write `{}`: {error}",
            path.display()
        )
    });
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

fn flatten_alpha_over_white(render: &mut RenderResult) {
    for pixel in render.rgba_data.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        let inv_alpha = 255_u16
            .checked_sub(alpha)
            .expect("preview alpha channel must be <= 255");
        for channel in &mut pixel[..3] {
            let source = u16::from(*channel);
            let blended = source
                .checked_mul(alpha)
                .and_then(|value| value.checked_add(255_u16.checked_mul(inv_alpha)?))
                .and_then(|value| value.checked_add(127))
                .map(|value| value / 255)
                .expect("preview RGB blending overflowed");
            *channel = u8::try_from(blended).expect("preview RGB channel must fit into u8");
        }
        pixel[3] = 255;
    }
}
