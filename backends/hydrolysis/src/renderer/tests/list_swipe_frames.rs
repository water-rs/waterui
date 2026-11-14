//! A row's swipe or reorder drag moves live operands: the frames that track
//! the pointer and the spring-back after release record no view content and
//! run no layout.
//!
//! The row's displacement, its neighbours' slide and the swipe progress the
//! dismiss background reads are operands of the recorded layers, so only a
//! drag beginning — listing the dismiss background beneath the row, or
//! lifting the row over its neighbours — records the list again.

use core::time::Duration;
use std::cell::Cell;
use std::time::Instant;

use waterui::component::list::{List, ListDelete, ListItem, ListMove};
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;

use super::{MinimalTestTheme, test_environment};
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::{HeadlessPumpResult, HeadlessRuntime};

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const POINTER_ID: u64 = 7;
/// The frame interval the test pumps at: a 60 Hz display's.
const FRAME: Duration = Duration::from_millis(16);
/// The second row's centre: `plain_list_look` rows are 56 points tall.
const ROW_Y: f32 = 84.0;
/// The second row's reorder handle: `plain_list_look`'s 20-point move
/// control ends 8 points inside the row's trailing edge.
const HANDLE_X: f32 = 382.0;

fn list_runtime(editing: bool) -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(
            List::for_each([0, 1, 2, 3, 4, 5], |index: i32| {
                ListItem::new(text(format!("Row {index}")))
            })
            .editing(editing)
            .on_delete(|_: ListDelete| {})
            .on_move(|_: ListMove| {}),
        )
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// Pumps `runtime` one frame interval after the last.
struct Frames {
    at: Cell<Instant>,
}

impl Frames {
    fn start(runtime: &mut HeadlessRuntime) -> Self {
        let at = Instant::now();
        let _ = runtime.pump_at(false, at);
        let frames = Self { at: Cell::new(at) };
        let _ = frames.pump(runtime);
        frames
    }

    fn pump(&self, runtime: &mut HeadlessRuntime) -> HeadlessPumpResult {
        self.at.set(self.at.get() + FRAME);
        runtime.pump_at(false, self.at.get())
    }

    fn pointer(&self, runtime: &mut HeadlessRuntime, event: InputEvent) -> HeadlessPumpResult {
        runtime.push_input_event(event);
        self.pump(runtime)
    }
}

const fn down(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    }
}

const fn moved(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerMove {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
    }
}

const fn up(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    }
}

fn assert_records_list(result: &HeadlessPumpResult, frame: &str) {
    assert!(
        result
            .profile()
            .counters()
            .frame_work()
            .recorded_view_contents
            > 0,
        "{frame} changes what the list lists and must record it"
    );
}

fn assert_moves_operands_only(result: &HeadlessPumpResult, frame: &str) {
    let counters = result.profile().counters();
    assert!(counters.rendered(), "{frame} must present the moved rows");
    let work = counters.frame_work();
    assert_eq!(
        work.recorded_view_contents, 0,
        "{frame} recorded {} view contents; the drag moves live operands only",
        work.recorded_view_contents
    );
    assert_eq!(work.measure_calls, 0, "{frame} measured views");
    assert_eq!(work.layout_calls, 0, "{frame} laid nodes out");
    assert_eq!(work.semantic_builds, 0, "{frame} built views");
    assert_eq!(work.structural_patches, 0, "{frame} patched the tree");
    assert_eq!(counters.rebuild_iterations(), 0, "{frame} rebuilt the tree");
}

fn assert_records_nothing(result: &HeadlessPumpResult, frame: &str) {
    let work = result.profile().counters().frame_work();
    assert_eq!(
        (
            work.recorded_view_contents,
            work.measure_calls,
            work.layout_calls
        ),
        (0, 0, 0),
        "{frame} recorded, measured or laid out the list"
    );
}

#[test]
fn a_swipe_and_its_spring_back_record_no_rows() {
    let mut runtime = list_runtime(false);
    let frames = Frames::start(&mut runtime);
    let mut x = 120.0;
    let _ = frames.pointer(&mut runtime, down(x, ROW_Y));

    // The first move past the drag slop begins the swipe, listing the row's
    // dismiss background beneath it.
    x += 20.0;
    let began = frames.pointer(&mut runtime, moved(x, ROW_Y));
    assert_records_list(&began, "the swipe's first frame");

    // Every later tracking frame only moves the row's layer and the swipe
    // progress, short of the dismiss threshold (half the row's width).
    for step in 0..6 {
        x += 10.0;
        let tracked = frames.pointer(&mut runtime, moved(x, ROW_Y));
        assert_moves_operands_only(&tracked, &format!("swipe tracking frame {step}"));
    }

    // Released short of the threshold, the row springs back under the
    // look's settle motion, which the engine samples.
    let released = frames.pointer(&mut runtime, up(x, ROW_Y));
    assert_records_nothing(&released, "the swipe's release frame");
    for step in 0..4 {
        let settling = frames.pump(&mut runtime);
        assert_records_nothing(&settling, &format!("swipe spring-back frame {step}"));
    }
}

#[test]
fn a_reorder_drag_records_only_its_lift_and_drop() {
    let mut runtime = list_runtime(true);
    let frames = Frames::start(&mut runtime);
    let mut y = ROW_Y;
    let _ = frames.pointer(&mut runtime, down(HANDLE_X, y));

    // The first move past the drag slop lifts the row over its neighbours.
    y += 12.0;
    let lifted = frames.pointer(&mut runtime, moved(HANDLE_X, y));
    assert_records_list(&lifted, "the lift frame");

    // Following the pointer across two slots moves the lifted row and
    // slides the rows it passes, all live.
    for step in 0..10 {
        y += 10.0;
        let tracked = frames.pointer(&mut runtime, moved(HANDLE_X, y));
        assert_moves_operands_only(&tracked, &format!("reorder tracking frame {step}"));
    }

    // Dropping the row lowers it back into the list.
    let dropped = frames.pointer(&mut runtime, up(HANDLE_X, y));
    assert_records_list(&dropped, "the drop frame");
}
