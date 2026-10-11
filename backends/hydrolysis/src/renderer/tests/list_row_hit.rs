//! Row hit-region regression test for water-rs/hydrolysis#208.
//!
//! In a virtualized `List`, a tap inside a row must land on the region
//! registered where that content painted — not on the shifted geometry of a
//! refresh the user has not seen yet.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use nami::Binding;
use nami::SignalExt as _;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::{AnyView, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const POINTER_ID: u64 = 7;
const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 700;
/// Enough rows that the window virtualizes: 40 × 56pt one-line rows >> 700pt.
const ROWS: usize = 40;

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
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

/// A tap that arrives while a reactive refresh is still pending must hit the
/// row where the user last saw it painted, not the position the pending flush
/// is about to move it to. A row above the tap target grows between the
/// presented frame and the input drain, so an un-presented flush pushes the
/// target's registered region down while the pointer still lands where the
/// paint shows the target.
#[test]
fn tap_inside_virtualized_row_during_pending_refresh() {
    // The row that grows mid-pump, well above the tapped one.
    const GROWN: u64 = 3;
    // The row the pointer taps: inside the first viewport at rest.
    const TAPPED: u64 = 11;
    let grow = Binding::bool(false);
    let taps: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let taps2 = taps.clone();
    let builder = {
        let grow = grow.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            let grow = grow.clone();
            let taps = taps2.clone();
            AnyView::new(List::for_each(SignalCollection::new(rows), move |row| {
                let index = row.into_inner() as u64;
                let taps = taps.clone();
                let size = grow
                    .clone()
                    .map(move |grow| if grow && index == GROWN { 60.0 } else { 20.0 });
                ListItem::new(
                    text(format!("Row {index}"))
                        .size(size)
                        .on_tap(move || taps.borrow_mut().push(index)),
                )
            }))
        })
    };
    let mut runtime = runtime(builder);
    let mut at = Instant::now();
    settle(&mut runtime, &mut at, 4);

    // The painted centre of row 11's tap target: 56pt rows put its slot at
    // y=616..672 and the text's gesture region is centred inside the slot.
    let tap_x = 200.0;
    let tap_y = 616.0 + 28.0;

    // Grow the row *after* the presented frame so the refresh is pending when
    // the tap drains — the same shape as content arriving during launch.
    grow.set(true);
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x: tap_x,
        y: tap_y,
        button: PointerButton::Primary,
    });
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x: tap_x,
        y: tap_y,
        button: PointerButton::Primary,
    });
    at += core::time::Duration::from_millis(16);
    let _ = runtime.pump_at(false, at);

    assert_eq!(
        taps.borrow().as_slice(),
        &[TAPPED],
        "a tap at the painted centre of row {TAPPED}'s target must fire it, \
         not the shifted geometry of a refresh the user never saw",
    );
}
