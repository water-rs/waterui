//! Scripted platform IME sequences replayed through the runner (#85).
//!
//! Every fixture in `tests/fixtures/ime` encodes the `InputEvent`s a real
//! platform's input method produces, in the order its documentation gives
//! (winit backend sources, IBus/fcitx5, TSF/IMM, AppKit NSTextInputClient).
//! The events are pushed through `HeadlessRuntime::push_input_event` one
//! platform batch per `[[step]]`, so what the receivers observe is what a
//! winit window would deliver.
//!
//! The receivers are a single-line `TextField`, a multi-line editor, a
//! `SecureField` (IME must not be allowed, `purpose == Password`), an
//! input-wanting `GpuSurface`, and an input-wanting `SceneView`. Beyond the
//! text and selection each fixture expects, the replay asserts that the
//! caret rect reported through `focused_text_input_state` follows the
//! pre-edit caret (#25) and that keys/text the platform already consumed for
//! the composition never reach ordinary key handling or the surface.

use core::time::Duration;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use nami::Signal as _;
use serde::Deserialize;
use waterui::ViewExt as _;
use waterui::component::text;
use waterui_controls::button::button;
use waterui_controls::text_field::field;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Binding, Str};
use waterui_form::secure::{Secure, secure};
use waterui_graphics::input::{Code, Key, NamedKey, SurfaceInputEvent};
use waterui_graphics::{
    GpuContext, GpuFrame, GpuSurface, GpuView, Scene2D, SceneContent, SceneView,
};
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{
    InputEvent, KeyCode, KeyState, Modifiers, PointerButton, PointerKind, TextInputPurpose,
};

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const HEADER_HEIGHT: f32 = 100.0;
const FIELD_WIDTH: f32 = 300.0;
const FIELD_HEIGHT: f32 = 60.0;
const EDITOR_HEIGHT: f32 = 120.0;
const SURFACE_WIDTH: f32 = 200.0;
const SURFACE_HEIGHT: f32 = 150.0;

/// Where the 200-wide surface lands in the 400-wide window, under the
/// full-width 100-high header plus the stack's 10-point default spacing.
const SURFACE_ORIGIN_X: f64 = 100.0;
const SURFACE_ORIGIN_Y: f64 = 110.0;

const POINTER_ID: u64 = 7;

#[derive(Debug, Deserialize)]
struct Fixture {
    final_text: String,
    selection: [usize; 2],
    committing_key_swallowed: bool,
    #[serde(rename = "step", default)]
    steps: Vec<FixtureStep>,
}

#[derive(Debug, Deserialize)]
struct FixtureStep {
    #[serde(default)]
    events: Vec<FixtureEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FixtureEvent {
    Preedit { preedit: FixturePreedit },
    Commit { commit: String },
    Disabled { disabled: bool },
    Key { key: FixtureKey },
    Text { text: String },
}

#[derive(Debug, Deserialize)]
struct FixturePreedit {
    text: String,
    caret: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FixtureKey {
    logical: String,
    code: String,
    state: String,
}

struct LoadedFixture {
    name: String,
    fixture: Fixture,
}

fn load_fixtures() -> Vec<LoadedFixture> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ime");
    let mut fixtures: Vec<LoadedFixture> = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("cannot enumerate {}: {err}", dir.display()))
    {
        let path = entry
            .expect("fixture directory entry must be readable")
            .path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
        let fixture: Fixture = toml::from_str(&source)
            .unwrap_or_else(|err| panic!("{} fails to parse: {err}", path.display()));
        fixtures.push(LoadedFixture {
            name: path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("fixture name is UTF-8")
                .to_owned(),
            fixture,
        });
    }
    fixtures.sort_by(|left, right| left.name.cmp(&right.name));
    assert!(
        !fixtures.is_empty(),
        "no IME fixtures found under {}",
        dir.display()
    );
    fixtures
}

fn to_input_event(event: &FixtureEvent) -> InputEvent {
    match event {
        FixtureEvent::Preedit { preedit } => InputEvent::ImePreedit {
            text: preedit.text.clone(),
            caret: preedit.caret,
        },
        FixtureEvent::Commit { commit } => InputEvent::ImeCommit {
            text: commit.clone(),
        },
        FixtureEvent::Disabled { disabled } => {
            assert!(*disabled, "a `disabled` fixture event must be true");
            InputEvent::ImeDisabled
        }
        FixtureEvent::Text { text } => InputEvent::TextInput { text: text.clone() },
        FixtureEvent::Key { key } => {
            let logical_key = key
                .logical
                .parse::<Key>()
                .unwrap_or(Key::Named(NamedKey::Unidentified));
            let key_code = match &logical_key {
                Key::Named(named) => KeyCode::Named(format!("{named:?}")),
                Key::Character(value) => KeyCode::Character(value.clone()),
            };
            InputEvent::Key {
                key: key_code,
                logical_key,
                physical_code: key.code.parse::<Code>().unwrap_or(Code::Unidentified),
                repeat: false,
                state: match key.state.as_str() {
                    "pressed" => KeyState::Pressed,
                    "released" => KeyState::Released,
                    other => panic!("unsupported fixture key state {other:?}"),
                },
                modifiers: Modifiers::default(),
            }
        }
    }
}

