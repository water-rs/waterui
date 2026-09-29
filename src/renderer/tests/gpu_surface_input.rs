//! Input delivery to a `GpuSurface` whose view handles its own input, and to a
//! `SceneView` whose content does.
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

use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::component::text;
use waterui_controls::button::button;
use waterui_core::AnyView;
use waterui_core::Binding;
use waterui_core::View;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::input::{
    Code, Key, Modifiers as W3cModifiers, NamedKey, ScrollUnit, SurfaceInputEvent,
    SurfacePointerButton,
};
use waterui_graphics::{
    GpuContext, GpuFrame, GpuSurface, GpuView, Scene2D, SceneContent, SceneInvalidator, SceneView,
};
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{vstack, zstack};

use super::{MinimalTestTheme, test_environment};
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
    caret: Option<kurbo::Rect>,
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

    fn ime_caret(&self) -> Option<kurbo::Rect> {
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

fn runtime_with(surface: impl View) -> HeadlessRuntime {
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
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
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
        logical_key: Key::Character(character.to_owned()),
        physical_code: code,
        repeat: false,
        state,
        modifiers: Modifiers::default(),
    }
}

/// A Tab press+release — the pair the platform layer produces for one
/// keypress. `shift` carries the reverse-traversal modifier.
fn tab(runtime: &mut HeadlessRuntime, shift: bool) {
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(InputEvent::Key {
            key: KeyCode::Named("Tab".to_owned()),
            logical_key: Key::Named(NamedKey::Tab),
            physical_code: Code::Tab,
            repeat: false,
            state,
            modifiers: Modifiers {
                shift,
                ..Modifiers::default()
            },
        });
    }
}

/// A Ctrl+Tab press+release — the traversal chord a focused surface does
/// not consume. `shift` carries the reverse direction.
fn ctrl_tab(runtime: &mut HeadlessRuntime, shift: bool) {
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(InputEvent::Key {
            key: KeyCode::Named("Tab".to_owned()),
            logical_key: Key::Named(NamedKey::Tab),
            physical_code: Code::Tab,
            repeat: false,
            state,
            modifiers: Modifiers {
                shift,
                control: true,
                ..Modifiers::default()
            },
        });
    }
}

