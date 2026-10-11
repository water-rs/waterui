//! Rendered-flush twin of the empty-label contract: the suppression that
//! keeps empty `Label` nodes out of the accessibility tree runs in the
//! rendered publish too (water-rs/hydrolysis#176). The semantic legs live
//! with the owning components — the navigation half in
//! `waterui-navigation`, the text half in the facade tests.

mod support;

use accesskit::Role;
use support::{mount_rendered, node, node_id_with_role, nodes};
use waterui::Str;
use waterui::component::text;
use waterui_core::Environment;
use waterui_navigation::NavigationView;

/// The issue's first shape: a titled navigation page that binds no subtitle.
fn inbox_view() -> NavigationView {
    NavigationView::new("Inbox", text("mail body"))
}

#[test]
fn navigation_without_subtitle_emits_no_subtitle_node_on_offscreen_mount() {
    let mut app = mount_rendered(Environment::new(), inbox_view, 390, 844);
    app.settle();
    let tree = app.tree();
    let bar_id = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Navigation)
        .map(|(id, _)| *id)
        .expect("the navigation bar must emit a node");
    let bar = node(tree, bar_id);
    let header_id = node_id_with_role(tree, Role::Header, "Inbox");
    assert!(
        bar.children().contains(&header_id),
        "the title header stays a child of the bar node"
    );
    assert!(
        !bar.children()
            .iter()
            .any(|id| node(tree, *id).role() == Role::Label),
        "an unbound subtitle must emit no Label child under the bar node"
    );
}

/// An empty `Str` leaf names nothing: like a decorative graphics leaf it
/// emits no node until there is a label to speak.
#[test]
fn empty_text_leaf_emits_no_node_on_offscreen_mount() {
    let mut app = mount_rendered(Environment::new(), || Str::from(""), 390, 844);
    app.settle();
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == Role::Label),
        "an empty text leaf must emit no Label node"
    );
}
