//! Renders a mounted app's accessibility tree into the text and JSON forms the
//! MCP tools return.
//!
//! The text format is one line per node, indented two spaces per depth:
//!
//! ```text
//! revision=3 viewport=200x100 focus=#2
//!   #2 button "Save" [focused] actions=[click,focus] bounds=8,8,64,32
//! ```

use std::fmt::Write as _;

use serde::Serialize;
use waterui_testing::{CheckedState, NodeId, NodeSnapshot, Role, SemanticApp, TreeSnapshot};

/// Lowercases an `AccessKit` `Debug` name into `snake_case` (`CheckBox` becomes
/// `check_box`); the spelling every role and action prints with.
pub fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// The `snake_case` name printed for `role`.
pub fn role_name(role: Role) -> String {
    snake_case(&format!("{:?}", role.as_accesskit()))
}

/// Renders the whole tree as the indented text form, including the
/// `revision`/`viewport`/`focus` header line.
pub fn render_text<R>(app: &SemanticApp<R>) -> String {
    let tree = app.tree();
    let (width, height) = app.viewport();
    let mut out = format!(
        "revision={} viewport={width}x{height} focus=#{}",
        tree.revision(),
        tree.focus().as_u64()
    );
    render_node_text(&mut out, tree, tree.root(), 0);
    out
}

fn render_node_text(out: &mut String, tree: &TreeSnapshot, id: NodeId, depth: usize) {
    // Indexing panics with the revision: a node id missing from the map is a
    // runtime bug, not a state to render around.
    let node = &tree[id];
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
    let _ = write!(out, "{}", node_line(node, tree.focus() == id));
    for child in node.children() {
        render_node_text(out, tree, *child, depth + 1);
    }
}

/// Renders a single node line without indentation: `#<id> <role> "<label>"
/// value=<v> [states] actions=[…] bounds=<x>,<y>,<w>,<h>`, omitting every part
/// that is absent or empty.
pub fn node_line(node: &NodeSnapshot, focused: bool) -> String {
    let mut parts = vec![format!(
        "#{} {}",
        node.id().as_u64(),
        role_name(node.role())
    )];
    if let Some(label) = node.label() {
        parts.push(format!("\"{label}\""));
    }
    if let Some(value) = node.value() {
        parts.push(format!("value={value}"));
    }
    let states = state_names(node, focused);
    if !states.is_empty() {
        parts.push(format!("[{}]", states.join("|")));
    }
    if !node.actions().is_empty() {
        let actions = node
            .actions()
            .iter()
            .map(|action| snake_case(&format!("{action:?}")))
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!("actions=[{actions}]"));
    }
    if let Some(bounds) = node.bounds() {
        parts.push(format!(
            "bounds={},{},{},{}",
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height()
        ));
    }
    parts.join(" ")
}

fn state_names(node: &NodeSnapshot, focused: bool) -> Vec<&'static str> {
    let mut states = Vec::new();
    match node.checked_state() {
        Some(CheckedState::True) => states.push("checked"),
        Some(CheckedState::False) => states.push("unchecked"),
        Some(CheckedState::Mixed) => states.push("mixed"),
        None => {}
    }
    if !node.enabled() {
        states.push("disabled");
    }
    if node.selected() {
        states.push("selected");
    }
    if let Some(expanded) = node.expanded() {
        states.push(if expanded { "expanded" } else { "collapsed" });
    }
    if node.busy() {
        states.push("busy");
    }
    if node.hidden() {
        states.push("hidden");
    }
    if focused {
        states.push("focused");
    }
    states
}

/// JSON mirror of [`NodeSnapshot`]: every field, plus the probed `actions` and
/// nested `children`, emitted by the `snapshot` tool's `json` format.
///
/// Boolean state flags serialize as the `states` array, using the same
/// spellings as the text format's bracket group.
#[derive(Debug, Serialize)]
struct JsonNode {
    id: u64,
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bounds: Option<JsonBounds>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    states: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expanded: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    actions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<Self>,
}

/// Rectangle in viewport coordinates, in logical pixels.
#[derive(Debug, Serialize)]
struct JsonBounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// Top-level JSON form: header fields plus the nested node tree.
#[derive(Debug, Serialize)]
struct JsonTree {
    revision: u64,
    viewport: [u32; 2],
    focus: u64,
    root: JsonNode,
}

