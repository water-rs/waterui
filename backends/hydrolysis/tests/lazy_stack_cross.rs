//! A lazy `VStack::for_each` item must answer its proposed cross extent.
//!
//! water-rs/waterui#1232: a row inside a lazy stack measured under the M3
//! runtime answered 310.86 against a 308 proposal while the same row answered
//! 308 eagerly, so the lazy stack reported the wider figure as its cross
//! extent and every sibling section stretched with it. Each shape in the
//! issue's table is mounted both ways inside the reported shell —
//! `scroll(vstack(..).padding_with((12, 16)))` in a 340-wide window, which
//! proposes every row 308 — and a sibling probe row reads the answered cross
//! extent back from its trailing label.

use waterui::prelude::*;
use waterui_core::id::SelfId;
use waterui_testing::{OffscreenApp, Role, ui};

/// Logical points between the viewport edge and a row: the issue's
/// `padding_with((12, 16))` inside a 340-wide window.
const PROPOSED: f32 = 340.0 - 2.0 * 16.0;

/// The label the probe row pins to its trailing edge.
const PROBE_LABEL: &str = "probe-trailing";

/// A sibling section under the measured rows. The stack's reported cross
/// extent stretches this row, so its trailing label's trailing edge is the
/// answer the stack's content negotiated — `16 + answer` in window points.
fn probe_row() -> impl View {
    hstack((text("probe"), spacer(), text(PROBE_LABEL)))
}

/// The answered cross extent, read back from the probe row's trailing label:
/// its trailing edge minus the 16-point leading inset.
fn probe_width(app: &mut OffscreenApp) -> f32 {
    let probe = app.query().role(Role::LABEL).label(PROBE_LABEL).single();
    probe.bounds().x() + probe.bounds().width() - 16.0
}

/// The lazy stack's first measure runs on a transient item; once a frame has
/// materialized the visible rows the retained-item path answers the next
/// layout. A scroll through the scroller schedules that second pass exactly
/// as it does in the app.
fn settle_after_scrolling(app: &mut OffscreenApp) {
    app.scroll_at(170.0, 450.0, 0.0, -40.0, false);
    app.settle();
}

/// Mounts the row through `VStack::for_each` — the lazy path — as a section
/// of the issue's padded vstack, and returns the cross extent the stack
/// answered.
fn lazy_probe_width(row: impl Fn() -> AnyView + 'static + Clone) -> f32 {
    let mut app = ui()
        .viewport(340, 900)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(move || {
            scroll(
                vstack((
                    VStack::for_each((0..60).map(SelfId::new).collect::<Vec<_>>(), {
                        let row = row.clone();
                        move |_| row()
                    }),
                    probe_row(),
                ))
                .padding_with((12.0, 16.0)),
            )
        });
    let first = probe_width(&mut app);
    settle_after_scrolling(&mut app);
    let relaid = probe_width(&mut app);
    first.max(relaid)
}

/// Mounts the rows eagerly — the `vstack` path the issue's last table line
/// measured 308 with — in the same shell, and returns the answered extent.
fn eager_probe_width(row: impl Fn() -> AnyView + 'static + Clone) -> f32 {
    let mut app = ui()
        .viewport(340, 900)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(move || {
            scroll(
                vstack((
                    vstack((0..60).map(|_| row()).collect::<Vec<_>>()),
                    probe_row(),
                ))
                .padding_with((12.0, 16.0)),
            )
        });
    let first = probe_width(&mut app);
    settle_after_scrolling(&mut app);
    let relaid = probe_width(&mut app);
    first.max(relaid)
}

/// One table line from the issue: the same row measured both ways must
/// answer the 308 it was proposed.
fn assert_cross_extent(name: &str, row: impl Fn() -> AnyView + 'static + Clone) {
    let lazy = lazy_probe_width(row.clone());
    let eager = eager_probe_width(row);
    assert!(
        (eager - PROPOSED).abs() <= 0.5,
        "{name}: the eager path answered {eager}, expected {PROPOSED}"
    );
    assert!(
        (lazy - PROPOSED).abs() <= 0.5,
        "{name}: the lazy path answered {lazy}, expected {PROPOSED} \
         (eager answered {eager})"
    );
}

#[test]
fn single_line_row_answers_its_proposal() {
    assert_cross_extent("hstack(text, spacer, text)", || {
        AnyView::new(hstack((text("one line"), spacer(), text("x"))))
    });
}

#[test]
fn multi_line_text_row_answers_its_proposal() {
    assert_cross_extent("hstack(text(\"a\\nb\"), spacer, text)", || {
        AnyView::new(hstack((text("a\nb"), spacer(), text("x"))))
    });
}

#[test]
fn nested_vstack_row_answers_its_proposal() {
    assert_cross_extent("hstack(vstack(text, text), spacer, text)", || {
        AnyView::new(hstack((
            vstack((text("a"), text("b"))),
            spacer(),
            text("x"),
        )))
    });
}

#[test]
fn breakable_text_row_answers_its_proposal() {
    assert_cross_extent("hstack(text(\"a — b\"), spacer)", || {
        AnyView::new(hstack((text("a — b"), spacer())))
    });
}

#[test]
fn nested_vstack_row_without_trailing_answers_its_proposal() {
    assert_cross_extent("hstack(vstack(text, text), spacer)", || {
        AnyView::new(hstack((vstack((text("a"), text("b"))), spacer())))
    });
}
