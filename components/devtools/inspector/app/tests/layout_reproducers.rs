//! Minimal reproducers for layout defects found while building the Inspector.
//!
//! These are kept because each one is a framework bug the Inspector currently
//! works around, and a reproducer is worth more than a bug report. They write
//! PNGs for direct image review and are ignored by default.
//!
//! - `stack_rows_with_visible_modifiers` — a `.visible(false)` row still
//!   reserves its space, and a `.visible(...)` wrapper ignores the parent
//!   stack's alignment.
//! - `chart_above_a_metric_grid` / `chart_containment_strategies` — a
//!   `LineChart` sibling leaves the other children of its stack no height, so
//!   they draw on top of each other. Neither `.height()` on the chart nor
//!   wrapping it in a sized stack contains it.
//! - `chart_with_fixed_height_rows` — the workaround the Inspector uses:
//!   give the starved rows an explicit height.
//!
//! The remaining tests are controls: they show the same structures laying out
//! correctly when the offending ingredient is absent.

use hydrolysis_m3::install;
use waterui::prelude::*;
use waterui_controls::LabelDisplayMode;
use waterui_testing::ui as test_ui;

#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn nested_vstack_with_and_without_alignment() {
    let mut app = test_ui().viewport(600, 200).theme(install).mount(move || {
        hstack((
            vstack((text("aligned one"), text("aligned two")))
                .alignment(HorizontalAlignment::Leading)
                .spacing(2.0),
            spacer(),
            vstack((text("plain one"), text("plain two"))).spacing(2.0),
        ))
        .padding_with(EdgeInsets::all(20.0))
    });
    let _ = app.capture_snapshot("inspector", "vstack-alignment", "probe");
}

