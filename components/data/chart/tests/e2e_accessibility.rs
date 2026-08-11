//! XCTest-style semantic accessibility coverage for the chart components.

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use waterui::{Binding, View};
use waterui_chart::{
    AreaChart, AreaDatum, AxisConfig, BarChart, BubbleChart, BubblePoint, Candle, CandlestickChart,
    ChartExt, DataBounds, DataPoint, DepthChart, DepthDatum, DepthSide, HitResult, LineChart,
    PieChart, ScatterChart, SliceDatum,
};
use waterui_testing::{Role, Selector, UiBuilder, WaitOptions, WaitResult};

use support::{
    area_data, area_hit_location, assert_chart_accessibility_ready, bar_hit_location,
    bubble_hit_location, bubble_series, candle_series, candlestick_hit_location, depth_data,
    depth_hit_location, pie_data, pie_hit_location, pie_slice_datum, point_hit_location,
    point_series, readout_view, semantic_chart_shell,
};

fn axis_tick_labels(bounds: DataBounds) -> BTreeSet<String> {
    AxisConfig::default()
        .compute_ticks(bounds.min_x, bounds.max_x)
        .into_iter()
        .map(|tick| tick.label().to_owned())
        .collect()
}

fn assert_chart_semantic_flow<T, V, F>(
    ui: UiBuilder,
    name: &'static str,
    hover_at: (f32, f32),
    expected_series: usize,
    expected_index: usize,
    expected_value: &T,
    formatter: fn(&HitResult<T>) -> String,
    build_chart: F,
) where
    T: Clone + PartialEq + core::fmt::Debug + 'static,
    V: View + 'static,
    F: Fn(Binding<Option<HitResult<T>>>, Binding<Option<HitResult<T>>>) -> V + 'static,
{
    let focused = Binding::container(None::<HitResult<T>>);
    let selected = Binding::container(None::<HitResult<T>>);
    let focused_for_view = focused.clone();
    let selected_for_view = selected.clone();

    let mut app = ui.mount(move || {
        let chart = build_chart(focused_for_view.clone(), selected_for_view.clone());
        let focused_readout = readout_view("focused", focused_for_view.clone(), formatter);
        let selected_readout = readout_view("selected", selected_for_view.clone(), formatter);
        semantic_chart_shell(name, chart, focused_readout, selected_readout)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, name);
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("focused:none")
        .assert_exists();
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("selected:none")
        .assert_exists();

    app.query()
        .role(waterui_testing::Role::IMAGE)
        .label(chart_label.clone())
        .hover_at(hover_at.0, hover_at.1);
    let focused_hit = focused
        .get()
        .expect("focused binding should hold a hit result after hover");
    assert_eq!(
        focused_hit.series, expected_series,
        "{name}: focused series mismatch"
    );
    assert_eq!(
        focused_hit.index, expected_index,
        "{name}: focused index mismatch"
    );
    assert_eq!(
        &focused_hit.value, expected_value,
        "{name}: focused value mismatch"
    );
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("focused:none")
        .assert_not_exists();
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label_contains("focused:")
        .assert_exists();
    app.query()
        .role(waterui_testing::Role::IMAGE)
        .label(chart_label)
        .tap_at(hover_at.0, hover_at.1);
    let selected_hit = selected
        .get()
        .expect("selected binding should hold a hit result after tap");
    assert_eq!(
        selected_hit.series, expected_series,
        "{name}: selected series mismatch"
    );
    assert_eq!(
        selected_hit.index, expected_index,
        "{name}: selected index mismatch"
    );
    assert_eq!(
        &selected_hit.value, expected_value,
        "{name}: selected value mismatch"
    );
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("selected:none")
        .assert_not_exists();
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label_contains("selected:")
        .assert_exists();
}

#[waterui::test(viewport = (320, 320))]
fn line_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = point_series();
    let index = 10;
    let expected = data[index];
    assert_chart_semantic_flow(
        ui,
        "line",
        point_hit_location(&data, index),
        0,
        index,
        &expected,
        |hit: &HitResult<DataPoint>| {
            format!(
                "series={} index={} x={:.2} y={:.2}",
                hit.series, hit.index, hit.value.x, hit.value.y
            )
        },
        move |focused, selected| {
            LineChart::new(Binding::container(data.clone()))
                .focused(&focused)
                .selected(&selected)
        },
    );
}

