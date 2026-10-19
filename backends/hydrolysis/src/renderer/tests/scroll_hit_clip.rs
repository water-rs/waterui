//! water-rs/hydrolysis#252 — a row straddling the scroll viewport's edge
//! keeps only the painted part of its hit bounds.
//!
//! Paint is clipped to the viewport (the scroll node's clipped inner layer
//! carries the content flush) but hit regions were registered from the
//! unclipped rects, so a row scrolled half under the sibling chrome above
//! still took taps up there. The fix clips every hit region — gestures,
//! pointer targets, row hit bounds — to the clip stack the paint layers
//! push.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use waterui::component::text;
use waterui::{AnyView, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{VStack, vstack};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const POINTER_ID: u64 = 7;
const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 700;
/// The sibling chrome band above the scroll view's viewport.
const BAND_HEIGHT: f32 = 100.0;
/// Rows are tall enough that scrolling 60pt leaves row 0 straddling the
/// viewport's top edge.
const ROW_HEIGHT: f32 = 80.0;
const ROWS: usize = 20;

fn runtime(taps: Rc<RefCell<Vec<usize>>>) -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        let taps = taps.clone();
        AnyView::new(vstack((
            vstack((text("chrome"),)).size(crate::num_cast::u32_as_f32(WINDOW_WIDTH), BAND_HEIGHT),
            scroll(VStack::for_each(rows, move |row| {
                let index = row.into_inner();
                let taps = taps.clone();
                vstack((text(format!("Row {index}")),))
                    .size(f32::INFINITY, ROW_HEIGHT)
                    .on_tap(move || taps.borrow_mut().push(index))
            })),
        )))
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant, min_frames: u32) {
    let mut frame = 0;
    loop {
        frame += 1;
        *at += core::time::Duration::from_millis(16);
        let _ = runtime.pump_at(false, *at);
        if frame >= min_frames
            && (frame >= 300 || (runtime.is_settled() && !runtime.has_pending_semantic_update()))
        {
            break;
        }
    }
}

fn tap(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
}

/// Scrolled 60pt in, row 0's painted sliver sits just below the viewport's
/// top edge — but its unclipped hit bounds reach 60pt up into the chrome
/// band. A tap inside the band on the overhang must not fire it; a tap on
/// the sliver still must.
#[test]
fn a_row_straddling_the_viewport_edge_cannot_be_tapped_above_the_clip() {
    let taps: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = runtime(taps.clone());
    let mut at = Instant::now();
    settle(&mut runtime, &mut at, 4);

    // Scroll 60pt: row 0 now paints from the band's bottom edge to 20pt into
    // the viewport.
    runtime.push_input_event(InputEvent::Scroll {
        x: 200.0,
        y: BAND_HEIGHT + 100.0,
        dx: 0.0,
        dy: -60.0,
        is_line_delta: false,
    });
    settle(&mut runtime, &mut at, 4);
    let metrics = runtime
        .renderer()
        .scroll_metrics_at(200.0, BAND_HEIGHT + 100.0)
        .expect("the scroll view registers a scroll target");
    assert!(
        approx::relative_eq!(metrics.offset_y, 60.0),
        "the scroll offset must reach 60pt before the taps: {metrics:?}: left {:?}, right {:?}",
        metrics.offset_y,
        60.0
    );

    taps.borrow_mut().clear();
    // Inside the chrome band, above the viewport — on the part of row 0's
    // unclipped hit bounds that hangs over the clip edge.
    tap(&mut runtime, 200.0, 60.0);
    settle(&mut runtime, &mut at, 2);
    assert!(
        taps.borrow().is_empty(),
        "a tap above the viewport must not reach the straddling row: {:?}",
        taps.borrow()
    );

    // The painted sliver of row 0 — window y = 100 + (80 - 60) - 5 — still
    // takes its tap.
    tap(&mut runtime, 200.0, BAND_HEIGHT + 15.0);
    settle(&mut runtime, &mut at, 2);
    assert_eq!(
        *taps.borrow(),
        vec![0],
        "the painted sliver of row 0 must still receive its tap"
    );
}
