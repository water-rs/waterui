//! Markdown headings publish `Role::Heading` and images publish their alt text
//! (or are hidden as decorative when alt is empty), in both `RichText` and
//! `FlowMarkdown`.
#![cfg(feature = "flow-markdown")]

use waterui::widget::{RichText, flow_markdown};
use waterui_testing::{AccessKitRole, Role, TreeSnapshot, UiBuilder};

const HEADING_ROLE: Role = Role::new(AccessKitRole::Heading);

fn find_role(tree: &TreeSnapshot, label: &str) -> Option<Role> {
    tree.nodes()
        .values()
        .find_map(|node| (node.label() == Some(label)).then_some(node.role()))
}

#[waterui::test(viewport = (600, 500))]
fn rich_text_publishes_heading_roles(ui: UiBuilder) {
    const DOC: &str = "# Setup\n\n## Pipeline\n\n### Details\n\nBody paragraph.";

    let app = ui.mount(|| RichText::from_markdown(DOC));
    let tree = app.tree();

    assert_eq!(find_role(tree, "Setup"), Some(HEADING_ROLE));
    assert_eq!(find_role(tree, "Pipeline"), Some(HEADING_ROLE));
    assert_eq!(find_role(tree, "Details"), Some(HEADING_ROLE));
    assert_eq!(find_role(tree, "Body paragraph."), Some(Role::LABEL));
}

#[waterui::test(viewport = (600, 500))]
fn flow_markdown_publishes_heading_roles(ui: UiBuilder) {
    const DOC: &str = "# Overview\n\n#### Deep Section\n\nPlain text.";

    let app = ui.mount(|| flow_markdown(DOC));
    let tree = app.tree();

    assert_eq!(find_role(tree, "Overview"), Some(HEADING_ROLE));
    assert_eq!(find_role(tree, "Deep Section"), Some(HEADING_ROLE));
    assert_eq!(find_role(tree, "Plain text."), Some(Role::LABEL));
}

#[cfg(feature = "image")]
mod images {
    use waterui::widget::{RichText, flow_markdown};
    use waterui_testing::{Role, UiBuilder};

    #[waterui::test(viewport = (600, 500))]
    fn rich_text_images_publish_alt_text_and_hide_decorative(ui: UiBuilder) {
        const DOC: &str = "![Diagram of the pipeline](https://example.com/p.png)\n\n![](https://example.com/d.png)";

        let mut app = ui.mount(|| RichText::from_markdown(DOC));
        app.query()
            .role(Role::IMAGE)
            .label("Diagram of the pipeline")
            .assert_exists();

        let image_nodes = app.query().role(Role::IMAGE).all();
        assert_eq!(
            image_nodes.len(),
            1,
            "decorative image with empty alt must be hidden from assistive technology"
        );
    }

    #[waterui::test(viewport = (600, 500))]
    fn flow_markdown_images_publish_alt_text_and_hide_decorative(ui: UiBuilder) {
        const DOC: &str =
            "![Architecture overview](https://example.com/a.png)\n\n![](https://example.com/d.png)";

        let mut app = ui.mount(|| flow_markdown(DOC));
        app.query()
            .role(Role::IMAGE)
            .label("Architecture overview")
            .assert_exists();

        let image_nodes = app.query().role(Role::IMAGE).all();
        assert_eq!(
            image_nodes.len(),
            1,
            "decorative image with empty alt must be hidden from assistive technology"
        );
    }
}
