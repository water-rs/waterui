//! Hydrolysis preview runtime for {{ ctx.app_display_name }}.

use std::{env, fs, path::PathBuf, time::{Duration, Instant}};

use crate::preview_symbol;
use hydrolysis::{HeadlessRuntime, HydrolysisViewRenderer, InputEvent, PointerButton};
use waterui_preview::{RenderResult, RenderResultExt as _, RenderSize, ViewRenderer};
use waterui_core::handler::AnyViewBuilder;

pub(crate) fn run() {
    let output_path = PathBuf::from(required_env(preview_symbol::PREVIEW_OUTPUT_ENV));
    let width = parse_dimension(preview_symbol::PREVIEW_WIDTH_ENV);
    let height = parse_dimension(preview_symbol::PREVIEW_HEIGHT_ENV);
    if let Some(output_dir) = env::var_os(preview_symbol::PREVIEW_OUTPUT_DIR_ENV) {
        run_scenario(PathBuf::from(output_dir), width, height);
        return;
    }

    let view = preview_symbol::load_preview_view();
    let renderer = ViewRenderer::new(HydrolysisViewRenderer::with_environment(
        preview_symbol::install_preview_theme,
    ));
    let mut render = pollster::block_on(renderer.render(view, RenderSize::new(width, height)));
    flatten_alpha_over_white(&mut render);
    let png_data = render
        .into_png()
        .unwrap_or_else(|error| panic!("hydrolysis preview: failed to encode PNG: {error}"));

    fs::write(&output_path, png_data).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview: failed to write `{}`: {error}",
            output_path.display()
        )
    });
}

