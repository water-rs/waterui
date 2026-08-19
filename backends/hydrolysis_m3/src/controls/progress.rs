use crate::dimensions::{
    PROGRESS_CIRCULAR_DIAMETER, PROGRESS_CIRCULAR_STROKE_WIDTH,
    PROGRESS_CIRCULAR_TRACK_ACTIVE_SPACE, PROGRESS_LINEAR_BAR_HEIGHT,
    PROGRESS_LINEAR_BAR_HORIZONTAL_INSET, PROGRESS_LINEAR_BAR_TOP_OFFSET,
    PROGRESS_LINEAR_LABEL_HEIGHT, PROGRESS_LINEAR_MIN_TRACK_WIDTH, PROGRESS_LINEAR_STOP_SIZE,
    PROGRESS_LINEAR_TRACK_ACTIVE_SPACE, PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
};
use crate::material_shapes::{material_shape_sequence, morph, radii_to_path};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, ProgressIndicatorStyle, ProgressMetrics, lerp_color};
use core::f64::consts::FRAC_PI_2;
use core::time::Duration;
use num_traits::ToPrimitive;
use vello::kurbo::{BezPath, Point, Rect};
use waterui::animation::Animation;

/// `LinearAnimationDuration` in Compose's `ProgressIndicator.kt`.
const LINEAR_INDETERMINATE_CYCLE: Duration = Duration::from_millis(1_750);
/// `CircularAnimationProgressDuration`: one full indeterminate cycle.
const CIRCULAR_CYCLE_DURATION: Duration = Duration::from_secs(6);
/// `CircularAnimationAdditionalRotationDuration`: how long each 90-degree
/// kick of the additional rotation takes.
const CIRCULAR_ADDITIONAL_ROTATION_DURATION: Duration = Duration::from_millis(300);
/// `CircularAnimationAdditionalRotationDelay`: the spacing between those kicks.
const CIRCULAR_ADDITIONAL_ROTATION_DELAY: Duration = Duration::from_millis(1_500);
/// `CircularGlobalRotationDegreesTarget`: three turns per cycle, linear.
const CIRCULAR_GLOBAL_ROTATION_DEGREES: f64 = 1_080.0;
/// `CircularAdditionalRotationDegreesTarget`, reached in four 90-degree steps.
const CIRCULAR_ADDITIONAL_ROTATION_DEGREES: f64 = 360.0;
/// `CircularIndeterminateMinProgress`: the shortest the arc gets.
const CIRCULAR_INDETERMINATE_MIN_PROGRESS: f64 = 0.1;
/// `CircularIndeterminateMaxProgress`: the longest it grows to, half a cycle in.
const CIRCULAR_INDETERMINATE_MAX_PROGRESS: f64 = 0.87;

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
        easing: Some((0.334_731, 0.12482, 0.785_844, 1.0)),
    },
    Segment {
        at: 0.6915,
        value: 0.661_479,
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
        easing: Some((0.5, 0.0, 0.701_732, 0.495_819)),
    },
    Segment {
        at: 0.5915,
        value: 83.6714,
        easing: Some((0.302_435, 0.381_352, 0.55, 0.956_352)),
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
        easing: Some((0.205_028, 0.057_051, 0.57661, 0.453_971)),
    },
    Segment {
        at: 0.1915,
        value: 0.457_104,
        easing: Some((0.152_313, 0.196_432, 0.648_374, 1.00432)),
    },
    Segment {
        at: 0.4415,
        value: 0.72796,
        easing: Some((0.257_759, -0.003_163, 0.211_762, 1.38179)),
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
        easing: Some((0.15, 0.0, 0.515_058, 0.409_685)),
    },
    Segment {
        at: 0.25,
        value: 37.6519,
        easing: Some((0.31033, 0.284_058, 0.8, 0.733_712)),
    },
    Segment {
        at: 0.4835,
        value: 84.3862,
        easing: Some((0.4, 0.627_035, 0.6, 0.902_026)),
    },
    Segment {
        at: 1.0,
        value: 160.278,
        easing: None,
    },
];

pub const fn metrics(style: ProgressIndicatorStyle) -> ProgressMetrics {
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
        ProgressIndicatorStyle::Loading => loading_metrics(),
    }
}