/// The pane a `.focused` binding names — one variant per surface, matching
/// how a form names its fields.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pane {
    Document,
    Canvas,
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
            position: kurbo::Point::new(30.0, 20.0),
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
                position: kurbo::Point::new(12.0, 34.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: true,
                button: SurfacePointerButton::Primary,
                position: kurbo::Point::new(12.0, 34.0),
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
                position: kurbo::Point::new(12.0, 34.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: false,
                button: SurfacePointerButton::Primary,
                position: kurbo::Point::new(12.0, 34.0),
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
                position: kurbo::Point::new(60.0, 40.0),
                delta_x: 0.0,
                delta_y: -3.0,
                unit: ScrollUnit::Line,
                finished: true,
            },
            SurfaceInputEvent::Scroll {
                position: kurbo::Point::new(60.0, 40.0),
                delta_x: 1.0,
                delta_y: -12.0,
                unit: ScrollUnit::Pixel,
                finished: false,
            },
            SurfaceInputEvent::Scroll {
                position: kurbo::Point::new(60.0, 40.0),
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
        caret: Some(kurbo::Rect::new(10.0, 20.0, 12.0, 38.0)),
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

/// Scene content that handles its own input, recording what reaches it and
/// redrawing through its invalidator on every key, the way a terminal redraws
/// the grid a keystroke changed.
struct SceneProbe {
    log: ProbeLog,
    builds: Rc<RefCell<usize>>,
    invalidator: Option<SceneInvalidator>,
}

impl SceneContent for SceneProbe {
    fn build_scene(&mut self, _scene: &mut dyn Scene2D, _width: f32, _height: f32) -> bool {
        *self.builds.borrow_mut() += 1;
        false
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.invalidator = invalidator;
    }

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, event: &SurfaceInputEvent) {
        self.log.0.borrow_mut().push(event.clone());
        if matches!(event, SurfaceInputEvent::Key { .. }) {
            let invalidator = self
                .invalidator
                .as_ref()
                .expect("a mounted scene holds its invalidator");
            invalidator();
        }
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        Some(kurbo::Rect::new(10.0, 20.0, 12.0, 38.0))
    }
}

#[test]
fn scene_content_that_wants_input_is_routed_like_a_surface() {
    let log = ProbeLog::default();
    let builds = Rc::new(RefCell::new(0));
    let mut runtime = runtime_with(SceneView::new(SceneProbe {
        log: log.clone(),
        builds: Rc::clone(&builds),
        invalidator: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);
    assert_eq!(log.drain(), Vec::new(), "nothing reaches unfocused content");

    press_at(&mut runtime, 12.0, 34.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Focus(true),
            SurfaceInputEvent::PointerMove {
                position: kurbo::Point::new(12.0, 34.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: true,
                button: SurfacePointerButton::Primary,
                position: kurbo::Point::new(12.0, 34.0),
            },
        ],
        "a press lands on the content in its own logical coordinates and \
         focuses it"
    );
    let state = runtime
        .focused_text_input_state()
        .expect("focused content publishes its caret");
    assert!(
        (state.x - (SURFACE_ORIGIN_X + 10.0)).abs() < 0.01
            && (state.y - (SURFACE_ORIGIN_Y + 20.0)).abs() < 0.01,
        "the content's caret is projected into the window (got {}, {})",
        state.x,
        state.y
    );

    // Idle frames later — the targets re-emitted from scratch each time — the
    // keyboard still reaches the content, and the redraw its invalidator asks
    // for runs.
    for frame in 8..12 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
    }
    let builds_before = *builds.borrow();
    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Pressed));
    let _ = runtime.pump_at(false, start + Duration::from_millis(300));
    let _ = runtime.pump_at(false, start + Duration::from_millis(316));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Key {
            pressed: true,
            key: Key::Character("a".to_owned()),
            code: Code::KeyA,
            modifiers: W3cModifiers::empty(),
            repeat: false,
        }],
    );
    assert!(
        *builds.borrow() > builds_before,
        "content that invalidated on input is drawn again"
    );
}