fn push_step(runtime: &mut HeadlessRuntime, step: &FixtureStep) {
    for event in &step.events {
        runtime.push_input_event(to_input_event(event));
    }
}

/// One key event as a platform would deliver it — the same construction the
/// fixture parser applies, for tests that write their batches in code.
fn key_event(key: Key, code: Code, state: KeyState, modifiers: Modifiers) -> InputEvent {
    let key_code = match &key {
        Key::Named(named) => KeyCode::Named(format!("{named:?}")),
        Key::Character(value) => KeyCode::Character(value.clone()),
    };
    InputEvent::Key {
        key: key_code,
        logical_key: key,
        physical_code: code,
        repeat: false,
        state,
        modifiers,
    }
}

/// Mounts a single-line field plus a Submit button — the form proves which
/// keys reached ordinary handling.
fn form_runtime() -> (HeadlessRuntime, Binding<Str>, Binding<bool>) {
    let value = Binding::container(Str::default());
    let submitted = Binding::bool(false);
    let view = {
        let value_for_view = value.clone();
        let submitted_for_action = submitted.clone();
        AnyView::new(vstack((
            field("Name", &value_for_view).size(FIELD_WIDTH, FIELD_HEIGHT),
            button("Submit").action(move || submitted_for_action.set(true)),
        )))
    };
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    press_text_input(&mut runtime, 0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    assert!(
        runtime.focused_text_input_state().is_some(),
        "pressing the field must focus it"
    );
    (runtime, value, submitted)
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

/// Pumps until the mounted view has registered every target the replay needs.
fn settled(runtime: &mut HeadlessRuntime, start: Instant) {
    for frame in 0..6 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
    }
}

fn press(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
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

/// Presses the center of the text input registered at `index` — the frame a
/// fixed `.size()` produces centers the material field, so its real bounds
/// sit inside the frame rather than filling it.
fn press_text_input(runtime: &mut HeadlessRuntime, index: usize) {
    let center = runtime.renderer().text_editing.text_input_targets[index]
        .bounds
        .center();
    press(runtime, center.x as f32, center.y as f32);
}

/// The observable state of a text-input receiver while a fixture runs.
struct TextReplay<'a> {
    name: &'a str,
    /// Accumulated committed text the fixture has produced so far.
    committed: String,
    /// The pre-edit the receiver should currently hold, and the byte offset
    /// of the caret the platform reported inside it.
    composing: Option<(String, Option<usize>)>,
    /// `(pre-edit text, caret, reported rect x)` per composition update, so a
    /// caret that moves inside unchanged marked text can be seen to move the
    /// reported rect (#25).
    caret_samples: Vec<(String, usize, f64)>,
}

impl<'a> TextReplay<'a> {
    fn new(name: &'a str) -> Self {
        Self {
            name,
            committed: String::new(),
            composing: None,
            caret_samples: Vec::new(),
        }
    }

    fn track_events(&mut self, step: &FixtureStep) {
        for event in &step.events {
            match event {
                FixtureEvent::Preedit { preedit } if preedit.text.is_empty() => {
                    self.composing = None;
                }
                FixtureEvent::Preedit { preedit } => {
                    self.composing = Some((preedit.text.clone(), preedit.caret));
                }
                FixtureEvent::Commit { commit } => {
                    self.committed += commit;
                    self.composing = None;
                }
                FixtureEvent::Disabled { .. }
                | FixtureEvent::Key { .. }
                | FixtureEvent::Text { .. } => {}
            }
        }
    }
}

/// Samples the reported caret rect after one pumped step and checks the live
/// composition state. `secure` receivers must never hold a pre-edit at all.
fn observe_step(
    replay: &mut TextReplay<'_>,
    runtime: &mut HeadlessRuntime,
    step: &FixtureStep,
    step_index: usize,
    secure: bool,
) {
    replay.track_events(step);
    let name = replay.name;
    let preedit = runtime.renderer().text_editing.ime_preedit.clone();
    if secure {
        assert!(
            preedit.is_none(),
            "{name} step {step_index}: a password-purpose field must never store \
             a pre-edit (got {preedit:?})"
        );
    } else {
        let expected = replay.composing.as_ref().map(|(text, _)| text.as_str());
        assert_eq!(
            preedit.as_deref(),
            expected,
            "{name} step {step_index}: the live composition must be the last \
             pre-edit the platform sent"
        );
    }
    let state = runtime
        .focused_text_input_state()
        .unwrap_or_else(|| panic!("{name} step {step_index}: focused field reports no caret rect"));
    assert_eq!(
        state.purpose,
        if secure {
            TextInputPurpose::Password
        } else {
            TextInputPurpose::Normal
        },
        "{name} step {step_index}: wrong input purpose"
    );
    assert!(
        state.height > 5.0,
        "{name} step {step_index}: the caret rect must span the composing \
         line so a candidate window can anchor below it (got {})",
        state.height
    );
    if !secure && let Some((text, Some(caret))) = replay.composing.clone() {
        replay.caret_samples.push((text, caret, state.x));
    }
}

/// Two composition updates on the same marked text whose caret moved must
/// move the reported rect in the same direction — the #25 check.
fn assert_caret_tracks_preedit(replay: &TextReplay<'_>) {
    for pair in replay.caret_samples.windows(2) {
        let [(text_a, caret_a, x_a), (text_b, caret_b, x_b)] = pair else {
            continue;
        };
        if text_a != text_b {
            continue;
        }
        match caret_b.cmp(caret_a) {
            core::cmp::Ordering::Greater => assert!(
                x_b > x_a,
                "{}: caret moved {caret_a} -> {caret_b} inside {text_a:?} but the \
                 reported rect stayed at x={x_a}",
                replay.name
            ),
            core::cmp::Ordering::Less => assert!(
                x_b < x_a,
                "{}: caret moved {caret_a} -> {caret_b} inside {text_a:?} but the \
                 reported rect stayed at x={x_a}",
                replay.name
            ),
            core::cmp::Ordering::Equal => {}
        }
    }
}

fn selection_of(runtime: &HeadlessRuntime) -> (usize, usize) {
    let targets = &runtime.renderer().text_editing.text_input_targets;
    assert_eq!(targets.len(), 1, "the fixture harness mounts one field");
    let slot = targets[0].selection.borrow();
    (slot.anchor, slot.focus)
}

fn replay_against_text_field(loaded: &LoadedFixture, multiline: bool) {
    let name = loaded.name.as_str();
    let receiver = if multiline {
        "multi-line editor"
    } else {
        "single-line TextField"
    };
    let value = Binding::container(Str::default());
    let submitted = Binding::bool(false);
    let view = {
        let value_for_view = value.clone();
        let submitted_for_action = submitted.clone();
        let field_view = field("Name", &value_for_view);
        let field_view = if multiline {
            field_view.disable_line_limit()
        } else {
            field_view
        };
        AnyView::new(vstack((
            field_view.size(
                FIELD_WIDTH,
                if multiline {
                    EDITOR_HEIGHT
                } else {
                    FIELD_HEIGHT
                },
            ),
            button("Submit").action(move || submitted_for_action.set(true)),
        )))
    };
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    let mut now = start;

    press_text_input(&mut runtime, 0);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_some(),
        "{name} ({receiver}): pressing the field must focus it"
    );

    let mut replay = TextReplay::new(name);
    for (index, step) in loaded.fixture.steps.iter().enumerate() {
        push_step(&mut runtime, step);
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        observe_step(&mut replay, &mut runtime, step, index, false);
        assert_eq!(
            value.snapshot().to_string().as_str(),
            replay.committed,
            "{name} ({receiver}) step {index}: committed text diverged from \
             the fixture's commits"
        );
    }

    assert_eq!(
        value.snapshot().to_string().as_str(),
        loaded.fixture.final_text,
        "{name} ({receiver}): final text"
    );
    assert_eq!(
        selection_of(&runtime),
        (loaded.fixture.selection[0], loaded.fixture.selection[1]),
        "{name} ({receiver}): final selection"
    );
    assert!(
        runtime.renderer().text_editing.ime_preedit.is_none(),
        "{name} ({receiver}): a finished fixture leaves no composition"
    );
    if loaded.fixture.committing_key_swallowed {
        assert!(
            !submitted.snapshot(),
            "{name} ({receiver}): the confirming key must not activate the form"
        );
    }
    assert_caret_tracks_preedit(&replay);
}

