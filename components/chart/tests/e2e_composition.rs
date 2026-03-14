//! Coverage for chart background/overlay composition APIs.

mod support;

use waterui::graphics::color::Srgb;
use waterui::layout::{PositionExt, absolute};
use waterui::{Binding, SignalExt as _, ViewExt as _};
use waterui_chart::LineChart;
use waterui_core::Environment;
use waterui_testing::{Role, TestHost};

use support::{
    CHART_HEIGHT, CHART_WIDTH, VIEWPORT_HEIGHT, VIEWPORT_WIDTH, assert_chart_accessibility_ready,
    chart_surface, mount_view, point_hit_location, point_series,
};

#[test]
fn chart_background_uses_plot_area_proxy_in_offscreen_rendering() {
    let data = point_series();
    let host = TestHost::new(Environment::new(), VIEWPORT_WIDTH, VIEWPORT_HEIGHT);

    let baseline = host.render(
        LineChart::new(Binding::container(data.clone()))
            .size(CHART_WIDTH, CHART_HEIGHT)
            .background(Srgb::BLACK),
    );
    let decorated = host.render(
        LineChart::new(Binding::container(data))
            .chart_background(|proxy| {
                absolute((Srgb::WHITE.size(10.0, 10.0).position(
                    proxy.plot_area_frame().map(|frame| frame.x + 6.0),
                    proxy.plot_area_frame().map(|frame| frame.y + 6.0),
                ),))
            })
            .size(CHART_WIDTH, CHART_HEIGHT)
            .background(Srgb::BLACK),
    );

    assert!(
        baseline.changed_pixels(&decorated) > 0,
        "chart_background should affect the offscreen render via plot-area proxy"
    );
}

#[test]
fn chart_overlay_static_layer_renders_in_offscreen_rendering() {
    let data = point_series();
    let host = TestHost::new(Environment::new(), VIEWPORT_WIDTH, VIEWPORT_HEIGHT);

    let baseline = host.render(
        LineChart::new(Binding::container(data.clone()))
            .size(CHART_WIDTH, CHART_HEIGHT)
            .background(Srgb::BLACK),
    );
    let decorated = host.render(
        LineChart::new(Binding::container(data))
            .chart_overlay(|_proxy| {
                absolute((Srgb::new(1.0, 0.25, 0.1)
                    .size(24.0, 24.0)
                    .position(24.0, 24.0),))
            })
            .size(CHART_WIDTH, CHART_HEIGHT)
            .background(Srgb::BLACK),
    );

    assert!(
        baseline.changed_pixels(&decorated) > 0,
        "chart_overlay should affect the offscreen render when static"
    );
}

#[test]
fn chart_overlay_reacts_to_proxy_selection_without_external_bindings() {
    let data = point_series();
    let index = 10;
    let location = point_hit_location(&data, index);
    let proxy_selected =
        Binding::container(None::<waterui_chart::HitResult<waterui_chart::DataPoint>>);
    let proxy_selected_for_view = proxy_selected.clone();

    let mut app = mount_view(move || {
        let proxy_selected_binding = proxy_selected_for_view.clone();
        chart_surface(
            "composition-line",
            LineChart::new(Binding::container(data.clone())).chart_overlay(move |proxy| {
                absolute((Srgb::new(1.0, 0.25, 0.1)
                    .size(24.0, 24.0)
                    .opacity(proxy.selected().is_some().map(
                        |selected| {
                            if selected { 1.0 } else { 0.0 }
                        },
                    ))
                    .position(24.0, 24.0)
                    .on_change(&proxy.selected(), {
                        let proxy_selected_binding = proxy_selected_binding.clone();
                        move |value| proxy_selected_binding.set(value)
                    }),))
            }),
        )
        .background(Srgb::BLACK)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "composition-line");
    let base = app.snapshot();

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .tap_at(location.0, location.1),
        "composition-line: tap should drive proxy selection state"
    );
    assert!(
        proxy_selected.get().is_some(),
        "chart_overlay proxy selection should update without external chart bindings"
    );
    let selected = app.snapshot();
    assert!(
        base.changed_pixels(&selected) > 0,
        "chart_overlay should react visually to proxy selection state"
    );
}

