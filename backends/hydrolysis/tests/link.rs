//! Integration tests for [`Link`]: semantic role, accessibility label,
//! disabled behavior, and non-stretching rendered layout.
#![cfg(feature = "link")]

mod support;

use accesskit::{Action as AccessibilityAction, Role};
use support::{mount_rendered, mount_semantic, node, node_bounds_with_role, node_id_with_role};
use waterui::component::link::link;
use waterui::component::{hstack, spacer};
use waterui::{View, ViewExt as _};
use waterui_core::Environment;

#[derive(Clone)]
struct DisabledLink;

impl View for DisabledLink {
    fn body(self, _env: &Environment) -> impl View {
        link("Disabled Docs", "https://waterui.dev").disabled(true)
    }
}

#[derive(Clone)]
struct LabelledLink;

impl View for LabelledLink {
    fn body(self, _env: &Environment) -> impl View {
        hstack((
            link("Docs", "https://waterui.dev").a11y_label("Documentation link"),
            spacer(),
        ))
    }
}

#[test]
fn link_emits_link_role_label_and_disabled_semantics() {
    let mut app = mount_semantic(
        Environment::new(),
        link("Visit Docs", "https://waterui.dev"),
        320,
        160,
    );
    let link_id = node_id_with_role(app.tree(), Role::Link, "Visit Docs");
    let link_node = node(app.tree(), link_id);
    assert!(link_node.supports_action(AccessibilityAction::Focus));
    assert!(link_node.supports_action(AccessibilityAction::Click));
    assert!(!link_node.is_disabled());

    let mut disabled_app = mount_semantic(Environment::new(), DisabledLink, 320, 160);
    let disabled_id = node_id_with_role(disabled_app.tree(), Role::Link, "Disabled Docs");
    let disabled_node = node(disabled_app.tree(), disabled_id);
    assert!(disabled_node.is_disabled());
    assert!(!disabled_node.supports_action(AccessibilityAction::Click));
}

#[test]
fn link_renders_content_sized_bounds_with_accessibility_override() {
    let mut app = mount_rendered(Environment::new(), LabelledLink, 400, 240);
    app.settle();

    let bounds = node_bounds_with_role(app.tree(), Role::Link, "Documentation link");
    let width = bounds.x1 - bounds.x0;
    let height = bounds.y1 - bounds.y0;
    assert!(width > 0.0 && height > 0.0);
    assert!(
        width < 400.0,
        "rendered link must size to its label instead of stretching (width={width})"
    );
}