/// An input-wanting surface joins keyboard traversal like any focusable
/// control: Tab reaches it in tree order, `Focus(true)` opens the input
/// session — and once it holds focus it follows GTK's text-view convention,
/// taking plain Tab and Shift-Tab as input while Ctrl+Tab and
/// Ctrl+Shift+Tab move focus out again. The pointer press that used to be
/// the only way in lands in the same slot traversal owns.
#[test]
fn tab_focuses_the_surface_and_ctrl_tab_leaves_it() {
    let log = ProbeLog::default();
    let view = vstack((
        GpuSurface::new(InputProbe {
            log: log.clone(),
            caret: Some(kurbo::Rect::new(10.0, 20.0, 12.0, 38.0)),
        }),
        button("next").action(|| {}),
        button("last").action(|| {}),
    ));
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    assert_eq!(
        log.drain(),
        Vec::new(),
        "nothing reaches the surface before it is focused"
    );

    // A pointer press still focuses, through the same keyboard-focus slot
    // traversal reads — so the very next Tab moves on instead of
    // re-focusing.
    press_at(&mut runtime, 10.0, 10.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Focus(true),
            SurfaceInputEvent::PointerMove {
                position: kurbo::Point::new(10.0, 10.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: true,
                button: SurfacePointerButton::Primary,
                position: kurbo::Point::new(10.0, 10.0),
            },
        ],
        "a press focuses the surface through the same slot traversal uses"
    );
    let (x, y) = window_point(10.0, 10.0);
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(116));
    let _ = log.drain();
    assert!(
        runtime.focused_text_input_state().is_some(),
        "a focused surface publishes its caret like a text field"
    );

    // Plain Tab and Shift-Tab are the surface's own input — a terminal's
    // completion and backtab — not traversal out of it.
    tab(&mut runtime, false);
    let _ = runtime.pump_at(false, start + Duration::from_millis(132));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Key {
                pressed: true,
                key: Key::Named(NamedKey::Tab),
                code: Code::Tab,
                modifiers: W3cModifiers::empty(),
                repeat: false,
            },
            SurfaceInputEvent::Key {
                pressed: false,
                key: Key::Named(NamedKey::Tab),
                code: Code::Tab,
                modifiers: W3cModifiers::empty(),
                repeat: false,
            },
        ],
        "a focused surface keeps Tab as input, not as traversal"
    );
    tab(&mut runtime, true);
    let _ = runtime.pump_at(false, start + Duration::from_millis(148));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Key {
                pressed: true,
                key: Key::Named(NamedKey::Tab),
                code: Code::Tab,
                modifiers: W3cModifiers::SHIFT,
                repeat: false,
            },
            SurfaceInputEvent::Key {
                pressed: false,
                key: Key::Named(NamedKey::Tab),
                code: Code::Tab,
                modifiers: W3cModifiers::SHIFT,
                repeat: false,
            },
        ],
        "and Shift-Tab likewise — focus does not move on either"
    );
    assert!(runtime.focused_text_input_state().is_some());

    // Ctrl+Tab moves keyboard focus out to the next focusable — the Focus
    // pair goes through the same transition the press used.
    ctrl_tab(&mut runtime, false);
    let _ = runtime.pump_at(false, start + Duration::from_millis(164));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Focus(false)],
        "Ctrl+Tab moves on to the next focusable"
    );
    assert!(runtime.focused_text_input_state().is_none());

    // Ctrl+Shift-Tab comes back to the surface and typing reaches it — no
    // pointer event involved at all.
    ctrl_tab(&mut runtime, true);
    let _ = runtime.pump_at(false, start + Duration::from_millis(180));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Focus(true)],
        "Ctrl+Shift-Tab returns to the surface"
    );
    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Pressed));
    runtime.push_input_event(InputEvent::TextInput {
        text: "a".to_owned(),
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(196));
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
        ],
        "keys reach a surface focused by the keyboard"
    );
}

/// `.focused(binding)` — the same `Metadata<Focused>` wiring a TextField
/// honours — focuses an input-wanting surface without a pointer press,
/// and a pointer press writes the binding back the way it does for a
/// field.
#[test]
fn the_focused_binding_focuses_the_surface_without_a_pointer() {
    let log = ProbeLog::default();
    let focus = Binding::container(None::<Pane>);
    let view = GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: None,
    })
    .focused(&focus, Pane::Document);
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    assert_eq!(log.drain(), Vec::new());

    focus.set(Some(Pane::Document));
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Focus(true)],
        "setting the .focused source focuses the surface"
    );

    runtime.push_input_event(key_event("b", Code::KeyB, KeyState::Pressed));
    let _ = runtime.pump_at(false, start + Duration::from_millis(116));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Key {
            pressed: true,
            key: Key::Character("b".to_owned()),
            code: Code::KeyB,
            modifiers: W3cModifiers::empty(),
            repeat: false,
        }],
        "keys reach a programmatically focused surface"
    );

    focus.set(None);
    let _ = runtime.pump_at(false, start + Duration::from_millis(132));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Focus(false)],
        "clearing the source unfocuses the surface"
    );

    // A pointer press writes the binding back, so the app's own
    // what-is-focused state tracks the surface too.
    press_at(&mut runtime, 10.0, 10.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(148));
    assert_eq!(
        focus.snapshot(),
        Some(Pane::Document),
        "a press writes its focus back through the .focused binding"
    );
}

