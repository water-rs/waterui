use crate::app::SemanticApp;
use crate::selector::{ElementRef, ElementSet, Selector};
use crate::semantics::Role;

/// Chainable query builder bound to a mounted app session.
#[derive(Debug)]
pub struct Query<'a> {
    pub(crate) app: &'a mut SemanticApp,
    pub(crate) selector: Selector,
}

impl Query<'_> {
    /// Restricts the query to nodes with the given accessibility role.
    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.selector = self.selector.role(role);
        self
    }

    /// Restricts the query to nodes with exactly matching labels.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.selector = self.selector.label(label);
        self
    }

    /// Restricts the query to nodes whose labels contain the provided text.
    #[must_use]
    pub fn label_contains(mut self, label: impl Into<String>) -> Self {
        self.selector = self.selector.label_contains(label);
        self
    }

    /// Restricts the query to descendants of an element.
    #[must_use]
    pub fn within(mut self, handle: &ElementRef) -> Self {
        self.selector = self.selector.within(handle.clone());
        self
    }

    /// Restricts the query to direct children of an element.
    #[must_use]
    pub fn children_of(mut self, handle: &ElementRef) -> Self {
        self.selector = self.selector.children_of(handle.clone());
        self
    }

    /// Restricts the query to nodes with the requested enabled state.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.selector = self.selector.enabled(enabled);
        self
    }

    /// Restricts the query to nodes with the requested selected state.
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selector = self.selector.selected(selected);
        self
    }

    /// Restricts the query to nodes with the requested checked state.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.selector = self.selector.checked(checked);
        self
    }

    /// Restricts the query to nodes with the requested expanded state.
    #[must_use]
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.selector = self.selector.expanded(expanded);
        self
    }

    /// Restricts the query to nodes with exactly matching values.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.selector = self.selector.value(value);
        self
    }

    /// Restricts the query to nodes whose values contain the provided text.
    #[must_use]
    pub fn value_contains(mut self, value: impl Into<String>) -> Self {
        self.selector = self.selector.value_contains(value);
        self
    }

    /// Includes or excludes hidden nodes.
    #[must_use]
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.selector = self.selector.hidden(hidden);
        self
    }

    /// Resolves all matching elements.
    #[must_use]
    pub fn all(self) -> ElementSet {
        self.app.resolve_elements(&self.selector)
    }

    /// Resolves zero or one matching element.
    ///
    /// # Panics
    ///
    /// Panics if the selector resolves more than one element.
    #[must_use]
    pub fn optional(self) -> Option<ElementRef> {
        let all = self.app.resolve_elements(&self.selector);
        if all.is_empty() {
            return None;
        }
        assert!(
            all.len() <= 1,
            "waterui-testing selector {} resolved {} nodes, expected at most 1; candidates: {}",
            self.selector.describe(),
            all.len(),
            all.debug_summary(3)
        );
        Some(all[0].clone())
    }

    /// Resolves exactly one matching element.
    #[must_use]
    pub fn single(self) -> ElementRef {
        self.app.resolve_single(&self.selector)
    }

    /// Returns whether at least one matching element exists.
    #[must_use]
    pub fn exists(self) -> bool {
        self.optional().is_some()
    }

    /// Asserts that at least one matching element exists.
    pub fn assert_exists(self) {
        self.app.assert_exists(&self.selector);
    }

    /// Asserts that no matching element exists.
    pub fn assert_not_exists(self) {
        self.app.assert_not_exists(&self.selector);
    }

    /// Asserts that the matching element owns `WaterUI` UI focus.
    pub fn assert_ui_focus(self) {
        self.app.assert_ui_focus(&self.selector);
    }

    /// Waits until at least one matching element exists.
    #[must_use]
    pub fn wait_for_existence(self, timeout: std::time::Duration) -> bool {
        self.app.wait_for_existence(&self.selector, timeout)
    }

    /// Waits until no matching element exists.
    #[must_use]
    pub fn wait_for_nonexistence(self, timeout: std::time::Duration) -> bool {
        self.app.wait_for_nonexistence(&self.selector, timeout)
    }

    /// Waits until one matching element has the expected value.
    #[must_use]
    pub fn wait_for_value_eq(self, value: impl Into<String>, timeout: std::time::Duration) -> bool {
        self.app.wait_for_value_eq(&self.selector, value, timeout)
    }

    /// Performs a tap on the matching element.
    #[must_use]
    pub fn tap(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.tap_node(element.id())
    }

    /// Requests accessibility focus on the matching element.
    #[must_use]
    pub fn focus(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.focus_node(element.id())
    }

    /// Sets text on the matching editable element.
    #[must_use]
    pub fn set_text(self, value: impl Into<String>) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.set_text_node(element.id(), value)
    }

    /// Performs an increment action on the matching element.
    #[must_use]
    pub fn increment(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.increment_node(element.id())
    }

    /// Performs a decrement action on the matching element.
    #[must_use]
    pub fn decrement(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.decrement_node(element.id())
    }

    /// Performs a scroll-down action on the matching element.
    #[must_use]
    pub fn scroll_down(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.scroll_down_node(element.id())
    }

    /// Moves hover to the matching element center.
    #[must_use]
    pub fn hover(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.hover(self.app)
    }

    /// Moves hover to a normalized point inside the matching element.
    #[must_use]
    pub fn hover_at(self, normalized_x: f32, normalized_y: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.hover_at(self.app, normalized_x, normalized_y)
    }

    /// Performs a tap at a normalized point inside the matching element.
    #[must_use]
    pub fn tap_at(self, normalized_x: f32, normalized_y: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.tap_at(self.app, normalized_x, normalized_y)
    }

    /// Drags from the matching element center by a delta.
    #[must_use]
    pub fn drag_by(self, dx: f32, dy: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.drag_by(self.app, dx, dy)
    }

    /// Drags between normalized points inside the matching element.
    #[must_use]
    pub fn drag_between(self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.drag_between(self.app, from_x, from_y, to_x, to_y)
    }

    /// Applies a magnification gesture to the matching element.
    #[must_use]
    pub fn magnify(self, factor: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.magnify(self.app, factor)
    }
}
