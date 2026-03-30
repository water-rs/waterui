mod support;

use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::graphics::color::Srgb;
use waterui::{Binding, Str};
use waterui_controls::{Slider, Stepper, TextField, Toggle, button, toggle};
use waterui_testing::{Role, Selector};

use support::{control_shell, mount_view, snapshot_suite};

const SUITE: &str = "semantics";

fn assert_close(actual: f64, expected: f64, epsilon: f64, context: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= epsilon,
        "{context}: expected {expected:.4}, got {actual:.4}, delta={delta:.4}, epsilon={epsilon:.4}"
    );
}

#[test]
fn button_tap_triggers_action() {
    let suite = snapshot_suite(SUITE);
    let count = Binding::i32(0);
    let count_for_view = count.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            button("Increment")
                .action(|waterui::State(count): waterui::State<Binding<i32>>| {
                    count.set(count.get() + 1);
                })
                .state(&count_for_view),
            waterui::text!("count:{count_for_view}").foreground(Srgb::WHITE),
        )))
    });

    let initial = app.capture_snapshot(&suite, "button-tap-triggers-action", "00_initial");
    assert!(
        initial.path().is_file(),
        "button-tap-triggers-action: initial snapshot missing"
    );

    assert!(
        app.query().role(Role::BUTTON).label("Increment").tap(),
        "button tap should succeed"
    );
    assert_eq!(count.get(), 1, "button tap should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:1")
        .assert_exists();

    let tapped = app.capture_snapshot(&suite, "button-tap-triggers-action", "01_tapped");
    assert!(
        tapped.path().is_file(),
        "button-tap-triggers-action: tapped snapshot missing"
    );
    assert!(
        initial.snapshot().changed_pixels(tapped.snapshot()) > 0,
        "button tap should change rendered pixels"
    );
}

#[test]
fn button_disabled_state_is_accessible() {
    let suite = snapshot_suite(SUITE);
    let mut app = mount_view(|| control_shell(button("Disabled").disabled(true)));

    let snapshot =
        app.capture_snapshot(&suite, "button-disabled-state-is-accessible", "00_initial");
    assert!(
        snapshot.path().is_file(),
        "button-disabled-state-is-accessible: snapshot missing"
    );

    let element = app.query().role(Role::BUTTON).label("Disabled").single();
    assert!(
        !element.node().enabled(),
        "button-disabled-state-is-accessible: button should expose disabled accessibility state"
    );
}

#[test]
fn button_label_is_accessible() {
    let suite = snapshot_suite(SUITE);
    let mut app = mount_view(|| control_shell(button("Submit")));

    let snapshot = app.capture_snapshot(&suite, "button-label-is-accessible", "00_initial");
    assert!(
        snapshot.path().is_file(),
        "button-label-is-accessible: snapshot missing"
    );

    app.query()
        .role(Role::BUTTON)
        .label("Submit")
        .assert_exists();
}

#[test]
fn toggle_tap_toggles_binding() {
    let suite = snapshot_suite(SUITE);
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            toggle("Airplane Mode", &enabled_for_view),
            waterui::text!("enabled:{enabled_for_view}").foreground(Srgb::WHITE),
        )))
    });

    let initial = app.capture_snapshot(&suite, "toggle-tap-toggles-binding", "00_initial");
    assert!(
        initial.path().is_file(),
        "toggle-tap-toggles-binding: initial snapshot missing"
    );

    app.query()
        .role(Role::SWITCH)
        .label("Airplane Mode")
        .checked(false)
        .assert_exists();
    assert!(
        app.query().role(Role::SWITCH).label("Airplane Mode").tap(),
        "toggle tap should succeed"
    );
    assert!(enabled.get(), "toggle tap should flip binding");
    app.query()
        .role(Role::SWITCH)
        .label("Airplane Mode")
        .checked(true)
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("enabled:true")
        .assert_exists();

    let toggled = app.capture_snapshot(&suite, "toggle-tap-toggles-binding", "01_toggled");
    assert!(
        toggled.path().is_file(),
        "toggle-tap-toggles-binding: toggled snapshot missing"
    );
    assert!(
        initial.snapshot().changed_pixels(toggled.snapshot()) > 0,
        "toggle tap should change rendered pixels"
    );
}

