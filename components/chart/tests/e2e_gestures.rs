use waterui::accessibility::AccessibilityRole;
use waterui::{Binding, View, ViewExt as _};
use waterui_chart::{BarChart, Candle, CandlestickChart, DataPoint, LineChart, ScatterChart};
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
    assert!(app.query().label(label.clone()).magnify(1.35), "{name}: magnify should be handled");
    let magnified = app.snapshot();
    let magnify_pixels = base.changed_pixels(&magnified);
    assert!(magnify_pixels > 0, "{name}: magnify produced no frame diff");

    assert!(app.query().label(label.clone()).drag_by(48.0, 0.0), "{name}: drag should be handled after magnify");
    let dragged = app.snapshot();
    let drag_pixels = magnified.changed_pixels(&dragged);
    assert!(drag_pixels > 0, "{name}: drag produced no frame diff after magnify");
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
    assert_chart_interactions("scatter", move || ScatterChart::new(data.clone()).radius(6.0));
}

#[test]
fn candlestick_chart_drag_and_magnify_update_frame() {
    let data = Binding::container(candle_series());
    assert_chart_interactions("candlestick", move || CandlestickChart::new(data.clone()));
}
