//! Per-state interaction styling (waterui#1270): the renderer computes a
//! control's `InteractionState` from the hover/press/focus bookkeeping it
//! already keeps plus `Selected`/`Disabled` metadata, resolves
//! `InteractionStyle::state_layer_radii` and `label_color` with it, strokes
//! `focus_ring` only while the state is FOCUSED (keyboard focus-visible), and
//! writes `.interaction_state` report bindings on change.

use std::cell::RefCell;
use std::rc::Rc;

use kurbo::RoundedRectRadii;
use nami::Signal as _;
use waterui::accessibility::AccessibilityRole;
use waterui::gesture::TapGesture;
use waterui::{AnyView, Color, ViewExt as _};
use waterui_backend_core::widget::{ButtonMetrics, FocusRing, InteractionStyle};
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::{InteractionState, StateValue};
use waterui_layout::stack::hstack;

use super::{MinimalTestTheme, test_environment};
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

/// The M3 list-item shape morph: 8 resting, 12 hovered, 16 pressed. The theme
/// sees the radii resolved against the control's reported state.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "each reported state maps to an exact theme radius token"
)]
fn state_layer_radii_morph_with_reported_state() {
    let mut env = test_environment();
    env.install(
        InteractionStyle::new(
            ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
            Color::srgb(0, 0, 0),
            8.0_f64,
        )
        .state_layer_radii(
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
    );
    let draws = Rc::new(RefCell::new(Vec::new()));
    let theme = MinimalTestTheme {
        state_layer_draws: Rc::clone(&draws),
        ..Default::default()
    };
    let mut runtime = runtime(env, tap_view(), theme);

    let last_radii = || {
        draws
            .borrow()
            .last()
            .map(|(_, radii)| radii.top_left)
            .expect("the state layer draws every frame")
    };
    assert_eq!(last_radii(), 8.0, "resting radii");

    pointer_move(&mut runtime, 400.0, 300.0);
    assert_eq!(last_radii(), 12.0, "hovered radii");

    pointer_down(&mut runtime, 400.0, 300.0);
    assert_eq!(last_radii(), 16.0, "pressed radii");

    pointer_up(&mut runtime, 400.0, 300.0);
    pointer_move(&mut runtime, 400.0, 300.0);
    assert_eq!(last_radii(), 12.0, "released but still hovered");

    pointer_move(&mut runtime, 10.0, 10.0);
    assert_eq!(last_radii(), 8.0, "back to rest");
}

/// `.interaction_state` receives the outermost interactive control's state,
/// written only on change: hover sets HOVERED, a press adds PRESSED, and
/// leaving resets to empty.
#[test]
fn interaction_report_follows_hover_and_press() {
    let mut env = test_environment();
    env.install(InteractionStyle::new(
        ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
        Color::srgb(0, 0, 0),
        8.0_f64,
    ));
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
    env.install(InteractionStyle::new(
        ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
        Color::srgb(0, 0, 0),
        8.0_f64,
    ));
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
    env.install(
        InteractionStyle::new(
            ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
            Color::srgb(0, 0, 0),
            8.0_f64,
        )
        .focus_ring(FocusRing {
            color: Color::srgb(255, 0, 0),
            width: 3.0,
            offset: 3.0,
        }),
    );
    let mut runtime = runtime(env, tap_view(), MinimalTestTheme::default());
    activate(&mut runtime);
    let snapshot = runtime
        .pump(true)
        .snapshot
        .expect("the pump captured a snapshot");
    snapshot
        .rgba8
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

/// `label_color` resolves against the reported state: a DISABLED button label
/// takes the disabled override, an enabled one the resting value.
#[test]
fn label_color_resolves_disabled() {
    let theme: Rc<dyn crate::engine::WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let style = InteractionStyle::new(
        ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
        Color::srgb(0, 0, 0),
        8.0_f64,
    )
    .label_colors(Color::srgb(0, 0, 255), Color::srgb(128, 128, 128));
    let env = test_environment();

    let resolve = |state| {
        crate::widgets::controls::button::button_label_color(
            &theme,
            waterui_controls::button::ButtonStyle::Automatic,
            state,
            Some(&style),
            None,
        )
        .expect("the style overrides the label color")
        .resolve(&env)
        .snapshot()
    };
    let to_srgb = waterui_graphics::color::working::to_srgb;

    let channel8 = |v: f32| crate::num_cast::f32_as_u8((v * 255.0).round());
    let enabled = to_srgb(resolve(InteractionState::empty()));
    assert_eq!(
        (
            channel8(enabled.red),
            channel8(enabled.green),
            channel8(enabled.blue)
        ),
        (0, 0, 255),
        "enabled label color"
    );

    let disabled = to_srgb(resolve(InteractionState::DISABLED));
    assert_eq!(
        (
            channel8(disabled.red),
            channel8(disabled.green),
            channel8(disabled.blue)
        ),
        (128, 128, 128),
        "disabled label color"
    );
}

/// The retained (non-title) label resolves `label_color` reactively through
/// the full `InteractionState`, not just DISABLED: writing HOVERED to the
/// control's state binding recolors it without a rebuild.
#[test]
fn retained_label_color_resolves_hovered() {
    let theme: Rc<dyn crate::engine::WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let style = InteractionStyle::new(
        ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
        Color::srgb(0, 0, 0),
        8.0_f64,
    )
    .label_color(
        StateValue::new(Some(Color::srgb(0, 0, 255)))
            .when(InteractionState::HOVERED, Some(Color::srgb(255, 0, 0))),
    );
    let env = test_environment();
    let state = nami::Binding::container(InteractionState::empty());
    let color = crate::widgets::controls::button::state_aware_label_color(
        &theme,
        waterui_controls::button::ButtonStyle::Automatic,
        &state,
        Some(&style),
        None,
    )
    .expect("the style overrides the label color");
    let resolved = color.resolve(&env);
    let channel8 = |v: f32| crate::num_cast::f32_as_u8((v * 255.0).round());
    let rgb = |color: cherenkov::WorkingColor| {
        let srgb = waterui_graphics::color::working::to_srgb(color);
        (
            channel8(srgb.red),
            channel8(srgb.green),
            channel8(srgb.blue),
        )
    };

    assert_eq!(rgb(resolved.snapshot()), (0, 0, 255), "resting");

    state.set(InteractionState::HOVERED);
    assert_eq!(rgb(resolved.snapshot()), (255, 0, 0), "hovered override");

    state.set(InteractionState::empty());
    assert_eq!(rgb(resolved.snapshot()), (0, 0, 255), "back to resting");
}

/// `Selected` is announced through the accessibility tree on the control it
/// modifies.
#[cfg(feature = "accessibility")]
#[test]
fn selected_sets_the_accessibility_selected_trait() {
    use accesskit::Role;

    let mut env = test_environment();
    env.install(InteractionStyle::new(
        ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
        Color::srgb(0, 0, 0),
        8.0_f64,
    ));
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
        if let Some(tree) = runtime.pump(false).tree_update {
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
