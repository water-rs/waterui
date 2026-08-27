//! Input delivery to a `GpuSurface` whose view handles its own input.
//!
//! These drive the real runner path — `push_input_event` →
//! `handle_input_events` → hit-test arbitration → sink — so what they observe
//! is what a browser engine or terminal embedded in a window would observe.
//! The one link they cannot reach is the winit translation itself: a
//! `winit::event::KeyEvent` cannot be constructed outside winit (its
//! `platform_specific` field is private), so the events injected here are the
//! platform-neutral `InputEvent`s the winit layer produces, and the
//! winit-specific quirks that layer depends on are pinned separately below.

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use waterui::ViewExt as _;
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::input::{
    Code, Key, Modifiers as W3cModifiers, NamedKey, ScrollUnit, SurfaceInputEvent,
    SurfacePointerButton,
};
use waterui_graphics::{GpuContext, GpuFrame, GpuSurface, GpuView};
use waterui_layout::stack::vstack;

use super::test_environment;
use crate::HeadlessRuntime;
use crate::platform::{
    InputEvent, KeyCode, KeyState, Modifiers, PointerButton, PointerKind, TouchPhase,
};

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const HEADER_HEIGHT: f32 = 100.0;
const SURFACE_WIDTH: f32 = 200.0;
const SURFACE_HEIGHT: f32 = 150.0;

/// Where the surface lands, in window coordinates: the column is anchored at
/// the window's top, so the 200-wide surface is centred across the 400-wide
/// window (`(400 - 200) / 2`) and sits under the full-width 100-high header
/// plus the stack's 10-point default spacing.
const SURFACE_ORIGIN_X: f64 = 100.0;
const SURFACE_ORIGIN_Y: f64 = 110.0;

const POINTER_ID: u64 = 7;

/// Records every event its surface receives, so a test can read them after the
/// frame that delivered them.
#[derive(Clone, Default)]
struct ProbeLog(Rc<RefCell<Vec<SurfaceInputEvent>>>);

impl ProbeLog {
    fn drain(&self) -> Vec<SurfaceInputEvent> {
        core::mem::take(&mut *self.0.borrow_mut())
    }
}

struct InputProbe {
    log: ProbeLog,
    caret: Option<vello::kurbo::Rect>,
}

impl GpuView for InputProbe {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {}

    fn render(&mut self, _frame: &mut GpuFrame) {}

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, event: &SurfaceInputEvent) {
        self.log.0.borrow_mut().push(event.clone());
    }

    fn ime_caret(&self) -> Option<vello::kurbo::Rect> {
        self.caret
    }
}

/// A GPU view that draws only: it must never be handed an input event, and
/// must never take focus away from the widgets around it.
struct SilentProbe {
    log: ProbeLog,
}

impl GpuView for SilentProbe {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {}

    fn render(&mut self, _frame: &mut GpuFrame) {}

    fn input(&mut self, event: &SurfaceInputEvent) {
        self.log.0.borrow_mut().push(event.clone());
    }
}

fn runtime_with(surface: GpuSurface) -> HeadlessRuntime {
    let surface = RefCell::new(Some(surface));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let surface = surface
            .borrow_mut()
            .take()
            .expect("the probe view is built once");
        AnyView::new(vstack((
            vstack((text("header"),)).size(WINDOW_WIDTH as f32, HEADER_HEIGHT),
            surface.size(SURFACE_WIDTH, SURFACE_HEIGHT),
        )))
    });
    HeadlessRuntime::new_for_tests(test_environment(), builder, WINDOW_WIDTH, WINDOW_HEIGHT)
}

/// Pumps until the surface has finished its async setup, so the events a test
/// injects have a view to reach.
fn settled(runtime: &mut HeadlessRuntime, start: Instant) {
    for frame in 0..4 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
    }
}

/// A window point inside the surface, given surface-local coordinates.
fn window_point(local_x: f64, local_y: f64) -> (f32, f32) {
    (
        (SURFACE_ORIGIN_X + local_x) as f32,
        (SURFACE_ORIGIN_Y + local_y) as f32,
    )
}

fn press_at(runtime: &mut HeadlessRuntime, local_x: f64, local_y: f64) {
    let (x, y) = window_point(local_x, local_y);
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
}

