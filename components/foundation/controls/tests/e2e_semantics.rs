//! End-to-end semantics coverage for controls.

mod support;

use core::num::NonZeroUsize;

use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::graphics::color::Srgb;
use nami::SignalExt as _;
use waterui::{Binding, Str};
use waterui_controls::{
    Menu, TextField, Toggle, button, label, slider::slider, stepper::stepper, toggle,
};
use waterui_testing::{Role, SemanticApp, Selector, UiBuilder};

use support::control_shell;

/// Asserts that an interaction panics because the runtime rejected it —
/// the contract for actions on disabled or clamped controls.
fn assert_rejected(context: &str, action: impl FnOnce()) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action));
    assert!(
        outcome.is_err(),
        "{context}: the runtime should reject this action"
    );
}

fn assert_close(actual: f64, expected: f64, epsilon: f64, context: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= epsilon,
        "{context}: expected {expected:.4}, got {actual:.4}, delta={delta:.4}, epsilon={epsilon:.4}"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn button_tap_triggers_action(ui: UiBuilder) {
    let count = Binding::i32(0);
    let count_for_view = count.clone();

    let mut app = ui.mount(move || {
        control_shell(vstack((
            button("Increment")
                .action(|waterui::State(count): waterui::State<Binding<i32>>| {
                    *count.get_mut() += 1;
                })
                .state(&count_for_view),
            waterui::text!("count:{count_for_view}").foreground(Srgb::WHITE),
        )))
    });

    app.query().role(Role::BUTTON).label("Increment").tap();
    assert_eq!(count.get(), 1, "button tap should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:1")
        .assert_exists();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn disabled_scope_reaches_every_control_in_the_subtree(ui: UiBuilder) {
    // No control carries a `disabled` field: `.disabled(...)` installs a scoped
    // environment attribute and each control reads the state in force at its
    // own position. A scope on the container must therefore reach a stepper and
    // a text field just as it reaches a button, toggle and slider.
    let value = Binding::i32(0);
    let text = Binding::container(Str::from("hello"));
    let flag = Binding::bool(false);
    let amount = Binding::f64(0.5);

    let mut app = ui.mount(move || {
        control_shell(
            vstack((
                button("Act").action(|| {}),
                toggle("Flag", &flag),
                slider(label("Amount"), &amount),
                stepper("Count", &value),
                TextField::new(&text).label("Note"),
            ))
            .disabled(true),
        )
    });

    for (role, label) in [
        (Some(Role::BUTTON), "Act"),
        (Some(Role::SWITCH), "Flag"),
        (Some(Role::SLIDER), "Amount"),
        // The stepper reports accesskit's spin-button role, which the testing
        // harness has no `Role` constant for yet, so it is matched by label.
        (None, "Count"),
        (Some(Role::TEXT_INPUT), "Note"),
    ] {
        let query = app.query();
        let query = match role {
            Some(role) => query.role(role),
            None => query,
        };
        let element = query.label(label).single();
        assert!(
            !element.node().enabled(),
            "{label}: an enclosing .disabled(...) scope must disable this control"
        );
    }
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn repeated_state_calls_bind_in_argument_order(ui: UiBuilder) {
    let first = Binding::i32(0);
    let second = Binding::i32(0);
    let first_for_view = first.clone();
    let second_for_view = second.clone();

    let mut app = ui.mount(move || {
        control_shell(
            button("Bind")
                .action(
                    |waterui::State(first): waterui::State<Binding<i32>>,
                     waterui::State(second): waterui::State<Binding<i32>>| {
                        first.set(1);
                        second.set(2);
                    },
                )
                .state(&first_for_view)
                .state(&second_for_view),
        )
    });

    app.query().role(Role::BUTTON).label("Bind").tap();

    assert_eq!(
        first.get(),
        1,
        "the first `State` parameter must bind to the first `.state()` call"
    );
    assert_eq!(
        second.get(),
        2,
        "the second `State` parameter must bind to the second `.state()` call"
    );
}

fn disabled_button_view() -> impl waterui::View {
    control_shell(button("Disabled").disabled(true))
}

#[waterui::test(disabled_button_view, theme = hydrolysis_m3::install, viewport = (320, 240))]
fn button_disabled_state_is_accessible(app: &mut SemanticApp) {
    let element = app.query().role(Role::BUTTON).label("Disabled").single();
    assert!(
        !element.node().enabled(),
        "button-disabled-state-is-accessible: button should expose disabled accessibility state"
    );
}

fn submit_button_view() -> impl waterui::View {
    control_shell(button("Submit"))
}

#[waterui::test(submit_button_view, theme = hydrolysis_m3::install, viewport = (320, 240))]
fn button_label_is_accessible(app: &mut SemanticApp) {
    app.query()
        .role(Role::BUTTON)
        .label("Submit")
        .assert_exists();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn toggle_tap_toggles_binding(ui: UiBuilder) {
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();

    let mut app = ui.mount(move || {
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
    app.query().role(Role::SWITCH).label("Airplane Mode").tap();
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

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn toggle_accessibility_role_is_switch(ui: UiBuilder) {
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled;

    let mut app = ui.mount(move || control_shell(Toggle::new(&enabled_for_view).label("Wi-Fi")));

    app.query()
        .role(Role::SWITCH)
        .label("Wi-Fi")
        .checked(false)
        .assert_exists();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn slider_increment_decrement_updates_value(ui: UiBuilder) {
    let value = Binding::f64(0.50);
    let value_for_view = value.clone();

    let mut app = ui.mount(move || {
        control_shell(vstack((
            slider("Volume", &value_for_view),
            waterui::text!("value:{value_for_view:.2}").foreground(Srgb::WHITE),
        )))
    });

    app.query().role(Role::SLIDER).label("Volume").increment();
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

    app.query().role(Role::SLIDER).label("Volume").decrement();
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

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn slider_accessibility_role_is_slider(ui: UiBuilder) {
    let value = Binding::f64(0.25);
    let value_for_view = value;

    let mut app = ui.mount(move || control_shell(slider("Exposure", &value_for_view)));

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

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn stepper_increment_decrement_updates_binding(ui: UiBuilder) {
    let value = Binding::i32(2);
    let value_for_view = value.clone();

    let mut app = ui.mount(move || {
        control_shell(vstack((
            stepper("Quantity", &value_for_view),
            waterui::text!("count:{value_for_view}").foreground(Srgb::WHITE),
        )))
    });

    app.query().label("Quantity").value("2").increment();
    assert_eq!(value.get(), 3, "stepper increment should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:3")
        .assert_exists();

    app.query().label("Quantity").value("3").decrement();
    assert_eq!(value.get(), 2, "stepper decrement should update binding");
    app.query()
        .role(Role::LABEL)
        .label("count:2")
        .assert_exists();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn stepper_respects_range_bounds(ui: UiBuilder) {
    let value = Binding::i32(2);
    let value_for_view = value.clone();

    let mut app =
        ui.mount(move || control_shell(stepper("Limited", &value_for_view).range(0..=2)));

    assert_rejected("stepper increment at max should report no change", || {
        app.query().label("Limited").value("2").increment();
    });
    assert_eq!(value.get(), 2, "stepper value should remain clamped at max");
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn text_field_set_text_updates_binding(ui: UiBuilder) {
    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();

    let mut app = ui.mount(move || {
        control_shell(vstack((
            TextField::new(&value_for_view).label("Name"),
            waterui::text!("value:{value_for_view}").foreground(Srgb::WHITE),
        )))
    });

    app.query()
            .role(Role::TEXT_INPUT)
            .label("Name")
            .set_text("Alice");
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

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn text_field_focus_updates_ui_focus(ui: UiBuilder) {
    let value = Binding::container(Str::from(""));
    let value_for_view = value;

    let mut app =
        ui.mount(move || control_shell(TextField::new(&value_for_view).label("Search")));

    let selector = Selector::default().role(Role::TEXT_INPUT).label("Search");
    app.query().role(Role::TEXT_INPUT).label("Search").focus();
    app.assert_ui_focus(&selector);
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn multi_line_text_field_accepts_newlines_up_to_its_limit(ui: UiBuilder) {
    // A multi-line field must be reachable through the public API: `line_limit`
    // takes any `NonZeroUsize`, and a field with a limit above one both reports
    // the multi-line accessibility role and accepts newline input, refusing only
    // the edit that would push it past the limit.
    let value = Binding::container(Str::from(""));
    let observed = value.clone();

    let mut app = ui.mount(move || {
        control_shell(
            TextField::new(&value)
                .label("Notes")
                .line_limit(NonZeroUsize::new(2).expect("two lines")),
        )
    });

    // A multi-line field reports accesskit's multi-line text-input role, which
    // the testing harness has no `Role` constant for yet, so it is matched by
    // label — the newline behaviour below is what actually pins the contract.
    app.query().label("Notes").focus();

    app.press_character_key("a");
    app.press_named_key("Enter");
    app.press_character_key("b");
    assert_eq!(
        observed.get().as_str(),
        "a\nb",
        "a multi-line field must accept a newline"
    );

    // The third line exceeds `line_limit(2)`, so the edit is refused and the
    // existing value survives intact.
    app.press_named_key("Enter");
    app.press_character_key("c");
    assert_eq!(
        observed.get().as_str(),
        "a\nbc",
        "input beyond the line limit must not add a line, and must not truncate"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn accessibility_label_follows_a_signal_without_rebuilding(ui: UiBuilder) {
    // An accessibility label derived from app state must stay current the way
    // accessibility *state* already does, instead of freezing at the value it
    // had when the subtree was built.
    let unread = Binding::i32(3);
    let label = unread.clone().map(|count| Str::from(format!("{count} unread messages")));

    let mut app = ui.mount(move || control_shell(button("Inbox").a11y_label(label.clone())));

    app.query()
        .role(Role::BUTTON)
        .label("3 unread messages")
        .assert_exists();

    unread.set(7);

    assert!(
        app.query()
            .role(Role::BUTTON)
            .label("7 unread messages")
            .wait_for_existence(core::time::Duration::from_secs(2)),
        "the accessibility label must follow its signal"
    );
}

fn icon_only_search_button_view() -> impl waterui::View {
    control_shell(button(label("Search").icon(()).icon_only()))
}

#[waterui::test(icon_only_search_button_view, theme = hydrolysis_m3::install, viewport = (320, 240))]
fn icon_only_label_preserves_button_accessible_name(app: &mut SemanticApp) {
    app.query()
        .role(Role::BUTTON)
        .label("Search")
        .assert_exists();
}

fn actions_menu_view() -> impl waterui::View {
    control_shell(Menu::new(
        label("Actions").icon(()),
        (
            button("Refresh").action(|| {}),
            Menu::new("Advanced", (button("Archive").action(|| {}),)),
        ),
    ))
}

#[waterui::test(actions_menu_view, theme = hydrolysis_m3::install, viewport = (320, 240))]
fn menu_button_exposes_accessible_name(app: &mut SemanticApp) {
    let menu = app.query().role(Role::BUTTON).label("Actions").single();
    let bounds = menu.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "menu-button-exposes-accessible-name: menu trigger bounds must be non-zero"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn disabled_toggle_ignores_input_and_reports_disabled(ui: UiBuilder) {
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();

    let mut app =
        ui.mount(move || control_shell(toggle("Wi-Fi", &enabled_for_view).disabled(true)));

    let element = app.query().role(Role::SWITCH).label("Wi-Fi").single();
    assert!(
        !element.node().enabled(),
        "disabled-toggle: switch should expose disabled accessibility state"
    );
    assert_rejected("disabled-toggle: accessibility tap should be rejected", || {
        app.query().role(Role::SWITCH).label("Wi-Fi").tap();
    });
    // The pointer event dispatches into the window but must not hit the
    // disabled control: the binding stays unchanged.
    let _ = app
        .query()
        .role(Role::SWITCH)
        .label("Wi-Fi")
        .tap_at(0.5, 0.5);
    assert!(
        !enabled.get(),
        "disabled-toggle: binding must stay unchanged"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn disabled_scope_cascades_and_reenables_reactively(ui: UiBuilder) {
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();
    let form_locked = Binding::bool(true);
    let form_locked_for_view = form_locked.clone();

    let mut app = ui.mount(move || {
        control_shell(
            vstack((toggle("Notifications", &enabled_for_view),))
                .disabled(form_locked_for_view.clone()),
        )
    });

    let element = app
        .query()
        .role(Role::SWITCH)
        .label("Notifications")
        .single();
    assert!(
        !element.node().enabled(),
        "disabled-scope: toggle inside a disabled container must report disabled"
    );
    assert_rejected(
        "disabled-scope: tap inside a disabled container must be rejected",
        || {
            app.query().role(Role::SWITCH).label("Notifications").tap();
        },
    );
    assert!(
        !enabled.get(),
        "disabled-scope: binding must stay unchanged"
    );

    form_locked.set(false);
    assert!(
        app.query()
            .role(Role::SWITCH)
            .label("Notifications")
            .enabled(true)
            .wait_for_existence(core::time::Duration::from_secs(2)),
        "disabled-scope: re-enabling the container must re-enable the toggle"
    );
    app.query().role(Role::SWITCH).label("Notifications").tap();
    assert!(
        enabled.get(),
        "disabled-scope: tap after re-enable must flip the binding"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn disabled_slider_ignores_value_actions(ui: UiBuilder) {
    let value = Binding::f64(0.5);
    let value_for_view = value.clone();

    let mut app =
        ui.mount(move || control_shell(slider("Volume", &value_for_view).disabled(true)));

    let element = app.query().role(Role::SLIDER).label("Volume").single();
    assert!(
        !element.node().enabled(),
        "disabled-slider: slider should expose disabled accessibility state"
    );
    assert_rejected("disabled-slider: increment must be rejected", || {
        app.query().role(Role::SLIDER).label("Volume").increment();
    });
    // The pointer drag dispatches into the window but must not hit the
    // disabled control: the value stays unchanged.
    let _ = app
        .query()
        .role(Role::SLIDER)
        .label("Volume")
        .drag_by(60.0, 0.0);
    assert_close(
        value.get(),
        0.5,
        0.0001,
        "disabled-slider: value must stay unchanged",
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn disabled_button_ignores_action(ui: UiBuilder) {
    let count = Binding::i32(0);
    let count_for_view = count.clone();

    let mut app = ui.mount(move || {
        control_shell(
            button("Submit")
                .action(|waterui::State(count): waterui::State<Binding<i32>>| {
                    *count.get_mut() += 1;
                })
                .disabled(true)
                .state(&count_for_view),
        )
    });

    assert_rejected("disabled-button: accessibility tap should be rejected", || {
        app.query().role(Role::BUTTON).label("Submit").tap();
    });
    // The pointer event dispatches into the window but must not hit the
    // disabled control: the action never runs.
    let _ = app
        .query()
        .role(Role::BUTTON)
        .label("Submit")
        .tap_at(0.5, 0.5);
    assert_eq!(count.get(), 0, "disabled-button: action must not run");
}
