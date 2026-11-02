//! Accessibility-tree shape contracts for decorated containers and gesture
//! observers, exercised on the semantic mount.
//!
//! - A labelled container carrying semantic state of its own is a distinct
//!   element: collapsing it into its only child would drop that state and
//!   overwrite the child's own label and role. Regression coverage for the
//!   modal-drawer panel behind water-rs/hydrolysis#148, whose decorative leaf
//!   stopped emitting and left the labelled panel with a single child.
//! - `ExcludeDescendants` belongs to the element that claims a naming scope,
//!   so a gesture observer that registers no node — a long-press over a
//!   tapped, labelled target — must not suppress the element inside it.
//!   Regression coverage for water-rs/hydrolysis#266.

use waterui::ViewExt as _;
use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::component::{button, text, vstack};
use waterui::shape::{Circle, ShapeExt as _};
use waterui_testing::{NodeSnapshot, Role, TreeSnapshot, UiBuilder};

/// A labelled `Group` container holding exactly one child — the shape a
/// navigation drawer or disclosure panel takes once its decorative leaves
/// stop emitting nodes.
fn stateful_panel(state: AccessibilityState) -> impl waterui::View {
    vstack((button("Inbox").action(|| {}),))
        .a11y_label("Panel")
        .a11y_role(AccessibilityRole::Group)
        .a11y_state(state)
}

/// The container node must survive intact: its own role, label, and state,
/// with the child still nested beneath it.
fn assert_panel(tree: &TreeSnapshot, check_state: impl Fn(&NodeSnapshot)) {
    let panel = tree
        .nodes()
        .values()
        .find(|node| node.label() == Some("Panel"))
        .expect("the labelled panel must survive as its own node");
    assert_eq!(
        panel.role(),
        Role::GROUP,
        "the panel keeps its container role instead of collapsing into the child"
    );
    check_state(panel);
    assert_eq!(
        panel.children().len(),
        1,
        "the panel keeps its single child beneath it"
    );
    let child = tree
        .node(panel.children()[0])
        .expect("the child node is still registered");
    assert_eq!(child.role(), Role::BUTTON);
    assert_eq!(
        child.label(),
        Some("Inbox"),
        "the child keeps its own label"
    );
}

fn check_expanded_panel_survives(tree: &TreeSnapshot) {
    assert_panel(tree, |panel| {
        assert_eq!(panel.expanded(), Some(true));
    });
}

fn check_selected_panel_survives(tree: &TreeSnapshot) {
    assert_panel(tree, |panel| {
        assert!(panel.selected());
    });
}

fn check_toggled_panel_survives(tree: &TreeSnapshot) {
    assert_panel(tree, |panel| {
        assert_eq!(panel.checked(), Some(true));
    });
}

/// The same single-child shape the modal navigation drawer takes once its
/// background leaf stops emitting: a labelled Group whose only node-bearing
/// child is a row, beside a decorative fill that suppresses itself.
fn panel_with_decorative_leaf(state: AccessibilityState) -> impl waterui::View {
    vstack((
        button("Inbox").action(|| {}),
        Circle
            .fill(waterui::Color::srgb_hex("#3B82F6"))
            .size(24.0, 24.0),
    ))
    .a11y_label("Panel")
    .a11y_role(AccessibilityRole::Group)
    .a11y_state(state)
}

/// A container hidden through a state signal still registers its own node —
/// the `hidden` flag follows the signal every publish — while its subtree
/// stays suppressed. Collapsing it would drop the flag and surface the child
/// as a visible standalone node.
fn signal_panel(state: AccessibilityState) -> impl waterui::View {
    vstack((button("Inbox").action(|| {}),))
        .a11y_label("Panel")
        .a11y_role(AccessibilityRole::Group)
        .a11y_state_signal(waterui::Computed::constant(state))
}

