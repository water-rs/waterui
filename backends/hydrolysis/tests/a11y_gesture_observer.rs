//! Rendered-flush twin of the facade's gesture-observer accessibility
//! coverage: `ExcludeDescendants` belongs to the element that claims a naming
//! scope, so a gesture observer that registers no node — a long-press over a
//! tapped, labelled target — must not suppress the element inside it.
//! Regression coverage for water-rs/hydrolysis#266: `on_long_press_gesture`
//! wrapping an `a11y_children(ExcludeDescendants)` button emitted zero
//! accessibility nodes because the observer pushed suppression around the
//! claim inside.

mod support;

use accesskit::Role as AccessibilityRole_;
use support::{mount_rendered, node, node_exists, node_id_with_role, nodes};
use waterui::ViewExt as _;
use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::component::{button, text};
use waterui_core::Environment;

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
    let mut app = mount_rendered(Environment::new(), icon_button_like, 320, 200);
    app.settle();
    assert!(
        node_exists(app.tree(), AccessibilityRole_::Button, "New tab"),
        "the claims' button node must survive a non-registering observer"
    );
}

/// A long-press over an `ExcludeDescendants` scope without an inner claim must
/// still suppress the text leaf — the flag is not silently dropped.
#[test]
fn long_press_observer_over_unclaimed_scope_still_excludes_descendants() {
    let mut app = mount_rendered(
        Environment::new(),
        || {
            text("+")
                .a11y_label("New tab")
                .a11y_role(AccessibilityRole::Button)
                .a11y_children(AccessibilityChildren::ExcludeDescendants)
                .on_long_press_gesture(500, || {})
        },
        320,
        200,
    );
    app.settle();
    // No gesture claims the scope, so the container itself claims it: the
    // button node exists and the "+" text leaf stays excluded.
    assert!(
        node_exists(app.tree(), AccessibilityRole_::Button, "New tab"),
        "the scope's own claim must still register"
    );
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == AccessibilityRole_::Label),
        "the text leaf stays excluded"
    );
}

/// Baseline: the tap claim without a long-press observer registers the node.
#[test]
fn tap_claim_registers_the_button_node() {
    let mut app = mount_rendered(
        Environment::new(),
        || {
            text("+")
                .on_tap(|| {})
                .a11y_label("New tab")
                .a11y_role(AccessibilityRole::Button)
                .a11y_children(AccessibilityChildren::ExcludeDescendants)
        },
        320,
        200,
    );
    app.settle();
    assert!(
        node_exists(app.tree(), AccessibilityRole_::Button, "New tab"),
        "the tap claim registers the button node"
    );
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == AccessibilityRole_::Label),
        "the text leaf stays excluded"
    );
}

/// A plain waterui `button` under a long-press observer keeps its node.
#[test]
fn long_press_observer_preserves_a_plain_button_node() {
    let mut app = mount_rendered(
        Environment::new(),
        || {
            button("New tab")
                .action(|| {})
                .on_long_press_gesture(500, || {})
        },
        320,
        200,
    );
    app.settle();
    assert!(
        node_exists(app.tree(), AccessibilityRole_::Button, "New tab"),
        "a plain button keeps its node under a long-press observer"
    );
}

/// A bare `.on_tap` wraps no naming scope: the claim the tap synthesizes
/// collapses onto the text's own node, so the rendered tree carries one
/// activatable `Button` element — the action on the node the gesture wraps
/// (water-rs/waterui#1070) — instead of nothing at all.
#[test]
fn bare_tap_observer_claims_an_activatable_node_on_offscreen_mount() {
    let mut app = mount_rendered(Environment::new(), || text("T").on_tap(|| {}), 320, 200);
    app.settle();
    let id = node_id_with_role(app.tree(), AccessibilityRole_::Button, "T");
    let node = node(app.tree(), id);
    assert!(
        node.supports_action(accesskit::Action::Click),
        "the wrapped node advertises Click"
    );
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == AccessibilityRole_::Label),
        "the claim collapses onto the text's node — no separate label remains"
    );
}

/// A bare tap inside an `a11y_hidden` subtree synthesizes no claim: the
/// hidden view is a pointer target only, so it neither publishes a node nor
/// takes keyboard focus ahead of the visible controls — a dialog's scrim
/// beside its card's buttons.
#[test]
fn hidden_bare_tap_claims_no_node_on_offscreen_mount() {
    let mut app = mount_rendered(
        Environment::new(),
        || text("T").on_tap(|| {}).a11y_hidden(true),
        320,
        200,
    );
    app.settle();
    assert!(
        !node_exists(app.tree(), AccessibilityRole_::Button, "T"),
        "a hidden tap must not synthesize a button claim"
    );
}

/// The semantic walk drops the hidden tap's claim the same way.
#[test]
fn hidden_bare_tap_claims_no_node_on_semantic_mount() {
    let mut app = support::mount_semantic(
        Environment::new(),
        || text("T").on_tap(|| {}).a11y_hidden(true),
        320,
        200,
    );
    app.settle();
    assert!(
        !node_exists(app.tree(), AccessibilityRole_::Button, "T"),
        "a hidden tap must not synthesize a button claim"
    );
}
