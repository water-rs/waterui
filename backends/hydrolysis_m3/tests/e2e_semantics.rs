//! End-to-end accessibility coverage for the Material Design 3 theme package.

use hydrolysis_m3::install;
use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::env::Environment;
use waterui::graphics::color::Srgb;
use waterui::{Binding, Str};
use waterui_controls::{Slider, Stepper, TextField, button, toggle};
use waterui_core::View;
use waterui_testing::{Role, Selector, UiTest};

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