fn replay_against_secure_field(loaded: &LoadedFixture) {
    let name = loaded.name.as_str();
    let secret = Binding::container(Secure::new(String::new()));
    let view = {
        let secret_for_view = secret.clone();
        AnyView::new(vstack((
            secure("Password", &secret_for_view).size(FIELD_WIDTH, FIELD_HEIGHT),
        )))
    };
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    let mut now = start;

    press_text_input(&mut runtime, 0);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    let state = runtime
        .focused_text_input_state()
        .unwrap_or_else(|| panic!("{name} (SecureField): pressing the field must focus it"));
    assert_eq!(
        state.purpose,
        TextInputPurpose::Password,
        "{name} (SecureField): IME must be told the field is a password"
    );

    let mut replay = TextReplay::new(name);
    for (index, step) in loaded.fixture.steps.iter().enumerate() {
        push_step(&mut runtime, step);
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        observe_step(&mut replay, &mut runtime, step, index, true);
    }

    assert_eq!(
        secret.snapshot().expose(),
        loaded.fixture.final_text,
        "{name} (SecureField): commits land as plain text edits"
    );
    assert_eq!(
        selection_of(&runtime),
        (loaded.fixture.selection[0], loaded.fixture.selection[1]),
        "{name} (SecureField): final selection"
    );
}