#[test]
fn toggle_accessibility_role_is_switch() {
    let suite = snapshot_suite(SUITE);
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();

    let mut app = mount_view(move || control_shell(Toggle::new(&enabled_for_view).label("Wi-Fi")));

    let snapshot =
        app.capture_snapshot(&suite, "toggle-accessibility-role-is-switch", "00_initial");
    assert!(
        snapshot.path().is_file(),
        "toggle-accessibility-role-is-switch: snapshot missing"
    );

    app.query()
        .role(Role::SWITCH)
        .label("Wi-Fi")
        .checked(false)
        .assert_exists();
}

#[test]
fn slider_increment_decrement_updates_value() {
    let suite = snapshot_suite(SUITE);
    let value = Binding::f64(0.50);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            Slider::new(0.0..=1.0, &value_for_view).label("Volume"),
            waterui::text!("value:{value_for_view:.2}").foreground(Srgb::WHITE),
        )))
    });

    let initial = app.capture_snapshot(
        &suite,
        "slider-increment-decrement-updates-value",
        "00_initial",
    );
    assert!(
        initial.path().is_file(),
        "slider-increment-decrement-updates-value: initial snapshot missing"
    );

    assert!(
        app.query().role(Role::SLIDER).label("Volume").increment(),
        "slider increment should succeed"
    );
    assert_close(
        value.get(),
        0.51,
        0.0001,
        "slider increment should update binding",
    );
    app.query()
        .role(Role::LABEL)
        .label("value:0.51")
        .assert_exists();

    let incremented = app.capture_snapshot(
        &suite,
        "slider-increment-decrement-updates-value",
        "01_incremented",
    );
    assert!(
        incremented.path().is_file(),
        "slider-increment-decrement-updates-value: incremented snapshot missing"
    );

    assert!(
        app.query().role(Role::SLIDER).label("Volume").decrement(),
        "slider decrement should succeed"
    );
    assert_close(
        value.get(),
        0.50,
        0.0001,
        "slider decrement should update binding",
    );
    app.query()
        .role(Role::LABEL)
        .label("value:0.50")
        .assert_exists();

    let decremented = app.capture_snapshot(
        &suite,
        "slider-increment-decrement-updates-value",
        "02_decremented",
    );
    assert!(
        decremented.path().is_file(),
        "slider-increment-decrement-updates-value: decremented snapshot missing"
    );
    assert!(
        initial.snapshot().changed_pixels(incremented.snapshot()) > 0,
        "slider increment should change rendered pixels"
    );
}

#[test]
fn slider_accessibility_role_is_slider() {
    let suite = snapshot_suite(SUITE);
    let value = Binding::f64(0.25);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(Slider::new(0.0..=1.0, &value_for_view).label("Exposure"))
    });

    let snapshot =
        app.capture_snapshot(&suite, "slider-accessibility-role-is-slider", "00_initial");
    assert!(
        snapshot.path().is_file(),
        "slider-accessibility-role-is-slider: snapshot missing"
    );

    let element = app.query().role(Role::SLIDER).label("Exposure").single();
    let numeric = element
        .node()
        .value()
        .expect("slider should expose a numeric accessibility value")
        .parse::<f64>()
        .expect("slider accessibility value should parse as f64");
    assert_close(
        numeric,
        0.25,
        0.0001,
        "slider accessibility value should match binding",
    );
}

