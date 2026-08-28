use core::ops::Index;
use std::collections::BTreeMap;

use accesskit::{
    Node as AccessibilityNode, NodeId as AccessibilityNodeId, Rect as AccessibilityRect,
    Role as AccessibilityRole, Toggled as AccessibilityToggled,
    TreeUpdate as AccessibilityTreeUpdate,
};

use crate::selector::{ScopeRelation, Selector};

/// Stable role wrapper exposed by the testing API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Role(AccessibilityRole);

impl Role {
    /// Button role.
    pub const BUTTON: Self = Self(AccessibilityRole::Button);
    /// Static label role.
    pub const LABEL: Self = Self(AccessibilityRole::Label);
    /// Editable text input role.
    pub const TEXT_INPUT: Self = Self(AccessibilityRole::TextInput);
    /// Secure text input role.
    pub const PASSWORD_INPUT: Self = Self(AccessibilityRole::PasswordInput);
    /// Checkbox role.
    pub const CHECKBOX: Self = Self(AccessibilityRole::CheckBox);
    /// Switch role.
    pub const SWITCH: Self = Self(AccessibilityRole::Switch);
    /// Slider role.
    pub const SLIDER: Self = Self(AccessibilityRole::Slider);
    /// Image role.
    pub const IMAGE: Self = Self(AccessibilityRole::Image);
    /// Scroll view role.
    pub const SCROLL_VIEW: Self = Self(AccessibilityRole::ScrollView);
    /// List role.
    pub const LIST: Self = Self(AccessibilityRole::List);
    /// List item role.
    pub const LIST_ITEM: Self = Self(AccessibilityRole::ListItem);
    /// Tab role.
    pub const TAB: Self = Self(AccessibilityRole::Tab);
    /// Tab list role.
    pub const TAB_LIST: Self = Self(AccessibilityRole::TabList);
    /// Combo box role.
    pub const COMBOBOX: Self = Self(AccessibilityRole::ComboBox);
    /// Selectable option role.
    pub const OPTION: Self = Self(AccessibilityRole::ListBoxOption);
    /// Multiline text input role.
    pub const MULTILINE_TEXT_INPUT: Self = Self(AccessibilityRole::MultilineTextInput);
    /// Link role.
    pub const LINK: Self = Self(AccessibilityRole::Link);
    /// Section heading role.
    pub const HEADER: Self = Self(AccessibilityRole::Header);
    /// Section footer role.
    pub const FOOTER: Self = Self(AccessibilityRole::Footer);
    /// Progress indicator role.
    pub const PROGRESS_INDICATOR: Self = Self(AccessibilityRole::ProgressIndicator);
    /// Stepper / spin button role.
    pub const SPIN_BUTTON: Self = Self(AccessibilityRole::SpinButton);
    /// Radio button role.
    pub const RADIO_BUTTON: Self = Self(AccessibilityRole::RadioButton);
    /// Menu role.
    pub const MENU: Self = Self(AccessibilityRole::Menu);
    /// Menu bar role.
    pub const MENU_BAR: Self = Self(AccessibilityRole::MenuBar);
    /// Menu item role.
    pub const MENU_ITEM: Self = Self(AccessibilityRole::MenuItem);
    /// Checkbox-style menu item role.
    pub const MENU_ITEM_CHECKBOX: Self = Self(AccessibilityRole::MenuItemCheckBox);
    /// Radio-style menu item role.
    pub const MENU_ITEM_RADIO: Self = Self(AccessibilityRole::MenuItemRadio);
    /// Tab panel role.
    pub const TAB_PANEL: Self = Self(AccessibilityRole::TabPanel);
    /// Table role.
    pub const TABLE: Self = Self(AccessibilityRole::Table);
    /// Table cell role.
    pub const CELL: Self = Self(AccessibilityRole::Cell);
    /// Column header role.
    pub const COLUMN_HEADER: Self = Self(AccessibilityRole::ColumnHeader);
    /// Logical grouping role.
    pub const GROUP: Self = Self(AccessibilityRole::Group);
    /// Window root role.
    pub const WINDOW: Self = Self(AccessibilityRole::Window);
    /// Main landmark role.
    pub const MAIN: Self = Self(AccessibilityRole::Main);
    /// Navigation landmark role.
    pub const NAVIGATION: Self = Self(AccessibilityRole::Navigation);
    /// Search landmark role.
    pub const SEARCH: Self = Self(AccessibilityRole::Search);
    /// Article role.
    pub const ARTICLE: Self = Self(AccessibilityRole::Article);
    /// Section role.
    pub const SECTION: Self = Self(AccessibilityRole::Section);

    /// Wraps any AccessKit role, covering roles without a named constant.
    #[must_use]
    pub const fn new(role: AccessibilityRole) -> Self {
        Self(role)
    }

    /// The underlying AccessKit role.
    #[must_use]
    pub const fn as_accesskit(self) -> AccessibilityRole {
        self.0
    }
}

/// Semantic checked state exposed by a checkable accessibility node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckedState {
    /// The node is not checked.
    False,
    /// The node is checked.
    True,
    /// The node is indeterminate or represents a mixed selection.
    Mixed,
}