/// Records every event its surface receives, so a test can read them after
/// the frame that delivered them.
#[derive(Clone, Default)]
struct ProbeLog(Rc<RefCell<Vec<SurfaceInputEvent>>>);

impl ProbeLog {
    fn drain(&self) -> Vec<SurfaceInputEvent> {
        core::mem::take(&mut *self.0.borrow_mut())
    }
}

/// A caret rect the embedded view reports in its own logical coordinates —
/// the point `focused_text_input_state` projects into the window.
fn probe_caret() -> Option<vello::kurbo::Rect> {
    Some(vello::kurbo::Rect::new(10.0, 20.0, 12.0, 38.0))
}

struct InputProbe {
    log: ProbeLog,
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
        probe_caret()
    }
}

struct SceneProbe {
    log: ProbeLog,
}

impl SceneContent for SceneProbe {
    fn build_scene(&mut self, _scene: &mut dyn Scene2D, _width: f32, _height: f32) -> bool {
        false
    }

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, event: &SurfaceInputEvent) {
        self.log.0.borrow_mut().push(event.clone());
    }

    fn ime_caret(&self) -> Option<vello::kurbo::Rect> {
        probe_caret()
    }
}

fn surface_view(scene: bool, log: ProbeLog) -> AnyView {
    let surface = if scene {
        AnyView::new(SceneView::new(SceneProbe { log }))
    } else {
        AnyView::new(GpuSurface::new(InputProbe { log }))
    };
    AnyView::new(vstack((
        vstack((text("header"),)).size(WINDOW_WIDTH as f32, HEADER_HEIGHT),
        surface.size(SURFACE_WIDTH, SURFACE_HEIGHT),
    )))
}

/// The input a surface should observe, derived mechanically from the
/// fixture through the same ordered ownership the runner applies: IME
/// events map onto the composition session, while a key/text event reaches
/// the sink only when it is ordinary input at that point in the batch —
/// the platform's composing and co-delivered keystrokes stay inside the
/// IME.
fn expected_surface_events(fixture: &Fixture) -> Vec<SurfaceInputEvent> {
    let mut composing = false;
    let mut expected = Vec::new();
    // The codes of presses the IME consumed: their releases are swallowed
    // whenever they arrive, even in a later batch after the commit.
    let mut swallowed: Vec<Code> = Vec::new();
    for step in &fixture.steps {
        let events: Vec<InputEvent> = step.events.iter().map(to_input_event).collect();
        let owned = crate::runner::ime::ime_owned_events(&events, composing);
        for (event, owned) in events.into_iter().zip(owned) {
            match event {
                InputEvent::ImePreedit { text, caret } => {
                    if text.is_empty() {
                        if composing {
                            composing = false;
                            expected.push(SurfaceInputEvent::CompositionCancel);
                        }
                    } else {
                        if !composing {
                            composing = true;
                            expected.push(SurfaceInputEvent::CompositionStart);
                        }
                        expected.push(SurfaceInputEvent::CompositionUpdate {
                            text: text.into(),
                            caret,
                        });
                    }
                }
                InputEvent::ImeCommit { text } => {
                    if !composing {
                        expected.push(SurfaceInputEvent::CompositionStart);
                    }
                    composing = false;
                    expected.push(SurfaceInputEvent::CompositionCommit(text.into()));
                }
                InputEvent::ImeDisabled => {
                    if composing {
                        composing = false;
                        expected.push(SurfaceInputEvent::CompositionCancel);
                    }
                }
                InputEvent::Key {
                    logical_key,
                    physical_code,
                    repeat,
                    state,
                    modifiers,
                    ..
                } => {
                    let pressed = state == KeyState::Pressed;
                    let consumed = if pressed {
                        owned
                    } else if let Some(index) =
                        swallowed.iter().position(|code| *code == physical_code)
                    {
                        swallowed.swap_remove(index);
                        true
                    } else {
                        owned
                    };
                    if pressed && owned {
                        swallowed.push(physical_code);
                    }
                    if !consumed {
                        expected.push(SurfaceInputEvent::Key {
                            pressed,
                            key: logical_key,
                            code: physical_code,
                            modifiers: modifiers.into(),
                            repeat,
                        });
                    }
                }
                InputEvent::TextInput { text, .. } if !owned => {
                    expected.push(SurfaceInputEvent::TextInput(text.into()));
                }
                _ => {}
            }
        }
    }
    expected
}

fn replay_against_surface(loaded: &LoadedFixture, scene: bool) {
    let name = loaded.name.as_str();
    let receiver = if scene { "SceneView" } else { "GpuSurface" };
    let log = ProbeLog::default();
    let mut runtime = runtime_with(surface_view(scene, log.clone()));
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log.drain();

    press(
        &mut runtime,
        (SURFACE_ORIGIN_X + 20.0) as f32,
        (SURFACE_ORIGIN_Y + 20.0) as f32,
    );
    let mut now = start + Duration::from_millis(100);
    let _ = runtime.pump_at(false, now);
    let _ = log.drain();
    assert!(
        runtime.focused_text_input_state().is_some(),
        "{name} ({receiver}): pressing the surface must focus it"
    );

    for step in &loaded.fixture.steps {
        push_step(&mut runtime, step);
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
    }

    assert_eq!(
        log.drain(),
        expected_surface_events(&loaded.fixture),
        "{name} ({receiver}): the surface must see the composition session, \
         never the platform's composing keystrokes"
    );
}