/// Draws the linear track: fully rounded, stopping clear of the active
/// indicator, with a stop indicator at the far end.
///
/// Expressive gives the track and indicator `CornerFull` ends and separates
/// them by `TrackActiveSpace`, so the bar reads as two pieces rather than one
/// square-ended strip filling from the left.
pub fn draw_linear_track(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    active_end: Option<f64>,
) {
    let radius = (bounds.height() / 2.0).into();
    let track_color = colors.surface_container_highest.peniko();
    let start = active_end.map_or(bounds.x0, |end| {
        (end + PROGRESS_LINEAR_TRACK_ACTIVE_SPACE).min(bounds.x1)
    });
    if start < bounds.x1 {
        draw.fill_rounded_rect(
            vello::kurbo::Rect::new(start, bounds.y0, bounds.x1, bounds.y1),
            radius,
            &Brush::from(track_color),
        );
    }
    // The stop indicator only exists on a determinate bar.
    if active_end.is_some() {
        let center = vello::kurbo::Point::new(
            bounds.x1 - PROGRESS_LINEAR_STOP_SIZE / 2.0,
            bounds.y0 + bounds.height() / 2.0,
        );
        if center.x > start {
            draw.fill_circle(
                center,
                PROGRESS_LINEAR_STOP_SIZE / 2.0,
                &Brush::from(colors.primary.peniko()),
            );
        }
    }
}

pub fn draw_linear_fill(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    draw.fill_rounded_rect(
        bounds,
        (bounds.height() / 2.0).into(),
        &Brush::from(colors.primary.peniko()),
    );
}

/// Draws the circular track: the arc the active sweep has not covered, held
/// clear of it by `TrackActiveSpace` at both ends.
///
/// The track was previously not drawn at all, so a determinate circular
/// indicator showed only its active arc against the background. Compose always
/// draws a track behind the indicator.
pub fn draw_circular_track(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    radius: f64,
    width: f64,
    active_turns: Option<f64>,
) {
    use core::f64::consts::{FRAC_PI_2, TAU};
    use vello::kurbo::Shape as _;

    let brush = Brush::from(colors.surface_container_highest.peniko());
    let Some(active_turns) = active_turns else {
        // Indeterminate: the sweep moves, so the whole ring stays behind it.
        draw.stroke_path(
            &vello::kurbo::Arc::new(center, (radius, radius), -FRAC_PI_2, TAU, 0.0).into_path(0.1),
            &brush,
            width,
        );
        return;
    };

    // Convert the gap from an arc length into an angle at this radius.
    let gap = if radius > 0.0 {
        PROGRESS_CIRCULAR_TRACK_ACTIVE_SPACE / radius
    } else {
        0.0
    };
    let active = TAU * active_turns.clamp(0.0, 1.0);
    let sweep = TAU - active - gap * 2.0;
    if sweep <= 0.0 {
        return;
    }
    draw.stroke_path(
        &vello::kurbo::Arc::new(
            center,
            (radius, radius),
            -FRAC_PI_2 + active + gap,
            sweep,
            0.0,
        )
        .into_path(0.1),
        &brush,
        width,
    );
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
    let x = track.width().mul_add(
        (initial_inset_percent + translate_percent) / 100.0,
        track.x0,
    );
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
        let eased = start.easing.map_or_else(
            || {
                local
                    .to_f32()
                    .expect("progress phase must be representable as f32")
            },
            |(x1, y1, x2, y2)| {
                Animation::bezier(Duration::from_millis(1), x1, y1, x2, y2)
                    .progress(Duration::from_secs_f64(local / 1_000.0))
            },
        );
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
        p if p < 0.25 => lerp_color(palette[0], palette[1], progress_phase((p - 0.15) / 0.10)),
        p if p < 0.40 => palette[1],
        p if p < 0.50 => lerp_color(palette[1], palette[2], progress_phase((p - 0.40) / 0.10)),
        p if p < 0.65 => palette[2],
        p if p < 0.75 => lerp_color(palette[2], palette[3], progress_phase((p - 0.65) / 0.10)),
        p if p < 0.90 => palette[3],
        _ => lerp_color(
            palette[3],
            palette[0],
            progress_phase((phase - 0.90) / 0.10),
        ),
    }
}

fn progress_phase(value: f64) -> f32 {
    value
        .to_f32()
        .expect("progress phase must be representable as f32")
}