#[test]
fn stepper_increment_decrement_updates_binding() {
    let suite = snapshot_suite(SUITE);
    let value = Binding::i32(2);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            Stepper::new(&value_for_view).label("Quantity"),
            waterui::text!("count:{value_for_view}").foreground(Srgb::WHITE),
        )))
    });

    let initial = app.capture_snapshot(
        &suite,
        "stepper-increment-decrement-updates-binding",
        "00_initial",
    );
    assert!(
        initial.path().is_file(),
        "stepper-increment-decrement-updates-binding: initial snapshot missing"
    );

    assert!(
        app.query().label("Quantity").value("2").increment(),
        "stepper increment should succeed"
    );
    assert_eq!(value.get(), 3, "stepper increment should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:3")
        .assert_exists();

    let incremented = app.capture_snapshot(
        &suite,
        "stepper-increment-decrement-updates-binding",
        "01_incremented",
    );
    assert!(
        incremented.path().is_file(),
        "stepper-increment-decrement-updates-binding: incremented snapshot missing"
    );

    assert!(
        app.query().label("Quantity").value("3").decrement(),
        "stepper decrement should succeed"
    );
    assert_eq!(value.get(), 2, "stepper decrement should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:2")
        .assert_exists();

    let decremented = app.capture_snapshot(
        &suite,
        "stepper-increment-decrement-updates-binding",
        "02_decremented",
    );
    assert!(
        decremented.path().is_file(),
        "stepper-increment-decrement-updates-binding: decremented snapshot missing"
    );
    assert!(
        initial.snapshot().changed_pixels(incremented.snapshot()) > 0,
        "stepper increment should change rendered pixels"
    );
}

#[test]
fn stepper_respects_range_bounds() {
    let suite = snapshot_suite(SUITE);
    let value = Binding::i32(2);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(Stepper::new(&value_for_view).label("Limited").range(0..=2))
    });

    let initial = app.capture_snapshot(&suite, "stepper-respects-range-bounds", "00_initial");
    assert!(
        initial.path().is_file(),
        "stepper-respects-range-bounds: initial snapshot missing"
    );

    assert!(
        !app.query().label("Limited").value("2").increment(),
        "stepper increment at max should report no change"
    );
    assert_eq!(value.get(), 2, "stepper value should remain clamped at max");

    let bounded = app.capture_snapshot(&suite, "stepper-respects-range-bounds", "01_after-max");
    assert!(
        bounded.path().is_file(),
        "stepper-respects-range-bounds: bounded snapshot missing"
    );
}

#[test]
fn text_field_set_text_updates_binding() {
    let suite = snapshot_suite(SUITE);
    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            TextField::new(&value_for_view).label("Name"),
            waterui::text!("value:{value_for_view}").foreground(Srgb::WHITE),
        )))
    });

    let initial = app.capture_snapshot(&suite, "text-field-set-text-updates-binding", "00_initial");
    assert!(
        initial.path().is_file(),
        "text-field-set-text-updates-binding: initial snapshot missing"
    );

    assert!(
        app.query()
            .role(Role::TEXT_INPUT)
            .label("Name")
            .set_text("Alice"),
        "text field set_text should succeed"
    );
    assert_eq!(
        value.get(),
        Str::from("Alice"),
        "text field should update binding"
    );
    app.query()
        .role(Role::LABEL)
        .label("value:Alice")
        .assert_exists();

    let edited = app.capture_snapshot(&suite, "text-field-set-text-updates-binding", "01_edited");
    assert!(
        edited.path().is_file(),
        "text-field-set-text-updates-binding: edited snapshot missing"
    );
    assert!(
        initial.snapshot().changed_pixels(edited.snapshot()) > 0,
        "text field edit should change rendered pixels"
    );
}

#[test]
fn text_field_focus_updates_ui_focus() {
    let suite = snapshot_suite(SUITE);
    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();

    let mut app =
        mount_view(move || control_shell(TextField::new(&value_for_view).label("Search")));

    let initial = app.capture_snapshot(&suite, "text-field-focus-updates-ui-focus", "00_initial");
    assert!(
        initial.path().is_file(),
        "text-field-focus-updates-ui-focus: initial snapshot missing"
    );

    let selector = Selector::default().role(Role::TEXT_INPUT).label("Search");
    assert!(
        app.query().role(Role::TEXT_INPUT).label("Search").focus(),
        "text field focus should succeed"
    );
    app.assert_ui_focus(selector);

    let focused = app.capture_snapshot(&suite, "text-field-focus-updates-ui-focus", "01_focused");
    assert!(
        focused.path().is_file(),
        "text-field-focus-updates-ui-focus: focused snapshot missing"
    );
}