#[test]
fn fixtures_replay_against_a_single_line_text_field() {
    for loaded in load_fixtures() {
        replay_against_text_field(&loaded, false);
    }
}

#[test]
fn fixtures_replay_against_a_multi_line_editor() {
    for loaded in load_fixtures() {
        replay_against_text_field(&loaded, true);
    }
}

#[test]
fn fixtures_replay_against_a_secure_field() {
    for loaded in load_fixtures() {
        replay_against_secure_field(&loaded);
    }
}

#[test]
fn fixtures_replay_against_an_input_wanting_gpu_surface() {
    for loaded in load_fixtures() {
        replay_against_surface(&loaded, false);
    }
}

#[test]
fn fixtures_replay_against_an_input_wanting_scene_view() {
    for loaded in load_fixtures() {
        replay_against_surface(&loaded, true);
    }
}

/// Moving focus to another field — or the window unfocusing — mid-composition
/// must cancel it: the marked text is dropped, never committed.
#[test]
fn focus_move_or_window_unfocus_cancels_the_composition() {
    let first = Binding::container(Str::default());
    let second = Binding::container(Str::default());
    let view = {
        let first_for_view = first.clone();
        let second_for_view = second.clone();
        AnyView::new(vstack((
            field("First", &first_for_view).size(FIELD_WIDTH, FIELD_HEIGHT),
            field("Second", &second_for_view).size(FIELD_WIDTH, FIELD_HEIGHT),
        )))
    };
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    let mut now = start;

    press_text_input(&mut runtime, 0);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    runtime.push_input_event(InputEvent::ImePreedit {
        text: "にほ".to_owned(),
        caret: Some(6),
    });
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.renderer().text_editing.ime_preedit.is_some(),
        "composition must be live before the focus move"
    );

    // Clicking the second field cancels the first's composition.
    press_text_input(&mut runtime, 1);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.renderer().text_editing.ime_preedit.is_none(),
        "moving focus must cancel the composition, not commit it"
    );
    assert!(first.snapshot().to_string().is_empty());

    // A window unfocus (Ime::Disabled) mid-composition cancels it too.
    runtime.push_input_event(InputEvent::ImePreedit {
        text: "かな".to_owned(),
        caret: Some(6),
    });
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(runtime.renderer().text_editing.ime_preedit.is_some());
    runtime.push_input_event(InputEvent::ImeDisabled);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.renderer().text_editing.ime_preedit.is_none(),
        "window unfocus must cancel the composition"
    );
    assert!(second.snapshot().to_string().is_empty());
}

/// A key press immediately followed by a commit is that keystroke delivered
/// twice — once as a key, once as the commit's text. The commit is the
/// authoritative text, so the press must not insert a second copy.
#[test]
fn a_key_co_delivered_with_its_commit_inserts_text_once() {
    let (mut runtime, value, _submitted) = form_runtime();
    let mut now = Instant::now() + Duration::from_millis(200);
    for event in [
        key_event(
            Key::Character("a".into()),
            Code::KeyA,
            KeyState::Pressed,
            Modifiers::default(),
        ),
        InputEvent::ImeCommit {
            text: "a".to_owned(),
        },
        key_event(
            Key::Character("a".into()),
            Code::KeyA,
            KeyState::Released,
            Modifiers::default(),
        ),
    ] {
        runtime.push_input_event(event);
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert_eq!(
        value.snapshot().to_string().as_str(),
        "a",
        "the commit carries the keystroke's text; its key press must not \
         insert a second copy"
    );
}

/// Direct-commit mode: a plain key delivered with its commit, then Enter in
/// the same batch. The commit ends nothing, so the Enter is ordinary input
/// and must activate the keyboard-focused control.
#[test]
fn enter_after_a_direct_commit_still_reaches_the_form() {
    let (mut runtime, value, submitted) = form_runtime();
    let mut now = Instant::now() + Duration::from_millis(200);
    // Tab moves keyboard focus to the button; the caret follows it, so
    // editing on the field ends (#95).
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_none(),
        "traversing to the button ends the field's text focus"
    );

    for event in [
        key_event(
            Key::Character("a".into()),
            Code::KeyA,
            KeyState::Pressed,
            Modifiers::default(),
        ),
        InputEvent::ImeCommit {
            text: "a".to_owned(),
        },
        key_event(
            Key::Character("a".into()),
            Code::KeyA,
            KeyState::Released,
            Modifiers::default(),
        ),
        key_event(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Pressed,
            Modifiers::default(),
        ),
        key_event(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Released,
            Modifiers::default(),
        ),
    ] {
        runtime.push_input_event(event);
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        value.snapshot().to_string().is_empty(),
        "editing ended on the field: the direct commit lands nowhere"
    );
    assert!(
        submitted.snapshot(),
        "the Enter after a commit is ordinary input and must activate the \
         focused control — batch-level ownership would have swallowed it"
    );
}

