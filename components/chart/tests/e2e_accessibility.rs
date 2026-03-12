mod support;

use waterui::component::text;
use waterui::{Binding, SignalExt as _, View};
use waterui_chart::{
    AreaChart, AreaDatum, BarChart, BubbleChart, BubblePoint, CandlestickChart, Candle,
    DataPoint, DepthChart, DepthDatum, DepthSide, HitResult, LineChart, PieChart, ScatterChart,
    SliceDatum,
};

use support::{
    SNAPSHOT_SUITE, area_data, area_hit_location, assert_chart_accessibility_ready,
    bar_hit_location, bubble_hit_location, bubble_series, candle_series, candlestick_hit_location,
    depth_data, depth_hit_location, mount_view, pie_data, pie_hit_location, pie_slice_datum,
    point_hit_location, point_series, readout_text, semantic_chart_shell, snapshot_suite,
};

fn assert_chart_semantic_flow<T, V, F>(
    name: &'static str,
    hover_at: (f32, f32),
    expected_series: usize,
    expected_index: usize,
    expected_value: T,
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

    let mut app = mount_view(move || {
        let chart = build_chart(focused_for_view.clone(), selected_for_view.clone());
        let focused_readout = text(focused_for_view
            .clone()
            .map(move |hit| readout_text("focused", hit, formatter)))
        .body();
        let selected_readout = text(selected_for_view
            .clone()
            .map(move |hit| readout_text("selected", hit, formatter)))
        .body();
        semantic_chart_shell(name, chart, focused_readout, selected_readout)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, name);
    let suite = snapshot_suite(SNAPSHOT_SUITE);

    let initial = app.capture_snapshot(&suite, name, "00_initial");
    assert!(initial.path().is_file(), "{name}: initial snapshot missing");
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("focused:none")
        .assert_exists();
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("selected:none")
        .assert_exists();

    assert!(
        app.query()
            .role(waterui_testing::Role::IMAGE)
            .label(chart_label.clone())
            .hover_at(hover_at.0, hover_at.1),
        "{name}: hover_at should update focused binding"
    );
    let focused_hit = focused
        .get()
        .expect("focused binding should hold a hit result after hover");
    assert_eq!(focused_hit.series, expected_series, "{name}: focused series mismatch");
    assert_eq!(focused_hit.index, expected_index, "{name}: focused index mismatch");
    assert_eq!(focused_hit.value, expected_value, "{name}: focused value mismatch");
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("focused:none")
        .assert_not_exists();
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label_contains("focused:")
        .assert_exists();
    let focused_snapshot = app.capture_snapshot(&suite, name, "01_focused");
    assert!(focused_snapshot.path().is_file(), "{name}: focused snapshot missing");

    assert!(
        app.query()
            .role(waterui_testing::Role::IMAGE)
            .label(chart_label)
            .tap_at(hover_at.0, hover_at.1),
        "{name}: tap_at should update selected binding"
    );
    let selected_hit = selected
        .get()
        .expect("selected binding should hold a hit result after tap");
    assert_eq!(selected_hit.series, expected_series, "{name}: selected series mismatch");
    assert_eq!(selected_hit.index, expected_index, "{name}: selected index mismatch");
    assert_eq!(selected_hit.value, expected_value, "{name}: selected value mismatch");
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("selected:none")
        .assert_not_exists();
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label_contains("selected:")
        .assert_exists();
    let selected_snapshot = app.capture_snapshot(&suite, name, "02_selected");
    assert!(selected_snapshot.path().is_file(), "{name}: selected snapshot missing");
}

#[test]
fn line_chart_xctest_like_focus_and_selection_flow() {
    let data = point_series();
    let index = 10;
    assert_chart_semantic_flow(
        "line",
        point_hit_location(&data, index),
        0,
        index,
        data[index],
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

#[test]
fn bar_chart_xctest_like_focus_and_selection_flow() {
    let data = point_series();
    let index = 8;
    assert_chart_semantic_flow(
        "bar",
        bar_hit_location(&data, index),
        0,
        index,
        data[index],
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

#[test]
fn scatter_chart_xctest_like_focus_and_selection_flow() {
    let data = point_series();
    let index = 14;
    assert_chart_semantic_flow(
        "scatter",
        point_hit_location(&data, index),
        0,
        index,
        data[index],
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

#[test]
fn bubble_chart_xctest_like_focus_and_selection_flow() {
    let data = bubble_series();
    let index = 11;
    assert_chart_semantic_flow(
        "bubble",
        bubble_hit_location(&data, index),
        0,
        index,
        data[index],
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

#[test]
fn candlestick_chart_xctest_like_focus_and_selection_flow() {
    let data = candle_series();
    let index = 12;
    assert_chart_semantic_flow(
        "candlestick",
        candlestick_hit_location(&data, index),
        0,
        index,
        data[index],
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

#[test]
fn depth_chart_xctest_like_focus_and_selection_flow() {
    let data = depth_data();
    let side = DepthSide::Bid;
    let index = 7;
    let level = data.bids[index];
    let value = DepthDatum::new(DepthSide::Bid, level.price, level.cumulative_volume);
    assert_chart_semantic_flow(
        "depth",
        depth_hit_location(&data, side, index),
        0,
        index,
        value,
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

#[test]
fn area_chart_xctest_like_focus_and_selection_flow() {
    let data = area_data();
    let series = 0;
    let index = 4;
    let value = AreaDatum::new(series, data.x_values[index], data.series[series].values[index]);
    assert_chart_semantic_flow(
        "area",
        area_hit_location(&data, series, index),
        series,
        index,
        value,
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

#[test]
fn pie_chart_xctest_like_focus_and_selection_flow() {
    let data = pie_data();
    let index = 1;
    let value = pie_slice_datum(&data, index);
    assert_chart_semantic_flow(
        "pie",
        pie_hit_location(&data, index, 0.0),
        0,
        index,
        value,
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
