//! End-to-end semantics coverage for controls.

mod support;

use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::graphics::color::Srgb;
use waterui::{Binding, Str};
use waterui_controls::{Label, Menu, Slider, Stepper, TextField, Toggle, button, toggle};
use waterui_icon::SystemIcon;
use waterui_testing::{Role, Selector};

use support::{control_shell, mount_view};

fn assert_close(actual: f64, expected: f64, epsilon: f64, context: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= epsilon,
        "{context}: expected {expected:.4}, got {actual:.4}, delta={delta:.4}, epsilon={epsilon:.4}"
    );
}

#[test]
fn button_tap_triggers_action() {
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

    assert!(
        app.query().role(Role::BUTTON).label("Increment").tap(),
        "button tap should succeed"
    );
    assert_eq!(count.get(), 1, "button tap should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:1")
        .assert_exists();
}

#[test]
fn button_disabled_state_is_accessible() {
    let mut app = mount_view(|| control_shell(button("Disabled").disabled(true)));

    let element = app.query().role(Role::BUTTON).label("Disabled").single();
    assert!(
        !element.node().enabled(),
        "button-disabled-state-is-accessible: button should expose disabled accessibility state"
    );
}

#[test]
fn button_label_is_accessible() {
    let mut app = mount_view(|| control_shell(button("Submit")));

    app.query()
        .role(Role::BUTTON)
        .label("Submit")
        .assert_exists();
}

#[test]
fn toggle_tap_toggles_binding() {
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            toggle("Airplane Mode", &enabled_for_view),
            waterui::text!("enabled:{enabled_for_view}").foreground(Srgb::WHITE),
        )))
    });

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
}

#[test]
fn toggle_accessibility_role_is_switch() {
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();

    let mut app = mount_view(move || control_shell(Toggle::new(&enabled_for_view).label("Wi-Fi")));

    app.query()
        .role(Role::SWITCH)
        .label("Wi-Fi")
        .checked(false)
        .assert_exists();
}

#[test]
fn slider_increment_decrement_updates_value() {
    let value = Binding::f64(0.50);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            Slider::new(0.0..=1.0, &value_for_view).label("Volume"),
            waterui::text!("value:{value_for_view:.2}").foreground(Srgb::WHITE),
        )))
    });

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
}

#[test]
fn slider_accessibility_role_is_slider() {
    let value = Binding::f64(0.25);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(Slider::new(0.0..=1.0, &value_for_view).label("Exposure"))
    });

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
    let value = Binding::i32(2);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            Stepper::new(&value_for_view).label("Quantity"),
            waterui::text!("count:{value_for_view}").foreground(Srgb::WHITE),
        )))
    });

    assert!(
        app.query().label("Quantity").value("2").increment(),
        "stepper increment should succeed"
    );
    assert_eq!(value.get(), 3, "stepper increment should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:3")
        .assert_exists();

    assert!(
        app.query().label("Quantity").value("3").decrement(),
        "stepper decrement should succeed"
    );
    assert_eq!(value.get(), 2, "stepper decrement should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:2")
        .assert_exists();
}

#[test]
fn stepper_respects_range_bounds() {
    let value = Binding::i32(2);
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(Stepper::new(&value_for_view).label("Limited").range(0..=2))
    });

    assert!(
        !app.query().label("Limited").value("2").increment(),
        "stepper increment at max should report no change"
    );
    assert_eq!(value.get(), 2, "stepper value should remain clamped at max");
}

#[test]
fn text_field_set_text_updates_binding() {
    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();

    let mut app = mount_view(move || {
        control_shell(vstack((
            TextField::new(&value_for_view).label("Name"),
            waterui::text!("value:{value_for_view}").foreground(Srgb::WHITE),
        )))
    });

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
}

#[test]
fn text_field_focus_updates_ui_focus() {
    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();

    let mut app =
        mount_view(move || control_shell(TextField::new(&value_for_view).label("Search")));

    let selector = Selector::default().role(Role::TEXT_INPUT).label("Search");
    assert!(
        app.query().role(Role::TEXT_INPUT).label("Search").focus(),
        "text field focus should succeed"
    );
    app.assert_ui_focus(&selector);
}

#[test]
fn icon_only_label_preserves_button_accessible_name() {
    let mut app = mount_view(|| {
        control_shell(button(
            Label::new("Search")
                .system_icon(SystemIcon::SEARCH)
                .icon_only(),
        ))
    });

    app.query()
        .role(Role::BUTTON)
        .label("Search")
        .assert_exists();
}

#[test]
fn menu_button_exposes_accessible_name() {
    let mut app = mount_view(|| {
        control_shell(Menu::new(
            Label::new("Actions").system_icon(SystemIcon::SEARCH),
            (
                button("Refresh").action(|| {}),
                Menu::new("Advanced", (button("Archive").action(|| {}),)),
            ),
        ))
    });

    let menu = app.query().role(Role::BUTTON).label("Actions").single();
    let bounds = menu.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "menu-button-exposes-accessible-name: menu trigger bounds must be non-zero"
    );
}
