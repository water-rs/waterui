//! Coverage for chart background/overlay composition APIs.

mod support;

use waterui::accessibility::{AccessibilityRole, AccessibilityState};
use waterui::component::text;
use waterui::graphics::color::Srgb;
use waterui::layout::{PositionExt, UnitPoint, absolute};
use waterui::{Binding, SignalExt as _, View, ViewExt as _};
use waterui_chart::LineChart;
use waterui_testing::{Role, SemanticApp, UiBuilder};

use support::{
    CHART_HEIGHT, CHART_WIDTH, assert_chart_accessibility_ready, assert_label_exists,
    chart_surface, point_hit_location, point_series,
};

fn overlay_badge(label: &'static str) -> impl View {
    text(label)
        .caption()
        .foreground(Srgb::WHITE)
        .padding_with(4.0)
        .background(Srgb::new(0.85, 0.2, 0.15))
        .a11y_role(AccessibilityRole::Text)
        .a11y_label(label)
}

fn assert_label_bounds(app: &mut SemanticApp, label: &str) {
    let badge = app.query().role(Role::LABEL).label(label).single();
    let bounds = badge.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "{label}: overlay badge should have non-zero bounds: {bounds:?}"
    );
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "chart proxy values are handed to composition callbacks by value"
)]
fn selected_visibility<T: Clone + PartialEq + 'static>(
    proxy: waterui_chart::ChartProxy<T>,
) -> impl waterui::Signal<Output = bool> + 'static {
    proxy
        .selected()
        .map(|selected| selected.is_some())
        .computed()
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the returned accessibility-state signal owns the composed visibility signal"
)]
fn hidden_accessibility_state(
    visible: impl waterui::Signal<Output = bool> + 'static,
) -> impl waterui::Signal<Output = AccessibilityState> + 'static {
    visible
        .map(|visible| AccessibilityState::new().hidden(!visible))
        .computed()
}

#[waterui::test(viewport = (320, 320))]
fn chart_background_uses_plot_area_proxy_in_accessibility_tree(ui: UiBuilder) {
    let data = point_series();
    let mut app = ui.mount(move || {
        chart_surface(
            "composition-background",
            LineChart::new(Binding::container(data.clone()))
                .chart_background(|proxy| {
                    absolute((overlay_badge("Plot area marker").position_anchor(
                        UnitPoint::TOP_LEADING,
                        proxy.plot_area_frame().map(|frame| frame.x + 6.0),
                        proxy.plot_area_frame().map(|frame| frame.y + 6.0),
                    ),))
                })
                .size(CHART_WIDTH, CHART_HEIGHT),
        )
        .background(Srgb::BLACK)
    });
    assert_chart_accessibility_ready(&mut app, "composition-background");
    app.query()
        .role(Role::LABEL)
        .label("Plot area marker")
        .assert_exists();
    assert_label_bounds(&mut app, "Plot area marker");
}

#[waterui::test(viewport = (320, 320))]
fn chart_overlay_static_layer_exposes_accessibility_node(ui: UiBuilder) {
    let data = point_series();
    let mut app = ui.mount(move || {
        chart_surface(
            "composition-overlay",
            LineChart::new(Binding::container(data.clone())).chart_overlay(|_proxy| {
                absolute((overlay_badge("Static overlay").position_anchor(
                    UnitPoint::TOP_LEADING,
                    24.0,
                    24.0,
                ),))
            }),
        )
        .background(Srgb::BLACK)
    });
    assert_chart_accessibility_ready(&mut app, "composition-overlay");
    app.query()
        .role(Role::LABEL)
        .label("Static overlay")
        .assert_exists();
    assert_label_bounds(&mut app, "Static overlay");
}

