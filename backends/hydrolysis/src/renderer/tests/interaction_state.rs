//! Per-state interaction styling (waterui#1270): the renderer computes a
//! control's `InteractionState` from the hover/press/focus bookkeeping it
//! already keeps plus `Selected`/`Disabled` metadata, moves the installed
//! [`Chrome`]'s surface shape with it, strokes its focus ring only while the
//! state is FOCUSED (keyboard focus-visible), and writes
//! `.interaction_state` report bindings on change.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::renderer::chrome::LiveChrome;
use crate::theme::{
    Chrome, CornerShape, FocusIndicator, Motion, PressFeedback, StateLayer, Surface,
};
use cherenkov_record::ShapeData;
use core::time::Duration;
use kurbo::RoundedRectRadii;
use nami::Signal as _;
use waterui::accessibility::AccessibilityRole;
use waterui::animation::Animation;
use waterui::gesture::TapGesture;
use waterui::{AnyView, Color, ViewExt as _};
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;
use waterui_interaction::interaction::{InteractionState, StateValue};
use waterui_layout::stack::hstack;

use super::{MinimalTestTheme, invisible_chrome, test_environment};
use crate::platform::{InputEvent, KeyCode, KeyState, Modifiers, PointerButton, PointerKind};
use crate::{HeadlessRuntime, keyboard_types};

const POINTER_ID: u64 = 9;
const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;

fn tap_view() -> AnyView {
    AnyView::new(
        hstack((
            AnyView::new(Color::srgb_hex("#18181B")),
            AnyView::new(
                Color::srgb_hex("#3F3F46")
                    .width(20.0)
                    .gesture(TapGesture::new(), || {})
                    .a11y_role(AccessibilityRole::Button),
            ),
            AnyView::new(Color::srgb_hex("#27272A")),
        ))
        .spacing(0.0),
    )
}

fn runtime(env: Environment, view: AnyView, theme: MinimalTestTheme) -> HeadlessRuntime {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT, theme);
    for _ in 0..4 {
        let _ = runtime.pump(false);
    }
    runtime
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

/// A chrome with `focus` and no visible surface or tint.
fn chrome_with(shape: CornerShape, focus: FocusIndicator) -> Chrome {
    Chrome::new(
        Surface::clear(shape),
        StateLayer::new(Color::srgb(0, 0, 0)),
        PressFeedback::new(),
        focus,
        Motion::uniform(Animation::linear(MOTION)),
    )
}

/// How long every transition of [`chrome_with`]'s chrome runs.
const MOTION: Duration = Duration::from_millis(15);

/// The tap control's live chrome.
fn the_chrome(runtime: &HeadlessRuntime) -> Rc<LiveChrome> {
    let chromes = runtime.renderer().hit_test.interaction.live_chromes();
    let [chrome] = <[_; 1]>::try_from(chromes).expect("exactly one control binds a live chrome");
    chrome
}

