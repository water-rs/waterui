//! XCTest-style semantic coverage for Swift Charts-aligned cartesian selection APIs.

mod support;

use core::ops::RangeInclusive;

use waterui::component::{text, vstack};
use waterui::graphics::color::Srgb;
use waterui::{Binding, SignalExt as _, View, ViewExt as _};
use waterui_chart::{DepthChart, DepthSide, LineChart};
use waterui_testing::Role;

use support::{
    assert_chart_accessibility_ready, chart_surface, depth_data, depth_hit_location, mount_view,
    point_hit_location, point_series,
};

fn assert_close(actual: f32, expected: f32, epsilon: f32, context: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= epsilon,
        "{context}: expected {expected:.4}, got {actual:.4}, delta={delta:.4}, epsilon={epsilon:.4}"
    );
}

fn selection_shell<V: View, R: View>(name: &str, chart: V, readout: R) -> impl View {
    vstack((chart_surface(name, chart), readout))
        .spacing(6.0)
        .background(Srgb::BLACK)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the readout owns a cloned Binding signal for the mounted view lifetime"
)]
fn x_value_readout(selection: Binding<Option<f32>>) -> impl View {
    text(
        selection
            .map(|value| value.map_or_else(|| String::from("x:none"), |x| format!("x:{x:.2}")))
            .computed(),
    )
    .caption()
    .body()
    .foreground(Srgb::WHITE)
    .padding_with(6.0)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the readout owns a cloned Binding signal for the mounted view lifetime"
)]
fn x_range_readout(selection: Binding<Option<RangeInclusive<f32>>>) -> impl View {
    text(
        selection
            .map(|value| {
                value.map_or_else(
                    || String::from("x-range:none"),
                    |range| format!("x-range:{:.2}..={:.2}", *range.start(), *range.end()),
                )
            })
            .computed(),
    )
    .caption()
    .body()
    .foreground(Srgb::WHITE)
    .padding_with(6.0)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the readout owns a cloned Binding signal for the mounted view lifetime"
)]
fn y_value_readout(selection: Binding<Option<f32>>) -> impl View {
    text(
        selection
            .map(|value| value.map_or_else(|| String::from("y:none"), |y| format!("y:{y:.2}")))
            .computed(),
    )
    .caption()
    .body()
    .foreground(Srgb::WHITE)
    .padding_with(6.0)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the readout owns a cloned Binding signal for the mounted view lifetime"
)]
fn y_range_readout(selection: Binding<Option<RangeInclusive<f32>>>) -> impl View {
    text(
        selection
            .map(|value| {
                value.map_or_else(
                    || String::from("y-range:none"),
                    |range| format!("y-range:{:.2}..={:.2}", *range.start(), *range.end()),
                )
            })
            .computed(),
    )
    .caption()
    .body()
    .foreground(Srgb::WHITE)
    .padding_with(6.0)
}

#[test]
fn line_chart_x_selection_updates_continuous_domain_value_on_tap() {
    let data = point_series();
    let selection = Binding::container(None::<f32>);
    let data_for_view = data.clone();
    let selection_for_view = selection.clone();
    let index = 10;
    let expected_x = data[index].x;
    let expected_label = format!("x:{expected_x:.2}");
    let location = point_hit_location(&data, index);

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_x_selection(&selection_for_view);
        selection_shell(
            "line-x-value",
            chart,
            x_value_readout(selection_for_view.clone()),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "line-x-value");
    app.query()
        .role(Role::LABEL)
        .label("x:none")
        .assert_exists();

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .tap_at(location.0, location.1),
        "line-x-value: tap_at should update x selection"
    );

    let actual_x = selection
        .get()
        .expect("line-x-value: tap should populate x selection");
    assert_close(actual_x, expected_x, 0.05, "line-x-value");
    app.query()
        .role(Role::LABEL)
        .label("x:none")
        .assert_not_exists();
    app.query()
        .role(Role::LABEL)
        .label(expected_label)
        .assert_exists();
}

