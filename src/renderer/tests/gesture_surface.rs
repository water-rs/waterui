//! water-rs/hydrolysis#163 — an in-flight `.gesture(DragGesture)` died as soon
//! as the pointer moved over an adjacent input-receiving embedded surface.
//! `handle_pointer_move_inner` handed the move to
//! `handle_embedded_pointer_move` before the gesture engine ever saw it and
//! early-returned over the embedded target, so a drag that started on the
//! 20-point handle between two `SceneView` panes fired nothing while every
//! move reached the neighbouring scene as `PointerMove`. The press path
//! already gives the gesture engine first look; moves now do too.

use std::cell::RefCell;
use std::rc::Rc;

use waterui::gesture::{DragEvent, DragGesture, GesturePhase};
use waterui::{AnyView, Color, ViewExt as _};
use waterui_core::extract::Use;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::cherenkov::Recorder;
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

/// The issue's sequence verbatim: press on the 20-point gesture handle at
/// x≈390-410, then moves onto the trailing scene pane at x>410. Before the
/// fix the recognizer saw none of those moves — `handle_embedded_pointer_move`
/// claimed each one and the drag failed silently on pointer-up.
#[test]
fn drag_on_handle_survives_moves_over_adjacent_scene_pane() {
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
        "an in-flight drag must keep receiving moves over an embedded surface"
    );
    // First look is also exclusive ownership: the handle's press captured
    // the pointer sequence, so the surface it crosses sees none of its moves
    // until release — then an unpressed move hovers it again.
    assert!(
        !trailing_events
            .borrow()
            .iter()
            .any(|event| event.starts_with("PointerMove")),
        "the scene under the pointer must see none of the captured sequence's moves"
    );
    pointer_move(&mut runtime, 500.0, 300.0);
    assert!(
        trailing_events
            .borrow()
            .iter()
            .any(|event| event.starts_with("PointerMove")),
        "after release the scene under the pointer sees its hover moves again"
    );
}

/// The control the issue names: the same handle between plain `Color` panes
/// dragged correctly even before the fix — nothing sits there to eat the
/// moves. Kept here so a future regression that breaks gesture moves
/// outright is told apart from embedded-surface starvation.
#[test]
fn drag_on_handle_between_plain_panes_is_the_control() {
    let phases = Rc::new(RefCell::new(Vec::new()));
    let view = hstack((
        AnyView::new(Color::srgb_hex("#222226")),
        drag_handle(Rc::clone(&phases)),
        AnyView::new(Color::srgb_hex("#222226")),
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

    assert_eq!(
        phases.borrow().as_slice(),
        &[
            GesturePhase::Started,
            GesturePhase::Updated,
            GesturePhase::Updated,
            GesturePhase::Ended
        ],
        "a drag between plain panes fires its full phase sequence"
    );
}