impl From<AccessibilityRole> for Role {
    fn from(role: AccessibilityRole) -> Self {
        Self(role)
    }
}

/// Stable node identifier wrapper exposed by the testing API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(AccessibilityNodeId);

impl NodeId {
    /// Returns the numeric AccessKit node id.
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
    /// Creates node bounds from x/y origin and width/height.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns the x origin.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the y origin.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    /// Returns the width.
    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    /// Returns the height.
    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    /// Returns the center point.
    #[must_use]
    pub const fn center(self) -> (f32, f32) {
        (
            self.width.mul_add(0.5, self.x),
            self.height.mul_add(0.5, self.y),
        )
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "waterui-testing exposes f32 logical coordinates for pointer synthesis"
)]
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors independent accessibility attributes reported by the platform tree"
)]
pub struct NodeSnapshot {
    pub(crate) id: NodeId,
    pub(crate) role: Role,
    pub(crate) label: Option<String>,
    pub(crate) identifier: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) bounds: Option<NodeBounds>,
    pub(crate) enabled: bool,
    pub(crate) selected: bool,
    pub(crate) checked: Option<CheckedState>,
    pub(crate) expanded: Option<bool>,
    pub(crate) busy: bool,
    pub(crate) hidden: bool,
    pub(crate) children: Vec<NodeId>,
}

impl NodeSnapshot {
    /// Returns the node id.
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// Returns the accessibility role.
    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    /// Returns the accessibility label, if any.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Returns the stable automation identifier (`a11y_id`), if any.
    #[must_use]
    pub fn identifier(&self) -> Option<&str> {
        self.identifier.as_deref()
    }

    /// Returns the accessibility value, if any.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Returns whether the node is enabled.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Returns whether the node is selected.
    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }

    /// Returns the checked state, if applicable.
    #[must_use]
    pub const fn checked(&self) -> Option<bool> {
        match self.checked_state() {
            Some(CheckedState::False) => Some(false),
            Some(CheckedState::True) => Some(true),
            Some(CheckedState::Mixed) | None => None,
        }
    }

    /// Returns the complete checked state, preserving an indeterminate value.
    #[must_use]
    pub const fn checked_state(&self) -> Option<CheckedState> {
        self.checked
    }

    /// Returns the expanded state, if applicable.
    #[must_use]
    pub const fn expanded(&self) -> Option<bool> {
        self.expanded
    }

    /// Returns whether the node reports that it is busy.
    #[must_use]
    pub const fn busy(&self) -> bool {
        self.busy
    }

    /// Returns node bounds, if present.
    #[must_use]
    pub const fn bounds(&self) -> Option<NodeBounds> {
        self.bounds
    }

    /// Returns whether the node is hidden.
    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.hidden
    }

    /// Returns child node ids.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    fn from_accesskit(id: AccessibilityNodeId, node: &AccessibilityNode) -> Self {
        let checked = match node.toggled() {
            Some(AccessibilityToggled::True) => Some(CheckedState::True),
            Some(AccessibilityToggled::False) => Some(CheckedState::False),
            Some(AccessibilityToggled::Mixed) => Some(CheckedState::Mixed),
            None => None,
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
            identifier: node.author_id().map(ToOwned::to_owned),
            value,
            bounds: node.bounds().map(accesskit_rect_to_node_bounds),
            enabled: !node.is_disabled(),
            selected: node.is_selected().unwrap_or(false),
            checked,
            expanded,
            busy: node.is_busy(),
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
        let root = update.tree.as_ref().map_or_else(
            || NodeId::from(AccessibilityNodeId(0)),
            |tree| NodeId::from(tree.root),
        );
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

    /// Returns the monotonically increasing snapshot revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the root node id.
    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    /// Returns the accessibility focus node id.
    #[must_use]
    pub const fn focus(&self) -> NodeId {
        self.focus
    }

    /// Returns all nodes keyed by stable id.
    #[must_use]
    pub const fn nodes(&self) -> &BTreeMap<NodeId, NodeSnapshot> {
        &self.nodes
    }

    /// Returns one node by id.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&NodeSnapshot> {
        self.nodes.get(&id)
    }

    pub(crate) fn matching(&self, selector: &Selector) -> Vec<NodeId> {
        self.scoped_ids(selector)
            .into_iter()
            .filter(|id| selector.matches(&self[*id]))
            .collect()
    }

    fn scoped_ids(&self, selector: &Selector) -> Vec<NodeId> {
        let Some(scope) = selector.scope() else {
            return self.nodes.keys().copied().collect();
        };
        match scope.relation() {
            ScopeRelation::Descendants => self.descendants_of(scope.handle().id()),
            ScopeRelation::Children => self[scope.handle().id()].children().to_vec(),
        }
    }

    fn descendants_of(&self, parent: NodeId) -> Vec<NodeId> {
        let mut descendants = Vec::new();
        let mut stack = self[parent]
            .children()
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(node_id) = stack.pop() {
            descendants.push(node_id);
            stack.extend(self[node_id].children().iter().rev().copied());
        }
        descendants
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