fn key_event(character: &str, code: Code, state: KeyState) -> InputEvent {
    InputEvent::Key {
        key: KeyCode::Character(character.to_owned()),
        native: None,
        logical_key: Key::Character(character.to_owned()),
        physical_code: code,
        repeat: false,
        state,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn pointer_events_arrive_in_logical_surface_local_coordinates() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log.drain();

    let (x, y) = window_point(30.0, 20.0);
    runtime.push_input_event(InputEvent::PointerMove {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));

    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::PointerMove {
            position: vello::kurbo::Point::new(30.0, 20.0),
        }],
        "a pointer over the surface must arrive with the surface's own origin \
         subtracted, in logical units"
    );

    // Outside the surface, above it in the header, the view sees nothing.
    runtime.push_input_event(InputEvent::PointerMove {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x: 10.0,
        y: 10.0,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(120));
    assert_eq!(
        log.drain(),
        Vec::new(),
        "a pointer outside the surface is not the surface's input"
    );
}

#[test]
fn a_press_focuses_the_surface_and_later_frames_keep_that_focus() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log.drain();

    press_at(&mut runtime, 12.0, 34.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Focus(true),
            SurfaceInputEvent::PointerMove {
                position: vello::kurbo::Point::new(12.0, 34.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: true,
                button: SurfacePointerButton::Primary,
                position: vello::kurbo::Point::new(12.0, 34.0),
            },
        ],
    );

    let (x, y) = window_point(12.0, 34.0);
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(116));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::PointerMove {
                position: vello::kurbo::Point::new(12.0, 34.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: false,
                button: SurfacePointerButton::Primary,
                position: vello::kurbo::Point::new(12.0, 34.0),
            },
        ],
    );

    // Several frames later — the targets have been re-emitted from scratch
    // every one of them — the keyboard still reaches the same surface. This is
    // the regression the sink's stable identity exists for: comparing the
    // per-frame sink allocations instead retires focus on the next frame and
    // silently swallows every keystroke.
    for frame in 8..12 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
    }
    let _ = log.drain();

    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Pressed));
    runtime.push_input_event(InputEvent::TextInput {
        text: "a".to_owned(),
    });
    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Released));
    let _ = runtime.pump_at(false, start + Duration::from_millis(300));

    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Key {
                pressed: true,
                key: Key::Character("a".to_owned()),
                code: Code::KeyA,
                modifiers: W3cModifiers::empty(),
                repeat: false,
            },
            SurfaceInputEvent::TextInput("a".into()),
            SurfaceInputEvent::Key {
                pressed: false,
                key: Key::Character("a".to_owned()),
                code: Code::KeyA,
                modifiers: W3cModifiers::empty(),
                repeat: false,
            },
        ],
    );
}

#[test]
fn modifiers_reach_the_focused_surface_with_its_keys() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);
    press_at(&mut runtime, 5.0, 5.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    let _ = log.drain();

    let modifiers = Modifiers {
        shift: false,
        control: true,
        alt: false,
        super_key: false,
    };
    runtime.push_input_event(InputEvent::ModifiersChanged(modifiers));
    runtime.push_input_event(InputEvent::Key {
        key: KeyCode::Named("ArrowLeft".to_owned()),
        native: None,
        logical_key: Key::Named(NamedKey::ArrowLeft),
        physical_code: Code::ArrowLeft,
        repeat: true,
        state: KeyState::Pressed,
        modifiers,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(120));

    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Modifiers(W3cModifiers::CONTROL),
            SurfaceInputEvent::Key {
                pressed: true,
                key: Key::Named(NamedKey::ArrowLeft),
                code: Code::ArrowLeft,
                modifiers: W3cModifiers::CONTROL,
                repeat: true,
            },
        ],
    );
}

#[test]
fn scrolls_carry_their_unit_and_the_end_of_the_gesture() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log.drain();

    let (x, y) = window_point(60.0, 40.0);
    runtime.push_input_event(InputEvent::Scroll {
        x,
        y,
        dx: 0.0,
        dy: -3.0,
        is_line_delta: true,
    });
    runtime.push_input_event(InputEvent::TrackpadPan {
        x,
        y,
        dx: 1.0,
        dy: -12.0,
        phase: TouchPhase::Moved,
    });
    runtime.push_input_event(InputEvent::TrackpadPan {
        x,
        y,
        dx: 0.0,
        dy: 0.0,
        phase: TouchPhase::Ended,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));

    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Scroll {
                position: vello::kurbo::Point::new(60.0, 40.0),
                delta_x: 0.0,
                delta_y: -3.0,
                unit: ScrollUnit::Line,
                finished: true,
            },
            SurfaceInputEvent::Scroll {
                position: vello::kurbo::Point::new(60.0, 40.0),
                delta_x: 1.0,
                delta_y: -12.0,
                unit: ScrollUnit::Pixel,
                finished: false,
            },
            SurfaceInputEvent::Scroll {
                position: vello::kurbo::Point::new(60.0, 40.0),
                delta_x: 0.0,
                delta_y: 0.0,
                unit: ScrollUnit::Pixel,
                finished: true,
            },
        ],
        "a wheel notch is a complete line-unit gesture; a trackpad glide is \
         pixel-unit and only its last event finishes"
    );
}

