use core::ops::Index;
use std::collections::BTreeMap;

use accesskit::{Action as AccessibilityAction, ActionData as AccessibilityActionData};

use crate::app::MountedApp;
use crate::semantics::{NodeBounds, NodeId, NodeSnapshot, Role};

/// Chainable semantic selector.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    role: Option<Role>,
    label_exact: Option<String>,
    label_contains: Option<String>,
    enabled: Option<bool>,
    selected: Option<bool>,
    checked: Option<bool>,
    expanded: Option<bool>,
    value_exact: Option<String>,
}

impl Selector {
    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label_exact = Some(label.into());
        self
    }

    #[must_use]
    pub fn label_contains(mut self, label: impl Into<String>) -> Self {
        self.label_contains = Some(label.into());
        self
    }

    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    #[must_use]
    pub const fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    #[must_use]
    pub const fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value_exact = Some(value.into());
        self
    }

    pub(crate) fn matches(&self, node: &NodeSnapshot) -> bool {
        if let Some(role) = self.role
            && node.role().as_accesskit() != role.as_accesskit()
        {
            return false;
        }

        if let Some(expected) = self.label_exact.as_deref()
            && node.label() != Some(expected)
        {
            return false;
        }

        if let Some(expected) = self.label_contains.as_deref() {
            let Some(label) = node.label() else {
                return false;
            };
            if !label.contains(expected) {
                return false;
            }
        }

        if let Some(expected) = self.enabled
            && node.enabled() != expected
        {
            return false;
        }

        if let Some(expected) = self.selected
            && node.selected() != expected
        {
            return false;
        }

        if let Some(expected) = self.checked
            && node.checked() != Some(expected)
        {
            return false;
        }

        if let Some(expected) = self.expanded
            && node.expanded() != Some(expected)
        {
            return false;
        }

        if let Some(expected) = self.value_exact.as_deref()
            && node.value() != Some(expected)
        {
            return false;
        }

        true
    }
}

/// Resolved element handle.
#[derive(Debug, Clone)]
pub struct ElementRef {
    pub(crate) node_id: NodeId,
    pub(crate) node: NodeSnapshot,
}

impl ElementRef {
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.node_id
    }

    #[must_use]
    pub fn node(&self) -> &NodeSnapshot {
        &self.node
    }

    #[must_use]
    pub fn bounds(&self) -> NodeBounds {
        self.node.bounds().unwrap_or_else(|| {
            panic!(
                "waterui-testing element {} is missing accessibility bounds",
                self.node_id.as_u64()
            )
        })
    }

    #[must_use]
    pub fn center(&self) -> (f32, f32) {
        self.bounds().center()
    }

    /// Performs a click/tap action.
    pub fn tap(&self, app: &mut MountedApp) -> bool {
        app.perform_action(self.node_id, AccessibilityAction::Click, None)
    }

    /// Requests accessibility focus on the element.
    pub fn focus(&self, app: &mut MountedApp) -> bool {
        app.perform_action(self.node_id, AccessibilityAction::Focus, None)
    }

    /// Moves hover to the element center.
    pub fn hover(&self, app: &mut MountedApp) -> bool {
        let (x, y) = self.center();
        app.hover_at(x, y)
    }

    /// Drags from the element center by the provided delta.
    pub fn drag_by(&self, app: &mut MountedApp, dx: f32, dy: f32) -> bool {
        let (x, y) = self.center();
        app.drag_from_to(x, y, x + dx, y + dy)
    }

    /// Applies a magnification gesture centered on the element.
    pub fn magnify(&self, app: &mut MountedApp, factor: f32) -> bool {
        let (x, y) = self.center();
        app.magnify_at(x, y, factor)
    }

    /// Sets textual value on editable controls.
    pub fn set_text(&self, app: &mut MountedApp, value: impl Into<String>) -> bool {
        app.perform_action(
            self.node_id,
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        )
    }

    /// Increments current value for slider/stepper-like controls.
    pub fn increment(&self, app: &mut MountedApp) -> bool {
        app.perform_action(self.node_id, AccessibilityAction::Increment, None)
    }

    /// Decrements current value for slider/stepper-like controls.
    pub fn decrement(&self, app: &mut MountedApp) -> bool {
        app.perform_action(self.node_id, AccessibilityAction::Decrement, None)
    }

    /// Scrolls down when supported by the node.
    pub fn scroll_down(&self, app: &mut MountedApp) -> bool {
        app.perform_action(self.node_id, AccessibilityAction::ScrollDown, None)
    }
}

/// A collection of resolved elements.
#[derive(Debug, Clone, Default)]
pub struct ElementSet {
    elements: Vec<ElementRef>,
    by_id: BTreeMap<NodeId, usize>,
}

impl ElementSet {
    pub(crate) fn new(elements: Vec<ElementRef>) -> Self {
        let by_id = elements
            .iter()
            .enumerate()
            .map(|(idx, element)| (element.node_id, idx))
            .collect();
        Self { elements, by_id }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.elements.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    #[must_use]
    pub fn iter(&self) -> impl Iterator<Item = &ElementRef> {
        self.elements.iter()
    }
}

impl Index<usize> for ElementSet {
    type Output = ElementRef;

    fn index(&self, index: usize) -> &Self::Output {
        self.elements.get(index).unwrap_or_else(|| {
            panic!(
                "waterui-testing element index {index} out of bounds (len={})",
                self.elements.len()
            )
        })
    }
}

impl Index<NodeId> for ElementSet {
    type Output = ElementRef;

    fn index(&self, index: NodeId) -> &Self::Output {
        let Some(position) = self.by_id.get(&index) else {
            panic!(
                "waterui-testing element id {} is not part of this result set",
                index.as_u64()
            );
        };
        &self.elements[*position]
    }
}