/// The M3 list-item shape morph: 8 resting, 12 hovered, 16 pressed. The
/// chrome's live silhouette takes the radii resolved against the control's
/// reported state.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "the radii are the theme's literal per-state tokens selected as-is, not interpolated values"
)]
fn surface_radii_morph_with_reported_state() {
    let mut env = test_environment();
    env.install(chrome_with(
        CornerShape::new(
            StateValue::new(RoundedRectRadii::from_single_radius(8.0))
                .when(
                    InteractionState::PRESSED,
                    RoundedRectRadii::from_single_radius(16.0),
                )
                .when(
                    InteractionState::HOVERED,
                    RoundedRectRadii::from_single_radius(12.0),
                ),
        ),
        FocusIndicator::Tint,
    ));
    // 40 pt wide, so no radius here is clamped to half the control's width.
    let view = AnyView::new(
        hstack((
            AnyView::new(Color::srgb_hex("#18181B")),
            AnyView::new(
                Color::srgb_hex("#3F3F46")
                    .width(40.0)
                    .gesture(TapGesture::new(), || {})
                    .a11y_role(AccessibilityRole::Button),
            ),
            AnyView::new(Color::srgb_hex("#27272A")),
        ))
        .spacing(0.0),
    );
    let mut runtime = runtime(env, view, MinimalTestTheme::default());
    let chrome = the_chrome(&runtime);

    let last_radii = || match chrome.shape() {
        ShapeData::RoundedRect(rect) => rect.radii().top_left,
        other => panic!("the surface is a rounded rectangle, not {other:?}"),
    };
    assert!(
        last_radii() == 8.0,
        "resting radii: left {:?}, right {:?}",
        last_radii(),
        8.0
    );

    pointer_move(&mut runtime, 400.0, 300.0);
    assert!(
        last_radii() == 12.0,
        "hovered radii: left {:?}, right {:?}",
        last_radii(),
        12.0
    );

    pointer_down(&mut runtime, 400.0, 300.0);
    assert!(
        last_radii() == 16.0,
        "pressed radii: left {:?}, right {:?}",
        last_radii(),
        16.0
    );

    pointer_up(&mut runtime, 400.0, 300.0);
    pointer_move(&mut runtime, 400.0, 300.0);
    assert!(
        last_radii() == 12.0,
        "released but still hovered: left {:?}, right {:?}",
        last_radii(),
        12.0
    );

    pointer_move(&mut runtime, 10.0, 10.0);
    assert!(
        last_radii() == 8.0,
        "back to rest: left {:?}, right {:?}",
        last_radii(),
        8.0
    );
}

/// `.interaction_state` receives the outermost interactive control's state,
/// written only on change: hover sets HOVERED, a press adds PRESSED, and
/// leaving resets to empty.
#[test]
fn interaction_report_follows_hover_and_press() {
    let mut env = test_environment();
    env.install(invisible_chrome());
    let report = nami::binding(InteractionState::empty());
    let view = AnyView::new(
        hstack((
            AnyView::new(Color::srgb_hex("#18181B")),
            AnyView::new(
                Color::srgb_hex("#3F3F46")
                    .width(20.0)
                    .gesture(TapGesture::new(), || {})
                    .interaction_state(&report),
            ),
            AnyView::new(Color::srgb_hex("#27272A")),
        ))
        .spacing(0.0),
    );
    let mut runtime = runtime(env, view, MinimalTestTheme::default());

    assert_eq!(report.snapshot(), InteractionState::empty(), "resting");

    pointer_move(&mut runtime, 400.0, 300.0);
    assert_eq!(report.snapshot(), InteractionState::HOVERED, "hovered");

    pointer_down(&mut runtime, 400.0, 300.0);
    assert_eq!(
        report.snapshot(),
        InteractionState::HOVERED | InteractionState::PRESSED,
        "pressed while hovered"
    );

    pointer_up(&mut runtime, 400.0, 300.0);
    let _ = runtime.pump(false);
    assert_eq!(report.snapshot(), InteractionState::HOVERED, "released");

    pointer_move(&mut runtime, 10.0, 10.0);
    assert_eq!(report.snapshot(), InteractionState::empty(), "pointer left");
}

/// `.selected(true)` contributes SELECTED to the reported state.
#[test]
fn selected_adds_selected_to_reported_state() {
    let mut env = test_environment();
    env.install(invisible_chrome());
    let report = nami::binding(InteractionState::empty());
    let view = AnyView::new(
        hstack((
            AnyView::new(Color::srgb_hex("#18181B")),
            AnyView::new(
                Color::srgb_hex("#3F3F46")
                    .width(20.0)
                    .gesture(TapGesture::new(), || {})
                    .selected(true)
                    .interaction_state(&report),
            ),
            AnyView::new(Color::srgb_hex("#27272A")),
        ))
        .spacing(0.0),
    );
    let mut runtime = runtime(env, view, MinimalTestTheme::default());

    assert_eq!(
        report.snapshot(),
        InteractionState::SELECTED,
        "the control reports SELECTED at rest"
    );

    pointer_move(&mut runtime, 400.0, 300.0);
    assert_eq!(
        report.snapshot(),
        InteractionState::SELECTED | InteractionState::HOVERED,
        "selected + hovered"
    );
}

