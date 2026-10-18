//! Typing into a text field on a pushed navigation page.
//!
//! water-rs/waterui#1938: on the Android host a field inside a navigation
//! page showed its focused state and caret while the IME mirror accepted
//! text, yet the committed text never reached the field's model — the
//! draft binding stayed empty. The renderer half of that report is driven
//! here: focus the pushed page's field, then send a key.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use nami::Signal as _;
use waterui::ViewExt as _;
use waterui_controls::text_field::field;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Binding, Str};
use waterui_layout::stack::vstack;
use waterui_navigation::{NavigationLink, NavigationStack, NavigationView};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, KeyCode, KeyState, Modifiers};

const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;
const FRAME: Duration = Duration::from_millis(16);
/// Longer than the test theme's 450ms navigation transition, so one pump past
/// this covers a push, a pop, and an interactive completion from progress 0.
const TRANSITION: Duration = Duration::from_millis(700);
const POINTER_ID: u64 = 1;

fn mount(view: AnyView) -> (HeadlessRuntime, Instant) {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    (
        HeadlessRuntime::new_for_tests(
            test_environment(),
            builder,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            MinimalTestTheme::default(),
        ),
        Instant::now(),
    )
}

fn pump(runtime: &mut HeadlessRuntime, clock: &mut Instant) {
    *clock += FRAME;
    let _ = runtime.pump_at(false, *clock);
}

fn pump_for(runtime: &mut HeadlessRuntime, clock: &mut Instant, duration: Duration) {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration {
        pump(runtime, clock);
        elapsed += FRAME;
    }
}

fn press(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    use crate::platform::{PointerButton, PointerKind};
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

/// Presses the center of the text input registered at `index`.
fn press_text_input(runtime: &mut HeadlessRuntime, index: usize) {
    let center = runtime.renderer().text_editing.text_input_targets[index]
        .bounds
        .center();
    press(
        runtime,
        crate::num_cast::f64_as_f32(center.x),
        crate::num_cast::f64_as_f32(center.y),
    );
}

fn key(runtime: &mut HeadlessRuntime, key: KeyCode, state: KeyState) {
    runtime.push_input_event(InputEvent::Key {
        logical_key: key.to_w3c_key(),
        physical_code: keyboard_types::Code::Unidentified,
        repeat: false,
        key,
        state,
        modifiers: Modifiers::default(),
    });
    let _ = runtime.pump(false);
}

fn stack_with_field_page(value: &Binding<Str>) -> AnyView {
    let value_for_page = value.clone();
    AnyView::new(NavigationStack::new(NavigationView::new(
        "Root",
        vstack((NavigationLink::new("Compose", move || {
            NavigationView::new(
                "Compose",
                field("Message", &value_for_page).size(300.0, 44.0),
            )
        }),)),
    )))
}

fn open_compose(runtime: &mut HeadlessRuntime, clock: &mut Instant) {
    use accesskit::Role;
    for _ in 0..80 {
        pump(runtime, clock);
        if let Some(update) = runtime.accessibility_tree()
            && let Some((target, _)) =
                super::popup_windows::find_by_label(&update, Role::Button, "Compose")
        {
            assert!(
                runtime.perform_accessibility_action(accesskit::ActionRequest {
                    action: accesskit::Action::Click,
                    target_node: target,
                    target_tree: accesskit::TreeId::ROOT,
                    data: None,
                }),
                "the Compose link activation changed nothing"
            );
            break;
        }
    }
    pump_for(runtime, clock, TRANSITION);
}

#[test]
fn a_field_on_a_pushed_navigation_page_receives_typed_text() {
    let value = Binding::container(Str::default());
    let (mut runtime, mut clock) = mount(stack_with_field_page(&value));
    open_compose(&mut runtime, &mut clock);

    assert!(
        !runtime.renderer().text_editing.text_input_targets.is_empty(),
        "the pushed page's field registers a text-input target"
    );
    press_text_input(&mut runtime, 0);
    pump(&mut runtime, &mut clock);
    assert!(
        runtime.focused_text_input_state().is_some(),
        "pressing the pushed page's field must focus it"
    );

    key(&mut runtime, KeyCode::Character("h".into()), KeyState::Pressed);
    key(
        &mut runtime,
        KeyCode::Character("h".into()),
        KeyState::Released,
    );
    pump(&mut runtime, &mut clock);
    assert_eq!(
        value.snapshot().to_string(),
        "h",
        "the typed character must reach the field's binding"
    );
}

/// The Android soft keyboard never sends `InputEvent::Key` for typed text:
/// `commitText`/`setComposingText` land as `edit_op`s on the
/// `EditingSession` mirror, which `flush`es through
/// `apply_editor_projection` into the focused model. That path must write
/// a pushed page's field the way a hardware key does.
#[test]
fn an_editing_session_commit_reaches_the_pushed_page_fields_binding() {
    use crate::runner::editing::EditingSession;

    let value = Binding::container(Str::default());
    let (mut runtime, mut clock) = mount(stack_with_field_page(&value));
    open_compose(&mut runtime, &mut clock);

    press_text_input(&mut runtime, 0);
    pump(&mut runtime, &mut clock);
    assert!(
        runtime.focused_text_input_state().is_some(),
        "pressing the pushed page's field must focus it"
    );

    // The Android session mirrors `edit_op`: adopt the renderer's
    // focused-editor snapshot, then commit and flush.
    let mut session = EditingSession::new();
    let snapshot = runtime
        .renderer()
        .focused_editor_snapshot()
        .expect("a focused pushed-page field must produce an editor snapshot");
    session.sync_from_renderer(Some(&snapshot));
    let editor_id = session.editor_id();
    assert!(
        session.commit_text(editor_id, "hi", 1),
        "the fresh editor id must be live"
    );
    assert!(
        session.flush(runtime.renderer_mut()),
        "flushing the committed text must change the focused model"
    );
    pump(&mut runtime, &mut clock);
    assert_eq!(
        value.snapshot().to_string(),
        "hi",
        "an editing-session commit must reach the field's binding"
    );
}

/// The keyboard's rise on Android arrives as a metrics/insets push; a
/// resize between focus and the first keypress must not orphan the
/// field's editing identity.
#[test]
fn a_resize_between_focus_and_typing_keeps_the_pushed_page_field_editable() {
    let value = Binding::container(Str::default());
    let (mut runtime, mut clock) = mount(stack_with_field_page(&value));
    open_compose(&mut runtime, &mut clock);

    press_text_input(&mut runtime, 0);
    pump(&mut runtime, &mut clock);
    assert!(
        runtime.focused_text_input_state().is_some(),
        "pressing the pushed page's field must focus it"
    );

    runtime.push_input_event(InputEvent::Resize {
        width: WINDOW_WIDTH,
        height: WINDOW_HEIGHT - 260,
    });
    pump_for(&mut runtime, &mut clock, 4 * FRAME);

    key(&mut runtime, KeyCode::Character("h".into()), KeyState::Pressed);
    key(
        &mut runtime,
        KeyCode::Character("h".into()),
        KeyState::Released,
    );
    pump(&mut runtime, &mut clock);
    assert_eq!(
        value.snapshot().to_string(),
        "h",
        "a resize must not orphan the pushed page field's editing identity"
    );
}