fn run_scenario(output_dir: PathBuf, width: f32, height: f32) {
    fs::create_dir_all(&output_dir).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview: failed to create scenario output dir `{}`: {error}",
            output_dir.display()
        )
    });
    let captures = parse_captures();
    let events = parse_events();
    let mut env = waterui::env::Environment::new();
    preview_symbol::install_preview_theme(&mut env);
    let content = AnyViewBuilder::new(preview_symbol::load_preview_view);
    let mut runtime = HeadlessRuntime::new(
        env,
        content,
        dimension_to_u32(width),
        dimension_to_u32(height),
    );
    let started_at = Instant::now();
    let mut event_index = 0usize;
    for capture_ms in captures {
        while event_index < events.len() && events[event_index].at_ms <= capture_ms {
            let event_at = started_at + Duration::from_millis(events[event_index].at_ms);
            runtime.push_input_event(events[event_index].input_event());
            let _ = runtime.pump_at(false, event_at);
            event_index += 1;
        }
        let capture_at = started_at + Duration::from_millis(capture_ms);
        let result = runtime.pump_at(true, capture_at);
        let Some(snapshot) = result.snapshot else {
            panic!("hydrolysis preview: scenario capture at {capture_ms}ms produced no snapshot");
        };
        let mut render = RenderResult {
            width: snapshot.width,
            height: snapshot.height,
            rgba_data: snapshot.rgba8,
        };
        flatten_alpha_over_white(&mut render);
        let png_data = render
            .into_png()
            .unwrap_or_else(|error| panic!("hydrolysis preview: failed to encode PNG: {error}"));
        let path = output_dir.join(format!("frame-{capture_ms:04}ms.png"));
        fs::write(&path, png_data).unwrap_or_else(|error| {
            panic!(
                "hydrolysis preview: failed to write scenario frame `{}`: {error}",
                path.display()
            )
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct ScenarioEvent {
    at_ms: u64,
    kind: ScenarioEventKind,
    x: f32,
    y: f32,
    button: PointerButton,
}

#[derive(Debug, Clone, Copy)]
enum ScenarioEventKind {
    PointerMove,
    PointerDown,
    PointerUp,
    PointerCancel,
}

impl ScenarioEvent {
    fn input_event(self) -> InputEvent {
        match self.kind {
            ScenarioEventKind::PointerMove => InputEvent::PointerMove {
                x: self.x,
                y: self.y,
            },
            ScenarioEventKind::PointerDown => InputEvent::PointerDown {
                x: self.x,
                y: self.y,
                button: self.button,
            },
            ScenarioEventKind::PointerUp => InputEvent::PointerUp {
                x: self.x,
                y: self.y,
                button: self.button,
            },
            ScenarioEventKind::PointerCancel => InputEvent::PointerCancel,
        }
    }
}

fn required_env(name: &str) -> String {
    env::var(name)
        .unwrap_or_else(|error| panic!("hydrolysis preview: missing environment variable `{name}`: {error}"))
}

fn parse_dimension(name: &str) -> f32 {
    let raw = required_env(name);
    raw.parse::<f32>()
        .unwrap_or_else(|error| panic!("hydrolysis preview: invalid `{name}` value `{raw}`: {error}"))
}

fn dimension_to_u32(value: f32) -> u32 {
    assert!(
        value.is_finite() && value > 0.0,
        "hydrolysis preview dimension must be finite and positive"
    );
    value.round() as u32
}

fn parse_captures() -> Vec<u64> {
    let raw = required_env(preview_symbol::PREVIEW_CAPTURE_MS_ENV);
    let captures = raw
        .split(',')
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<u64>().unwrap_or_else(|error| {
                panic!("hydrolysis preview: invalid capture timestamp `{part}`: {error}")
            })
        })
        .collect::<Vec<_>>();
    assert!(
        !captures.is_empty(),
        "hydrolysis preview scenario requires at least one capture"
    );
    captures
}

fn parse_events() -> Vec<ScenarioEvent> {
    let raw = env::var(preview_symbol::PREVIEW_EVENTS_ENV).unwrap_or_default();
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split(';')
        .filter(|part| !part.is_empty())
        .map(parse_event)
        .collect()
}

fn parse_event(raw: &str) -> ScenarioEvent {
    let mut parts = raw.split(':');
    let at = parts
        .next()
        .unwrap_or_else(|| panic!("hydrolysis preview: scenario event `{raw}` is missing time"));
    let kind = parts
        .next()
        .unwrap_or_else(|| panic!("hydrolysis preview: scenario event `{raw}` is missing kind"));
    let x = parts
        .next()
        .unwrap_or_else(|| panic!("hydrolysis preview: scenario event `{raw}` is missing x"));
    let y = parts
        .next()
        .unwrap_or_else(|| panic!("hydrolysis preview: scenario event `{raw}` is missing y"));
    let button = parts.next().unwrap_or("");
    assert!(
        parts.next().is_none(),
        "hydrolysis preview: scenario event `{raw}` has too many fields"
    );
    ScenarioEvent {
        at_ms: at.parse::<u64>().unwrap_or_else(|error| {
            panic!("hydrolysis preview: invalid event timestamp `{at}`: {error}")
        }),
        kind: match kind {
            "pointer_move" => ScenarioEventKind::PointerMove,
            "pointer_down" => ScenarioEventKind::PointerDown,
            "pointer_up" => ScenarioEventKind::PointerUp,
            "pointer_cancel" => ScenarioEventKind::PointerCancel,
            _ => panic!("hydrolysis preview: unsupported scenario event kind `{kind}`"),
        },
        x: x.parse::<f32>()
            .unwrap_or_else(|error| panic!("hydrolysis preview: invalid event x `{x}`: {error}")),
        y: y.parse::<f32>()
            .unwrap_or_else(|error| panic!("hydrolysis preview: invalid event y `{y}`: {error}")),
        button: match button {
            "" | "primary" => PointerButton::Primary,
            "secondary" => PointerButton::Secondary,
            "middle" => PointerButton::Middle,
            _ => panic!("hydrolysis preview: unsupported pointer button `{button}`"),
        },
    }
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
