//! `ViewExt::report_size` (water-rs/waterui#4298): the backend writes a view's
//! committed placed size into the bound `Binding<Size>` after layout placement
//! commits, skips writes on unchanged frames, and drives adaptive layout on
//! subsequent frames without triggering a layout cycle.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use support::{mount_rendered, node_label_exists};
use waterui::graphics::color::Srgb;
use waterui::layout::Size;
use waterui::prelude::*;
use waterui_core::Environment;

fn counting_size() -> (Binding<Size>, Rc<Cell<usize>>) {
    let writes = Rc::new(Cell::new(0usize));
    let counted = binding(Size::zero()).filter({
        let writes = Rc::clone(&writes);
        move |_| {
            writes.set(writes.get() + 1);
            true
        }
    });
    (counted, writes)
}

#[test]
fn report_size_reports_initial_placed_size_and_stays_silent_when_unchanged() {
    let (size, writes) = counting_size();
    let mut app = mount_rendered(
        Environment::new(),
        {
            let size = size.clone();
            move || {
                Color::new(Srgb::new(0.2, 0.4, 0.7))
                    .size(120.0, 80.0)
                    .report_size(&size)
            }
        },
        400,
        600,
    );
    app.settle();

    assert_eq!(
        size.snapshot(),
        Size::new(120.0, 80.0),
        "initial layout must report the view's committed placed size, not the window allocation"
    );
    assert_eq!(
        writes.get(),
        1,
        "initial placement must write the size binding exactly once"
    );

    for _ in 0..5 {
        app.pump();
    }
    assert_eq!(
        writes.get(),
        1,
        "unchanged frames must not re-write or re-notify size watchers"
    );
}

#[test]
fn report_size_tracks_resize_and_drives_adaptive_when_without_cycle() {
    let pane_width = binding(420.0_f32);
    let (reported, writes) = counting_size();

    let mut app = mount_rendered(
        Environment::new(),
        {
            let pane_width = pane_width.clone();
            let reported = reported.clone();
            move || {
                when(reported.map(|s| s.width > 600.0), || text("two-column"))
                    .otherwise(|| text("one-column"))
                    .max_width(f32::INFINITY)
                    .report_size(&reported)
                    .width(pane_width.clone())
            }
        },
        800,
        600,
    );
    app.settle();

    assert!(
        (reported.snapshot().width - 420.0).abs() < 0.5,
        "initial pane width must be reported as 420, got {:?}",
        reported.snapshot()
    );
    assert!(
        node_label_exists(app.tree(), "one-column"),
        "narrow width must keep the one-column branch"
    );
    assert!(
        !node_label_exists(app.tree(), "two-column"),
        "narrow width must not mount the two-column branch"
    );
    let writes_after_initial = writes.get();

    // Steady frames must stay silent and settled.
    for _ in 0..4 {
        app.pump();
    }
    assert_eq!(
        writes.get(),
        writes_after_initial,
        "unchanged adaptive layout must not re-write the size binding"
    );

    // Widen past the 600 pt breakpoint: the next frame adapts to two-column.
    pane_width.set(720.0);
    app.settle();
    assert!(
        (reported.snapshot().width - 720.0).abs() < 0.5,
        "resized pane width must be reported as 720, got {:?}",
        reported.snapshot()
    );
    assert!(
        node_label_exists(app.tree(), "two-column"),
        "widening past 600 pt must adapt to the two-column branch"
    );
    assert!(
        !node_label_exists(app.tree(), "one-column"),
        "widening past 600 pt must replace the one-column branch"
    );
    let writes_after_widen = writes.get();
    for _ in 0..4 {
        app.pump();
    }
    assert_eq!(
        writes.get(),
        writes_after_widen,
        "settled adapted layout must not cycle or re-write the size binding"
    );

    // Shrink back below the 600 pt breakpoint: adapts back to one-column.
    pane_width.set(500.0);
    app.settle();
    assert!(
        (reported.snapshot().width - 500.0).abs() < 0.5,
        "shrunk pane width must be reported as 500, got {:?}",
        reported.snapshot()
    );
    assert!(
        node_label_exists(app.tree(), "one-column"),
        "shrinking below 600 pt must adapt back to the one-column branch"
    );
    assert!(
        !node_label_exists(app.tree(), "two-column"),
        "shrinking below 600 pt must remove the two-column branch"
    );
}
