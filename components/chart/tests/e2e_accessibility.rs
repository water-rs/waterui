mod support;

use waterui::View;
use waterui_chart::{
    AreaChart, BarChart, BubbleChart, CandlestickChart, DepthChart, LineChart, ScatterChart,
};

use support::{
    assert_chart_accessibility_ready, binding_area, binding_bubbles, binding_candles,
    binding_depth, binding_points, chart_artifacts, mount_chart,
};

const SNAPSHOT_SUITE: &str = "xctest-like";

fn assert_chart_xctest_flow<V, F>(name: &str, build: F)
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let artifacts = chart_artifacts(SNAPSHOT_SUITE);
    let mut app = mount_chart(name, build);
    let label = assert_chart_accessibility_ready(&mut app, name);

    let initial = app.snapshot();
    let initial_path = artifacts.save_snapshot(name, "00_initial", &initial);
    assert!(
        initial_path.is_file(),
        "{name}: initial snapshot path missing"
    );

    assert!(
        app.query().label(label.clone()).magnify(1.35),
        "{name}: magnify should be handled through accessibility-selected element"
    );
    let magnified = app.snapshot();
    let magnified_path = artifacts.save_snapshot(name, "01_magnified", &magnified);
    assert!(
        initial.changed_pixels(&magnified) > 0,
        "{name}: magnify should change the rendered frame"
    );
    assert!(
        magnified_path.is_file(),
        "{name}: magnified snapshot path missing"
    );

    assert!(
        app.query().label(label).drag_by(48.0, 0.0),
        "{name}: drag should be handled through accessibility-selected element"
    );
    let dragged = app.snapshot();
    let dragged_path = artifacts.save_snapshot(name, "02_dragged", &dragged);
    assert!(
        magnified.changed_pixels(&dragged) > 0,
        "{name}: drag should change the rendered frame"
    );
    assert!(
        dragged_path.is_file(),
        "{name}: dragged snapshot path missing"
    );
}

#[test]
fn xctest_like_line_chart_accessibility_flow() {
    let data = binding_points();
    assert_chart_xctest_flow("line", move || LineChart::new(data.clone()));
}

#[test]
fn xctest_like_bar_chart_accessibility_flow() {
    let data = binding_points();
    assert_chart_xctest_flow("bar", move || BarChart::new(data.clone()));
}

#[test]
fn xctest_like_scatter_chart_accessibility_flow() {
    let data = binding_points();
    assert_chart_xctest_flow("scatter", move || {
        ScatterChart::new(data.clone()).radius(6.0)
    });
}

#[test]
fn xctest_like_bubble_chart_accessibility_flow() {
    let data = binding_bubbles();
    assert_chart_xctest_flow("bubble", move || {
        BubbleChart::new(data.clone())
            .min_radius(4.0)
            .max_radius(18.0)
    });
}

#[test]
fn xctest_like_candlestick_chart_accessibility_flow() {
    let data = binding_candles();
    assert_chart_xctest_flow("candlestick", move || CandlestickChart::new(data.clone()));
}

#[test]
fn xctest_like_depth_chart_accessibility_flow() {
    let data = binding_depth();
    assert_chart_xctest_flow("depth", move || DepthChart::new(data.clone()));
}

#[test]
fn xctest_like_area_chart_accessibility_flow() {
    let data = binding_area();
    assert_chart_xctest_flow("area", move || AreaChart::new(data.clone()));
}
