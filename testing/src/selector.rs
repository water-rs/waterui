use core::ops::Index;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use accesskit::{Action as AccessibilityAction, ActionData as AccessibilityActionData};

use crate::app::SemanticApp;
use crate::semantics::{NodeBounds, NodeId, NodeSnapshot, Role};

/// Chainable semantic selector.
#[derive(Debug, Clone)]
pub struct Selector {
    role: Option<Role>,
    label_exact: Option<String>,
    label_contains: Option<String>,
    enabled: Option<bool>,
    selected: Option<bool>,
    checked: Option<bool>,
    expanded: Option<bool>,
    value_exact: Option<String>,
    value_contains: Option<String>,
    hidden: Option<bool>,
    scope: Option<QueryScope>,
}

impl Default for Selector {
    fn default() -> Self {
        Self {
            role: None,
            label_exact: None,
            label_contains: None,
            enabled: None,
            selected: None,
            checked: None,
            expanded: None,
            value_exact: None,
            value_contains: None,
            hidden: Some(false),
            scope: None,
        }
    }
}

impl Selector {
    /// Restricts matches to an accessibility role.
    #[must_use]
    pub const fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    /// Restricts matches to an exact label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label_exact = Some(label.into());
        self
    }

    /// Restricts matches to labels containing text.
    #[must_use]
    pub fn label_contains(mut self, label: impl Into<String>) -> Self {
        self.label_contains = Some(label.into());
        self
    }

    /// Restricts matches to an enabled state.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    /// Restricts matches to a selected state.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Restricts matches to a checked state.
    #[must_use]
    pub const fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Restricts matches to an expanded state.
    #[must_use]
    pub const fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Restricts matches to an exact value.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value_exact = Some(value.into());
        self
    }

    /// Restricts matches to values containing text.
    #[must_use]
    pub fn value_contains(mut self, value: impl Into<String>) -> Self {
        self.value_contains = Some(value.into());
        self
    }

    /// Includes or excludes hidden nodes.
    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = Some(hidden);
        self
    }

    #[must_use]
    pub(crate) fn within(mut self, handle: ElementRef) -> Self {
        self.scope = Some(QueryScope::descendants(handle));
        self
    }

    #[must_use]
    pub(crate) fn children_of(mut self, handle: ElementRef) -> Self {
        self.scope = Some(QueryScope::children(handle));
        self
    }

    #[must_use]
    pub(crate) const fn scope(&self) -> Option<&QueryScope> {
        self.scope.as_ref()
    }

    #[must_use]
    pub(crate) fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(role) = self.role {
            parts.push(format!("role={role:?}"));
        }
        if let Some(label) = self.label_exact.as_deref() {
            parts.push(format!("label={label:?}"));
        }
        if let Some(label) = self.label_contains.as_deref() {
            parts.push(format!("label_contains={label:?}"));
        }
        if let Some(enabled) = self.enabled {
            parts.push(format!("enabled={enabled}"));
        }
        if let Some(selected) = self.selected {
            parts.push(format!("selected={selected}"));
        }
        if let Some(checked) = self.checked {
            parts.push(format!("checked={checked}"));
        }
        if let Some(expanded) = self.expanded {
            parts.push(format!("expanded={expanded}"));
        }
        if let Some(value) = self.value_exact.as_deref() {
            parts.push(format!("value={value:?}"));
        }
        if let Some(value) = self.value_contains.as_deref() {
            parts.push(format!("value_contains={value:?}"));
        }
        if let Some(hidden) = self.hidden {
            parts.push(format!("hidden={hidden}"));
        }
        if let Some(scope) = self.scope() {
            parts.push(scope.describe());
        }
        format!("[{}]", parts.join(", "))
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

        if let Some(expected) = self.value_contains.as_deref() {
            let Some(value) = node.value() else {
                return false;
            };
            if !value.contains(expected) {
                return false;
            }
        }

        if let Some(expected) = self.hidden
            && node.hidden() != expected
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
    pub(crate) revision: u64,
}

