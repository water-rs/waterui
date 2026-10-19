//! water-rs/hydrolysis#163 capture follow-up — a press that landed on a
//! gesture recognizer owns the pointer sequence until release (pointer
//! capture semantics), so an input-receiving surface the drag crosses must
//! see none of its moves. `handle_embedded_pointer_move` hit-tested the
//! pointer position and fed the `SceneView`/`GpuSurface` underneath every
//! mid-drag move anyway — hydrolysis's own capture slots
//! (`active_embedded_target`, `captures_drag` → `active_pointer_drag_target`,
//! `active_text_selection_drag`) simply were not consulted for surfaces that
//! did not take the press. The sequence is now routed exclusively.

use std::cell::RefCell;
use std::rc::Rc;

use waterui::gesture::{DragEvent, DragGesture, GesturePhase};
use waterui::{AnyView, Color, ViewExt as _};
use waterui_core::extract::Use;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::draw::Recorder;
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::{RecordingResources, SceneContent, SceneInvalidator, SceneView};
use waterui_layout::stack::hstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const POINTER_ID: u64 = 9;
const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;
const HANDLE_WIDTH: f32 = 20.0;

/// An input-receiving scene pane that records every event the embedded
/// input routing hands it — the `wants_input_events()` `SceneView` from the
/// issue's `[SceneView | 20pt drag handle | SceneView]` row.
struct RecorderPane {
    events: Rc<RefCell<Vec<String>>>,
}

impl SceneContent for RecorderPane {
    fn build_scene(
        &mut self,
        _recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        false
    }

    fn set_invalidator(&mut self, _invalidator: Option<SceneInvalidator>) {}

    fn rebuild_for_engine(&mut self) {}

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, event: &SurfaceInputEvent) {
        self.events.borrow_mut().push(format!("{event:?}"));
    }
}

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
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
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

fn drag_handle(phases: Rc<RefCell<Vec<GesturePhase>>>) -> AnyView {
    AnyView::new(Color::srgb_hex("#3F3F46").width(HANDLE_WIDTH).gesture(
        DragGesture::new(0.0),
        move |drag: Use<DragEvent>| {
            phases.borrow_mut().push(drag.phase);
        },
    ))
}

/// The issue's row with the capture semantics the question asks for: press
/// on the 20-point gesture handle at x≈390-410, then drag onto the trailing
/// pane at x>410. The handle's press owns the pointer sequence, so the pane
/// must see no move until the press is released; afterwards an unpressed
/// move hovers it again.
#[test]
fn a_started_drag_gesture_captures_the_pointer_sequence_over_adjacent_panes() {
    let phases = Rc::new(RefCell::new(Vec::new()));
    let leading_events = Rc::new(RefCell::new(Vec::new()));
    let trailing_events = Rc::new(RefCell::new(Vec::new()));
    let view = hstack((
        AnyView::new(SceneView::new(RecorderPane {
            events: Rc::clone(&leading_events),
        })),
        drag_handle(Rc::clone(&phases)),
        AnyView::new(SceneView::new(RecorderPane {
            events: Rc::clone(&trailing_events),
        })),
    ))
    .spacing(0.0);
    let mut runtime = runtime_with(AnyView::new(view));
    for _ in 0..4 {
        let _ = runtime.pump(false);
    }

    pointer_down(&mut runtime, 400.0, 300.0);
    pointer_move(&mut runtime, 430.0, 300.0);
    pointer_move(&mut runtime, 500.0, 300.0);
    pointer_up(&mut runtime, 500.0, 300.0);

    let seen = phases.borrow().clone();
    eprintln!("gesture phases: {seen:?}");
    eprintln!(
        "trailing pane events: {:?}",
        trailing_events.borrow().as_slice()
    );
    assert_eq!(
        seen.as_slice(),
        &[
            GesturePhase::Started,
            GesturePhase::Updated,
            GesturePhase::Updated,
            GesturePhase::Ended
        ],
        "the drag keeps its moves — they are the captured sequence's, not the pane's"
    );
    assert!(
        !trailing_events
            .borrow()
            .iter()
            .any(|event| event.contains("Move")),
        "a surface the captured sequence crosses must see none of its moves"
    );

    // Capture ends at release: the next unpressed move is hover again.
    pointer_move(&mut runtime, 500.0, 300.0);
    assert!(
        trailing_events
            .borrow()
            .iter()
            .any(|event| event.contains("Move")),
        "after release the pane under the pointer sees its hover moves again"
    );
}

/// The no-owner control: pressing a handle that carries no recognizer and no
/// drag capture leaves the sequence unowned, so the pane under the pointer
/// still receives its moves mid-press.
#[test]
fn a_plain_press_leaves_pane_moves_alone() {
    let trailing_events = Rc::new(RefCell::new(Vec::new()));
    let view = hstack((
        AnyView::new(Color::srgb_hex("#222226")),
        AnyView::new(Color::srgb_hex("#3F3F46").width(HANDLE_WIDTH)),
        AnyView::new(SceneView::new(RecorderPane {
            events: Rc::clone(&trailing_events),
        })),
    ))
    .spacing(0.0);
    let mut runtime = runtime_with(AnyView::new(view));
    for _ in 0..4 {
        let _ = runtime.pump(false);
    }

    pointer_down(&mut runtime, 400.0, 300.0);
    pointer_move(&mut runtime, 430.0, 300.0);
    pointer_move(&mut runtime, 500.0, 300.0);

    eprintln!(
        "trailing pane events: {:?}",
        trailing_events.borrow().as_slice()
    );
    assert!(
        trailing_events
            .borrow()
            .iter()
            .any(|event| event.contains("Move")),
        "an unowned sequence still delivers its moves to the surface underneath"
    );
    pointer_up(&mut runtime, 500.0, 300.0);
}