#[test]
fn line_chart_x_selection_range_tracks_drag_span() {
    let data = point_series();
    let selection = Binding::container(None::<RangeInclusive<f32>>);
    let data_for_view = data.clone();
    let selection_for_view = selection.clone();
    let from_index = 4;
    let to_index = 9;
    let expected_start = data[from_index].x;
    let expected_end = data[to_index].x;
    let expected_label = format!("x-range:{expected_start:.2}..={expected_end:.2}");
    let from = point_hit_location(&data, from_index);
    let to = point_hit_location(&data, to_index);

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_x_selection_range(&selection_for_view);
        selection_shell(
            "line-x-range",
            chart,
            x_range_readout(selection_for_view.clone()),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "line-x-range");
    app.query()
        .role(Role::LABEL)
        .label("x-range:none")
        .assert_exists();

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .drag_between(from.0, from.1, to.0, to.1),
        "line-x-range: drag_between should update x selection range"
    );

    let actual = selection
        .get()
        .expect("line-x-range: drag should populate x selection range");
    assert_close(*actual.start(), expected_start, 0.05, "line-x-range start");
    assert_close(*actual.end(), expected_end, 0.05, "line-x-range end");
    app.query()
        .role(Role::LABEL)
        .label("x-range:none")
        .assert_not_exists();
    app.query()
        .role(Role::LABEL)
        .label(expected_label)
        .assert_exists();
}

#[test]
fn line_chart_y_selection_updates_continuous_domain_value_on_tap() {
    let data = point_series();
    let selection = Binding::container(None::<f32>);
    let data_for_view = data.clone();
    let selection_for_view = selection.clone();
    let index = 10;
    let expected_y = data[index].y;
    let expected_label = format!("y:{expected_y:.2}");
    let location = point_hit_location(&data, index);

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_y_selection(&selection_for_view);
        selection_shell(
            "line-y-value",
            chart,
            y_value_readout(selection_for_view.clone()),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "line-y-value");
    app.query()
        .role(Role::LABEL)
        .label("y:none")
        .assert_exists();

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .tap_at(location.0, location.1),
        "line-y-value: tap_at should update y selection"
    );

    let actual_y = selection
        .get()
        .expect("line-y-value: tap should populate y selection");
    assert_close(actual_y, expected_y, 0.05, "line-y-value");
    app.query()
        .role(Role::LABEL)
        .label("y:none")
        .assert_not_exists();
    app.query()
        .role(Role::LABEL)
        .label(expected_label)
        .assert_exists();
}

#[test]
fn line_chart_y_selection_range_tracks_drag_span() {
    let data = point_series();
    let selection = Binding::container(None::<RangeInclusive<f32>>);
    let data_for_view = data.clone();
    let selection_for_view = selection.clone();
    let from_index = 4;
    let to_index = 9;
    let expected_start = data[from_index].y.min(data[to_index].y);
    let expected_end = data[from_index].y.max(data[to_index].y);
    let expected_label = format!("y-range:{expected_start:.2}..={expected_end:.2}");
    let from = point_hit_location(&data, from_index);
    let to = point_hit_location(&data, to_index);

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_y_selection_range(&selection_for_view);
        selection_shell(
            "line-y-range",
            chart,
            y_range_readout(selection_for_view.clone()),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "line-y-range");
    app.query()
        .role(Role::LABEL)
        .label("y-range:none")
        .assert_exists();

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .drag_between(from.0, from.1, to.0, to.1),
        "line-y-range: drag_between should update y selection range"
    );

    let actual = selection
        .get()
        .expect("line-y-range: drag should populate y selection range");
    assert_close(*actual.start(), expected_start, 0.05, "line-y-range start");
    assert_close(*actual.end(), expected_end, 0.05, "line-y-range end");
    app.query()
        .role(Role::LABEL)
        .label("y-range:none")
        .assert_not_exists();
    app.query()
        .role(Role::LABEL)
        .label(expected_label)
        .assert_exists();
}