#[test]
fn chart_overlay_reacts_to_proxy_selection_with_external_binding() {
    let data = point_series();
    let index = 10;
    let location = point_hit_location(&data, index);
    let selected = Binding::container(None::<waterui_chart::HitResult<waterui_chart::DataPoint>>);
    let selected_for_view = selected.clone();

    let mut app = mount_view(move || {
        chart_surface(
            "composition-line-external",
            LineChart::new(Binding::container(data.clone()))
                .selected(&selected_for_view)
                .chart_overlay(|proxy| {
                    absolute((Srgb::new(1.0, 0.25, 0.1)
                        .size(24.0, 24.0)
                        .opacity(proxy.selected().is_some().map(
                            |selected| {
                                if selected { 1.0 } else { 0.0 }
                            },
                        ))
                        .position(24.0, 24.0),))
                }),
        )
        .background(Srgb::BLACK)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "composition-line-external");
    let base = app.snapshot();
    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .tap_at(location.0, location.1),
        "composition-line-external: tap should drive selected binding"
    );
    assert!(
        selected.get().is_some(),
        "external selected binding should update"
    );
    let tapped = app.snapshot();
    assert!(
        base.changed_pixels(&tapped) > 0,
        "chart_overlay should react visually when proxy reads external selected binding"
    );
}

#[test]
fn chart_overlay_reactive_layer_is_initially_hidden_without_selection() {
    let data = point_series();
    let host = TestHost::new(Environment::new(), VIEWPORT_WIDTH, VIEWPORT_HEIGHT);

    let baseline = host.render(
        LineChart::new(Binding::container(data.clone()))
            .size(CHART_WIDTH, CHART_HEIGHT)
            .background(Srgb::BLACK),
    );
    let decorated = host.render(
        LineChart::new(Binding::container(data))
            .chart_overlay(|proxy| {
                absolute((Srgb::new(1.0, 0.25, 0.1)
                    .size(24.0, 24.0)
                    .opacity(proxy.selected().is_some().map(
                        |selected| {
                            if selected { 1.0 } else { 0.0 }
                        },
                    ))
                    .position(24.0, 24.0),))
            })
            .size(CHART_WIDTH, CHART_HEIGHT)
            .background(Srgb::BLACK),
    );

    assert_eq!(
        baseline.changed_pixels(&decorated),
        0,
        "reactive chart_overlay should stay hidden before selection"
    );
}

#[test]
fn selected_tooltip_stacks_with_existing_overlay_layers() {
    let data = point_series();
    let index = 10;
    let location = point_hit_location(&data, index);

    let mut app = mount_view(move || {
        chart_surface(
            "composition-line-tooltip",
            LineChart::new(Binding::container(data.clone()))
                .chart_overlay(|_proxy| {
                    absolute((Srgb::WHITE.size(12.0, 12.0).position(18.0, 18.0),))
                })
                .selected_tooltip(|hit| {
                    waterui_chart::Tooltip::new(
                        waterui_chart::TooltipContent::new()
                            .title("Selected")
                            .value("Series", hit.series.to_string())
                            .value("Index", hit.index.to_string()),
                    )
                }),
        )
        .background(Srgb::BLACK)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "composition-line-tooltip");
    let base = app.snapshot();
    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(chart_label)
            .tap_at(location.0, location.1),
        "composition-line-tooltip: tap should drive tooltip selection"
    );
    app.query()
        .role(Role::LABEL)
        .label("Selected")
        .assert_exists();
    let selected = app.snapshot();
    assert!(
        base.changed_pixels(&selected) > 0,
        "selected_tooltip should render on top of an existing chart_overlay layer"
    );
}