/// A shortcut pressed right after a commit — in the same batch — is not the
/// composition's: the chord reaches the application.
#[test]
fn a_shortcut_after_a_commit_reaches_the_application() {
    let log = ProbeLog::default();
    let mut runtime = runtime_with(surface_view(false, log.clone()));
    let start = Instant::now();
    settled(&mut runtime, start);
    let _ = log.drain();
    press(
        &mut runtime,
        (SURFACE_ORIGIN_X + 20.0) as f32,
        (SURFACE_ORIGIN_Y + 20.0) as f32,
    );
    let mut now = start + Duration::from_millis(100);
    let _ = runtime.pump_at(false, now);
    let _ = log.drain();

    let chord = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    for event in [
        InputEvent::ImeCommit {
            text: "x".to_owned(),
        },
        key_event(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Pressed,
            chord,
        ),
        key_event(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Released,
            chord,
        ),
    ] {
        runtime.push_input_event(event);
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert_eq!(
        log.drain(),
        vec![
            SurfaceInputEvent::CompositionStart,
            SurfaceInputEvent::CompositionCommit("x".into()),
            SurfaceInputEvent::Key {
                pressed: true,
                key: Key::Named(NamedKey::Enter),
                code: Code::Enter,
                modifiers: keyboard_types::Modifiers::CONTROL,
                repeat: false,
            },
            SurfaceInputEvent::Key {
                pressed: false,
                key: Key::Named(NamedKey::Enter),
                code: Code::Enter,
                modifiers: keyboard_types::Modifiers::CONTROL,
                repeat: false,
            },
        ],
        "the chord after a direct commit must reach the embedded sink"
    );
}

/// One batch holding a whole composition lifecycle: the confirming Enter is
/// swallowed (no stray newline), the commit lands, and the key events after
/// it are ordinary input again.
#[test]
fn keys_after_a_composition_commit_in_one_batch_are_ordinary_input() {
    for multiline in [false, true] {
        let value = Binding::container(Str::default());
        let view = {
            let value_for_view = value.clone();
            let field_view = field("Name", &value_for_view);
            let field_view = if multiline {
                field_view.disable_line_limit()
            } else {
                field_view
            };
            AnyView::new(vstack((field_view.size(
                FIELD_WIDTH,
                if multiline {
                    EDITOR_HEIGHT
                } else {
                    FIELD_HEIGHT
                },
            ),)))
        };
        let mut runtime = runtime_with(view);
        let start = Instant::now();
        settled(&mut runtime, start);
        press_text_input(&mut runtime, 0);
        let mut now = start + Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);

        for event in [
            InputEvent::ImePreedit {
                text: "n".to_owned(),
                caret: Some(1),
            },
            key_event(
                Key::Named(NamedKey::Enter),
                Code::Enter,
                KeyState::Pressed,
                Modifiers::default(),
            ),
            InputEvent::ImeCommit {
                text: "你".to_owned(),
            },
            key_event(
                Key::Named(NamedKey::Enter),
                Code::Enter,
                KeyState::Released,
                Modifiers::default(),
            ),
            key_event(
                Key::Character("x".into()),
                Code::KeyX,
                KeyState::Pressed,
                Modifiers::default(),
            ),
            key_event(
                Key::Character("x".into()),
                Code::KeyX,
                KeyState::Released,
                Modifiers::default(),
            ),
        ] {
            runtime.push_input_event(event);
        }
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        assert_eq!(
            value.snapshot().to_string().as_str(),
            "你x",
            "multiline={multiline}: the confirming Enter must stay inside \
             the composition — a leaked newline would read `你\nx` — while \
             the later `x` is ordinary input"
        );
    }
}

