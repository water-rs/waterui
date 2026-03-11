use core::ops::Index;
use std::collections::BTreeMap;

use accesskit::{
    Node as AccessibilityNode, NodeId as AccessibilityNodeId, Rect as AccessibilityRect,
    Role as AccessibilityRole, Toggled as AccessibilityToggled,
    TreeUpdate as AccessibilityTreeUpdate,
};

use crate::selector::Selector;

/// Stable role wrapper exposed by the testing API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Role(AccessibilityRole);

impl Role {
    pub const BUTTON: Self = Self(AccessibilityRole::Button);
    pub const LABEL: Self = Self(AccessibilityRole::Label);
    pub const TEXT_INPUT: Self = Self(AccessibilityRole::TextInput);
    pub const PASSWORD_INPUT: Self = Self(AccessibilityRole::PasswordInput);
    pub const CHECKBOX: Self = Self(AccessibilityRole::CheckBox);
    pub const SWITCH: Self = Self(AccessibilityRole::Switch);
    pub const SLIDER: Self = Self(AccessibilityRole::Slider);
    pub const LIST: Self = Self(AccessibilityRole::List);
    pub const LIST_ITEM: Self = Self(AccessibilityRole::ListItem);
    pub const COMBOBOX: Self = Self(AccessibilityRole::ComboBox);
    pub const OPTION: Self = Self(AccessibilityRole::ListBoxOption);

    pub(crate) const fn as_accesskit(self) -> AccessibilityRole {
        self.0
    }
}

/// Stable node identifier wrapper exposed by the testing API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(AccessibilityNodeId);

impl NodeId {
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0.0
    }

    pub(crate) const fn as_accesskit(self) -> AccessibilityNodeId {
        self.0
    }
}

impl From<AccessibilityNodeId> for NodeId {
    fn from(value: AccessibilityNodeId) -> Self {
        Self(value)
    }
}

/// Stable node bounds wrapper exposed by the testing API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeBounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl NodeBounds {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub fn center(self) -> (f32, f32) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }
}

fn accesskit_rect_to_node_bounds(rect: AccessibilityRect) -> NodeBounds {
    NodeBounds::new(
        rect.x0 as f32,
        rect.y0 as f32,
        (rect.x1 - rect.x0) as f32,
        (rect.y1 - rect.y0) as f32,
    )
}

/// Immutable accessibility node snapshot used by assertions and queries.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSnapshot {
    pub(crate) id: NodeId,
    pub(crate) role: Role,
    pub(crate) label: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) bounds: Option<NodeBounds>,
    pub(crate) enabled: bool,
    pub(crate) selected: bool,
    pub(crate) checked: Option<bool>,
    pub(crate) expanded: Option<bool>,
    pub(crate) hidden: bool,
    pub(crate) children: Vec<NodeId>,
}

impl NodeSnapshot {
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }

    #[must_use]
    pub const fn checked(&self) -> Option<bool> {
        self.checked
    }

    #[must_use]
    pub const fn expanded(&self) -> Option<bool> {
        self.expanded
    }

    #[must_use]
    pub fn bounds(&self) -> Option<NodeBounds> {
        self.bounds
    }

    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    fn from_accesskit(id: AccessibilityNodeId, node: &AccessibilityNode) -> Self {
        let checked = match node.toggled() {
            Some(AccessibilityToggled::True) => Some(true),
            Some(AccessibilityToggled::False) => Some(false),
            Some(AccessibilityToggled::Mixed) | None => None,
        };
        let expanded = node.is_expanded();

        let value = node
            .value()
            .map(ToOwned::to_owned)
            .or_else(|| node.numeric_value().map(|v| v.to_string()));

        Self {
            id: NodeId::from(id),
            role: Role(node.role()),
            label: node.label().map(ToOwned::to_owned),
            value,
            bounds: node.bounds().map(accesskit_rect_to_node_bounds),
            enabled: !node.is_disabled(),
            selected: node.is_selected().unwrap_or(false),
            checked,
            expanded,
            hidden: node.is_hidden(),
            children: node.children().iter().copied().map(NodeId::from).collect(),
        }
    }
}

/// Immutable accessibility tree snapshot.
#[derive(Debug, Clone)]
pub struct TreeSnapshot {
    pub(crate) revision: u64,
    pub(crate) root: NodeId,
    pub(crate) focus: NodeId,
    pub(crate) nodes: BTreeMap<NodeId, NodeSnapshot>,
}

impl TreeSnapshot {
    pub(crate) fn empty() -> Self {
        let root = NodeId::from(AccessibilityNodeId(0));
        Self {
            revision: 0,
            root,
            focus: root,
            nodes: BTreeMap::new(),
        }
    }

    pub(crate) fn from_update(revision: u64, update: AccessibilityTreeUpdate) -> Self {
        let root = update
            .tree
            .as_ref()
            .map(|tree| NodeId::from(tree.root))
            .unwrap_or_else(|| NodeId::from(AccessibilityNodeId(0)));
        let focus = NodeId::from(update.focus);
        let mut nodes = BTreeMap::new();
        for (id, node) in update.nodes {
            let stable_id = NodeId::from(id);
            nodes.insert(stable_id, NodeSnapshot::from_accesskit(id, &node));
        }

        Self {
            revision,
            root,
            focus,
            nodes,
        }
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub const fn focus(&self) -> NodeId {
        self.focus
    }

    #[must_use]
    pub fn nodes(&self) -> &BTreeMap<NodeId, NodeSnapshot> {
        &self.nodes
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&NodeSnapshot> {
        self.nodes.get(&id)
    }

    pub(crate) fn matching(&self, selector: &Selector) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter_map(|(id, node)| selector.matches(node).then_some(*id))
            .collect()
    }
}

impl Index<NodeId> for TreeSnapshot {
    type Output = NodeSnapshot;

    fn index(&self, index: NodeId) -> &Self::Output {
        self.nodes.get(&index).unwrap_or_else(|| {
            panic!(
                "waterui-testing tree index missing node id {} (revision {})",
                index.as_u64(),
                self.revision
            )
        })
    }
}
