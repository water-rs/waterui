use waterui::accessibility::AccessibilityRole;
use waterui::{Binding, View, ViewExt as _};
use waterui_chart::{
    AreaChart, AreaData, AreaSeries, BarChart, BubbleChart, BubblePoint, Candle, CandlestickChart,
    DataPoint, DepthChart, DepthData, DepthLevel, LineChart, ScatterChart,
};
use waterui_testing::UiTest;

fn point_series() -> Vec<DataPoint> {
    (0..24)
        .map(|index| {
            let x = index as f32;
            let y = 24.0 + (x * 0.42).sin() * 9.0 + (x * 0.17).cos() * 5.0;
            DataPoint::new(x, y)
        })
        .collect()
}

fn bubble_series() -> Vec<BubblePoint> {
    (0..32)
        .map(|index| {
            let x = index as f32 * 0.75;
            let y = 18.0 + (x * 0.28).sin() * 10.0 + (x * 0.09).cos() * 4.0;
            let size = 4.0 + (index % 7) as f32 * 2.25;
            BubblePoint::new(x, y, size)
        })
        .collect()
}

fn candle_series() -> Vec<Candle> {
    let mut candles = Vec::with_capacity(32);
    let mut price = 120.0_f32;
    for index in 0..32 {
        let timestamp = index as f32 * 60.0;
        let drift = (index as f32 * 0.31).sin() * 4.5;
        let open = price;
        let close = open + drift;
        let high = open.max(close) + 1.4 + (index % 4) as f32 * 0.25;
        let low = open.min(close) - 1.1 - (index % 5) as f32 * 0.2;
        let volume = 20_000.0 + index as f32 * 350.0;
        candles.push(Candle::new(timestamp, open, high, low, close, volume));
        price = close;
    }
    candles
}

fn depth_data() -> DepthData {
    let mut bids = Vec::with_capacity(24);
    let mut asks = Vec::with_capacity(24);
    let mut bid_cumulative = 0.0_f32;
    let mut ask_cumulative = 0.0_f32;
    for index in 0..24 {
        let bid_price = 99.8 - index as f32 * 0.08;
        bid_cumulative += 6.0 + (index % 5) as f32 * 1.2;
        bids.push(DepthLevel::new(bid_price, bid_cumulative));

        let ask_price = 100.2 + index as f32 * 0.08;
        ask_cumulative += 6.5 + (index % 4) as f32 * 1.1;
        asks.push(DepthLevel::new(ask_price, ask_cumulative));
    }
    DepthData::new(bids, asks)
}

fn area_data() -> AreaData {
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

fn assert_chart_interactions<V, F>(name: &str, build: F)
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let label = format!("chart-{name}");
    let mut app = UiTest::new().viewport(320, 240).mount({
        let label = label.clone();
        move || {
            build()
                .size(240.0, 180.0)
                .a11y_label(label.clone())
                .a11y_role(AccessibilityRole::Image)
        }
    });

    let base = app.snapshot();
    assert!(
        app.query().label(label.clone()).magnify(1.35),
        "{name}: magnify should be handled"
    );
    let magnified = app.snapshot();
    let magnify_pixels = base.changed_pixels(&magnified);
    assert!(magnify_pixels > 0, "{name}: magnify produced no frame diff");

    assert!(
        app.query().label(label.clone()).drag_by(48.0, 0.0),
        "{name}: drag should be handled after magnify"
    );
    let dragged = app.snapshot();
    let drag_pixels = magnified.changed_pixels(&dragged);
    assert!(
        drag_pixels > 0,
        "{name}: drag produced no frame diff after magnify"
    );
}

#[test]
fn line_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(point_series());
    assert_chart_interactions("line", move || LineChart::new(data.clone()));
}

#[test]
fn bar_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(point_series());
    assert_chart_interactions("bar", move || BarChart::new(data.clone()));
}

#[test]
fn scatter_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(point_series());
    assert_chart_interactions("scatter", move || {
        ScatterChart::new(data.clone()).radius(6.0)
    });
}

#[test]
fn bubble_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(bubble_series());
    assert_chart_interactions("bubble", move || {
        BubbleChart::new(data.clone())
            .min_radius(4.0)
            .max_radius(18.0)
    });
}

#[test]
fn candlestick_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(candle_series());
    assert_chart_interactions("candlestick", move || CandlestickChart::new(data.clone()));
}

#[test]
fn depth_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(depth_data());
    assert_chart_interactions("depth", move || DepthChart::new(data.clone()));
}

#[test]
fn area_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(area_data());
    assert_chart_interactions("area", move || AreaChart::new(data.clone()));
}
