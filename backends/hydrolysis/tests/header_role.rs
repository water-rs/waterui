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