fn check_hidden_panel_survives(tree: &TreeSnapshot) {
    let panel = tree
        .nodes()
        .values()
        .find(|node| node.label() == Some("Panel"))
        .expect("the hidden panel must survive as its own node");
    assert_eq!(
        panel.role(),
        Role::GROUP,
        "the panel keeps its container role instead of collapsing into the child"
    );
    assert!(
        panel.hidden(),
        "the panel keeps its hidden state instead of collapsing into the child"
    );
    assert!(
        !tree
            .nodes()
            .values()
            .any(|node| !node.hidden() && node.role() == Role::BUTTON),
        "the child must not surface as a visible standalone node"
    );
}

fn check_leaf_panel_survives(tree: &TreeSnapshot) {
    assert_panel(tree, |panel| {
        assert_eq!(panel.expanded(), Some(true));
    });
}

#[waterui::test(viewport = (300, 300))]
fn expanded_panel_survives(ui: UiBuilder) {
    let mut app = ui.mount(move || stateful_panel(AccessibilityState::new().expanded(Some(true))));
    app.settle();
    check_expanded_panel_survives(app.tree());
}

#[waterui::test(viewport = (300, 300))]
fn selected_panel_survives(ui: UiBuilder) {
    let mut app = ui.mount(move || stateful_panel(AccessibilityState::new().selected(true)));
    app.settle();
    check_selected_panel_survives(app.tree());
}

#[waterui::test(viewport = (300, 300))]
fn toggled_panel_survives(ui: UiBuilder) {
    let mut app = ui.mount(move || stateful_panel(AccessibilityState::new().checked(Some(true))));
    app.settle();
    check_toggled_panel_survives(app.tree());
}

#[waterui::test(viewport = (300, 300))]
fn hidden_panel_survives(ui: UiBuilder) {
    let mut app = ui.mount(move || signal_panel(AccessibilityState::new().hidden(true)));
    app.settle();
    check_hidden_panel_survives(app.tree());
}

#[waterui::test(viewport = (300, 300))]
fn leaf_reduced_panel_survives(ui: UiBuilder) {
    let mut app = ui
        .mount(move || panel_with_decorative_leaf(AccessibilityState::new().expanded(Some(true))));
    app.settle();
    check_leaf_panel_survives(app.tree());
}

/// The m3 `icon_button` shape: a tap inside, with the button's role, label
/// and `ExcludeDescendants` env wrapping it, and a long-press observer
/// outermost.
fn icon_button_like() -> impl waterui::View {
    text("+")
        .on_tap(|| {})
        .a11y_label("New tab")
        .a11y_role(AccessibilityRole::Button)
        .a11y_children(AccessibilityChildren::ExcludeDescendants)
        .on_long_press_gesture(500, || {})
}

/// The semantic walk must keep the claim's node when a non-registering
/// gesture observer sits outside it.
#[waterui::test(viewport = (320, 200))]
fn long_press_observer_preserves_the_claims_node(ui: UiBuilder) {
    let mut app = ui.mount(icon_button_like);
    app.query()
        .role(Role::BUTTON)
        .label("New tab")
        .assert_exists();
}

/// A long-press over an `ExcludeDescendants` scope without an inner claim must
/// still suppress the text leaf — the flag is not silently dropped.
#[waterui::test(viewport = (320, 200))]
fn long_press_observer_over_unclaimed_scope_still_excludes_descendants(ui: UiBuilder) {
    let mut app = ui.mount(|| {
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
#[waterui::test(viewport = (320, 200))]
fn tap_claim_registers_the_button_node(ui: UiBuilder) {
    let mut app = ui.mount(|| {
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
#[waterui::test(viewport = (320, 200))]
fn long_press_observer_preserves_a_plain_button_node(ui: UiBuilder) {
    let mut app = ui.mount(|| {
        button("New tab")
            .action(|| {})
            .on_long_press_gesture(500, || {})
    });
    app.query()
        .role(Role::BUTTON)
        .label("New tab")
        .assert_exists();
}
