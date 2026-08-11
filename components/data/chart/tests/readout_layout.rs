//! Snapshot-layout regression coverage for chart readouts.

mod support;

use waterui::Binding;
use waterui::component::text;
use waterui_chart::LineChart;
use waterui_testing::Role;

use support::{VIEWPORT_HEIGHT, point_series, semantic_chart_shell};
use waterui_testing::UiBuilder;

fn viewport_height() -> f32 {
    num_traits::cast(VIEWPORT_HEIGHT).expect("viewport height must fit f32")
}

const FOCUSED_READOUT: &str =
    "focused:series=0 index=12 t=720.00 open=120.00 high=124.00 low=119.00 close=123.00";
const SELECTED_READOUT: &str =
    "selected:series=0 index=12 t=720.00 open=120.00 high=124.00 low=119.00 close=123.00";

#[waterui::test(viewport = (320, 320))]
fn semantic_chart_shell_keeps_long_readouts_inside_viewport(ui: UiBuilder) {
    let data = point_series();
    let mut app = ui.mount(move || {
        semantic_chart_shell(
            "line",
            LineChart::new(Binding::container(data.clone())),
            text(FOCUSED_READOUT).caption().body(),
            text(SELECTED_READOUT).caption().body(),
        )
    });

    let focused = app
        .query()
        .role(Role::LABEL)
        .label(FOCUSED_READOUT)
        .single();
    let focused_bounds = focused.bounds();
    assert!(
        focused_bounds.height() > 0.0,
        "focused readout height must be positive: {focused_bounds:?}"
    );
    assert!(
        focused_bounds.y() + focused_bounds.height() <= viewport_height(),
        "focused readout must stay inside viewport: {focused_bounds:?}"
    );

    let selected = app
        .query()
        .role(Role::LABEL)
        .label(SELECTED_READOUT)
        .single();
    let selected_bounds = selected.bounds();
    assert!(
        selected_bounds.height() > 0.0,
        "selected readout height must be positive: {selected_bounds:?}"
    );
    assert!(
        selected_bounds.y() + selected_bounds.height() <= viewport_height(),
        "selected readout must stay inside viewport: {selected_bounds:?}"
    );
}