/// Fast direct-commit typing: every unmarked key arrives beside its own
/// commit, and a Backspace lands between two of them. Each commit claims
/// only the press it answers, so the Backspace stays ordinary input and
/// deletes the earlier commit — the field reads `i`, never `hi`.
#[test]
fn a_key_between_direct_commits_stays_ordinary_input() {
    for multiline in [false, true] {
        let value = Binding::container(Str::default());
        let view = {
            let value_for_view = value.clone();
            let field_view = field("Name", &value_for_view);
            let field_view = if multiline {
                field_view.disable_line_limit()
            } else {
                field_view
            };
            AnyView::new(vstack((field_view.size(
                FIELD_WIDTH,
                if multiline {
                    EDITOR_HEIGHT
                } else {
                    FIELD_HEIGHT
                },
            ),)))
        };
        let mut runtime = runtime_with(view);
        let start = Instant::now();
        settled(&mut runtime, start);
        press_text_input(&mut runtime, 0);
        let mut now = start + Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);

        for event in [
            key_event(
                Key::Character("h".into()),
                Code::KeyH,
                KeyState::Pressed,
                Modifiers::default(),
            ),
            InputEvent::ImeCommit {
                text: "h".to_owned(),
            },
            key_event(
                Key::Character("h".into()),
                Code::KeyH,
                KeyState::Released,
                Modifiers::default(),
            ),
            key_event(
                Key::Named(NamedKey::Backspace),
                Code::Backspace,
                KeyState::Pressed,
                Modifiers::default(),
            ),
            key_event(
                Key::Named(NamedKey::Backspace),
                Code::Backspace,
                KeyState::Released,
                Modifiers::default(),
            ),
            key_event(
                Key::Character("i".into()),
                Code::KeyI,
                KeyState::Pressed,
                Modifiers::default(),
            ),
            InputEvent::ImeCommit {
                text: "i".to_owned(),
            },
            key_event(
                Key::Character("i".into()),
                Code::KeyI,
                KeyState::Released,
                Modifiers::default(),
            ),
        ] {
            runtime.push_input_event(event);
        }
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        assert_eq!(
            value.snapshot().to_string().as_str(),
            "i",
            "multiline={multiline}: the Backspace is not the commit `i`'s \
             producer, so it is ordinary input and must erase `h`"
        );
    }
}

/// A key before the press a commit answers is not that commit's producer:
/// `[Enter, a, Commit "a"]` activates the keyboard-focused control on the
/// Enter and the commit's own press is the only one it claims.
#[test]
fn a_commit_claims_only_the_nearest_press() {
    let (mut runtime, value, submitted) = form_runtime();
    let mut now = Instant::now() + Duration::from_millis(200);
    // Tab moves keyboard focus to the button; the caret follows it, so
    // editing on the field ends (#95).
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_none(),
        "traversing to the button ends the field's text focus"
    );

    for event in [
        key_event(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Pressed,
            Modifiers::default(),
        ),
        key_event(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Released,
            Modifiers::default(),
        ),
        key_event(
            Key::Character("a".into()),
            Code::KeyA,
            KeyState::Pressed,
            Modifiers::default(),
        ),
        InputEvent::ImeCommit {
            text: "a".to_owned(),
        },
        key_event(
            Key::Character("a".into()),
            Code::KeyA,
            KeyState::Released,
            Modifiers::default(),
        ),
    ] {
        runtime.push_input_event(event);
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        value.snapshot().to_string().is_empty(),
        "editing ended on the field: the commit lands nowhere"
    );
    assert!(
        submitted.snapshot(),
        "the Enter the commit cannot be answering is ordinary input and \
         must activate the focused control"
    );
}

/// `ImeDisabled` is the platform acknowledging that the app turned IME
/// off — not the answer to a keystroke — so the Tab that moved focus out
/// of the field in the same batch stays ordinary input and still
/// traverses.
#[test]
fn a_tab_beside_ime_disabled_still_moves_focus() {
    let (mut runtime, _value, submitted) = form_runtime();
    let mut now = Instant::now() + Duration::from_millis(200);
    for event in [
        key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            KeyState::Pressed,
            Modifiers::default(),
        ),
        InputEvent::ImeDisabled,
        key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            KeyState::Released,
            Modifiers::default(),
        ),
    ] {
        runtime.push_input_event(event);
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);

    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        submitted.snapshot(),
        "the Tab beside ImeDisabled is ordinary input: keyboard focus \
         moved to the button, so Enter submits the form"
    );
}

/// Keyboard traversal away from a field ends text editing exactly as a
/// pointer press on a non-text node does (#95): the caret follows keyboard
/// focus, so the field's `.focused(binding)` clears, the IME anchor goes
/// away, and typing lands nowhere. Shift-Tab back restores editing at the
/// caret it left, and Space activates the button keyboard focus moved to.
#[test]
fn tabbing_away_ends_editing_and_shift_tab_restores_the_caret() {
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Name,
    }
    let value = Binding::container(Str::default());
    let focus = Binding::container(None::<Field>);
    let submitted = Binding::bool(false);
    let view = {
        let value_for_view = value.clone();
        let focus_for_view = focus.clone();
        let submitted_for_action = submitted.clone();
        AnyView::new(vstack((
            field("Name", &value_for_view)
                .focused(&focus_for_view, Field::Name)
                .size(FIELD_WIDTH, FIELD_HEIGHT),
            button("Submit").action(move || submitted_for_action.set(true)),
        )))
    };
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    let mut now = start;
    press_text_input(&mut runtime, 0);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_some(),
        "pressing the field must focus it"
    );
    assert_eq!(focus.snapshot(), Some(Field::Name));

    // Type "ab" and park the caret between the characters — the position
    // Shift-Tab back must restore.
    runtime.push_input_event(InputEvent::TextInput {
        text: "ab".to_owned(),
    });
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert_eq!(value.snapshot().to_string().as_str(), "ab");
    // Caret moves measure against the retained text layout — pump first so it
    // reflects the committed text.
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::ArrowLeft),
            Code::ArrowLeft,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert_eq!(
        selection_of(&runtime),
        (1, 1),
        "the caret must park between `a` and `b`"
    );

    // Tab to the button: editing ends — the caret follows keyboard focus.
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_none(),
        "traversal onto the button must end text editing"
    );
    assert_eq!(
        focus.snapshot(),
        None,
        "the field's .focused(binding) must clear when the caret leaves"
    );
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Character("x".into()),
            Code::KeyX,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert_eq!(
        value.snapshot().to_string().as_str(),
        "ab",
        "typing must not reach the field once editing ended"
    );

    // Shift-Tab back: editing resumes and the caret is where it was left.
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            state,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_some(),
        "Shift-Tab back onto the field must restore editing"
    );
    assert_eq!(focus.snapshot(), Some(Field::Name));
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Character("y".into()),
            Code::KeyY,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert_eq!(
        value.snapshot().to_string().as_str(),
        "ayb",
        "typing must resume at the caret Shift-Tab restored"
    );

    // Tab forward again, then Space activates the button keyboard focus
    // moved to.
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(runtime.focused_text_input_state().is_none());
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Character(" ".into()),
            Code::Space,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        submitted.snapshot(),
        "Space must activate the button keyboard focus moved to"
    );
}