/// A structural rebuild that adds a pane keeps the `.focused` wiring live:
/// re-asserting the same source after the rebuild focuses the new pane
/// through the one transition point, and unfocuses the old one.
#[test]
fn a_structural_rebuild_re_focuses_the_surface_programmatically() {
    let log_document = ProbeLog::default();
    let log_canvas = ProbeLog::default();
    let focus = Binding::container(None::<Pane>);
    let show_canvas = Binding::container(false);
    let view = vstack((
        GpuSurface::new(InputProbe {
            log: log_document.clone(),
            caret: None,
        })
        .focused(&focus, Pane::Document),
        GpuSurface::new(InputProbe {
            log: log_canvas.clone(),
            caret: None,
        })
        .focused(&focus, Pane::Canvas)
        .visible(show_canvas.clone()),
    ));
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);

    focus.set(Some(Pane::Document));
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    assert_eq!(log_document.drain(), vec![SurfaceInputEvent::Focus(true)]);
    assert_eq!(log_canvas.drain(), Vec::new());

    // The new pane joins the tree mid-session; focus stays where it was
    // while the structure around it changes.
    show_canvas.set(true);
    let _ = runtime.pump_at(false, start + Duration::from_millis(116));
    let _ = runtime.pump_at(false, start + Duration::from_millis(132));
    assert_eq!(log_document.drain(), Vec::new());
    assert_eq!(log_canvas.drain(), Vec::new());

    focus.set(Some(Pane::Canvas));
    let _ = runtime.pump_at(false, start + Duration::from_millis(148));
    assert_eq!(
        log_document.drain(),
        vec![SurfaceInputEvent::Focus(false)],
        "programmatic focus moving on unfocuses the old pane"
    );
    assert_eq!(
        log_canvas.drain(),
        vec![SurfaceInputEvent::Focus(true)],
        "and focuses the pane it names"
    );

    runtime.push_input_event(key_event("c", Code::KeyC, KeyState::Pressed));
    let _ = runtime.pump_at(false, start + Duration::from_millis(164));
    assert_eq!(log_document.drain(), Vec::new());
    assert_eq!(
        log_canvas.drain(),
        vec![SurfaceInputEvent::Key {
            pressed: true,
            key: Key::Character("c".to_owned()),
            code: Code::KeyC,
            modifiers: W3cModifiers::empty(),
            repeat: false,
        }],
        "keys follow programmatic focus to the new pane"
    );
}

/// The pane a tab switch shows — each tab's surface stays mounted under
/// `.visible(selected.equal_to(...))` the way #103's app keeps every tab's
/// `SceneView` alive and only switches which is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tab {
    One,
    Two,
}

/// Switching the selected tab hides the focused surface: the hidden surface
/// releases focus with `Focus(false)`, the newly shown tab takes it with
/// `Focus(true)` by the same move rule Tab traversal uses, and typing reaches
/// the shown tab without another pointer press. water-rs/hydrolysis#126.
#[test]
fn hiding_the_focused_tab_moves_focus_to_the_shown_tab() {
    let log_one = ProbeLog::default();
    let log_two = ProbeLog::default();
    let selected = Binding::container(Tab::One);
    let view = vstack((
        GpuSurface::new(InputProbe {
            log: log_one.clone(),
            caret: None,
        })
        .visible(selected.equal_to(Tab::One)),
        GpuSurface::new(InputProbe {
            log: log_two.clone(),
            caret: None,
        })
        .visible(selected.equal_to(Tab::Two)),
    ));
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log_one.drain();
    let _ = log_two.drain();

    press_at(&mut runtime, 10.0, 10.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    assert_eq!(
        log_one.drain(),
        vec![
            SurfaceInputEvent::Focus(true),
            SurfaceInputEvent::PointerMove {
                position: kurbo::Point::new(10.0, 10.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: true,
                button: SurfacePointerButton::Primary,
                position: kurbo::Point::new(10.0, 10.0),
            },
        ],
        "the press focuses the visible tab's surface"
    );
    let (x, y) = window_point(10.0, 10.0);
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(116));
    let _ = log_one.drain();

    // The user switches tabs: the focused surface is still mounted but no
    // longer visible — it must not keep keyboard focus. The freed focus
    // relocates to the next focusable in tree order — the tab the switch
    // just showed — the same move a Tab press would make.
    selected.set(Tab::Two);
    let _ = runtime.pump_at(false, start + Duration::from_millis(132));
    let _ = runtime.pump_at(false, start + Duration::from_millis(148));
    assert_eq!(
        log_one.drain(),
        vec![SurfaceInputEvent::Focus(false)],
        "hiding the focused surface releases focus and tells it so"
    );
    assert_eq!(
        log_two.drain(),
        vec![SurfaceInputEvent::Focus(true)],
        "the shown tab takes the relocated focus — not a click, the move rule"
    );

    // Typing right after the switch must reach the shown tab — and the
    // hidden tab must hear none of it.
    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Pressed));
    runtime.push_input_event(InputEvent::TextInput {
        text: "a".to_owned(),
    });
    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Released));
    let _ = runtime.pump_at(false, start + Duration::from_millis(164));
    assert_eq!(
        log_one.drain(),
        Vec::new(),
        "a hidden surface receives no keys — the bug #103 reports"
    );
    assert_eq!(
        log_two.drain(),
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
        "typing reaches the newly shown tab right after the switch"
    );
}

