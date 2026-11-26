//! `Slider` and `Stepper` publish their formatted value text as the
//! accessibility node's value (`aria-valuetext` / Android `stateDescription`),
//! while an explicit `.a11y_value(...)` still overrides it.

mod support;

use accesskit::Role;
use nami::Binding;
use support::{mount_rendered, mount_semantic, nodes};
use waterui::ViewExt as _;
use waterui::component::{slider, stepper};
use waterui_core::Environment;

fn node_value(tree: &accesskit::TreeUpdate, role: Role, label: &str) -> Option<String> {
    nodes(tree)
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .unwrap_or_else(|| panic!("expected {role:?} node labelled {label:?}"))
        .1
        .value()
        .map(str::to_owned)
}

#[test]
fn slider_publishes_formatted_value_and_tracks_updates_semantic() {
    let temp = Binding::new(21.0_f64);
    let temp_for_view = temp.clone();
    let mut app = mount_semantic(
        Environment::new(),
        move || {
            slider("Temperature", &temp_for_view)
                .range(10.0..=30.0)
                .value_formatter(|v| format!("{v:.0} °C"))
        },
        360,
        120,
    );
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::Slider, "Temperature").as_deref(),
        Some("21 °C")
    );

    temp.set(25.0);
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::Slider, "Temperature").as_deref(),
        Some("25 °C")
    );
}

#[test]
fn slider_publishes_formatted_value_and_tracks_updates_rendered() {
    let temp = Binding::new(21.0_f64);
    let temp_for_view = temp.clone();
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            slider("Temperature", &temp_for_view)
                .range(10.0..=30.0)
                .value_formatter(|v| format!("{v:.0} °C"))
        },
        360,
        120,
    );
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::Slider, "Temperature").as_deref(),
        Some("21 °C")
    );

    temp.set(25.0);
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::Slider, "Temperature").as_deref(),
        Some("25 °C")
    );
}

#[test]
fn slider_explicit_a11y_value_overrides_formatter() {
    let temp = Binding::new(21.0_f64);
    let temp_for_view = temp.clone();
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            slider("Temperature", &temp_for_view)
                .range(10.0..=30.0)
                .value_formatter(|v| format!("{v:.0} °C"))
                .a11y_value("Comfort")
        },
        360,
        120,
    );
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::Slider, "Temperature").as_deref(),
        Some("Comfort")
    );

    temp.set(26.0);
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::Slider, "Temperature").as_deref(),
        Some("Comfort")
    );
}

#[test]
fn stepper_publishes_formatted_value_and_tracks_updates() {
    let count = Binding::new(3_i64);
    let count_for_semantic = count.clone();
    let mut semantic = mount_semantic(
        Environment::new(),
        move || {
            stepper("Guests", &count_for_semantic)
                .range(1..=10)
                .value_formatter(|v| format!("{v} guests"))
        },
        360,
        120,
    );
    semantic.settle();
    assert_eq!(
        node_value(semantic.tree(), Role::SpinButton, "Guests").as_deref(),
        Some("3 guests")
    );

    let count_for_rendered = count.clone();
    let mut rendered = mount_rendered(
        Environment::new(),
        move || {
            stepper("Guests", &count_for_rendered)
                .range(1..=10)
                .value_formatter(|v| format!("{v} guests"))
        },
        360,
        120,
    );
    rendered.settle();
    assert_eq!(
        node_value(rendered.tree(), Role::SpinButton, "Guests").as_deref(),
        Some("3 guests")
    );

    count.set(5);
    semantic.settle();
    rendered.settle();
    assert_eq!(
        node_value(semantic.tree(), Role::SpinButton, "Guests").as_deref(),
        Some("5 guests")
    );
    assert_eq!(
        node_value(rendered.tree(), Role::SpinButton, "Guests").as_deref(),
        Some("5 guests")
    );
}

#[test]
fn stepper_explicit_a11y_value_overrides_formatter() {
    let count = Binding::new(3_i64);
    let count_for_view = count.clone();
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            stepper("Guests", &count_for_view)
                .range(1..=10)
                .value_formatter(|v| format!("{v} guests"))
                .a11y_value("Small party")
        },
        360,
        120,
    );
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::SpinButton, "Guests").as_deref(),
        Some("Small party")
    );

    count.set(6);
    app.settle();
    assert_eq!(
        node_value(app.tree(), Role::SpinButton, "Guests").as_deref(),
        Some("Small party")
    );
}
