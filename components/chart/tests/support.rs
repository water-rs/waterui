use std::time::Duration;

use waterui::accessibility::AccessibilityRole;
use waterui::{Binding, View, ViewExt as _};
use waterui_chart::{AreaData, AreaSeries, BubblePoint, Candle, DataPoint, DepthData, DepthLevel};
use waterui_testing::{MountedApp, Role, Selector, TestArtifacts, UiTest};

const VIEWPORT_WIDTH: u32 = 320;
const VIEWPORT_HEIGHT: u32 = 240;
const CHART_WIDTH: f32 = 240.0;
const CHART_HEIGHT: f32 = 180.0;

pub fn point_series() -> Vec<DataPoint> {
    (0..24)
        .map(|index| {
            let x = index as f32;
            let y = 24.0 + (x * 0.42).sin() * 9.0 + (x * 0.17).cos() * 5.0;
            DataPoint::new(x, y)
        })
        .collect()
}

pub fn bubble_series() -> Vec<BubblePoint> {
    (0..32)
        .map(|index| {
            let x = index as f32 * 0.75;
            let y = 18.0 + (x * 0.28).sin() * 10.0 + (x * 0.09).cos() * 4.0;
            let size = 4.0 + (index % 7) as f32 * 2.25;
            BubblePoint::new(x, y, size)
        })
        .collect()
}

pub fn candle_series() -> Vec<Candle> {
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

pub fn depth_data() -> DepthData {
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

pub fn chart_label(name: &str) -> String {
    format!("chart-{name}")
}

pub fn chart_artifacts(suite: &str) -> TestArtifacts {
    TestArtifacts::new(format!("chart/{suite}"))
}

pub fn mount_chart<V, F>(name: &str, build: F) -> MountedApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let label = chart_label(name);
    UiTest::new()
        .viewport(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
        .mount(move || {
            build()
                .size(CHART_WIDTH, CHART_HEIGHT)
                .a11y_label(label.clone())
                .a11y_role(AccessibilityRole::Image)
        })
}

pub fn image_selector(name: &str) -> Selector {
    Selector::default()
        .role(Role::IMAGE)
        .label(chart_label(name))
}

pub fn assert_chart_accessibility_ready(app: &mut MountedApp, name: &str) -> String {
    let selector = image_selector(name);
    assert!(
        app.wait_for_existence(selector.clone(), Duration::from_secs(1)),
        "{name}: accessibility image element did not appear"
    );
    app.assert_exists(selector.clone());
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

pub fn binding_points() -> Binding<Vec<DataPoint>> {
    Binding::container(point_series())
}

pub fn binding_bubbles() -> Binding<Vec<BubblePoint>> {
    Binding::container(bubble_series())
}

pub fn binding_candles() -> Binding<Vec<Candle>> {
    Binding::container(candle_series())
}

pub fn binding_depth() -> Binding<DepthData> {
    Binding::container(depth_data())
}

pub fn binding_area() -> Binding<AreaData> {
    Binding::container(area_data())
}
