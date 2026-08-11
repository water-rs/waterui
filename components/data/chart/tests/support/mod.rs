#![allow(
    dead_code,
    reason = "integration tests include this shared support module independently and use different helper subsets"
)]

use std::time::Duration;

use waterui::accessibility::AccessibilityRole;
use waterui::component::{text, vstack};
use waterui::graphics::color::Srgb;
use waterui::{Signal, SignalExt as _, View, ViewExt as _};
use waterui_chart::{
    AreaData, AreaDatum, AreaSeries, BubblePoint, Candle, ChartAnchor, DataBounds, DataPoint,
    DepthData, DepthDatum, DepthLevel, DepthSide, HitResult, SliceDatum,
};
use waterui_testing::{Role, Selector, SemanticApp};

pub const VIEWPORT_WIDTH: u32 = 320;
pub const VIEWPORT_HEIGHT: u32 = 320;
pub const CHART_WIDTH: f32 = 240.0;
pub const CHART_HEIGHT: f32 = 180.0;
pub const SNAPSHOT_SUITE: &str = "semantic-selection";

pub const CHART_PADDING_RATIO: f32 = 0.1;
const PIE_PADDING_RATIO: f32 = 0.06;
const SEMANTIC_CHART_SHELL_SPACING: f32 = 4.0;

pub fn horizontal_drag_domain_delta(visible_length: f32, from_x: f32, to_x: f32) -> f32 {
    (from_x - to_x) * CHART_WIDTH / (CHART_WIDTH * CHART_PADDING_RATIO.mul_add(-2.0, 1.0))
        * visible_length
}

pub fn vertical_drag_domain_delta(visible_length: f32, from_y: f32, to_y: f32) -> f32 {
    (from_y - to_y) * CHART_HEIGHT / (CHART_HEIGHT * CHART_PADDING_RATIO.mul_add(-2.0, 1.0))
        * visible_length
}

pub fn point_series() -> Vec<DataPoint> {
    (0_u16..24)
        .map(|index| {
            let x = f32::from(index);
            let y = (x * 0.17)
                .cos()
                .mul_add(5.0, (x * 0.42).sin().mul_add(9.0, 24.0));
            DataPoint::new(x, y)
        })
        .collect()
}

pub fn bubble_series() -> Vec<BubblePoint> {
    (0_u16..32)
        .map(|index| {
            let x = f32::from(index) * 0.75;
            let y = (x * 0.09)
                .cos()
                .mul_add(4.0, (x * 0.28).sin().mul_add(10.0, 18.0));
            let size = f32::from(index % 7).mul_add(2.25, 4.0);
            BubblePoint::new(x, y, size)
        })
        .collect()
}

pub fn candle_series() -> Vec<Candle> {
    let mut candles = Vec::with_capacity(32);
    let mut price = 120.0_f32;
    for index in 0_u16..32 {
        let timestamp = f32::from(index) * 60.0;
        let drift = (f32::from(index) * 0.31).sin() * 4.5;
        let open = price;
        let close = open + drift;
        let high = f32::from(index % 4).mul_add(0.25, open.max(close) + 1.4);
        let low = f32::from(index % 5).mul_add(-0.2, open.min(close) - 1.1);
        let volume = f32::from(index).mul_add(350.0, 20_000.0);
        candles.push(Candle::new(timestamp, open, high, low, close, volume));
        price = close;
    }
    candles
}

pub fn depth_data() -> DepthData {
    let mut bids = Vec::with_capacity(24);
    let mut asks = Vec::with_capacity(24);
    let mut bid_cumulative = 0.0_f32;
    let mut ask_cumulative = 0.0_f32;
    for index in 0_u16..24 {
        let bid_price = f32::from(index).mul_add(-0.08, 99.8);
        bid_cumulative += f32::from(index % 5).mul_add(1.2, 6.0);
        bids.push(DepthLevel::new(bid_price, bid_cumulative));

        let ask_price = f32::from(index).mul_add(0.08, 100.2);
        ask_cumulative += f32::from(index % 4).mul_add(1.1, 6.5);
        asks.push(DepthLevel::new(ask_price, ask_cumulative));
    }
    DepthData::new(bids, asks)
}

pub fn area_data() -> AreaData {
    AreaData::new(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0])
        .series(
            AreaSeries::new("North", vec![10.0, 14.0, 12.0, 18.0, 20.0, 22.0])
                .color(0.23, 0.51, 0.96, 0.72),
        )
        .series(
            AreaSeries::new("South", vec![6.0, 8.0, 9.0, 11.0, 12.0, 14.0])
                .color(0.06, 0.72, 0.51, 0.64),
        )
        .series(
            AreaSeries::new("West", vec![4.0, 5.0, 6.0, 7.0, 8.0, 9.0])
                .color(0.98, 0.53, 0.12, 0.56),
        )
}

pub fn pie_data() -> Vec<DataPoint> {
    vec![
        DataPoint::new(0.0, 30.0),
        DataPoint::new(1.0, 45.0),
        DataPoint::new(2.0, 25.0),
    ]
}