/// The focus ring strokes only while the control is FOCUSED: Tab makes the
/// keyboard focus visible and paints the ring; a click focuses the control
/// without making focus visible, so no ring is painted. A fully saturated red
/// is used so a single pixel scan detects it.
fn pump_red_ring_pixels(env: Environment, activate: impl Fn(&mut HeadlessRuntime)) -> bool {
    let mut env = env;
    env.install(chrome_with(
        CornerShape::radius(8.0),
        FocusIndicator::Ring {
            width: 3.0,
            offset: 3.0,
            color: Color::srgb(255, 0, 0),
        },
    ));
    let mut runtime = runtime(env, tap_view(), MinimalTestTheme::default());
    activate(&mut runtime);
    // The ring fades in over the chrome's motion: sample it settled.
    let snapshot = runtime
        .pump_at(true, Instant::now() + MOTION)
        .into_snapshot()
        .expect("the pump captured a snapshot");
    snapshot
        .rgba8()
        .as_chunks::<4>()
        .0
        .iter()
        .any(|px| px[0] > 200 && px[1] < 80 && px[2] < 80)
}

#[test]
fn focus_ring_draws_only_for_keyboard_focus() {
    // Clicking focuses the control invisibly: no ring.
    let clicked = pump_red_ring_pixels(test_environment(), |runtime| {
        pointer_down(runtime, 400.0, 300.0);
        pointer_up(runtime, 400.0, 300.0);
    });
    assert!(!clicked, "a click must not draw the focus ring");

    // Tab makes keyboard focus visible: the ring is painted.
    let tabbed = pump_red_ring_pixels(test_environment(), |runtime| {
        key(runtime, KeyCode::Named("Tab".into()), KeyState::Pressed);
        key(runtime, KeyCode::Named("Tab".into()), KeyState::Released);
    });
    assert!(tabbed, "Tab focus must draw the focus ring");
}

/// `Selected` is announced through the accessibility tree on the control it
/// modifies.
#[cfg(feature = "accessibility")]
#[test]
fn selected_sets_the_accessibility_selected_trait() {
    use accesskit::Role;

    let mut env = test_environment();
    env.install(invisible_chrome());
    let view = AnyView::new(
        hstack((
            AnyView::new(Color::srgb_hex("#18181B")),
            AnyView::new(
                Color::srgb_hex("#3F3F46")
                    .width(20.0)
                    .gesture(TapGesture::new(), || {})
                    .a11y_role(AccessibilityRole::Button)
                    .a11y_label("handle")
                    .selected(true),
            ),
            AnyView::new(Color::srgb_hex("#27272A")),
        ))
        .spacing(0.0),
    );
    // The initial accessibility tree emits on the first pump, so this test
    // builds its runtime directly rather than going through `runtime()`,
    // whose warm-up pumps would consume it.
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        env,
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );

    let mut update = None;
    for _ in 0..64 {
        if let Some(tree) = runtime.pump(false).into_tree_update() {
            update = Some(tree);
            break;
        }
    }
    let update = update.expect("the pump produced a tree update");
    let node = update
        .nodes
        .iter()
        .map(|(_, node)| node)
        .find(|node| node.role() == Role::Button)
        .expect("the tap control publishes a Button node");
    assert_eq!(
        node.is_selected(),
        Some(true),
        "Selected must set the selected trait"
    );
}

struct TestButtonStyle {
    state_out: Rc<RefCell<Option<nami::Computed<InteractionState>>>>,
}

impl waterui::component::button::CustomButtonStyle for TestButtonStyle {
    fn make_body<L: waterui::View>(
        &self,
        configuration: waterui::component::button::ButtonStyleConfiguration<L>,
    ) -> impl waterui::View + use<L> {
        *self.state_out.borrow_mut() = Some(configuration.interaction_state().clone());
        configuration
            .into_label()
            .width(120.0)
            .height(44.0)
            .background(Color::srgb_hex("#2563EB"))
    }
}

struct TestToggleStyle {
    state_out: Rc<RefCell<Option<nami::Computed<InteractionState>>>>,
}