/// The window's own focus changes reach the surface holding keyboard focus:
/// blur reports `Focus(false)` and the refocus `Focus(true)`, while keyboard
/// focus inside the window is kept — as platforms report it. The winit arm
/// that produces [`InputEvent::Focused`] is the one link a headless test
/// cannot reach, so this drives the platform-neutral event the runner would
/// have drained. water-rs/hydrolysis#139.
#[test]
fn window_focus_changes_reach_the_focused_surface() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(GpuSurface::new(InputProbe {
        log: log.clone(),
        caret: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);
    press_at(&mut runtime, 10.0, 10.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    let _ = log.drain();

    // The window losing focus tells the surface — without moving keyboard
    // focus off it.
    runtime.push_input_event(InputEvent::Focused(false));
    let _ = runtime.pump_at(false, start + Duration::from_millis(116));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Focus(false)],
        "window blur must reach the focused surface"
    );

    // Keyboard focus inside the window is kept: the surface still owns the
    // keys while the window is unfocused.
    runtime.push_input_event(key_event("a", Code::KeyA, KeyState::Pressed));
    let _ = runtime.pump_at(false, start + Duration::from_millis(132));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Key {
            pressed: true,
            key: Key::Character("a".to_owned()),
            code: Code::KeyA,
            modifiers: W3cModifiers::empty(),
            repeat: false,
        }],
        "keyboard focus inside the window is preserved across the blur"
    );

    // Regaining window focus restores the report — one Focus(true), to the
    // surface that kept the keyboard focus.
    runtime.push_input_event(InputEvent::Focused(true));
    let _ = runtime.pump_at(false, start + Duration::from_millis(148));
    assert_eq!(
        log.drain(),
        vec![SurfaceInputEvent::Focus(true)],
        "window refocus restores the focused surface's report"
    );
}

/// The winit translation this backend delegates to `ui-events-winit` is not
/// reachable from a headless test, so the two quirks the mapping depends on
/// are pinned here directly: get either wrong and space stops activating
/// buttons, or the platform modifier arrives as the wrong key.
#[cfg(hydrolysis_winit)]
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

/// A secondary press into a `.context_menu`-wrapped surface still focuses it
/// — the menu's commands act on the focused content — but the enclosing menu
/// claims the button itself: the surface sees only the pointer move.
/// water-rs/hydrolysis#110.
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
#[test]
fn context_menu_claims_the_secondary_button_over_an_input_surface() {
    use accesskit::Role;
    use waterui_controls::menu::CommandExt as _;

    let log = ProbeLog::default();
    let mut runtime = runtime_with(
        SceneView::new(SceneProbe {
            log: log.clone(),
            builds: Rc::new(RefCell::new(0)),
            invalidator: None,
        })
        .context_menu(vec!["Copy".action(|| {})]),
    );
    let start = Instant::now();
    settled(&mut runtime, start);

    let (x, y) = window_point(12.0, 34.0);
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Secondary,
    });
    let update = runtime
        .pump_at(false, start + Duration::from_millis(100))
        .tree_update
        .expect("the click frame must publish an accessibility tree");
    assert!(
        super::popup_windows::find_by_label(&update, Role::Button, "Copy").is_some(),
        "the context menu must open over the input surface"
    );
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Focus(true),
            SurfaceInputEvent::PointerMove {
                position: kurbo::Point::new(12.0, 34.0),
            },
        ],
        "the surface takes focus and the pointer move, not the secondary button"
    );
}