pub fn chart_label(name: &str) -> String {
    format!("chart-{name}")
}

pub fn snapshot_suite(name: &str) -> String {
    format!("chart/{name}")
}

pub fn chart_surface<V: View>(name: &str, chart: V) -> impl View {
    chart
        .size(CHART_WIDTH, CHART_HEIGHT)
        .a11y_label(chart_label(name))
        .a11y_role(AccessibilityRole::Image)
}

pub fn semantic_chart_shell<V: View, F: View, S: View>(
    name: &str,
    chart: V,
    focused_readout: F,
    selected_readout: S,
) -> impl View {
    vstack((
        chart_surface(name, chart),
        focused_readout.foreground(Srgb::WHITE).padding_with(6.0),
        selected_readout.foreground(Srgb::WHITE).padding_with(6.0),
    ))
    .spacing(SEMANTIC_CHART_SHELL_SPACING)
    .background(Srgb::BLACK)
}

/// Builds a compact readout label view from a chart interaction binding.
#[allow(
    clippy::needless_pass_by_value,
    reason = "SignalExt::map consumes signal-like values through the extension trait API"
)]
pub fn readout_view<T, S>(
    prefix: &'static str,
    signal: S,
    formatter: fn(&HitResult<T>) -> String,
) -> impl View
where
    T: Clone + 'static,
    S: Signal<Output = Option<HitResult<T>>> + 'static,
    S::Guard: 'static,
{
    text(
        signal
            .map(move |hit| readout_text(prefix, hit, formatter))
            .computed(),
    )
    .caption()
    .body()
}

pub fn image_selector(name: &str) -> Selector {
    Selector::default()
        .role(Role::IMAGE)
        .label(chart_label(name))
}

pub fn assert_chart_accessibility_ready(app: &mut SemanticApp, name: &str) -> String {
    let selector = image_selector(name);
    assert!(
        app.wait_for_existence(&selector, Duration::from_secs(1)),
        "{name}: accessibility image element did not appear"
    );
    app.assert_exists(&selector);
    let element = app
        .query()
        .role(Role::IMAGE)
        .label(chart_label(name))
        .single();
    let bounds = element.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "{name}: chart accessibility bounds must be non-zero"
    );
    chart_label(name)
}

pub fn assert_label_exists(app: &mut SemanticApp, label: &str) {
    let selector = Selector::default()
        .role(Role::LABEL)
        .label(label.to_owned());
    assert!(
        app.wait_for_existence(&selector, Duration::from_secs(1)),
        "expected label to appear: {label}"
    );
    app.assert_exists(&selector);
}

pub fn readout_text<T>(
    prefix: &'static str,
    hit: Option<HitResult<T>>,
    formatter: fn(&HitResult<T>) -> String,
) -> String {
    hit.map_or_else(
        || format!("{prefix}:none"),
        |hit| format!("{prefix}:{}", formatter(&hit)),
    )
}

pub fn expected_readout<T: Clone>(
    prefix: &'static str,
    series: usize,
    index: usize,
    value: T,
    formatter: fn(&HitResult<T>) -> String,
) -> String {
    readout_text(
        prefix,
        Some(HitResult::new(
            series,
            index,
            value,
            ChartAnchor::new(0.0, 0.0),
        )),
        formatter,
    )
}

pub fn point_hit_label(hit: &HitResult<DataPoint>) -> String {
    format!(
        "series={} index={} x={:.2} y={:.2}",
        hit.series, hit.index, hit.value.x, hit.value.y
    )
}

pub fn bubble_hit_label(hit: &HitResult<BubblePoint>) -> String {
    format!(
        "series={} index={} x={:.2} y={:.2} size={:.2}",
        hit.series, hit.index, hit.value.x, hit.value.y, hit.value.size
    )
}

pub fn candle_hit_label(hit: &HitResult<Candle>) -> String {
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
}

pub fn depth_hit_label(hit: &HitResult<DepthDatum>) -> String {
    let side = match hit.value.side {
        DepthSide::Bid => "bid",
        DepthSide::Ask => "ask",
    };
    format!(
        "series={} index={} side={} price={:.2} cumulative={:.2}",
        hit.series, hit.index, side, hit.value.price, hit.value.cumulative_volume
    )
}

pub fn area_hit_label(hit: &HitResult<AreaDatum>) -> String {
    format!(
        "series={} index={} x={:.2} y={:.2}",
        hit.series, hit.index, hit.value.x, hit.value.y
    )
}

pub fn slice_hit_label(hit: &HitResult<SliceDatum>) -> String {
    format!(
        "series={} index={} value={:.2} start={:.3} end={:.3}",
        hit.series, hit.index, hit.value.value, hit.value.start_angle, hit.value.end_angle
    )
}