impl waterui::component::toggle::CustomToggleStyle for TestToggleStyle {
    fn make_body<L: waterui::View>(
        &self,
        configuration: waterui::component::toggle::ToggleStyleConfiguration<L>,
    ) -> impl waterui::View + use<L> {
        *self.state_out.borrow_mut() = Some(configuration.interaction_state().clone());
        let bg = nami::SignalExt::select(
            configuration.is_on(),
            Color::srgb_hex("#16A34A"),
            Color::srgb_hex("#52525B"),
        );
        configuration
            .into_label()
            .width(120.0)
            .height(44.0)
            .background(bg)
    }
}

#[test]
fn custom_button_and_toggle_styles_report_interaction_without_chrome() {
    use core::cell::Cell;
    use nami::{Binding, Signal as _, binding};
    use waterui::component::button::button;
    use waterui::component::toggle::Toggle;

    let button_state = Rc::new(RefCell::new(None));
    let toggle_state = Rc::new(RefCell::new(None));
    let clicks = Rc::new(Cell::new(0u32));
    let toggled = Binding::bool(false);
    let disabled = binding(false);

    let view = AnyView::new(
        hstack((
            button("Act")
                .action({
                    let clicks = Rc::clone(&clicks);
                    move || clicks.set(clicks.get() + 1)
                })
                .style(TestButtonStyle {
                    state_out: Rc::clone(&button_state),
                })
                .disabled(disabled.clone()),
            Toggle::new("Switch", &toggled).style(TestToggleStyle {
                state_out: Rc::clone(&toggle_state),
            }),
        ))
        .spacing(20.0),
    );

    // Deliberately no Chrome installed in env: custom styles own their surface.
    let mut rt = runtime(test_environment(), view, MinimalTestTheme::default());
    let btn_state = button_state
        .borrow_mut()
        .take()
        .expect("custom button style ran");
    let tgl_state = toggle_state
        .borrow_mut()
        .take()
        .expect("custom toggle style ran");

    assert_eq!(btn_state.snapshot(), InteractionState::empty());
    assert_eq!(tgl_state.snapshot(), InteractionState::empty());

    // Hover -> press -> release over first control (centered horizontally in 800pt window:
    // total hstack width = 120 + 20 + 120 = 260, so button is x=270..390, toggle x=410..530).
    pointer_move(&mut rt, 330.0, 300.0);
    assert!(btn_state.snapshot().contains(InteractionState::HOVERED));
    assert!(!btn_state.snapshot().contains(InteractionState::PRESSED));

    pointer_down(&mut rt, 330.0, 300.0);
    assert!(btn_state.snapshot().contains(InteractionState::HOVERED));
    assert!(btn_state.snapshot().contains(InteractionState::PRESSED));

    pointer_up(&mut rt, 330.0, 300.0);
    let _ = rt.pump(false);
    assert_eq!(clicks.get(), 1);
    assert!(!btn_state.snapshot().contains(InteractionState::PRESSED));

    // Toggle sits at x=410..530.
    pointer_move(&mut rt, 470.0, 300.0);
    assert!(tgl_state.snapshot().contains(InteractionState::HOVERED));
    pointer_down(&mut rt, 470.0, 300.0);
    assert!(tgl_state.snapshot().contains(InteractionState::PRESSED));
    pointer_up(&mut rt, 470.0, 300.0);
    let _ = rt.pump(false);
    assert!(toggled.snapshot(), "clicking custom toggle flips is_on");

    // Keyboard Tab focuses button and reports FOCUSED.
    pointer_move(&mut rt, 10.0, 10.0);
    key(&mut rt, KeyCode::Named("Tab".into()), KeyState::Pressed);
    key(&mut rt, KeyCode::Named("Tab".into()), KeyState::Released);
    assert!(
        btn_state.snapshot().contains(InteractionState::FOCUSED),
        "Tab must report FOCUSED on custom-styled button"
    );

    // Disabling suppresses hover/click and surfaces DISABLED.
    disabled.set(true);
    let _ = rt.pump(false);
    assert!(btn_state.snapshot().contains(InteractionState::DISABLED));
    pointer_down(&mut rt, 330.0, 300.0);
    pointer_up(&mut rt, 330.0, 300.0);
    let _ = rt.pump(false);
    assert_eq!(clicks.get(), 1, "disabled custom button must not fire");
}
