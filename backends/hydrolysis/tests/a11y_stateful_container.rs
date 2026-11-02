//! Rendered-flush twin of the facade's `a11y_containers` suite: a labelled
//! container carrying semantic state of its own is a distinct element, and
//! the collapse that would merge it into its only child runs in the rendered
//! flush's accessibility publish exactly as it does on the semantic walk.
//! Regression coverage for the modal-drawer panel behind
//! water-rs/hydrolysis#148.

mod support;

use accesskit::{Role as AccessibilityRole_, Toggled, TreeUpdate as AccessibilityTreeUpdate};
use support::{mount_rendered, node, node_id_with_role};
use waterui::ViewExt as _;
use waterui::accessibility::{AccessibilityRole, AccessibilityState};
use waterui::component::vstack;
use waterui::shape::{Circle, ShapeExt as _};
use waterui_controls::button;
use waterui_core::Computed;
use waterui_core::Environment;

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
fn assert_panel(tree: &AccessibilityTreeUpdate, check_state: impl Fn(&accesskit::Node)) {
    let panel_id = node_id_with_role(tree, AccessibilityRole_::Group, "Panel");
    let panel = node(tree, panel_id);
    check_state(panel);
    assert_eq!(
        panel.children().len(),
        1,
        "the panel keeps its single child beneath it"
    );
    let child_id = panel.children()[0];
    let child = node(tree, child_id);
    assert_eq!(child.role(), AccessibilityRole_::Button);
    assert_eq!(
        child.label(),
        Some("Inbox"),
        "the child keeps its own label"
    );
}

fn expanded_panel_survives(tree: &AccessibilityTreeUpdate) {
    assert_panel(tree, |panel| {
        assert_eq!(panel.is_expanded(), Some(true));
    });
}

fn selected_panel_survives(tree: &AccessibilityTreeUpdate) {
    assert_panel(tree, |panel| {
        assert_eq!(panel.is_selected(), Some(true));
    });
}

fn toggled_panel_survives(tree: &AccessibilityTreeUpdate) {
    assert_panel(tree, |panel| {
        assert_eq!(panel.toggled(), Some(Toggled::True));
    });
}

/// A container hidden through a state signal still registers its own node —
/// the `hidden` flag follows the signal every publish — while its subtree
/// stays suppressed.
fn signal_panel(state: AccessibilityState) -> impl waterui::View {
    vstack((button("Inbox").action(|| {}),))
        .a11y_label("Panel")
        .a11y_role(AccessibilityRole::Group)
        .a11y_state_signal(Computed::constant(state))
}

fn hidden_panel_survives(tree: &AccessibilityTreeUpdate) {
    let panel_id = node_id_with_role(tree, AccessibilityRole_::Group, "Panel");
    let panel = node(tree, panel_id);
    assert!(
        panel.is_hidden(),
        "the panel keeps its hidden state instead of collapsing into the child"
    );
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, node)| !node.is_hidden() && node.role() == AccessibilityRole_::Button),
        "the child must not surface as a visible standalone node"
    );
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

fn leaf_panel_survives(tree: &AccessibilityTreeUpdate) {
    assert_panel(tree, |panel| {
        assert_eq!(panel.is_expanded(), Some(true));
    });
}

#[test]
fn expanded_panel_survives_on_offscreen_mount() {
    let mut app = mount_rendered(
        Environment::new(),
        move || stateful_panel(AccessibilityState::new().expanded(Some(true))),
        300,
        300,
    );
    app.settle();
    expanded_panel_survives(app.tree());
}

#[test]
fn selected_panel_survives_on_offscreen_mount() {
    let mut app = mount_rendered(
        Environment::new(),
        move || stateful_panel(AccessibilityState::new().selected(true)),
        300,
        300,
    );
    app.settle();
    selected_panel_survives(app.tree());
}

#[test]
fn toggled_panel_survives_on_offscreen_mount() {
    let mut app = mount_rendered(
        Environment::new(),
        move || stateful_panel(AccessibilityState::new().checked(Some(true))),
        300,
        300,
    );
    app.settle();
    toggled_panel_survives(app.tree());
}

#[test]
fn hidden_panel_survives_on_offscreen_mount() {
    let mut app = mount_rendered(
        Environment::new(),
        move || signal_panel(AccessibilityState::new().hidden(true)),
        300,
        300,
    );
    app.settle();
    hidden_panel_survives(app.tree());
}

#[test]
fn leaf_reduced_panel_survives_on_offscreen_mount() {
    let mut app = mount_rendered(
        Environment::new(),
        move || panel_with_decorative_leaf(AccessibilityState::new().expanded(Some(true))),
        300,
        300,
    );
    app.settle();
    leaf_panel_survives(app.tree());
}