fn normalize_bounds(mut bounds: DataBounds) -> DataBounds {
    if bounds.max_x <= bounds.min_x {
        bounds.max_x = bounds.min_x + 1.0;
    }
    if bounds.max_y <= bounds.min_y {
        bounds.max_y = bounds.min_y + 1.0;
    }
    bounds
}

fn normalized_plot_point(bounds: DataBounds, x: f32, y: f32) -> (f32, f32) {
    let nx = (x - bounds.min_x) / (bounds.max_x - bounds.min_x);
    let ny = (y - bounds.min_y) / (bounds.max_y - bounds.min_y);
    (
        nx.mul_add(CHART_PADDING_RATIO.mul_add(-2.0, 1.0), CHART_PADDING_RATIO),
        (1.0 - ny).mul_add(CHART_PADDING_RATIO.mul_add(-2.0, 1.0), CHART_PADDING_RATIO),
    )
}

pub fn point_hit_location(data: &[DataPoint], index: usize) -> (f32, f32) {
    let bounds = normalize_bounds(DataBounds::from_points(data).with_padding(0.1));
    let point = data[index];
    normalized_plot_point(bounds, point.x, point.y)
}

pub fn bar_hit_location(data: &[DataPoint], index: usize) -> (f32, f32) {
    let source = DataBounds::from_points(data);
    let bounds = normalize_bounds(DataBounds::new(
        source.min_x,
        source.max_x,
        source.min_y.min(0.0),
        source.max_y.max(0.0),
    ));
    let point = data[index];
    normalized_plot_point(bounds, point.x, point.y * 0.5)
}

pub fn bubble_hit_location(data: &[BubblePoint], index: usize) -> (f32, f32) {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for point in data {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    let bounds = normalize_bounds(DataBounds::new(min_x, max_x, min_y, max_y).with_padding(0.1));
    let point = data[index];
    normalized_plot_point(bounds, point.x, point.y)
}

pub fn candlestick_hit_location(data: &[Candle], index: usize) -> (f32, f32) {
    let bounds = normalize_bounds(DataBounds::from_candles(data).with_padding(0.05));
    let candle = data[index];
    normalized_plot_point(
        bounds,
        candle.timestamp,
        f32::midpoint(candle.open, candle.close),
    )
}

pub fn depth_hit_location(data: &DepthData, side: DepthSide, index: usize) -> (f32, f32) {
    let bounds = normalize_bounds(data.bounds().with_padding(0.04));
    let level = match side {
        DepthSide::Bid => data.bids[index],
        DepthSide::Ask => data.asks[index],
    };
    normalized_plot_point(bounds, level.price, level.cumulative_volume)
}

pub fn area_hit_location(data: &AreaData, series: usize, index: usize) -> (f32, f32) {
    let bounds = normalize_bounds(data.bounds().with_padding(0.05));
    let x = data.x_values[index];
    let y = data.series[series].values[index];
    normalized_plot_point(bounds, x, y)
}

pub fn pie_hit_location(data: &[DataPoint], index: usize, inner_radius: f32) -> (f32, f32) {
    let total: f32 = data.iter().map(|point| point.y.max(0.0)).sum();
    let plot_width = CHART_WIDTH * PIE_PADDING_RATIO.mul_add(-2.0, 1.0);
    let plot_height = CHART_HEIGHT * PIE_PADDING_RATIO.mul_add(-2.0, 1.0);
    let outer_radius = plot_width.min(plot_height) * 0.45;
    let inner_radius = outer_radius * inner_radius;
    let mid_radius = f32::midpoint(inner_radius, outer_radius);
    let center_x = CHART_WIDTH * 0.5;
    let center_y = CHART_HEIGHT * 0.5;

    let mut start_angle = -core::f32::consts::FRAC_PI_2;
    for (current_index, point) in data.iter().enumerate() {
        let value = point.y.max(0.0);
        let sweep = core::f32::consts::TAU * (value / total.max(f32::EPSILON));
        let end_angle = start_angle + sweep;
        if current_index == index {
            let mid_angle = start_angle + sweep * 0.5;
            let x = mid_angle.cos().mul_add(mid_radius, center_x);
            let y = mid_angle.sin().mul_add(mid_radius, center_y);
            return (x / CHART_WIDTH, y / CHART_HEIGHT);
        }
        start_angle = end_angle;
    }
    panic!("pie_hit_location: slice index {index} is out of range")
}

pub fn pie_slice_datum(data: &[DataPoint], index: usize) -> SliceDatum {
    let total: f32 = data.iter().map(|point| point.y.max(0.0)).sum();
    let mut start_angle = -core::f32::consts::FRAC_PI_2;
    for (current_index, point) in data.iter().enumerate() {
        let value = point.y.max(0.0);
        let sweep = core::f32::consts::TAU * (value / total.max(f32::EPSILON));
        let end_angle = start_angle + sweep;
        if current_index == index {
            return SliceDatum::new(index, point.y, start_angle, end_angle);
        }
        start_angle = end_angle;
    }
    panic!("pie_slice_datum: slice index {index} is out of range")
}
