//! XCTest-style semantic coverage for chart scrolling and visible-domain APIs.

mod support;

use waterui::component::{text, vstack};
use waterui::graphics::color::Srgb;
use waterui::{Binding, SignalExt as _, View, ViewExt as _};
use waterui_chart::{ChartScrollableAxes, DataBounds, LineChart};
use waterui_testing::{Role, Selector, WaitOptions, WaitResult};

use support::{
    assert_chart_accessibility_ready, chart_surface, horizontal_drag_domain_delta, mount_view,
    point_series, vertical_drag_domain_delta,
};

fn assert_close(actual: f32, expected: f32, epsilon: f32, context: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= epsilon,
        "{context}: expected {expected:.4}, got {actual:.4}, delta={delta:.4}, epsilon={epsilon:.4}"
    );
}

fn scroll_shell<V: View, R: View>(name: &str, chart: V, readout: R) -> impl View {
    vstack((chart_surface(name, chart), readout))
        .spacing(6.0)
        .background(Srgb::BLACK)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the readout owns a cloned Binding signal for the mounted view lifetime"
)]
fn scalar_readout(prefix: &'static str, binding: Binding<f32>) -> impl View {
    text(
        binding
            .map(move |value| format!("{prefix}:{value:.2}"))
            .computed(),
    )
    .caption()
    .body()
    .foreground(Srgb::WHITE)
    .padding_with(6.0)
}

#[test]
fn line_chart_horizontal_drag_updates_scroll_position_binding() {
    let data = point_series();
    let bounds = DataBounds::from_points(&data).with_padding(0.1);
    let visible_length = 8.0_f32;
    let from = (0.75, 0.5);
    let to = (0.25, 0.5);
    let start = 0.0_f32.clamp(bounds.min_x, bounds.max_x - visible_length);
    let expected = (start + horizontal_drag_domain_delta(visible_length, from.0, to.0))
        .clamp(bounds.min_x, bounds.max_x - visible_length);
    let scroll_position = Binding::f32(0.0);
    let data_for_view = data;
    let scroll_for_view = scroll_position.clone();

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_x_visible_domain(visible_length)
            .chart_x_scroll_position(&scroll_for_view)
            .chart_scrollable_axes(ChartScrollableAxes::horizontal());
        scroll_shell(
            "line-scroll-x",
            chart,
            scalar_readout("x-pos", scroll_for_view.clone()),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "line-scroll-x");
    assert_close(scroll_position.get(), start, 0.05, "line-scroll-x initial");

    app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .drag_between(from.0, from.1, to.0, to.1);
    assert_close(scroll_position.get(), expected, 0.08, "line-scroll-x drag");
    app.query()
        .role(Role::LABEL)
        .label(format!("x-pos:{expected:.2}"))
        .assert_exists();
}

#[test]
fn line_chart_vertical_drag_updates_scroll_position_binding() {
    let data = point_series();
    let bounds = DataBounds::from_points(&data).with_padding(0.1);
    let visible_length = 10.0_f32;
    let from = (0.5, 0.75);
    let to = (0.5, 0.25);
    let expected = (bounds.min_y + vertical_drag_domain_delta(visible_length, from.1, to.1))
        .clamp(bounds.min_y, bounds.max_y - visible_length);
    let scroll_position = Binding::f32(0.0);
    let data_for_view = data;
    let scroll_for_view = scroll_position.clone();

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .chart_y_visible_domain(visible_length)
            .chart_y_scroll_position(&scroll_for_view)
            .chart_scrollable_axes(ChartScrollableAxes::vertical());
        scroll_shell(
            "line-scroll-y",
            chart,
            scalar_readout("y-pos", scroll_for_view.clone()),
        )
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "line-scroll-y");
    assert_close(
        scroll_position.get(),
        bounds.min_y,
        0.05,
        "line-scroll-y initial",
    );

    app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .drag_between(from.0, from.1, to.0, to.1);
    assert_close(scroll_position.get(), expected, 0.08, "line-scroll-y drag");
    app.query()
        .role(Role::LABEL)
        .label(format!("y-pos:{expected:.2}"))
        .assert_exists();
}

#[test]
#[should_panic(expected = "chart_x_visible_domain requires chart_x_scroll_position")]
fn line_chart_visible_domain_requires_scroll_position() {
    let data = point_series();
    let data_for_view = data;
    let _app = mount_view(move || {
        chart_surface(
            "line-visible-domain-invalid",
            LineChart::new(Binding::container(data_for_view.clone())).chart_x_visible_domain(8.0),
        )
    });
}

#[test]
fn line_chart_reactive_visible_domain_length_triggers_redraw() {
    let data = point_series();
    let visible_length = Binding::f32(8.0);
    let x_position = Binding::f32(0.0);
    let data_for_view = data;
    let visible_length_for_view = visible_length.clone();
    let x_position_for_view = x_position.clone();

    let mut app = mount_view(move || {
        scroll_shell(
            "line-visible-domain-reactive",
            LineChart::new(Binding::container(data_for_view.clone()))
                .chart_x_visible_domain(visible_length_for_view.clone())
                .chart_x_scroll_position(&x_position_for_view),
            scalar_readout("visible", visible_length_for_view.clone()),
        )
    });

    assert_chart_accessibility_ready(&mut app, "line-visible-domain-reactive");
    visible_length.set(4.0);
    assert!(
        app.wait_for(
            &[app.expect_exists(Selector::default().role(Role::LABEL).label("visible:4.00"),)],
            WaitOptions::new(std::time::Duration::from_millis(200)),
        ) == WaitResult::Completed,
        "line-visible-domain-reactive: visible-domain readout should update after binding change"
    );
    assert_close(
        x_position.get(),
        0.0,
        0.05,
        "line-visible-domain-reactive position",
    );
}

#[test]
#[should_panic(expected = "chart_scrollable_axes(horizontal) requires chart_x_scroll_position")]
fn line_chart_scrollable_axes_require_visible_domain() {
    let data = point_series();
    let data_for_view = data;
    let _app = mount_view(move || {
        chart_surface(
            "line-scroll-invalid",
            LineChart::new(Binding::container(data_for_view.clone()))
                .chart_scrollable_axes(ChartScrollableAxes::horizontal()),
        )
    });
}