/// `.visible(false)` on a subtree releases a focused field inside it
/// (#103): the caret and the IME anchor go away, the `.focused` binding
/// reads `None`, and typing lands nowhere.
#[test]
fn hiding_a_subtree_releases_the_focused_field_inside_it() {
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Name,
    }
    let value = Binding::container(Str::default());
    let focus = Binding::container(None::<Field>);
    let shown = Binding::container(true);
    let view = {
        let value_for_view = value.clone();
        let focus_for_view = focus.clone();
        let shown_for_view = shown.clone();
        AnyView::new(
            field("Name", &value_for_view)
                .focused(&focus_for_view, Field::Name)
                .visible(shown_for_view)
                .size(FIELD_WIDTH, FIELD_HEIGHT),
        )
    };
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    let mut now = start;

    press_text_input(&mut runtime, 0);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_some(),
        "pressing the field must focus it"
    );
    assert_eq!(focus.snapshot(), Some(Field::Name));

    // The subtree turns invisible while the field holds focus.
    shown.set(false);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        runtime.focused_text_input_state().is_none(),
        "a field under an invisible subtree must not keep the caret"
    );
    assert_eq!(
        focus.snapshot(),
        None,
        "the field's .focused(binding) must clear when it goes invisible"
    );

    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Character("x".into()),
            Code::KeyX,
            state,
            Modifiers::default(),
        ));
    }
    runtime.push_input_event(InputEvent::TextInput {
        text: "x".to_owned(),
    });
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert_eq!(
        value.snapshot().to_string().as_str(),
        "",
        "typing must not reach the hidden field"
    );
}

/// Hidden nodes are not Tab targets, and with focus released onto nothing
/// the next Tab resumes from the slot it left — the nearest still-visible
/// focusable in tree order, not the top of the walk.
#[test]
fn tab_skips_hidden_focusables_and_resumes_from_the_released_slot() {
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Name,
    }
    let value = Binding::container(Str::default());
    let focus = Binding::container(None::<Field>);
    let shown = Binding::container(true);
    let first_pressed = Binding::bool(false);
    let last_pressed = Binding::bool(false);
    let view = {
        let value_for_view = value.clone();
        let focus_for_view = focus.clone();
        let shown_for_view = shown.clone();
        let first_for_action = first_pressed.clone();
        let last_for_action = last_pressed.clone();
        AnyView::new(vstack((
            button("first").action(move || first_for_action.set(true)),
            field("Name", &value_for_view)
                .focused(&focus_for_view, Field::Name)
                .visible(shown_for_view)
                .size(FIELD_WIDTH, FIELD_HEIGHT),
            button("last").action(move || last_for_action.set(true)),
        )))
    };
    let mut runtime = runtime_with(view);
    let start = Instant::now();
    settled(&mut runtime, start);
    let mut now = start;

    press_text_input(&mut runtime, 0);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(runtime.focused_text_input_state().is_some());
    assert_eq!(focus.snapshot(), Some(Field::Name));

    // Hide the field mid-focus: focus is released onto nothing.
    shown.set(false);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(runtime.focused_text_input_state().is_none());
    assert_eq!(focus.snapshot(), None);

    // The next Tab skips the hidden field and lands on the focusable after
    // its slot — the nearest visible one in tree order, not the first.
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Named(NamedKey::Tab),
            Code::Tab,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(key_event(
            Key::Character(" ".into()),
            Code::Space,
            state,
            Modifiers::default(),
        ));
    }
    now += Duration::from_millis(16);
    let _ = runtime.pump_at(false, now);
    assert!(
        !first_pressed.snapshot(),
        "Tab must not restart traversal from the first focusable"
    );
    assert!(
        last_pressed.snapshot(),
        "Tab after the release must land on the nearest visible focusable \
         after the hidden slot"
    );
}
