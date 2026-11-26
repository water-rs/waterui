//! `AccessibilityRole::Header` is documented as a "Section heading" and is a
//! heading on iOS, but Hydrolysis once published accesskit's `Role::Header`,
//! which is the banner landmark (web: `role="banner"`). A section heading must
//! be `Role::Heading`.

mod support;

use accesskit::Role;
use support::{mount_semantic, nodes};
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::component::text;
use waterui_core::Environment;

#[test]
fn header_role_publishes_heading_not_banner() {
    let mut app = mount_semantic(
        Environment::new(),
        || text("Section").a11y_role(AccessibilityRole::Header),
        300,
        300,
    );
    app.settle();
    let roles: Vec<_> = nodes(app.tree())
        .filter(|(_, n)| n.label() == Some("Section"))
        .map(|(_, n)| n.role())
        .collect();
    assert_eq!(
        roles,
        [Role::Heading],
        "Header must publish a heading, got {roles:?}"
    );
}

#[test]
fn markdown_headings_publish_heading_role_and_level() {
    use waterui::widget::{RichText, flow_markdown};

    const DOC: &str = "# Setup\n\n## Pipeline\n\n### Details\n\nBody paragraph.";

    let mut rich = mount_semantic(
        Environment::new(),
        || RichText::from_markdown(DOC),
        400,
        300,
    );
    rich.settle();
    let rich_headings: Vec<_> = nodes(rich.tree())
        .filter(|(_, n)| n.role() == Role::Heading)
        .map(|(_, n)| (n.label().map(str::to_owned), n.level()))
        .collect();
    assert_eq!(
        rich_headings,
        vec![
            (Some(String::from("Setup")), Some(1)),
            (Some(String::from("Pipeline")), Some(2)),
            (Some(String::from("Details")), Some(3)),
        ]
    );

    let mut flow = mount_semantic(Environment::new(), || flow_markdown(DOC), 400, 300);
    flow.settle();
    let flow_headings: Vec<_> = nodes(flow.tree())
        .filter(|(_, n)| n.role() == Role::Heading)
        .map(|(_, n)| (n.label().map(str::to_owned), n.level()))
        .collect();
    assert_eq!(
        flow_headings,
        vec![
            (Some(String::from("Setup")), Some(1)),
            (Some(String::from("Pipeline")), Some(2)),
            (Some(String::from("Details")), Some(3)),
        ]
    );
}