/// An empty `.context_menu` must behave the same in every build: nothing
/// mounts and the secondary press keeps going to the surface. A debug build
/// used to append "Inspect element" to the empty item list, growing a
/// one-item popup that swallowed the press — water-rs/hydrolysis#188.
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
#[test]
fn an_empty_context_menu_mounts_no_popup_and_keeps_the_secondary_press() {
    use accesskit::Role;

    let log = ProbeLog::default();
    let mut runtime = runtime_with(
        SceneView::new(SceneProbe {
            log: log.clone(),
            builds: Rc::new(RefCell::new(0)),
            invalidator: None,
        })
        .context_menu(()),
    );
    let start = Instant::now();
    settled(&mut runtime, start);

    let (x, y) = window_point(12.0, 34.0);
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Secondary,
    });
    let update = runtime
        .pump_at(false, start + Duration::from_millis(100))
        .tree_update
        .expect("the click frame must publish an accessibility tree");
    assert!(
        super::popup_windows::find_by_label(&update, Role::Button, "Inspect element").is_none(),
        "an empty menu must not grow an Inspect element popup in a debug build"
    );
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Focus(true),
            SurfaceInputEvent::PointerMove {
                position: kurbo::Point::new(12.0, 34.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: true,
                button: SurfacePointerButton::Secondary,
                position: kurbo::Point::new(12.0, 34.0),
            },
        ],
        "with no menu to open the secondary button must reach the surface, \
         exactly as it does in a release build"
    );
}

/// Without an enclosing context menu the surface keeps the secondary button.
#[test]
fn secondary_button_reaches_an_input_surface_without_a_context_menu() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(SceneView::new(SceneProbe {
        log: log.clone(),
        builds: Rc::new(RefCell::new(0)),
        invalidator: None,
    }));
    let start = Instant::now();
    settled(&mut runtime, start);

    let (x, y) = window_point(12.0, 34.0);
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Secondary,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::Focus(true),
            SurfaceInputEvent::PointerMove {
                position: kurbo::Point::new(12.0, 34.0),
            },
            SurfaceInputEvent::PointerButton {
                pressed: true,
                button: SurfacePointerButton::Secondary,
                position: kurbo::Point::new(12.0, 34.0),
            },
        ],
        "the secondary button reaches the surface when no menu claims it"
    );
}

/// water-rs/hydrolysis#249 — scroll targets carried no depth/order, so the
/// wheel handler consulted the embedded surface first and the topmost
/// overlay never saw the delta. A scroll view stacked above an
/// input-receiving surface must win the wheel and the trackpad pan.
#[test]
fn a_scroll_view_stacked_above_a_surface_receives_the_wheel_and_pan() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(zstack((
        GpuSurface::new(InputProbe {
            log: log.clone(),
            caret: None,
        }),
        scroll(().size(SURFACE_WIDTH, 1_500.0)),
    )));
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log.drain();

    let (x, y) = window_point(20.0, 20.0);
    runtime.push_input_event(InputEvent::Scroll {
        x,
        y,
        dx: 0.0,
        dy: -40.0,
        is_line_delta: false,
    });
    runtime.push_input_event(InputEvent::TrackpadPan {
        x,
        y,
        dx: 0.0,
        dy: -40.0,
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

    let metrics = runtime
        .renderer()
        .scroll_metrics_at(x, y)
        .expect("the overlay scroll registers a scroll target");
    assert!(
        metrics.offset_y > 0.0,
        "the topmost scroll view must scroll; before the fix the surface \
         swallowed the deltas: {metrics:?}"
    );
    assert!(
        !log.drain()
            .iter()
            .any(|event| matches!(event, SurfaceInputEvent::Scroll { .. })),
        "the surface beneath the scroll view must see none of its deltas"
    );
}
