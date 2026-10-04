//! `ExcludeDescendants` belongs to the element that claims a naming scope, so
//! a gesture observer that registers no node — a long-press over a tapped,
//! labelled target — must not suppress the element inside it. Regression
//! coverage for water-rs/hydrolysis#266: `on_long_press_gesture` wrapping an
//! `a11y_children(ExcludeDescendants)` button emitted zero accessibility nodes
//! because the observer pushed suppression around the claim inside.
//!
//! Exercises both runtimes — the headless semantic walk (`mount`) and the
//! rendered flush (`mount_offscreen`).

use hydrolysis_m3::Material3;
use waterui::ViewExt as _;
use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::component::{button, text};
use waterui_testing::{Role, ui};

/// The m3 `icon_button` shape: a tap inside, with the button's role, label and
/// `ExcludeDescendants` env wrapping it, and a long-press observer outermost.
fn icon_button_like() -> impl waterui::View {
    text("+")
        .on_tap(|| {})
        .a11y_label("New tab")
        .a11y_role(AccessibilityRole::Button)
        .a11y_children(AccessibilityChildren::ExcludeDescendants)
        .on_long_press_gesture(500, || {})
}

/// The rendered runtime must keep the claim's node when a non-registering
/// gesture observer sits outside it.
#[test]
fn long_press_observer_preserves_the_claims_node_on_offscreen_mount() {
    let mut app = ui()
        .viewport(320, 200)
        .theme(Material3::defaults())
        .mount_offscreen(icon_button_like);
    app.query()
        .role(Role::BUTTON)
        .label("New tab")
        .assert_exists();
}

/// The semantic walk must keep the claim's node for the same tree.
#[test]
fn long_press_observer_preserves_the_claims_node_on_semantic_mount() {
    let mut app = ui()
        .viewport(320, 200)
        .theme(Material3::defaults())
        .mount(icon_button_like);
    app.query()
        .role(Role::BUTTON)
        .label("New tab")
        .assert_exists();
}

/// A long-press over an `ExcludeDescendants` scope without an inner claim must
/// still suppress the text leaf — the flag is not silently dropped.
#[test]
fn long_press_observer_over_unclaimed_scope_still_excludes_descendants() {
    let mut app = ui()
        .viewport(320, 200)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            text("+")
                .a11y_label("New tab")
                .a11y_role(AccessibilityRole::Button)
                .a11y_children(AccessibilityChildren::ExcludeDescendants)
                .on_long_press_gesture(500, || {})
        });
    // No gesture claims the scope, so the container itself claims it: the
    // button node exists and the "+" text leaf stays excluded.
    app.query()
        .role(Role::BUTTON)
        .label("New tab")
        .assert_exists();
    app.query().role(Role::LABEL).assert_not_exists();
}

/// Baseline: the tap claim without a long-press observer registers the node.
#[test]
fn tap_claim_registers_the_button_node() {
    let mut app = ui()
        .viewport(320, 200)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            text("+")
                .on_tap(|| {})
                .a11y_label("New tab")
                .a11y_role(AccessibilityRole::Button)
                .a11y_children(AccessibilityChildren::ExcludeDescendants)
        });
    app.query()
        .role(Role::BUTTON)
        .label("New tab")
        .assert_exists();
    app.query().role(Role::LABEL).assert_not_exists();
}

/// A plain waterui `button` under a long-press observer keeps its node.
#[test]
fn long_press_observer_preserves_a_plain_button_node() {
    let mut app = ui()
        .viewport(320, 200)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            button("New tab")
                .action(|| {})
                .on_long_press_gesture(500, || {})
        });
    app.query()
        .role(Role::BUTTON)
        .label("New tab")
        .assert_exists();
}