#[waterui::test(viewport = (320, 320))]
fn chart_overlay_reacts_to_proxy_selection_without_external_bindings(ui: UiBuilder) {
    let data = point_series();
    let index = 10;
    let location = point_hit_location(&data, index);
    let proxy_selected =
        Binding::container(None::<waterui_chart::HitResult<waterui_chart::DataPoint>>);
    let proxy_selected_for_view = proxy_selected.clone();

    let mut app = ui.mount(move || {
        let proxy_selected_binding = proxy_selected_for_view.clone();
        chart_surface(
            "composition-line",
            LineChart::new(Binding::container(data.clone())).chart_overlay(move |proxy| {
                let visible = selected_visibility(proxy.clone());
                absolute((overlay_badge("Proxy selection overlay")
                    .position_anchor(UnitPoint::TOP_LEADING, 24.0, 24.0)
                    .a11y_state_signal(hidden_accessibility_state(visible))
                    .on_change(&proxy.selected(), {
                        let proxy_selected_binding = proxy_selected_binding.clone();
                        move |value| proxy_selected_binding.set(value)
                    }),))
            }),
        )
        .background(Srgb::BLACK)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "composition-line");
    app.query()
        .role(Role::LABEL)
        .label("Proxy selection overlay")
        .assert_not_exists();

    app.query()
        .role(Role::IMAGE)
        .label(chart_label)
        .tap_at(location.0, location.1);
    assert!(
        proxy_selected.get().is_some(),
        "chart_overlay proxy selection should update without external chart bindings"
    );
    app.query()
        .role(Role::LABEL)
        .label("Proxy selection overlay")
        .assert_exists();
    assert_label_bounds(&mut app, "Proxy selection overlay");
}

#[waterui::test(viewport = (320, 320))]
fn chart_overlay_reacts_to_proxy_selection_with_external_binding(ui: UiBuilder) {
    let data = point_series();
    let index = 10;
    let location = point_hit_location(&data, index);
    let selected = Binding::container(None::<waterui_chart::HitResult<waterui_chart::DataPoint>>);
    let selected_for_view = selected.clone();

    let mut app = ui.mount(move || {
        chart_surface(
            "composition-line-external",
            LineChart::new(Binding::container(data.clone()))
                .selected(&selected_for_view)
                .chart_overlay(|proxy| {
                    let visible = selected_visibility(proxy);
                    absolute((overlay_badge("External selection overlay")
                        .position_anchor(UnitPoint::TOP_LEADING, 24.0, 24.0)
                        .a11y_state_signal(hidden_accessibility_state(visible)),))
                }),
        )
        .background(Srgb::BLACK)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "composition-line-external");
    app.query()
        .role(Role::LABEL)
        .label("External selection overlay")
        .assert_not_exists();
    app.query()
        .role(Role::IMAGE)
        .label(chart_label)
        .tap_at(location.0, location.1);
    assert!(
        selected.get().is_some(),
        "external selected binding should update"
    );
    app.query()
        .role(Role::LABEL)
        .label("External selection overlay")
        .assert_exists();
    assert_label_bounds(&mut app, "External selection overlay");
}

#[waterui::test(viewport = (320, 320))]
fn chart_overlay_reactive_layer_is_initially_hidden_without_selection(ui: UiBuilder) {
    let data = point_series();
    let mut app = ui.mount(move || {
        chart_surface(
            "composition-hidden",
            LineChart::new(Binding::container(data.clone())).chart_overlay(|proxy| {
                let visible = selected_visibility(proxy);
                absolute((overlay_badge("Hidden overlay")
                    .position_anchor(UnitPoint::TOP_LEADING, 24.0, 24.0)
                    .a11y_state_signal(hidden_accessibility_state(visible)),))
            }),
        )
        .background(Srgb::BLACK)
    });
    assert_chart_accessibility_ready(&mut app, "composition-hidden");
    app.query()
        .role(Role::LABEL)
        .label("Hidden overlay")
        .assert_not_exists();
}

#[waterui::test(viewport = (320, 320))]
fn selected_overlay_exposes_accessibility_labels(ui: UiBuilder) {
    let data = point_series();
    let index = 10;
    let location = point_hit_location(&data, index);
    let selected = Binding::container(None::<waterui_chart::HitResult<waterui_chart::DataPoint>>);
    let selected_for_view = selected.clone();

    let mut app = ui.mount(move || {
        chart_surface(
            "composition-line-tooltip",
            LineChart::new(Binding::container(data.clone()))
                .selected(&selected_for_view)
                .chart_overlay({
                    let selected_for_overlay = selected_for_view.clone();
                    move |_proxy| {
                        let overlay_label = selected_for_overlay
                            .clone()
                            .map(|selected| {
                                selected.map_or_else(String::new, |hit| {
                                    format!("Selected series={} index={}", hit.series, hit.index)
                                })
                            })
                            .computed();
                        let visible = selected_for_overlay
                            .clone()
                            .map(|selected| selected.is_some())
                            .computed();
                        absolute((text(overlay_label)
                            .caption()
                            .foreground(Srgb::WHITE)
                            .padding_with(6.0)
                            .background(Srgb::new(0.12, 0.18, 0.32))
                            .position_anchor(UnitPoint::TOP_LEADING, 24.0, 24.0)
                            .a11y_state_signal(hidden_accessibility_state(visible)),))
                    }
                }),
        )
        .background(Srgb::BLACK)
    });

    let chart_label = assert_chart_accessibility_ready(&mut app, "composition-line-tooltip");
    app.query()
        .role(Role::IMAGE)
        .label(chart_label)
        .tap_at(location.0, location.1);
    assert!(
        selected.get().is_some(),
        "composition-line-tooltip: tap should update the selected binding"
    );
    app.query()
        .role(Role::LABEL)
        .label_contains("Selected series=0 index=10")
        .assert_exists();
}

#[waterui::test(viewport = (320, 320))]
fn selected_tooltip_exposes_accessibility_labels(ui: UiBuilder) {
    let data = point_series();
    let index = 10;
    let location = point_hit_location(&data, index);
    let selected = Binding::container(None::<waterui_chart::HitResult<waterui_chart::DataPoint>>);
    let selected_for_view = selected.clone();

    let mut app = ui.mount(move || {
        chart_surface(
            "composition-line-selected-tooltip",
            LineChart::new(Binding::container(data.clone()))
                .selected(&selected_for_view)
                .selected_tooltip(|hit| {
                    waterui_chart::Tooltip::new(
                        waterui_chart::TooltipContent::new()
                            .title("Selected")
                            .value("Series", format!("{}", hit.series))
                            .value("Index", format!("{}", hit.index)),
                    )
                    .background(Srgb::new(0.12, 0.18, 0.32))
                }),
        )
        .background(Srgb::BLACK)
    });

    let chart_label =
        assert_chart_accessibility_ready(&mut app, "composition-line-selected-tooltip");
    app.query()
        .role(Role::LABEL)
        .label("Selected")
        .assert_not_exists();
    app.query()
        .role(Role::IMAGE)
        .label(chart_label)
        .tap_at(location.0, location.1);
    assert!(
        selected.get().is_some(),
        "composition-line-selected-tooltip: tap should update selected binding"
    );
    assert_label_exists(&mut app, "Selected");
    app.query()
        .role(Role::LABEL)
        .label("Series: 0")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Index: 10")
        .assert_exists();
}

#[waterui::test(viewport = (320, 320))]
fn tooltip_component_exposes_accessible_labels(ui: UiBuilder) {
    let mut app = ui.mount(|| {
        waterui_chart::Tooltip::new(
            waterui_chart::TooltipContent::new()
                .title("Selected")
                .value("Series", "0")
                .value("Index", "10"),
        )
        .background(Srgb::new(0.12, 0.18, 0.32))
    });

    app.query()
        .role(Role::LABEL)
        .label("Selected")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Series: 0")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Index: 10")
        .assert_exists();
}