/// The Inspector's metric grid: a vertical stack of horizontal rows, each row
/// holding small vertical label/value pairs.
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn vstack_of_hstacks_of_vstacks() {
    fn cell(label: &'static str, value: &'static str) -> impl View {
        vstack((text(value).headline(), text(label).caption()))
            .alignment(HorizontalAlignment::Leading)
            .spacing(2.0)
    }

    let mut app = test_ui().viewport(600, 300).theme(install).mount(move || {
        vstack((
            hstack((cell("Kind", "reencode"), cell("Total", "3.39ms"))).spacing(32.0),
            hstack((cell("Layout", "1.13ms"), cell("Encode", "1.13ms"))).spacing(32.0),
            hstack((cell("Layers", "14"), cell("Clip", "3"))).spacing(32.0),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(16.0)
        .padding_with(EdgeInsets::all(20.0))
    });
    let _ = app.capture_snapshot("inspector", "vstack-alignment", "grid");
}

/// The same grid, but every value is a reactive `Computed<Str>` rendered with
/// `text!` — which is what the Inspector's panes actually do.
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn vstack_of_hstacks_with_reactive_text() {
    fn cell(label: &'static str, value: &Computed<Str>) -> impl View {
        vstack((text!("{value}").headline(), text(label).caption()))
            .alignment(HorizontalAlignment::Leading)
            .spacing(2.0)
    }

    fn constant(value: &'static str) -> Computed<Str> {
        Computed::constant(Str::from(value))
    }

    let mut app = test_ui().viewport(600, 300).theme(install).mount(move || {
        vstack((
            hstack((
                cell("Kind", &constant("reencode").computed()),
                cell("Total", &constant("3.39ms").computed()),
            ))
            .spacing(32.0),
            hstack((
                cell("Layout", &constant("1.13ms").computed()),
                cell("Encode", &constant("1.13ms").computed()),
            ))
            .spacing(32.0),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(16.0)
        .padding_with(EdgeInsets::all(20.0))
    });
    let _ = app.capture_snapshot("inspector", "vstack-alignment", "reactive");
}

/// A stack whose rows carry `.visible(...)`, as the Inspector's panes do to
/// swap between an empty state and content.
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn stack_rows_with_visible_modifiers() {
    let shown = Binding::bool(true);
    let hidden = Binding::bool(false);

    let mut app = test_ui().viewport(600, 300).theme(install).mount(move || {
        vstack((
            text("first row").headline(),
            text("hidden row").headline().visible(hidden.clone()),
            text("second row").headline().visible(shown.clone()),
            Divider,
            vstack((text("grouped one"), text("grouped two")))
                .spacing(2.0)
                .visible(shown.clone()),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(12.0)
        .padding_with(EdgeInsets::all(20.0))
    });
    let _ = app.capture_snapshot("inspector", "vstack-alignment", "visible");
}

/// A toggle driven by a derived (`Binding::mapping`) binding, beside one driven
/// by a plain container binding. Both read `true`.
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn toggle_reflects_a_derived_binding() {
    let plain = Binding::bool(true);
    let source = Binding::u32(0b0000_0001);
    let derived = Binding::mapping(
        &source,
        |bits: u32| bits & 1 != 0,
        |bits: &Binding<u32>, on: bool| {
            let next = if on { bits.get() | 1 } else { bits.get() & !1 };
            bits.set(next);
        },
    );
    assert!(derived.get(), "the derived binding reads true");

    let mut app = test_ui().viewport(400, 160).theme(install).mount(move || {
        vstack((
            toggle("plain container", &plain),
            toggle("derived mapping", &derived),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(16.0)
        .padding_with(EdgeInsets::all(20.0))
    });
    let _ = app.capture_snapshot("inspector", "toggle-binding", "derived");
}

/// Isolates which modifier the Inspector's channel toggles lose their state to.
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn toggle_with_modifiers() {
    let a = Binding::bool(true);
    let b = Binding::bool(true);
    let c = Binding::bool(true);
    let never = Binding::bool(false);

    let mut app = test_ui().viewport(460, 220).theme(install).mount(move || {
        vstack((
            toggle("bare", &a),
            toggle("disabled-false", &b).disabled(never.clone()),
            toggle("label hidden", &c).install(LabelDisplayMode::Hidden),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(16.0)
        .padding_with(EdgeInsets::all(20.0))
    });
    let _ = app.capture_snapshot("inspector", "toggle-binding", "modifiers");
}

/// A chart above a metric grid: does the chart leave its siblings any height?
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn chart_above_a_metric_grid() {
    use waterui::chart::{DataPoint, LineChart};

    fn cell(label: &'static str, value: &'static str) -> impl View {
        vstack((text(value).headline(), text(label).caption()))
            .alignment(HorizontalAlignment::Leading)
            .spacing(2.0)
    }

    let series: Vec<DataPoint> = (0..40_u16)
        .map(|index| DataPoint::new(f32::from(index), f32::from(index % 7).mul_add(0.3, 2.0)))
        .collect();

    let mut app = test_ui().viewport(600, 500).theme(install).mount(move || {
        vstack((
            LineChart::new(series.clone()).line_width(1.5).height(160.0),
            Divider,
            vstack((
                hstack((cell("Kind", "reencode"), cell("Total", "3.39ms"))).spacing(32.0),
                hstack((cell("Layout", "1.13ms"), cell("Encode", "1.13ms"))).spacing(32.0),
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(16.0),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(16.0)
        .padding_with(EdgeInsets::all(20.0))
    });
    let _ = app.capture_snapshot("inspector", "chart-siblings", "grid");
}

/// Which containment actually stops the chart starving its siblings?
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn chart_containment_strategies() {
    use waterui::chart::{DataPoint, LineChart};

    fn cell(label: &'static str, value: &'static str) -> impl View {
        vstack((text(value).headline(), text(label).caption()))
            .alignment(HorizontalAlignment::Leading)
            .spacing(2.0)
    }

    fn grid() -> impl View {
        vstack((
            hstack((cell("Kind", "reencode"), cell("Total", "3.39ms"))).spacing(32.0),
            hstack((cell("Layout", "1.13ms"), cell("Encode", "1.13ms"))).spacing(32.0),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(12.0)
    }

    let series: Vec<DataPoint> = (0..40_u16)
        .map(|index| DataPoint::new(f32::from(index), f32::from(index % 7).mul_add(0.3, 2.0)))
        .collect();

    let mut app = test_ui().viewport(600, 520).theme(install).mount(move || {
        vstack((
            text("wrapped in a sized vstack").caption(),
            vstack((LineChart::new(series.clone()).line_width(1.5),)).height(120.0),
            grid(),
            Divider,
            text("wrapped in a sized zstack").caption(),
            zstack((LineChart::new(series.clone()).line_width(1.5),)).height(120.0),
            grid(),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(12.0)
        .padding_with(EdgeInsets::all(16.0))
    });
    let _ = app.capture_snapshot("inspector", "chart-siblings", "containment");
}

/// Does giving the starved rows an explicit height restore them?
#[core::prelude::v1::test]
#[ignore = "writes a visual acceptance PNG for direct image review"]
fn chart_with_fixed_height_rows() {
    use waterui::chart::{DataPoint, LineChart};

    fn cell(label: &'static str, value: &'static str) -> impl View {
        vstack((text(value).headline(), text(label).caption()))
            .alignment(HorizontalAlignment::Leading)
            .spacing(2.0)
            .height(44.0)
    }

    let series: Vec<DataPoint> = (0..40_u16)
        .map(|index| DataPoint::new(f32::from(index), f32::from(index % 7).mul_add(0.3, 2.0)))
        .collect();

    let mut app = test_ui().viewport(600, 420).theme(install).mount(move || {
        vstack((
            hstack((cell("Kind", "reencode"), cell("Total", "3.39ms"))).spacing(32.0),
            hstack((cell("Layout", "1.13ms"), cell("Encode", "1.13ms"))).spacing(32.0),
            Divider,
            LineChart::new(series.clone()).line_width(1.5),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(12.0)
        .padding_with(EdgeInsets::all(16.0))
    });
    let _ = app.capture_snapshot("inspector", "chart-siblings", "fixed-rows");
}
