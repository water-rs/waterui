mod support;

use waterui::View;
use waterui_chart::{
    AreaChart, BarChart, BubbleChart, CandlestickChart, DepthChart, LineChart, ScatterChart,
};

use support::{
    assert_chart_accessibility_ready, binding_area, binding_bubbles, binding_candles,
    binding_depth, binding_points, chart_label, mount_chart,
};

fn assert_chart_interaction_smoke<V, F>(name: &str, build: F)
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let mut app = mount_chart(name, build);
    let label = assert_chart_accessibility_ready(&mut app, name);

    let base = app.snapshot();
    assert!(
        app.query().label(label.clone()).magnify(1.35),
        "{name}: magnify should be handled"
    );
    let magnified = app.snapshot();
    let magnify_pixels = base.changed_pixels(&magnified);
    assert!(magnify_pixels > 0, "{name}: magnify produced no frame diff");

    assert!(
        app.query().label(label).drag_by(48.0, 0.0),
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
fn line_chart_drag_and_magnify_smoke() {
    let data = binding_points();
    assert_chart_interaction_smoke("line", move || LineChart::new(data.clone()));
}

#[test]
fn bar_chart_drag_and_magnify_smoke() {
    let data = binding_points();
    assert_chart_interaction_smoke("bar", move || BarChart::new(data.clone()));
}

#[test]
fn scatter_chart_drag_and_magnify_smoke() {
    let data = binding_points();
    assert_chart_interaction_smoke("scatter", move || {
        ScatterChart::new(data.clone()).radius(6.0)
    });
}

#[test]
fn bubble_chart_drag_and_magnify_smoke() {
    let data = binding_bubbles();
    assert_chart_interaction_smoke("bubble", move || {
        BubbleChart::new(data.clone())
            .min_radius(4.0)
            .max_radius(18.0)
    });
}

#[test]
fn candlestick_chart_drag_and_magnify_smoke() {
    let data = binding_candles();
    assert_chart_interaction_smoke("candlestick", move || CandlestickChart::new(data.clone()));
}

#[test]
fn depth_chart_drag_and_magnify_smoke() {
    let data = binding_depth();
    assert_chart_interaction_smoke("depth", move || DepthChart::new(data.clone()));
}

#[test]
fn area_chart_drag_and_magnify_smoke() {
    let data = binding_area();
    assert_chart_interaction_smoke("area", move || AreaChart::new(data.clone()));
}

#[test]
fn chart_labels_are_stable() {
    assert_eq!(chart_label("line"), "chart-line");
}
