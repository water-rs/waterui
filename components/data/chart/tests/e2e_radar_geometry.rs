//! Coverage for radar chart hit geometry: axis indexing, anchor placement, and
//! the axis-count truncation of a series that carries trailing values.

mod support;

use core::f32::consts::{FRAC_PI_2, TAU};

use waterui::graphics::color::Srgb;
use waterui::{Binding, ViewExt as _};
use waterui_chart::{HitResult, RadarChart, RadarData, RadarDatum, RadarSeries};
use waterui_testing::{Role, SemanticApp, UiBuilder};

use support::{CHART_HEIGHT, CHART_WIDTH, assert_chart_accessibility_ready, chart_surface};

/// Mirrors `plot_rect(ctx, 0.08)` in the radar drawing code.
const RADAR_PADDING_RATIO: f32 = 0.08;
/// Mirrors the radar radius factor applied to the shorter plot dimension.
const RADAR_RADIUS_RATIO: f32 = 0.45;
const AXIS_COUNT: u16 = 5;
const AXIS_VALUES: [f32; 5] = [10.0, 20.0, 30.0, 40.0, 50.0];
const AXIS_LABELS: [&str; 5] = ["Speed", "Power", "Range", "Agility", "Armor"];
/// Values past `AXIS_COUNT` that the radar geometry must ignore.
const TRAILING_VALUES: [f32; 2] = [999.0, 999.0];
const MAX_VALUE: f32 = 50.0;
/// Anchors are compared in chart-local points, so a sub-pixel tolerance is enough.
const ANCHOR_TOLERANCE: f32 = 0.5;

fn plot_size() -> (f32, f32) {
    (
        (CHART_WIDTH * RADAR_PADDING_RATIO)
            .mul_add(-2.0, CHART_WIDTH)
            .max(1.0),
        (CHART_HEIGHT * RADAR_PADDING_RATIO)
            .mul_add(-2.0, CHART_HEIGHT)
            .max(1.0),
    )
}

fn radar_center() -> (f32, f32) {
    let (width, height) = plot_size();
    (
        width.mul_add(0.5, CHART_WIDTH * RADAR_PADDING_RATIO),
        height.mul_add(0.5, CHART_HEIGHT * RADAR_PADDING_RATIO),
    )
}

fn radar_radius() -> f32 {
    let (width, height) = plot_size();
    width.min(height) * RADAR_RADIUS_RATIO
}

fn radar_axis_angle(axis: u16) -> f32 {
    (f32::from(axis) * TAU / f32::from(AXIS_COUNT)) - FRAC_PI_2
}

/// Chart-local anchor point for `axis` at `value`, matching `radar_geometry`.
fn radar_anchor(axis: u16, value: f32) -> (f32, f32) {
    let (center_x, center_y) = radar_center();
    let radius = radar_radius();
    let ratio = (value / MAX_VALUE).clamp(0.0, 1.0);
    let angle = radar_axis_angle(axis);
    (
        (angle.cos() * radius).mul_add(ratio, center_x),
        (angle.sin() * radius).mul_add(ratio, center_y),
    )
}

fn normalized(point: (f32, f32)) -> (f32, f32) {
    (point.0 / CHART_WIDTH, point.1 / CHART_HEIGHT)
}

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= ANCHOR_TOLERANCE,
        "{what}: expected {expected}, got {actual}"
    );
}

fn radar_data(trailing: &[f32]) -> RadarData {
    let mut values = AXIS_VALUES.to_vec();
    values.extend_from_slice(trailing);
    RadarData::new(u32::from(AXIS_COUNT))
        .labels(AXIS_LABELS.to_vec())
        .series(RadarSeries::new("Primary", values))
        .max_value(MAX_VALUE)
}

fn mount_radar(
    ui: UiBuilder,
    data: RadarData,
    focused: &Binding<Option<HitResult<RadarDatum>>>,
) -> SemanticApp {
    let focused = focused.clone();
    ui.mount(move || {
        chart_surface(
            "radar",
            RadarChart::new(Binding::container(data.clone())).focused(&focused),
        )
        .background(Srgb::BLACK)
    })
}

/// Hovers the chart at a chart-local point and returns the resulting focus.
fn hover_focus(
    app: &mut SemanticApp,
    label: &str,
    point: (f32, f32),
    focused: &Binding<Option<HitResult<RadarDatum>>>,
) -> Option<HitResult<RadarDatum>> {
    let (x, y) = normalized(point);
    app.query()
        .role(Role::IMAGE)
        .label(label.to_owned())
        .hover_at(x, y);
    focused.get()
}