impl JsonNode {
    fn from_snapshot(tree: &TreeSnapshot, node: &NodeSnapshot) -> Self {
        Self {
            id: node.id().as_u64(),
            role: role_name(node.role()),
            label: node.label().map(ToOwned::to_owned),
            identifier: node.identifier().map(ToOwned::to_owned),
            value: node.value().map(ToOwned::to_owned),
            bounds: node.bounds().map(|bounds| JsonBounds {
                x: bounds.x(),
                y: bounds.y(),
                width: bounds.width(),
                height: bounds.height(),
            }),
            states: state_names(node, tree.focus() == node.id()),
            expanded: node.expanded(),
            actions: node
                .actions()
                .iter()
                .map(|action| snake_case(&format!("{action:?}")))
                .collect(),
            children: node
                .children()
                .iter()
                .map(|id| Self::from_snapshot(tree, &tree[*id]))
                .collect(),
        }
    }
}

/// Renders the tree as pretty-printed JSON.
///
/// # Errors
///
/// Returns the `serde_json` error if serialization fails; the value tree is
/// self-contained, so this cannot fail in practice.
pub fn render_json<R>(app: &SemanticApp<R>) -> serde_json::Result<String> {
    let tree = app.tree();
    let root = JsonNode::from_snapshot(tree, &tree[tree.root()]);
    serde_json::to_string_pretty(&JsonTree {
        revision: tree.revision(),
        viewport: app.viewport().into(),
        focus: tree.focus().as_u64(),
        root,
    })
}

#[cfg(test)]
mod tests {
    use waterui::component::{button, toggle, vstack};
    use waterui_testing::ui;

    use super::{render_json, render_text};

    // `TreeSnapshot` cannot be built outside `waterui-testing`, so the
    // renderer is exercised against a mounted view.
    fn mounted_text() -> String {
        let enabled = waterui::Binding::bool(false);
        let app = ui()
            .viewport(200, 100)
            .mount(move || vstack((button("Save"), toggle("Enable", &enabled))));
        render_text(&app)
    }

    #[test]
    fn text_tree_has_header_and_node_lines() {
        let text = mounted_text();
        let header = text.lines().next().expect("tree has a header line");
        assert!(header.starts_with("revision="), "header: {header}");
        assert!(header.contains("viewport=200x100"), "header: {header}");
        assert!(header.contains("focus=#"), "header: {header}");

        let button = text
            .lines()
            .find(|line| line.contains("button \"Save\""))
            .unwrap_or_else(|| panic!("no button line in:\n{text}"));
        assert!(button.contains("actions=["), "button line: {button}");
        assert!(button.contains("click"), "button line: {button}");
        assert!(button.contains("focus"), "button line: {button}");

        let toggle_line = text
            .lines()
            .find(|line| line.contains("\"Enable\""))
            .unwrap_or_else(|| panic!("no toggle line in:\n{text}"));
        assert!(
            toggle_line.contains("unchecked"),
            "toggle line: {toggle_line}"
        );
    }

    #[test]
    fn text_tree_indents_children() {
        let text = mounted_text();
        let depths: Vec<usize> = text
            .lines()
            .skip(1)
            .map(|line| line.len() - line.trim_start().len())
            .collect();
        assert!(depths.len() >= 2, "expected several node lines:\n{text}");
        assert_eq!(depths[0], 0, "root is not indented:\n{text}");
        assert!(
            depths.iter().skip(1).all(|depth| *depth > 0),
            "children should be indented:\n{text}"
        );
    }

    fn find_label<'a>(node: &'a serde_json::Value, label: &str) -> Option<&'a serde_json::Value> {
        if node["label"] == label {
            return Some(node);
        }
        node["children"]
            .as_array()?
            .iter()
            .find_map(|child| find_label(child, label))
    }

    #[test]
    fn json_tree_mirrors_node_fields() {
        let enabled = waterui::Binding::bool(false);
        let app = ui()
            .viewport(200, 100)
            .mount(move || vstack((button("Save"), toggle("Enable", &enabled))));
        let json: serde_json::Value =
            serde_json::from_str(&render_json(&app).expect("json renders")).expect("json parses");
        assert_eq!(json["viewport"], serde_json::json!([200, 100]));

        let button = find_label(&json["root"], "Save").expect("button node in json");
        assert_eq!(button["role"], "button");
        assert!(
            button["actions"]
                .as_array()
                .is_some_and(|actions| actions.iter().any(|a| a == "click")),
            "button actions: {button}"
        );
    }
}