impl ElementRef {
    /// Returns the stable node id.
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.node_id
    }

    /// Returns the node snapshot captured when this handle was resolved.
    #[must_use]
    pub const fn node(&self) -> &NodeSnapshot {
        &self.node
    }

    #[must_use]
    pub(crate) const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub(crate) fn debug_summary(&self) -> String {
        let mut summary = format!(
            "id={}, revision={}, role={:?}",
            self.node_id.as_u64(),
            self.revision,
            self.node.role()
        );
        if let Some(label) = self.node.label() {
            let _ = write!(summary, ", label={label:?}");
        }
        if let Some(value) = self.node.value() {
            let _ = write!(summary, ", value={value:?}");
        }
        let _ = write!(
            summary,
            ", enabled={}, hidden={}",
            self.node.enabled(),
            self.node.hidden()
        );
        summary
    }

    /// Returns node bounds.
    ///
    /// # Panics
    ///
    /// Panics if the accessibility node does not expose bounds.
    #[must_use]
    pub fn bounds(&self) -> NodeBounds {
        self.node.bounds().unwrap_or_else(|| {
            panic!(
                "waterui-testing element {} is missing accessibility bounds",
                self.node_id.as_u64()
            )
        })
    }

    /// Returns the element center point.
    #[must_use]
    pub fn center(&self) -> (f32, f32) {
        self.bounds().center()
    }

    /// Returns a point inside the element from normalized coordinates.
    ///
    /// # Panics
    ///
    /// Panics if either coordinate is non-finite or outside `[0, 1]`.
    #[must_use]
    pub fn normalized_point(&self, normalized_x: f32, normalized_y: f32) -> (f32, f32) {
        assert!(
            normalized_x.is_finite() && normalized_y.is_finite(),
            "waterui-testing normalized coordinates must be finite"
        );
        assert!(
            (0.0..=1.0).contains(&normalized_x) && (0.0..=1.0).contains(&normalized_y),
            "waterui-testing normalized coordinates must be within [0, 1]"
        );
        let bounds = self.bounds();
        (
            bounds.width().mul_add(normalized_x, bounds.x()),
            bounds.height().mul_add(normalized_y, bounds.y()),
        )
    }

    /// Performs a click/tap action.
    pub fn tap(&self, app: &mut SemanticApp) -> bool {
        app.assert_current_element(self, "tap");
        app.perform_action(self.node_id, AccessibilityAction::Click, None)
    }

    /// Performs a pointer tap at the provided normalized coordinates.
    pub fn tap_at(&self, app: &mut SemanticApp, normalized_x: f32, normalized_y: f32) -> bool {
        app.assert_current_element(self, "tap_at");
        let (x, y) = self.normalized_point(normalized_x, normalized_y);
        app.tap_at(x, y)
    }

    /// Requests accessibility focus on the element.
    pub fn focus(&self, app: &mut SemanticApp) -> bool {
        app.assert_current_element(self, "focus");
        app.perform_action(self.node_id, AccessibilityAction::Focus, None)
    }

    /// Moves hover to the element center.
    pub fn hover(&self, app: &mut SemanticApp) -> bool {
        app.assert_current_element(self, "hover");
        let (x, y) = self.center();
        app.hover_at(x, y)
    }

    /// Moves hover to the provided normalized coordinates within the element.
    pub fn hover_at(&self, app: &mut SemanticApp, normalized_x: f32, normalized_y: f32) -> bool {
        app.assert_current_element(self, "hover_at");
        let (x, y) = self.normalized_point(normalized_x, normalized_y);
        app.hover_at(x, y)
    }

    /// Drags from the element center by the provided delta.
    pub fn drag_by(&self, app: &mut SemanticApp, dx: f32, dy: f32) -> bool {
        app.assert_current_element(self, "drag_by");
        let (x, y) = self.center();
        app.drag_from_to(x, y, x + dx, y + dy)
    }

    /// Drags between two normalized coordinates within the element.
    pub fn drag_between(
        &self,
        app: &mut SemanticApp,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
    ) -> bool {
        app.assert_current_element(self, "drag_between");
        let (start_x, start_y) = self.normalized_point(from_x, from_y);
        let (end_x, end_y) = self.normalized_point(to_x, to_y);
        app.drag_from_to(start_x, start_y, end_x, end_y)
    }

    /// Applies a magnification gesture centered on the element.
    pub fn magnify(&self, app: &mut SemanticApp, factor: f32) -> bool {
        app.assert_current_element(self, "magnify");
        let (x, y) = self.center();
        app.magnify_at(x, y, factor)
    }

    /// Sets textual value on editable controls.
    pub fn set_text(&self, app: &mut SemanticApp, value: impl Into<String>) -> bool {
        app.assert_current_element(self, "set_text");
        app.perform_action(
            self.node_id,
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        )
    }

    /// Increments current value for slider/stepper-like controls.
    pub fn increment(&self, app: &mut SemanticApp) -> bool {
        app.assert_current_element(self, "increment");
        app.perform_action(self.node_id, AccessibilityAction::Increment, None)
    }

    /// Decrements current value for slider/stepper-like controls.
    pub fn decrement(&self, app: &mut SemanticApp) -> bool {
        app.assert_current_element(self, "decrement");
        app.perform_action(self.node_id, AccessibilityAction::Decrement, None)
    }

    /// Scrolls down when supported by the node.
    pub fn scroll_down(&self, app: &mut SemanticApp) -> bool {
        app.assert_current_element(self, "scroll_down");
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
    pub(crate) fn new(elements: Vec<ElementRef>, _revision: u64) -> Self {
        let by_id = elements
            .iter()
            .enumerate()
            .map(|(idx, element)| (element.node_id, idx))
            .collect();
        Self { elements, by_id }
    }

    /// Returns the number of resolved elements.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.elements.len()
    }

    /// Returns whether the set contains no elements.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Iterates over resolved elements.
    pub fn iter(&self) -> impl Iterator<Item = &ElementRef> {
        self.elements.iter()
    }

    #[must_use]
    pub(crate) fn debug_summary(&self, limit: usize) -> String {
        if self.elements.is_empty() {
            return String::from("none");
        }
        let mut entries = self
            .elements
            .iter()
            .take(limit.max(1))
            .map(ElementRef::debug_summary)
            .collect::<Vec<_>>();
        if self.elements.len() > entries.len() {
            entries.push(format!("... {} more", self.elements.len() - entries.len()));
        }
        entries.join(" | ")
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

#[derive(Debug, Clone)]
pub struct QueryScope {
    relation: ScopeRelation,
    handle: ElementRef,
}

impl QueryScope {
    const fn descendants(handle: ElementRef) -> Self {
        Self {
            relation: ScopeRelation::Descendants,
            handle,
        }
    }

    const fn children(handle: ElementRef) -> Self {
        Self {
            relation: ScopeRelation::Children,
            handle,
        }
    }

    pub(crate) const fn relation(&self) -> ScopeRelation {
        self.relation
    }

    pub(crate) const fn handle(&self) -> &ElementRef {
        &self.handle
    }

    fn describe(&self) -> String {
        let relation = match self.relation {
            ScopeRelation::Descendants => "within",
            ScopeRelation::Children => "children_of",
        };
        format!("{relation}=({})", self.handle.debug_summary())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeRelation {
    Descendants,
    Children,
}