#[waterui::test(viewport = (320, 320))]
fn line_chart_axes_reactive_updates_accessibility_labels_when_bounds_change(ui: UiBuilder) {
    let initial_data = vec![
        DataPoint::new(0.0, 0.0),
        DataPoint::new(1.0, 1.0),
        DataPoint::new(2.0, 2.0),
        DataPoint::new(3.0, 3.0),
        DataPoint::new(4.0, 4.0),
    ];
    let updated_data = vec![
        DataPoint::new(100.0, 100.0),
        DataPoint::new(101.0, 101.0),
        DataPoint::new(102.0, 102.0),
        DataPoint::new(103.0, 103.0),
        DataPoint::new(104.0, 104.0),
    ];
    let initial_labels = axis_tick_labels(DataBounds::from_points(&initial_data));
    let updated_labels = axis_tick_labels(DataBounds::from_points(&updated_data));
    let removed_label = initial_labels
        .difference(&updated_labels)
        .next()
        .cloned()
        .expect("reactive axes test requires an initial-only tick label");
    let added_label = updated_labels
        .difference(&initial_labels)
        .next()
        .cloned()
        .expect("reactive axes test requires an updated-only tick label");
    let chart_data = Binding::container(initial_data.clone());
    let chart_data_for_view = chart_data;
    let bounds = Binding::container(DataBounds::from_points(&initial_data));
    let bounds_for_view = bounds.clone();

    let mut app = ui.mount(move || {
        semantic_chart_shell(
            "line-axes-reactive",
            LineChart::new(chart_data_for_view.clone()).axes_reactive(bounds_for_view.clone()),
            (),
            (),
        )
    });

    assert_chart_accessibility_ready(&mut app, "line-axes-reactive");
    app.query()
        .role(Role::LABEL)
        .label(removed_label.clone())
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label(added_label.clone())
        .assert_not_exists();

    bounds.set(DataBounds::from_points(&updated_data));
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::LABEL)
                    .label(added_label.clone()),
            )],
            WaitOptions::new(Duration::from_millis(400)),
        ) == WaitResult::Completed,
        "line-axes-reactive: expected updated axis label {added_label:?} to appear after bounds change"
    );
    app.query()
        .role(Role::LABEL)
        .label(removed_label)
        .assert_not_exists();
}

#[waterui::test(viewport = (320, 320))]
fn bar_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = point_series();
    let index = 8;
    let expected = data[index];
    assert_chart_semantic_flow(
        ui,
        "bar",
        bar_hit_location(&data, index),
        0,
        index,
        &expected,
        |hit: &HitResult<DataPoint>| {
            format!(
                "series={} index={} x={:.2} y={:.2}",
                hit.series, hit.index, hit.value.x, hit.value.y
            )
        },
        move |focused, selected| {
            BarChart::new(Binding::container(data.clone()))
                .focused(&focused)
                .selected(&selected)
        },
    );
}

#[waterui::test(viewport = (320, 320))]
fn scatter_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = point_series();
    let index = 14;
    let expected = data[index];
    assert_chart_semantic_flow(
        ui,
        "scatter",
        point_hit_location(&data, index),
        0,
        index,
        &expected,
        |hit: &HitResult<DataPoint>| {
            format!(
                "series={} index={} x={:.2} y={:.2}",
                hit.series, hit.index, hit.value.x, hit.value.y
            )
        },
        move |focused, selected| {
            ScatterChart::new(Binding::container(data.clone()))
                .radius(6.0)
                .focused(&focused)
                .selected(&selected)
        },
    );
}

