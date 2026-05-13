use crate::dimensions::{
    PROGRESS_CIRCULAR_DIAMETER, PROGRESS_CIRCULAR_STROKE_WIDTH, PROGRESS_LINEAR_BAR_HEIGHT,
    PROGRESS_LINEAR_BAR_HORIZONTAL_INSET, PROGRESS_LINEAR_BAR_TOP_OFFSET,
    PROGRESS_LINEAR_LABEL_HEIGHT, PROGRESS_LINEAR_MIN_TRACK_WIDTH,
    PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, ProgressIndicatorStyle, ProgressMetrics, lerp_color};
use core::f64::consts::FRAC_PI_2;
use core::time::Duration;
use vello::kurbo::{BezPath, Point, Rect};
use waterui::animation::Animation;

const LINEAR_INDETERMINATE_CYCLE: Duration = Duration::from_millis(2_000);
const CIRCULAR_ARC_DURATION: Duration = Duration::from_millis(1_333);
const CIRCULAR_CYCLE_DURATION: Duration = Duration::from_millis(5_332);
const CIRCULAR_LINEAR_ROTATE_DURATION_SECS: f64 = 1.333 * 360.0 / 306.0;
const CIRCULAR_MIN_SWEEP_DEGREES: f64 = 10.0;
const CIRCULAR_MAX_SWEEP_DEGREES: f64 = 270.0;

#[derive(Debug, Clone, Copy)]
struct Segment {
    at: f64,
    value: f64,
    easing: Option<(f32, f32, f32, f32)>,
}

const PRIMARY_SCALE: &[Segment] = &[
    Segment {
        at: 0.0,
        value: 0.08,
        easing: None,
    },
    Segment {
        at: 0.3665,
        value: 0.08,
        easing: Some((0.334731, 0.12482, 0.785844, 1.0)),
    },
    Segment {
        at: 0.6915,
        value: 0.661479,
        easing: Some((0.06, 0.11, 0.6, 1.0)),
    },
    Segment {
        at: 1.0,
        value: 0.08,
        easing: None,
    },
];
const PRIMARY_TRANSLATE: &[Segment] = &[
    Segment {
        at: 0.0,
        value: 0.0,
        easing: None,
    },
    Segment {
        at: 0.20,
        value: 0.0,
        easing: Some((0.5, 0.0, 0.701732, 0.495819)),
    },
    Segment {
        at: 0.5915,
        value: 83.6714,
        easing: Some((0.302435, 0.381352, 0.55, 0.956352)),
    },
    Segment {
        at: 1.0,
        value: 200.611,
        easing: None,
    },
];
const SECONDARY_SCALE: &[Segment] = &[
    Segment {
        at: 0.0,
        value: 0.08,
        easing: Some((0.205028, 0.057051, 0.57661, 0.453971)),
    },
    Segment {
        at: 0.1915,
        value: 0.457104,
        easing: Some((0.152313, 0.196432, 0.648374, 1.00432)),
    },
    Segment {
        at: 0.4415,
        value: 0.72796,
        easing: Some((0.257759, -0.003163, 0.211762, 1.38179)),
    },
    Segment {
        at: 1.0,
        value: 0.08,
        easing: None,
    },
];
const SECONDARY_TRANSLATE: &[Segment] = &[
    Segment {
        at: 0.0,
        value: 0.0,
        easing: Some((0.15, 0.0, 0.515058, 0.409685)),
    },
    Segment {
        at: 0.25,
        value: 37.6519,
        easing: Some((0.31033, 0.284058, 0.8, 0.733712)),
    },
    Segment {
        at: 0.4835,
        value: 84.3862,
        easing: Some((0.4, 0.627035, 0.6, 0.902026)),
    },
    Segment {
        at: 1.0,
        value: 160.278,
        easing: None,
    },
];

