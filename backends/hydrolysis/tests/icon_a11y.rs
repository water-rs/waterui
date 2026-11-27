//! Rendered and semantic accessibility tests for `Icon` and `IconGlyph`
//! (`water-rs/waterui#4372`).

mod support;

use accesskit::Role;
use support::{count_role_label, mount_rendered, mount_semantic, node_exists, nodes};
use waterui::ViewExt as _;
use waterui::component::{button, label};
use waterui::icon::{Icon, IconGlyph};
use waterui_core::Environment;

const PUA_CODEPOINT: char = '\u{e5c4}';

const fn glyph_icon() -> Icon {
    Icon::from_glyph(IconGlyph::new(PUA_CODEPOINT, "Roboto"))
}

fn svg_icon() -> Icon {
    waterui_icons_lucide::trash()
}

fn assert_no_pua_node(tree: &accesskit::TreeUpdate) {
    let pua = PUA_CODEPOINT.to_string();
    assert!(
        !nodes(tree).any(|(_, node)| node.label().is_some_and(|l| l.contains(&pua))),
        "private-use codepoint must never appear in an accessibility label"
    );
}

#[test]
fn unnamed_icons_emit_no_accessibility_nodes_in_semantic_or_rendered_trees() {
    type IconBuilder = fn() -> Icon;
    let cases: [(&str, IconBuilder); 2] = [("glyph", glyph_icon), ("svg", svg_icon)];
    for (kind, build) in cases {
        let mut semantic = mount_semantic(Environment::new(), build, 120, 120);
        semantic.settle();
        assert!(
            !nodes(semantic.tree())
                .any(|(_, node)| matches!(node.role(), Role::Image | Role::Label)),
            "{kind}: unnamed icon must not emit Image or Label in semantic tree"
        );
        assert_no_pua_node(semantic.tree());

        let mut rendered = mount_rendered(Environment::new(), build, 120, 120);
        rendered.settle();
        assert!(
            !nodes(rendered.tree())
                .any(|(_, node)| matches!(node.role(), Role::Image | Role::Label)),
            "{kind}: unnamed icon must not emit Image or Label in rendered tree"
        );
        assert_no_pua_node(rendered.tree());
    }

    let mut raw_glyph = mount_rendered(
        Environment::new(),
        || IconGlyph::new(PUA_CODEPOINT, "Roboto"),
        120,
        120,
    );
    raw_glyph.settle();
    assert!(
        !nodes(raw_glyph.tree()).any(|(_, node)| matches!(node.role(), Role::Image | Role::Label)),
        "unnamed IconGlyph must stay decorative"
    );
    assert_no_pua_node(raw_glyph.tree());
}

#[test]
fn named_icons_publish_single_image_node_without_pua_in_semantic_and_rendered_trees() {
    for (kind, build) in [
        ("glyph", glyph_icon as fn() -> Icon),
        ("svg", svg_icon as fn() -> Icon),
    ] {
        let mut semantic = mount_semantic(
            Environment::new(),
            move || build().a11y_label("Remove draft"),
            120,
            120,
        );
        semantic.settle();
        assert_eq!(
            count_role_label(semantic.tree(), Role::Image, "Remove draft"),
            1,
            "{kind}: named icon must publish one Image node in semantic tree"
        );
        assert!(
            !nodes(semantic.tree()).any(|(_, node)| node.role() == Role::Label),
            "{kind}: named icon must suppress internal glyph Label node in semantic tree"
        );
        assert_no_pua_node(semantic.tree());

        let mut rendered = mount_rendered(
            Environment::new(),
            move || build().a11y_label("Remove draft"),
            120,
            120,
        );
        rendered.settle();
        assert_eq!(
            count_role_label(rendered.tree(), Role::Image, "Remove draft"),
            1,
            "{kind}: named icon must publish one Image node in rendered tree"
        );
        assert!(
            !nodes(rendered.tree()).any(|(_, node)| node.role() == Role::Label),
            "{kind}: named icon must suppress internal glyph Label node in rendered tree"
        );
        assert_no_pua_node(rendered.tree());
    }
}

#[test]
fn hidden_named_icons_emit_no_accessibility_nodes() {
    for build in [glyph_icon as fn() -> Icon, svg_icon as fn() -> Icon] {
        let mut rendered = mount_rendered(
            Environment::new(),
            move || build().a11y_label("Remove draft").a11y_hidden(true),
            120,
            120,
        );
        rendered.settle();
        assert!(
            !nodes(rendered.tree())
                .any(|(_, node)| node.role() == Role::Image || node.label() == Some("Remove draft")),
            "a11y_hidden(true) must suppress named icon completely"
        );
        assert_no_pua_node(rendered.tree());
    }
}

#[test]
fn button_with_icon_announces_only_control_label() {
    let mut rendered = mount_rendered(
        Environment::new(),
        || button(label("Delete").icon(glyph_icon())).action(|| {}),
        200,
        80,
    );
    rendered.settle();
    assert!(node_exists(rendered.tree(), Role::Button, "Delete"));
    assert_eq!(count_role_label(rendered.tree(), Role::Button, "Delete"), 1);
    assert!(
        !nodes(rendered.tree()).any(|(_, node)| node.role() == Role::Image),
        "button icon must not emit separate Image node"
    );
    assert_no_pua_node(rendered.tree());

    let mut rendered_svg = mount_rendered(
        Environment::new(),
        || button(label("Delete").icon(svg_icon())).action(|| {}),
        200,
        80,
    );
    rendered_svg.settle();
    assert_eq!(
        count_role_label(rendered_svg.tree(), Role::Button, "Delete"),
        1
    );
    assert!(
        !nodes(rendered_svg.tree()).any(|(_, node)| node.role() == Role::Image),
        "button svg icon must not emit separate Image node"
    );
}
