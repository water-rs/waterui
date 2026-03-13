//! Focused interaction smoke coverage for chart hover, tap, and drag flows.

mod support;

use waterui::Binding;
use waterui_chart::{HitResult, LineChart, PieChart};

use support::{
    assert_chart_accessibility_ready, assert_label_exists, chart_label, mount_view, pie_data,
    pie_hit_location, pie_slice_datum, point_hit_location, point_series, readout_view,
    semantic_chart_shell,
};

#[test]
fn chart_labels_are_stable() {
    assert_eq!(chart_label("line"), "chart-line");
    assert_eq!(chart_label("pie"), "chart-pie");
}

#[test]
fn line_chart_drag_between_updates_selection_smoke() {
    let data = point_series();
    let focused = Binding::container(None::<HitResult<waterui_chart::DataPoint>>);
    let selected = Binding::container(None::<HitResult<waterui_chart::DataPoint>>);
    let data_for_view = data.clone();
    let focused_for_view = focused.clone();
    let selected_for_view = selected.clone();
    let from_index = 6;
    let to_index = 7;
    let from = point_hit_location(&data, from_index);
    let to = point_hit_location(&data, to_index);

    let mut app = mount_view(move || {
        let chart = LineChart::new(Binding::container(data_for_view.clone()))
            .focused(&focused_for_view)
            .selected(&selected_for_view);
        let focused_readout = readout_view(
            "focused",
            focused_for_view.clone(),
            |hit: &HitResult<waterui_chart::DataPoint>| {
                format!(
                    "series={} index={} x={:.2} y={:.2}",
                    hit.series, hit.index, hit.value.x, hit.value.y
                )
            },
        );
        let selected_readout = readout_view(
            "selected",
            selected_for_view.clone(),
            |hit: &HitResult<waterui_chart::DataPoint>| {
                format!(
                    "series={} index={} x={:.2} y={:.2}",
                    hit.series, hit.index, hit.value.x, hit.value.y
                )
            },
        );
        semantic_chart_shell("line", chart, focused_readout, selected_readout)
    });

    let label = assert_chart_accessibility_ready(&mut app, "line");
    assert!(
        app.query()
            .role(waterui_testing::Role::IMAGE)
            .label(label)
            .drag_between(from.0, from.1, to.0, to.1),
        "line: drag_between should be handled"
    );
    assert!(focused.get().is_none(), "line: drag end should clear focus");
    let selected_hit = selected
        .get()
        .expect("line: drag end should produce a selected hit");
    assert_eq!(selected_hit.series, 0);
    assert!(selected_hit.index >= from_index && selected_hit.index <= to_index);
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("selected:none")
        .assert_not_exists();
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label_contains("selected:series=0 index=")
        .assert_exists();
}

#[test]
fn pie_chart_hover_and_tap_coordinate_smoke() {
    let data = pie_data();
    let focused = Binding::container(None::<HitResult<waterui_chart::SliceDatum>>);
    let selected = Binding::container(None::<HitResult<waterui_chart::SliceDatum>>);
    let data_for_view = data.clone();
    let focused_for_view = focused.clone();
    let selected_for_view = selected.clone();
    let index = 2;
    let location = pie_hit_location(&data, index, 0.0);
    let expected = pie_slice_datum(&data, index);

    let mut app = mount_view(move || {
        let chart = PieChart::new(Binding::container(data_for_view.clone()))
            .focused(&focused_for_view)
            .selected(&selected_for_view);
        let focused_readout = readout_view(
            "focused",
            focused_for_view.clone(),
            |hit: &HitResult<waterui_chart::SliceDatum>| {
                format!(
                    "series={} index={} value={:.2}",
                    hit.series, hit.index, hit.value.value
                )
            },
        );
        let selected_readout = readout_view(
            "selected",
            selected_for_view.clone(),
            |hit: &HitResult<waterui_chart::SliceDatum>| {
                format!(
                    "series={} index={} value={:.2}",
                    hit.series, hit.index, hit.value.value
                )
            },
        );
        semantic_chart_shell("pie", chart, focused_readout, selected_readout)
    });

    let label = assert_chart_accessibility_ready(&mut app, "pie");
    assert!(
        app.query()
            .role(waterui_testing::Role::IMAGE)
            .label(label.clone())
            .hover_at(location.0, location.1),
        "pie: hover_at should be handled"
    );
    let focused_hit = focused
        .get()
        .expect("pie: hover should produce a focused hit");
    assert_eq!(focused_hit.series, 0);
    assert_eq!(focused_hit.index, index);
    assert_eq!(focused_hit.value, expected);
    assert_label_exists(&mut app, "selected:none");

    assert!(
        app.query()
            .role(waterui_testing::Role::IMAGE)
            .label(label)
            .tap_at(location.0, location.1),
        "pie: tap_at should be handled"
    );
    let selected_hit = selected
        .get()
        .expect("pie: tap should produce a selected hit");
    assert_eq!(selected_hit.series, 0);
    assert_eq!(selected_hit.index, index);
    assert_eq!(selected_hit.value, expected);
    app.query()
        .role(waterui_testing::Role::LABEL)
        .label("selected:none")
        .assert_not_exists();
}