#[waterui::test(viewport = (320, 320))]
fn bubble_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = bubble_series();
    let index = 11;
    let expected = data[index];
    assert_chart_semantic_flow(
        ui,
        "bubble",
        bubble_hit_location(&data, index),
        0,
        index,
        &expected,
        |hit: &HitResult<BubblePoint>| {
            format!(
                "series={} index={} x={:.2} y={:.2} size={:.2}",
                hit.series, hit.index, hit.value.x, hit.value.y, hit.value.size
            )
        },
        move |focused, selected| {
            BubbleChart::new(Binding::container(data.clone()))
                .min_radius(4.0)
                .max_radius(18.0)
                .focused(&focused)
                .selected(&selected)
        },
    );
}

#[waterui::test(viewport = (320, 320))]
fn candlestick_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = candle_series();
    let index = 12;
    let expected = data[index];
    assert_chart_semantic_flow(
        ui,
        "candlestick",
        candlestick_hit_location(&data, index),
        0,
        index,
        &expected,
        |hit: &HitResult<Candle>| {
            format!(
                "series={} index={} t={:.2} open={:.2} high={:.2} low={:.2} close={:.2}",
                hit.series,
                hit.index,
                hit.value.timestamp,
                hit.value.open,
                hit.value.high,
                hit.value.low,
                hit.value.close
            )
        },
        move |focused, selected| {
            CandlestickChart::new(Binding::container(data.clone()))
                .focused(&focused)
                .selected(&selected)
        },
    );
}

#[waterui::test(viewport = (320, 320))]
fn depth_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = depth_data();
    let side = DepthSide::Bid;
    let index = 7;
    let level = data.bids[index];
    let value = DepthDatum::new(DepthSide::Bid, level.price, level.cumulative_volume);
    assert_chart_semantic_flow(
        ui,
        "depth",
        depth_hit_location(&data, side, index),
        0,
        index,
        &value,
        |hit: &HitResult<DepthDatum>| {
            let side = match hit.value.side {
                DepthSide::Bid => "bid",
                DepthSide::Ask => "ask",
            };
            format!(
                "series={} index={} side={} price={:.2} cumulative={:.2}",
                hit.series, hit.index, side, hit.value.price, hit.value.cumulative_volume
            )
        },
        move |focused, selected| {
            DepthChart::new(Binding::container(data.clone()))
                .focused(&focused)
                .selected(&selected)
        },
    );
}

#[waterui::test(viewport = (320, 320))]
fn area_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = area_data();
    let series = 0;
    let index = 4;
    let value = AreaDatum::new(
        series,
        data.x_values[index],
        data.series[series].values[index],
    );
    assert_chart_semantic_flow(
        ui,
        "area",
        area_hit_location(&data, series, index),
        series,
        index,
        &value,
        |hit: &HitResult<AreaDatum>| {
            format!(
                "series={} index={} x={:.2} y={:.2}",
                hit.series, hit.index, hit.value.x, hit.value.y
            )
        },
        move |focused, selected| {
            AreaChart::new(Binding::container(data.clone()))
                .focused(&focused)
                .selected(&selected)
        },
    );
}

#[waterui::test(viewport = (320, 320))]
fn pie_chart_xctest_like_focus_and_selection_flow(ui: UiBuilder) {
    let data = pie_data();
    let index = 1;
    let value = pie_slice_datum(&data, index);
    assert_chart_semantic_flow(
        ui,
        "pie",
        pie_hit_location(&data, index, 0.0),
        0,
        index,
        &value,
        |hit: &HitResult<SliceDatum>| {
            format!(
                "series={} index={} value={:.2} start={:.3} end={:.3}",
                hit.series, hit.index, hit.value.value, hit.value.start_angle, hit.value.end_angle
            )
        },
        move |focused, selected| {
            PieChart::new(Binding::container(data.clone()))
                .focused(&focused)
                .selected(&selected)
        },
    );
}
