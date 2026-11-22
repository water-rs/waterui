//! Nested accessibility-state modifiers combine field by field.
//!
//! `.disabled(..)` and `.visible(..)` each attach an `AccessibilityState`
//! scope; a view carrying both must report both flags whichever modifier is
//! outermost. Regression coverage for water-rs/waterui#4244, where the inner
//! scope replaced the outer one and a disabled control the author hid stayed
//! exposed to assistive technology.

use waterui::ViewExt as _;
use waterui::component::button;
use waterui_testing::{Role, ui};

#[test]
fn disabled_inside_hidden_keeps_both_flags() {
    let mut app = ui()
        .viewport(400, 300)
        .mount(|| button("Delete").action(|| {}).disabled(true).visible(false));
    app.settle();
    let found = app
        .query()
        .role(Role::BUTTON)
        .label("Delete")
        .hidden(true)
        .single();
    let node = found.node();
    assert!(
        node.hidden(),
        "button inside .visible(false) must be hidden"
    );
    assert!(
        !node.enabled(),
        "button inside .disabled(true) must be disabled"
    );
}

#[test]
fn hidden_inside_disabled_keeps_both_flags() {
    let mut app = ui()
        .viewport(400, 300)
        .mount(|| button("Delete").action(|| {}).visible(false).disabled(true));
    app.settle();
    let found = app
        .query()
        .role(Role::BUTTON)
        .label("Delete")
        .hidden(true)
        .single();
    let node = found.node();
    assert!(
        node.hidden(),
        "button inside .visible(false) must be hidden"
    );
    assert!(
        !node.enabled(),
        "button inside .disabled(true) must be disabled"
    );
}

#[test]
fn disabled_inside_hidden_is_not_announced() {
    let mut app = ui()
        .viewport(400, 300)
        .mount(|| button("Delete").action(|| {}).disabled(true).visible(false));
    app.settle();
    app.query()
        .role(Role::BUTTON)
        .label("Delete")
        .assert_not_exists();
}