#[test]
fn line_chart_updates_x_and_y_selection_together_on_tap() {
    let data = point_series();
    let x_selection = Binding::container(None::<f32>);
    let y_selection = Binding::container(None::<f32>);
    let data_for_view = data.clone();
    let x_for_view = x_selection.clone();
    let y_for_view = y_selection.clone();
    let index = 6;
    let expected_x = data[index].x;
    let expected_y = data[index].y;
    let location = point_hit_location(&data, index);

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_x_selection(&x_for_view)
            .chart_y_selection(&y_for_view);
        selection_shell(
            "line-xy-value",
            chart,
            vstack((
                x_value_readout(x_for_view.clone()),
                y_value_readout(y_for_view.clone()),
            ))
            .spacing(2.0),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "line-xy-value");
    app.query()
        .role(Role::LABEL)
        .label("x:none")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("y:none")
        .assert_exists();

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .tap_at(location.0, location.1),
        "line-xy-value: tap_at should update x and y selections"
    );

    assert_close(
        x_selection
            .get()
            .expect("line-xy-value: tap should populate x selection"),
        expected_x,
        0.05,
        "line-xy-value x",
    );
    assert_close(
        y_selection
            .get()
            .expect("line-xy-value: tap should populate y selection"),
        expected_y,
        0.05,
        "line-xy-value y",
    );
    app.query()
        .role(Role::LABEL)
        .label(format!("x:{expected_x:.2}"))
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label(format!("y:{expected_y:.2}"))
        .assert_exists();
}

#[test]
fn depth_chart_x_selection_tracks_price_domain_on_tap() {
    let data = depth_data();
    let selection = Binding::container(None::<f32>);
    let data_for_view = data.clone();
    let selection_for_view = selection.clone();
    let index = 6;
    let side = DepthSide::Ask;
    let expected_x = data.asks[index].price;
    let expected_label = format!("x:{expected_x:.2}");
    let location = depth_hit_location(&data, side, index);

    let mut app = mount_view(move || {
        let chart = DepthChart::new(Binding::container(data_for_view.clone()))
            .chart_x_selection(&selection_for_view);
        selection_shell(
            "depth-x-value",
            chart,
            x_value_readout(selection_for_view.clone()),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "depth-x-value");
    app.query()
        .role(Role::LABEL)
        .label("x:none")
        .assert_exists();

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .tap_at(location.0, location.1),
        "depth-x-value: tap_at should update x selection"
    );

    let actual_x = selection
        .get()
        .expect("depth-x-value: tap should populate x selection");
    assert_close(actual_x, expected_x, 0.05, "depth-x-value");
    app.query()
        .role(Role::LABEL)
        .label("x:none")
        .assert_not_exists();
    app.query()
        .role(Role::LABEL)
        .label(expected_label)
        .assert_exists();
}

#[test]
#[should_panic(
    expected = "chart_x_selection and chart_x_selection_range cannot be active on the same chart"
)]
fn line_chart_rejects_simultaneous_x_value_and_range_selection() {
    let data = point_series();
    let x_value = Binding::container(None::<f32>);
    let x_range = Binding::container(None::<RangeInclusive<f32>>);
    let data_for_view = data;
    let x_value_for_view = x_value;
    let x_range_for_view = x_range;

    let _app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_x_selection(&x_value_for_view)
            .chart_x_selection_range(&x_range_for_view);
        chart_surface("line-x-conflict", chart)
    });
}

#[test]
#[should_panic(
    expected = "chart_y_selection and chart_y_selection_range cannot be active on the same chart"
)]
fn line_chart_rejects_simultaneous_y_value_and_range_selection() {
    let data = point_series();
    let y_value = Binding::container(None::<f32>);
    let y_range = Binding::container(None::<RangeInclusive<f32>>);
    let data_for_view = data;
    let y_value_for_view = y_value;
    let y_range_for_view = y_range;

    let _app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_y_selection(&y_value_for_view)
            .chart_y_selection_range(&y_range_for_view);
        chart_surface("line-y-conflict", chart)
    });
}