#[test]
fn composition_reaches_the_surface_as_a_session() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);
    press_at(&mut runtime, 5.0, 5.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    let _ = log.drain();

    runtime.push_input_event(InputEvent::ImePreedit {
        text: "に".to_owned(),
        caret: Some(3),
    });
    runtime.push_input_event(InputEvent::ImePreedit {
        text: "にほ".to_owned(),
        caret: Some(6),
    });
    runtime.push_input_event(InputEvent::ImeCommit {
        text: "日本".to_owned(),
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(120));

    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::CompositionStart,
            SurfaceInputEvent::CompositionUpdate {
                text: "に".into(),
                caret: Some(3),
            },
            SurfaceInputEvent::CompositionUpdate {
                text: "にほ".into(),
                caret: Some(6),
            },
            SurfaceInputEvent::CompositionCommit("日本".into()),
        ],
    );

    // An empty pre-edit is the platform abandoning the session.
    runtime.push_input_event(InputEvent::ImePreedit {
        text: "ま".to_owned(),
        caret: None,
    });
    runtime.push_input_event(InputEvent::ImePreedit {
        text: String::new(),
        caret: None,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(140));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::CompositionStart,
            SurfaceInputEvent::CompositionUpdate {
                text: "ま".into(),
                caret: None,
            },
            SurfaceInputEvent::CompositionCancel,
        ],
    );
}

#[test]
fn the_focused_surface_places_the_input_method_panel() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: Some(vello::kurbo::Rect::new(10.0, 20.0, 12.0, 38.0)),
    }));
    let start = Instant::now();
    settled(&mut runtime, start);

    assert!(
        runtime.focused_text_input_state().is_none(),
        "an unfocused surface has no caret to place a panel against"
    );

    press_at(&mut runtime, 5.0, 5.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));

    let state = runtime
        .focused_text_input_state()
        .expect("a focused surface publishes its caret");
    assert!(
        (state.x - (SURFACE_ORIGIN_X + 10.0)).abs() < 0.01
            && (state.y - (SURFACE_ORIGIN_Y + 20.0)).abs() < 0.01,
        "the surface reports its caret in its own coordinates and the backend \
         projects it into the window (got {}, {})",
        state.x,
        state.y
    );
    assert!((state.width - 2.0).abs() < 0.01 && (state.height - 18.0).abs() < 0.01);
}

#[test]
fn a_view_that_does_not_want_input_receives_none() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(SilentProbe { log: log.clone() }));
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log.drain();

    press_at(&mut runtime, 20.0, 20.0);
    let (x, y) = window_point(20.0, 20.0);
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Pressed));
    runtime.push_input_event(InputEvent::Scroll {
        x,
        y,
        dx: 0.0,
        dy: -3.0,
        is_line_delta: true,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));

    assert_eq!(
        log.drain(),
        Vec::new(),
        "a GPU view that only draws must not be handed input, and must not \
         claim the keyboard from the widgets around it"
    );
    assert!(runtime.focused_text_input_state().is_none());
}

/// The winit translation this backend delegates to `ui-events-winit` is not
/// reachable from a headless test, so the two quirks the mapping depends on
/// are pinned here directly: get either wrong and space stops activating
/// buttons, or the platform modifier arrives as the wrong key.
#[cfg(feature = "winit")]
#[test]
fn the_winit_translation_follows_the_w3c_vocabulary() {
    use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey, PhysicalKey};

    assert_eq!(
        ui_events_winit::keyboard::from_winit_key(WinitKey::Named(WinitNamedKey::Space)),
        Key::Character(" ".to_owned()),
        "the W3C vocabulary has no named Space: it is the character it types"
    );
    assert_eq!(
        ui_events_winit::keyboard::from_winit_key(WinitKey::Named(WinitNamedKey::Super)),
        Key::Named(NamedKey::Meta),
        "winit's Super is the W3C Meta key"
    );
    assert_eq!(
        ui_events_winit::keyboard::from_winit_code(PhysicalKey::Code(
            winit::keyboard::KeyCode::KeyA
        )),
        Code::KeyA
    );
}