/// The indeterminate arc's start angle and sweep, following Compose's
/// `CircularProgressIndicator`.
///
/// Three animations run over one 6s cycle and compose into the motion: a linear
/// global rotation of three full turns, an additional rotation that kicks
/// through four 90-degree steps spaced 1.5s apart, and a progress value that
/// grows the arc from 10% to 87% of the circle by mid-cycle and back. The two
/// rotations are summed, exactly as Compose's `rotate(globalRotation +
/// additionalRotation)` does, and the sweep is `progress * 360`.
fn circular_indeterminate_arc(elapsed: Duration) -> (f64, f64) {
    let cycle = cycle_phase(elapsed, CIRCULAR_CYCLE_DURATION);
    let global_rotation = cycle * CIRCULAR_GLOBAL_ROTATION_DEGREES;
    let additional_rotation = circular_additional_rotation(cycle);
    let progress = circular_indeterminate_progress(cycle);
    (
        -FRAC_PI_2 + (global_rotation + additional_rotation).to_radians(),
        (progress * 360.0).to_radians(),
    )
}

/// The additional rotation at `cycle`: four 90-degree steps, each eased with
/// the emphasized-decelerate curve and held until the next one begins.
fn circular_additional_rotation(cycle: f64) -> f64 {
    const STEPS: f64 = 4.0;
    let cycle_secs = CIRCULAR_CYCLE_DURATION.as_secs_f64();
    let delay_secs = CIRCULAR_ADDITIONAL_ROTATION_DELAY.as_secs_f64();
    let step_secs = CIRCULAR_ADDITIONAL_ROTATION_DURATION.as_secs_f64();
    let elapsed_secs = cycle * cycle_secs;
    let completed = (elapsed_secs / delay_secs).floor().min(STEPS);
    let into_step = completed.mul_add(-delay_secs, elapsed_secs);
    let step_progress = if step_secs > 0.0 {
        (into_step / step_secs).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let eased = f64::from(
        crate::theme::motion::tokens::easing::EMPHASIZED_DECELERATE
            .ease(progress_phase(step_progress)),
    );
    (completed + eased).min(STEPS) / STEPS * CIRCULAR_ADDITIONAL_ROTATION_DEGREES
}

/// The arc length at `cycle`: growing to `CircularIndeterminateMaxProgress` by
/// mid-cycle, then shrinking back to `CircularIndeterminateMinProgress`.
fn circular_indeterminate_progress(cycle: f64) -> f64 {
    let span = CIRCULAR_INDETERMINATE_MAX_PROGRESS - CIRCULAR_INDETERMINATE_MIN_PROGRESS;
    let half = if cycle <= 0.5 {
        cycle * 2.0
    } else {
        (1.0 - cycle) * 2.0
    };
    let eased =
        f64::from(crate::theme::motion::tokens::easing::STANDARD.ease(progress_phase(half)));
    span.mul_add(eased, CIRCULAR_INDETERMINATE_MIN_PROGRESS)
}

fn circle_arc_path(center: Point, radius: f64, start_angle: f64, sweep: f64) -> BezPath {
    let mut path = BezPath::new();
    if sweep == 0.0 {
        return path;
    }
    let segments = 64usize;
    let step = sweep
        / segments
            .to_f64()
            .expect("progress arc segment count must be representable as f64");
    let mut angle = start_angle;
    path.move_to(Point::new(
        radius.mul_add(angle.cos(), center.x),
        radius.mul_add(angle.sin(), center.y),
    ));
    for _ in 0..segments {
        angle += step;
        path.line_to(Point::new(
            radius.mul_add(angle.cos(), center.x),
            radius.mul_add(angle.sin(), center.y),
        ));
    }
    path
}

// ---------------------------------------------------------------------------
// Loading indicator
// ---------------------------------------------------------------------------

/// `LoadingIndicatorTokens.ContainerWidth` / `ContainerHeight`.
pub const LOADING_CONTAINER_SIZE: f64 = 48.0;
/// `LoadingIndicatorTokens.ActiveSize`: the shape inside that container.
pub const LOADING_INDICATOR_SIZE: f64 = 38.0;
/// `MorphIntervalMillis`: how long the indicator rests on each shape.
const LOADING_MORPH_INTERVAL_MS: u64 = 650;
const LOADING_MORPH_INTERVAL: Duration = Duration::from_millis(LOADING_MORPH_INTERVAL_MS);
/// `GlobalRotationDurationMillis`: one full turn, linear.
const LOADING_GLOBAL_ROTATION_MS: u64 = 4_666;
const LOADING_GLOBAL_ROTATION: Duration = Duration::from_millis(LOADING_GLOBAL_ROTATION_MS);
/// `QuarterRotation`: each morph also kicks the shape a quarter turn on.
const LOADING_QUARTER_ROTATION: f64 = core::f64::consts::FRAC_PI_2;

/// Returns the metrics for a loading indicator.
pub const fn loading_metrics() -> ProgressMetrics {
    ProgressMetrics::loading(LOADING_CONTAINER_SIZE, LOADING_INDICATOR_SIZE)
}

/// The period over which the morph and the rotation come back into phase.
///
/// The shape cycle is seven morph intervals and the rotation is its own
/// duration; neither divides the other, so a single repeating clock has to run
/// for their least common multiple. Sampling one clock — rather than two —
/// keeps both beats derived from the same instant, so they can never drift
/// apart across a dropped frame.
pub fn loading_cycle() -> Duration {
    let shapes =
        u64::try_from(material_shape_sequence().len()).expect("the shape sequence is seven long");
    let morph = LOADING_MORPH_INTERVAL_MS * shapes;
    Duration::from_millis(
        morph / gcd(morph, LOADING_GLOBAL_ROTATION_MS) * LOADING_GLOBAL_ROTATION_MS,
    )
}

/// Greatest common divisor, for putting the two beats on one clock.
const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

/// The indicator's outline at `elapsed` into its cycle, in its own unit space.
///
/// Compose springs between consecutive shapes; the spring is critically damped
/// enough at these intervals that the visible motion is the settle, so this
/// eases the morph with the standard emphasized curve over the interval.
fn loading_outline(elapsed: Duration) -> Vec<f64> {
    let shapes = material_shape_sequence();
    // Which shape is counted in whole milliseconds, so a long-running indicator
    // cannot drift onto the wrong one through accumulated float error; only the
    // fraction within an interval needs to be continuous.
    let millis =
        u64::try_from(elapsed.as_millis()).expect("the loading phase is bounded by its own cycle");
    let steps = millis / LOADING_MORPH_INTERVAL_MS;
    let index = usize::try_from(steps).unwrap_or(0) % shapes.len();
    let next = (index + 1) % shapes.len();
    let interval = LOADING_MORPH_INTERVAL.as_secs_f64();
    let progress =
        Duration::from_millis(millis % LOADING_MORPH_INTERVAL_MS).as_secs_f64() / interval;
    morph(&shapes[index], &shapes[next], ease_in_out(progress))
}

/// A smoothstep, standing in for the settle of Compose's morph spring.
fn ease_in_out(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    progress * progress * 3.0f64.mul_add(-2.0 * progress / 3.0, 3.0) / 3.0
}

/// How far the indicator has turned at `elapsed`: a steady rotation, plus the
/// quarter turn each completed morph adds.
fn loading_rotation(elapsed: Duration) -> f64 {
    let turns = elapsed.as_secs_f64() / LOADING_GLOBAL_ROTATION.as_secs_f64();
    let morphs = (elapsed.as_secs_f64() / LOADING_MORPH_INTERVAL.as_secs_f64()).floor();
    core::f64::consts::TAU.mul_add(turns, LOADING_QUARTER_ROTATION * morphs)
}

pub fn draw_loading(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    elapsed: Duration,
    four_color: bool,
) {
    let radii = loading_outline(elapsed);
    let centre = bounds.center();
    // `ActiveIndicatorScale`: the shape is drawn at ActiveSize inside a
    // ContainerSize box, so it never touches the container's edge.
    let scale = LOADING_INDICATOR_SIZE.min(bounds.width().min(bounds.height())) / 2.0;
    let mut path = radii_to_path(&radii, Point::ORIGIN, scale);
    path.apply_affine(
        vello::kurbo::Affine::translate((centre.x, centre.y))
            * vello::kurbo::Affine::rotate(loading_rotation(elapsed)),
    );
    let color = progress_color(colors, elapsed, four_color, LOADING_GLOBAL_ROTATION);
    draw.fill_path(&path, &Brush::from(color));
}

#[cfg(test)]
mod tests {
    use super::{ProgressIndicatorStyle, metrics};
    use crate::dimensions::{
        PROGRESS_CIRCULAR_TRACK_ACTIVE_SPACE, PROGRESS_LINEAR_STOP_SIZE,
        PROGRESS_LINEAR_TRACK_ACTIVE_SPACE,
    };

    /// The indeterminate circular motion follows Compose's
    /// `CircularProgressIndicator`: a 6s cycle whose arc grows from
    /// `CircularIndeterminateMinProgress` to `CircularIndeterminateMaxProgress`
    /// at the halfway point and shrinks back.
    #[test]
    fn circular_indeterminate_sweep_follows_the_compose_cycle() {
        use super::{
            CIRCULAR_CYCLE_DURATION, CIRCULAR_INDETERMINATE_MAX_PROGRESS,
            CIRCULAR_INDETERMINATE_MIN_PROGRESS, circular_indeterminate_arc,
        };
        use core::time::Duration;

        let sweep_turns = |at: Duration| circular_indeterminate_arc(at).1 / core::f64::consts::TAU;

        let start = sweep_turns(Duration::ZERO);
        assert!(
            (start - CIRCULAR_INDETERMINATE_MIN_PROGRESS).abs() < 1e-6,
            "the cycle starts at the minimum arc, was {start}"
        );

        let middle = sweep_turns(CIRCULAR_CYCLE_DURATION / 2);
        assert!(
            (middle - CIRCULAR_INDETERMINATE_MAX_PROGRESS).abs() < 1e-6,
            "the arc peaks at the half cycle, was {middle}"
        );

        // The cycle is closed: it ends where it began, so a repeat does not jump.
        let end = sweep_turns(CIRCULAR_CYCLE_DURATION);
        assert!(
            (end - CIRCULAR_INDETERMINATE_MIN_PROGRESS).abs() < 1e-6,
            "the cycle returns to the minimum arc, was {end}"
        );

        // Never inverted or past a full turn at any point in the cycle.
        for step in 0..=120 {
            let at = CIRCULAR_CYCLE_DURATION.mul_f64(f64::from(step) / 120.0);
            let turns = sweep_turns(at);
            assert!(
                (CIRCULAR_INDETERMINATE_MIN_PROGRESS - 1e-6
                    ..=CIRCULAR_INDETERMINATE_MAX_PROGRESS + 1e-6)
                    .contains(&turns),
                "sweep left its range at {at:?}: {turns}"
            );
        }
    }

    /// The additional rotation reaches a full turn by the end of the cycle, in
    /// four 90-degree steps.
    #[test]
    fn circular_additional_rotation_completes_a_turn_in_four_steps() {
        use super::{CIRCULAR_ADDITIONAL_ROTATION_DEGREES, circular_additional_rotation};

        assert!(circular_additional_rotation(0.0).abs() < 1e-6);
        assert!(
            (circular_additional_rotation(1.0) - CIRCULAR_ADDITIONAL_ROTATION_DEGREES).abs() < 1e-6,
            "a full cycle turns the arc once more"
        );
        // Compose's keyframes ramp each step over 300ms and then hold it until
        // the next 1.5s boundary: 90 at 300ms, still 90 at 1.4s, 180 at 1.8s.
        let first_step = circular_additional_rotation(300.0 / 6_000.0);
        assert!(
            (first_step - 90.0).abs() < 1e-6,
            "the first step completes at 300ms, was {first_step}"
        );
        let holding = circular_additional_rotation(1_400.0 / 6_000.0);
        assert!(
            (holding - 90.0).abs() < 1e-6,
            "the step holds until the next delay boundary, was {holding}"
        );
        let second_step = circular_additional_rotation(1_800.0 / 6_000.0);
        assert!(
            (second_step - 180.0).abs() < 1e-6,
            "the second step completes at 1.8s, was {second_step}"
        );
        // Monotonic: the arc never rotates backwards.
        let mut previous = 0.0_f64;
        for step in 0..=200 {
            let value = circular_additional_rotation(f64::from(step) / 200.0);
            assert!(
                value >= previous - 1e-9,
                "rotation went backwards at {step}"
            );
            previous = value;
        }
    }

    /// Values from `androidx.compose.material3.tokens`
    /// `LinearProgressIndicatorTokens` and `CircularProgressIndicatorTokens`.
    #[test]
    fn progress_metrics_match_compose_progress_tokens() {
        let linear = metrics(ProgressIndicatorStyle::Linear);
        // LinearProgressIndicatorTokens.Height / TrackThickness
        assert_eq!(linear.bar_height, 4.0);
        assert_eq!(linear.min_track_width, 80.0);

        let circular = metrics(ProgressIndicatorStyle::Circular);
        // CircularProgressIndicatorTokens.Size
        assert_eq!(circular.circular_diameter, 40.0);
        // CircularProgressIndicatorTokens.ActiveThickness / TrackThickness
        assert_eq!(circular.circular_stroke_width, 4.0);
        // TrackActiveSpace on both indicators
        assert_eq!(PROGRESS_CIRCULAR_TRACK_ACTIVE_SPACE, 4.0);
        assert_eq!(PROGRESS_LINEAR_TRACK_ACTIVE_SPACE, 4.0);
        // LinearProgressIndicatorTokens.StopSize
        assert_eq!(PROGRESS_LINEAR_STOP_SIZE, 4.0);
    }
}