#[waterui::test(viewport = (320, 320))]
fn radar_chart_exposes_one_hit_target_per_axis(ui: UiBuilder) {
    let focused = Binding::container(None::<HitResult<RadarDatum>>);
    let mut app = mount_radar(ui, radar_data(&[]), &focused);
    let label = assert_chart_accessibility_ready(&mut app, "radar");

    for axis in 0..AXIS_COUNT {
        let index = usize::from(axis);
        let value = AXIS_VALUES[index];
        let anchor = radar_anchor(axis, value);
        let hit = hover_focus(&mut app, &label, anchor, &focused)
            .unwrap_or_else(|| panic!("radar: axis {axis} anchor should produce a focused hit"));
        assert_eq!(hit.series, 0, "radar: axis {axis} belongs to series 0");
        assert_eq!(hit.index, index, "radar: hit index must be the axis index");
        assert_eq!(
            hit.value,
            RadarDatum::new(index, Some(AXIS_LABELS[index].to_owned()), value),
            "radar: axis {axis} datum must carry that axis's label and value"
        );
        assert_close(
            hit.anchor.x,
            anchor.0,
            &format!("radar: axis {axis} anchor x"),
        );
        assert_close(
            hit.anchor.y,
            anchor.1,
            &format!("radar: axis {axis} anchor y"),
        );
    }
}

#[waterui::test(viewport = (320, 320))]
fn radar_chart_first_axis_anchor_points_straight_up(ui: UiBuilder) {
    let focused = Binding::container(None::<HitResult<RadarDatum>>);
    let mut app = mount_radar(ui, radar_data(&[]), &focused);
    let label = assert_chart_accessibility_ready(&mut app, "radar");

    let anchor = radar_anchor(0, AXIS_VALUES[0]);
    let hit = hover_focus(&mut app, &label, anchor, &focused)
        .expect("radar: axis 0 anchor should produce a focused hit");
    assert_eq!(hit.index, 0);

    // Axis 0 sits at -FRAC_PI_2, so its anchor is directly above the plot centre.
    let (center_x, center_y) = radar_center();
    let expected_y = (AXIS_VALUES[0] / MAX_VALUE).mul_add(-radar_radius(), center_y);
    assert_close(
        hit.anchor.x,
        center_x,
        "radar: axis 0 anchor must sit on the centre column",
    );
    assert_close(
        hit.anchor.y,
        expected_y,
        "radar: axis 0 anchor must sit above the centre",
    );
    assert!(
        hit.anchor.y < center_y,
        "radar: axis 0 anchor must be above the centre, got {}",
        hit.anchor.y
    );
}

#[waterui::test(viewport = (320, 320))]
fn radar_chart_ignores_series_values_beyond_axis_count(ui: UiBuilder) {
    let focused = Binding::container(None::<HitResult<RadarDatum>>);
    let mut app = mount_radar(ui, radar_data(&TRAILING_VALUES), &focused);
    let label = assert_chart_accessibility_ready(&mut app, "radar");

    // Every declared axis still resolves with the trailing values present.
    for axis in 0..AXIS_COUNT {
        let index = usize::from(axis);
        let anchor = radar_anchor(axis, AXIS_VALUES[index]);
        let hit = hover_focus(&mut app, &label, anchor, &focused)
            .unwrap_or_else(|| panic!("radar: axis {axis} must still be hittable"));
        assert_eq!(hit.index, index);
        assert_close(
            hit.value.value,
            AXIS_VALUES[index],
            &format!("radar: axis {axis} value"),
        );
    }

    // A trailing value would land on a phantom axis whose angle wraps back onto
    // an existing spoke at the outer radius. Nothing may be hit there, and the
    // focus set by the loop above must be cleared by hovering empty space.
    let trailing_count =
        u16::try_from(TRAILING_VALUES.len()).expect("trailing value count fits in u16");
    for extra in 0..trailing_count {
        let phantom = radar_anchor(AXIS_COUNT + extra, TRAILING_VALUES[usize::from(extra)]);
        let hit = hover_focus(&mut app, &label, phantom, &focused);
        assert!(
            hit.is_none(),
            "radar: trailing value {extra} must not create a hit target at {phantom:?}, got {hit:?}"
        );
    }
}
