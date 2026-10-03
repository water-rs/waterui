//! A `.gesture` recognizer engaged by a pointer-down must survive the
//! signal-driven repaint between it and the next move
//! (water-rs/hydrolysis#138): every scene flush re-registers every target from
//! scratch under `clear_targets`, so a recognizer that is not retained across
//! that re-registration is rebuilt mid-sequence and the rest of the drag dies
//! silently.

use std::cell::RefCell;
use std::rc::Rc;

use waterui::component::text::Text;
use waterui::gesture::{DragEvent, DragGesture, GesturePhase};
use waterui::{Binding, Str, ViewExt as _};
use waterui_core::AnyView;
use waterui_core::extract::Use;
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::stack::zstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const POINTER_ID: u64 = 9;
const WINDOW: u32 = 160;

fn runtime_with(view: AnyView) -> HeadlessRuntime {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW,
        WINDOW,
        MinimalTestTheme::default(),
    )
}

fn pointer_down(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump(false);
}

fn pointer_move(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerMove {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
    });
    let _ = runtime.pump(false);
}

fn pointer_up(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump(false);
}

/// `PointerDown`, then a `Binding` write that forces a repaint before every
/// `PointerMove`, then `PointerUp` — the sequence issue #138 replays. The drag
/// phases must still arrive in order: the recognizer that saw the down event is
/// the one that must keep seeing the moves.
#[test]
fn gesture_recognizer_survives_repaint_mid_drag() {
    let repaint = Binding::container(Str::from_static("frame 0"));
    let phases = Rc::new(RefCell::new(Vec::new()));
    let view = {
        let phases = Rc::clone(&phases);
        zstack((
            ().size(WINDOW as f32, WINDOW as f32),
            Text::computed(repaint.clone()),
        ))
        .gesture(DragGesture::new(0.0), move |drag: Use<DragEvent>| {
            phases.borrow_mut().push(drag.phase);
        })
    };
    let mut runtime = runtime_with(AnyView::new(view));
    for _ in 0..4 {
        let _ = runtime.pump(false);
    }

    pointer_down(&mut runtime, 80.0, 80.0);
    let mut frame = 0u32;
    for point in [(100.0f32, 80.0f32), (120.0f32, 80.0f32)] {
        frame += 1;
        repaint.set(Str::from(format!("frame {frame}")));
        // The signal-driven repaint: clear_targets, layout, re-registration,
        // and the post-layout interaction sync — the whole refresh pass.
        let _ = runtime.pump(false);
        pointer_move(&mut runtime, point.0, point.1);
    }
    pointer_up(&mut runtime, 120.0, 80.0);

    // `Started` on the first move, one `Updated` per further position — the
    // release itself delivers a final synthesized move before the up — and a
    // single `Ended`, in order.
    assert_eq!(
        phases.borrow().as_slice(),
        &[
            GesturePhase::Started,
            GesturePhase::Updated,
            GesturePhase::Updated,
            GesturePhase::Ended
        ],
        "an in-flight drag must survive the repaint between its pointer events"
    );
}
