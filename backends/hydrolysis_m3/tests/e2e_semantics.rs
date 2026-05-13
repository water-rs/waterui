//! End-to-end accessibility coverage for the Material Design 3 theme package.

use core::time::Duration;

use hydrolysis_m3::{assist_chip, filter_chip, install, suggestion_chip};
use waterui::ViewExt as _;
use waterui::component::{hstack, vstack};
use waterui::env::Environment;
use waterui::graphics::color::Srgb;
use waterui::{Binding, Str};
use waterui_controls::{Slider, Stepper, TextField, button, toggle};
use waterui_core::View;
use waterui_testing::{Role, Selector, UiTest, WaitOptions, WaitResult};

fn mount_m3<V, F>(build: F) -> waterui_testing::MountedApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let mut env = Environment::new();
    install(&mut env);

    UiTest::new()
        .environment(env)
        .viewport(360, 320)
        .mount(move || {
            vstack((build(),))
                .spacing(12.0)
                .padding_with(16.0)
                .background(Srgb::WHITE)
        })
}

#[test]
fn material_controls_expose_accessibility_semantics() {
    let enabled = Binding::bool(true);
    let enabled_for_view = enabled.clone();
    let value = Binding::f64(0.32);
    let value_for_view = value.clone();
    let count = Binding::i32(2);
    let count_for_view = count.clone();
    let name = Binding::container(Str::from("Hydrolysis"));
    let name_for_view = name.clone();

    let mut app = mount_m3(move || {
        vstack((
            button("Save").action(|| {}),
            toggle("Wi-Fi", &enabled_for_view),
            Slider::new("Volume", &value_for_view),
            Stepper::new("Quantity", &count_for_view),
            TextField::new(&name_for_view).label("Project"),
        ))
        .spacing(12.0)
    });

    app.query().role(Role::BUTTON).label("Save").assert_exists();
    app.query()
        .role(Role::SWITCH)
        .label("Wi-Fi")
        .checked(true)
        .assert_exists();
    app.query()
        .role(Role::SLIDER)
        .label("Volume")
        .assert_exists();
    app.query().label("Quantity").value("2").assert_exists();
    app.query()
        .role(Role::TEXT_INPUT)
        .label("Project")
        .value("Hydrolysis")
        .assert_exists();
}

#[test]
fn material_text_field_focus_is_routed_through_hydrolysis_accessibility_tree() {
    let name = Binding::container(Str::from(""));
    let name_for_view = name.clone();
    let mut app = mount_m3(move || TextField::new(&name_for_view).label("Search"));

    let selector = Selector::default().role(Role::TEXT_INPUT).label("Search");
    assert!(
        app.query().role(Role::TEXT_INPUT).label("Search").focus(),
        "material text field focus should be routable through waterui-testing"
    );
    app.assert_ui_focus(&selector);
}

#[test]
fn material_assist_chip_exposes_button_semantics_and_tap_action() {
    let tapped = Binding::bool(false);
    let tapped_for_view = tapped.clone();
    let tapped_for_action = tapped.clone();
    let mut app = mount_m3(move || {
        assist_chip("Assist").action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Assist")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Assist").tap(),
        "material assist chip should route tap actions through Hydrolysis gestures"
    );
    assert!(tapped_for_view.get(), "assist chip tap should update state");
}

#[test]
fn material_suggestion_chip_exposes_button_semantics_and_tap_action() {
    let tapped = Binding::bool(false);
    let tapped_for_view = tapped.clone();
    let tapped_for_action = tapped.clone();
    let mut app = mount_m3(move || {
        suggestion_chip("Suggestion").action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Suggestion")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Suggestion").tap(),
        "material suggestion chip should route tap actions through Hydrolysis gestures"
    );
    assert!(
        tapped_for_view.get(),
        "suggestion chip tap should update state"
    );
}

#[test]
fn material_filter_chip_toggles_selection_and_exposes_button_semantics() {
    let selected = Binding::bool(false);
    let selected_for_view = selected.clone();
    let tapped = Binding::bool(false);
    let tapped_for_action = tapped.clone();
    let mut app = mount_m3(move || {
        filter_chip("Filter", &selected_for_view).action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Filter")
        .selected(false)
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Filter").tap(),
        "material filter chip should route tap actions through Hydrolysis gestures"
    );
    assert!(selected.get(), "filter chip tap should toggle selected state");
    assert!(tapped.get(), "filter chip tap should invoke user action");
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::BUTTON)
                    .label("Filter")
                    .selected(true),
            )],
            WaitOptions::new(Duration::from_millis(200)),
        ) == WaitResult::Completed,
        "filter chip should expose selected state after tap"
    );
}

#[test]
fn material_filter_chip_selection_remeasures_parent_layout() {
    let selected = Binding::bool(false);
    let selected_for_view = selected.clone();
    let already_selected = Binding::bool(true);
    let mut app = mount_m3(move || {
        hstack((
            filter_chip("Filter", &selected_for_view),
            filter_chip("Selected", &already_selected),
        ))
        .spacing(8.0)
    });

    let initial_filter_bounds = app.query().role(Role::BUTTON).label("Filter").single().bounds();
    let initial_selected_bounds = app
        .query()
        .role(Role::BUTTON)
        .label("Selected")
        .single()
        .bounds();

    assert!(
        app.query().role(Role::BUTTON).label("Filter").tap(),
        "material filter chip should be tappable before layout remeasurement"
    );
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::BUTTON)
                    .label("Filter")
                    .selected(true),
            )],
            WaitOptions::new(Duration::from_millis(200)),
        ) == WaitResult::Completed,
        "filter chip should expose selected state before checking new layout"
    );

    let selected_filter_bounds = app.query().role(Role::BUTTON).label("Filter").single().bounds();
    let shifted_selected_bounds = app
        .query()
        .role(Role::BUTTON)
        .label("Selected")
        .single()
        .bounds();

    assert!(
        selected_filter_bounds.width() > initial_filter_bounds.width(),
        "selected filter chip should grow to include the leading checkmark"
    );
    assert!(
        shifted_selected_bounds.x() > initial_selected_bounds.x(),
        "sibling chip should shift after filter chip selection changes intrinsic width"
    );
}

#[test]
#[ignore = "captures a real Hydrolysis focused text field PNG for direct visual review"]
fn material_focused_text_field_snapshot() {
    let name = Binding::container(Str::from("Hydrolysis"));
    let name_for_view = name.clone();
    let mut app = mount_m3(move || TextField::new(&name_for_view).label("Project"));

    assert!(
        app.query().role(Role::TEXT_INPUT).label("Project").focus(),
        "material text field should accept focus before visual capture"
    );
    let captured = app.capture_snapshot("material3-preview", "text-field-focused-caret", "focused");

    assert!(captured.path().is_file());
}