pub fn metrics(style: ProgressIndicatorStyle) -> ProgressMetrics {
    match style {
        ProgressIndicatorStyle::Linear => ProgressMetrics::linear(
            PROGRESS_LINEAR_LABEL_HEIGHT,
            PROGRESS_LINEAR_BAR_TOP_OFFSET,
            PROGRESS_LINEAR_BAR_HEIGHT,
            PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
            PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
            PROGRESS_LINEAR_MIN_TRACK_WIDTH,
        ),
        ProgressIndicatorStyle::Circular => {
            ProgressMetrics::circular(PROGRESS_CIRCULAR_DIAMETER, PROGRESS_CIRCULAR_STROKE_WIDTH)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgressIndicatorStyle, metrics};

    #[test]
    fn progress_metrics_match_material_web_v0_192() {
        let linear = metrics(ProgressIndicatorStyle::Linear);
        assert_eq!(linear.bar_height, 4.0);
        assert_eq!(linear.min_track_width, 80.0);

        let circular = metrics(ProgressIndicatorStyle::Circular);
        assert_eq!(circular.circular_diameter, 48.0);
        assert_eq!(circular.circular_stroke_width, 4.0);
    }
}

pub fn draw_linear_track(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    draw.fill_rect(
        bounds,
        &Brush::from(colors.surface_container_highest.peniko()),
    );
}

pub fn draw_linear_fill(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    draw.fill_rect(bounds, &Brush::from(colors.primary.peniko()));
}

pub fn draw_circular_track(
    _colors: &MaterialColorScheme,
    _draw: &mut dyn DrawContext,
    _center: vello::kurbo::Point,
    _radius: f64,
    _width: f64,
) {
}

pub fn draw_circular_fill(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    path: &vello::kurbo::BezPath,
    width: f64,
) {
    draw.stroke_path(path, &Brush::from(colors.primary.peniko()), width);
}

pub fn draw_linear_indeterminate(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    elapsed: Duration,
    four_color: bool,
) {
    let color = progress_color(colors, elapsed, four_color, LINEAR_INDETERMINATE_CYCLE * 2);
    draw.push_layer(1.0, Some(&bounds));
    draw_indeterminate_bar(
        draw,
        bounds,
        -145.167,
        sample_segments(PRIMARY_TRANSLATE, elapsed, LINEAR_INDETERMINATE_CYCLE),
        sample_segments(PRIMARY_SCALE, elapsed, LINEAR_INDETERMINATE_CYCLE),
        color,
    );
    draw_indeterminate_bar(
        draw,
        bounds,
        -54.8889,
        sample_segments(SECONDARY_TRANSLATE, elapsed, LINEAR_INDETERMINATE_CYCLE),
        sample_segments(SECONDARY_SCALE, elapsed, LINEAR_INDETERMINATE_CYCLE),
        color,
    );
    draw.pop_layer();
}

pub fn draw_circular_indeterminate(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: Point,
    radius: f64,
    width: f64,
    elapsed: Duration,
    four_color: bool,
) {
    let color = progress_color(colors, elapsed, four_color, CIRCULAR_CYCLE_DURATION);
    let (start, sweep) = circular_indeterminate_arc(elapsed);
    let arc = circle_arc_path(center, radius, start, sweep);
    draw.stroke_path(&arc, &Brush::from(color), width);
}

fn draw_indeterminate_bar(
    draw: &mut dyn DrawContext,
    track: Rect,
    initial_inset_percent: f64,
    translate_percent: f64,
    scale: f64,
    color: vello::peniko::Color,
) {
    let x = track.x0 + track.width() * ((initial_inset_percent + translate_percent) / 100.0);
    let width = (track.width() * scale).max(0.0);
    let rect = Rect::new(x, track.y0, x + width, track.y1);
    draw.fill_rect(rect, &Brush::from(color));
}

fn sample_segments(segments: &[Segment], elapsed: Duration, cycle: Duration) -> f64 {
    let phase = cycle_phase(elapsed, cycle);
    for window in segments.windows(2) {
        let start = window[0];
        let end = window[1];
        if phase < start.at || phase > end.at {
            continue;
        }
        if (end.at - start.at).abs() <= f64::EPSILON {
            return end.value;
        }
        let local = ((phase - start.at) / (end.at - start.at)).clamp(0.0, 1.0);
        let eased = start.easing.map_or(local as f32, |(x1, y1, x2, y2)| {
            Animation::bezier(Duration::from_millis(1), x1, y1, x2, y2)
                .progress(Duration::from_secs_f64(local / 1_000.0))
        });
        return (end.value - start.value).mul_add(f64::from(eased), start.value);
    }
    segments
        .last()
        .expect("indeterminate progress segments must not be empty")
        .value
}

fn cycle_phase(elapsed: Duration, cycle: Duration) -> f64 {
    let cycle = cycle.as_secs_f64();
    assert!(
        cycle > 0.0,
        "Material progress cycle duration must be positive"
    );
    (elapsed.as_secs_f64() % cycle) / cycle
}

fn progress_color(
    colors: &MaterialColorScheme,
    elapsed: Duration,
    four_color: bool,
    cycle: Duration,
) -> vello::peniko::Color {
    if !four_color {
        return colors.primary.peniko();
    }
    let phase = cycle_phase(elapsed, cycle);
    let palette = [
        colors.primary.peniko(),
        colors.primary_container.peniko(),
        colors.tertiary.peniko(),
        colors.tertiary_container.peniko(),
    ];
    sample_color_phase(phase, palette)
}

fn sample_color_phase(phase: f64, palette: [vello::peniko::Color; 4]) -> vello::peniko::Color {
    match phase {
        p if p < 0.15 => palette[0],
        p if p < 0.25 => lerp_color(palette[0], palette[1], ((p - 0.15) / 0.10) as f32),
        p if p < 0.40 => palette[1],
        p if p < 0.50 => lerp_color(palette[1], palette[2], ((p - 0.40) / 0.10) as f32),
        p if p < 0.65 => palette[2],
        p if p < 0.75 => lerp_color(palette[2], palette[3], ((p - 0.65) / 0.10) as f32),
        p if p < 0.90 => palette[3],
        _ => lerp_color(palette[3], palette[0], ((phase - 0.90) / 0.10) as f32),
    }
}

fn circular_indeterminate_arc(elapsed: Duration) -> (f64, f64) {
    let arc_phase = cycle_phase(elapsed, CIRCULAR_ARC_DURATION);
    let arc_ease = if arc_phase <= 0.5 {
        f64::from(
            Animation::bezier(Duration::from_millis(1), 0.4, 0.0, 0.2, 1.0)
                .progress(Duration::from_secs_f64(arc_phase * 2.0 / 1_000.0)),
        )
    } else {
        1.0 - f64::from(
            Animation::bezier(Duration::from_millis(1), 0.4, 0.0, 0.2, 1.0)
                .progress(Duration::from_secs_f64((arc_phase - 0.5) * 2.0 / 1_000.0)),
        )
    };
    let sweep_degrees = CIRCULAR_MIN_SWEEP_DEGREES
        + (CIRCULAR_MAX_SWEEP_DEGREES - CIRCULAR_MIN_SWEEP_DEGREES) * arc_ease;
    let rotate_arc_degrees = cycle_phase(elapsed, CIRCULAR_CYCLE_DURATION) * 1080.0;
    let linear_rotate_degrees = (elapsed.as_secs_f64() % CIRCULAR_LINEAR_ROTATE_DURATION_SECS)
        / CIRCULAR_LINEAR_ROTATE_DURATION_SECS
        * 360.0;
    (
        -FRAC_PI_2 + (rotate_arc_degrees + linear_rotate_degrees).to_radians(),
        sweep_degrees.to_radians(),
    )
}

fn circle_arc_path(center: Point, radius: f64, start_angle: f64, sweep: f64) -> BezPath {
    let mut path = BezPath::new();
    if sweep == 0.0 {
        return path;
    }
    let segments = 64usize;
    let step = sweep / segments as f64;
    let mut angle = start_angle;
    path.move_to(Point::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    ));
    for _ in 0..segments {
        angle += step;
        path.line_to(Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    path
}
