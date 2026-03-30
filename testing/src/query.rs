use crate::app::MountedApp;
use crate::selector::{ElementRef, ElementSet, Selector};
use crate::semantics::Role;

/// Chainable query builder bound to a mounted app session.
#[derive(Debug)]
pub struct Query<'a> {
    pub(crate) app: &'a mut MountedApp,
    pub(crate) selector: Selector,
}

impl<'a> Query<'a> {
    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.selector = self.selector.role(role);
        self
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.selector = self.selector.label(label);
        self
    }

    #[must_use]
    pub fn label_contains(mut self, label: impl Into<String>) -> Self {
        self.selector = self.selector.label_contains(label);
        self
    }

    #[must_use]
    pub fn within(mut self, handle: &ElementRef) -> Self {
        self.selector = self.selector.within(handle.clone());
        self
    }

    #[must_use]
    pub fn children_of(mut self, handle: &ElementRef) -> Self {
        self.selector = self.selector.children_of(handle.clone());
        self
    }

    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.selector = self.selector.enabled(enabled);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selector = self.selector.selected(selected);
        self
    }

    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.selector = self.selector.checked(checked);
        self
    }

    #[must_use]
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.selector = self.selector.expanded(expanded);
        self
    }

    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.selector = self.selector.value(value);
        self
    }

    #[must_use]
    pub fn value_contains(mut self, value: impl Into<String>) -> Self {
        self.selector = self.selector.value_contains(value);
        self
    }

    #[must_use]
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.selector = self.selector.hidden(hidden);
        self
    }

    #[must_use]
    pub fn all(self) -> ElementSet {
        self.app.resolve_elements(&self.selector)
    }

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

    #[must_use]
    pub fn single(self) -> ElementRef {
        self.app.resolve_single(&self.selector)
    }

    #[must_use]
    pub fn exists(self) -> bool {
        self.optional().is_some()
    }

    pub fn assert_exists(self) {
        self.app.assert_exists(self.selector.clone());
    }

    pub fn assert_not_exists(self) {
        self.app.assert_not_exists(self.selector.clone());
    }

    pub fn assert_ui_focus(self) {
        self.app.assert_ui_focus(self.selector.clone());
    }

    pub fn wait_for_existence(self, timeout: std::time::Duration) -> bool {
        self.app.wait_for_existence(self.selector.clone(), timeout)
    }

    pub fn wait_for_nonexistence(self, timeout: std::time::Duration) -> bool {
        self.app
            .wait_for_nonexistence(self.selector.clone(), timeout)
    }

    pub fn wait_for_value_eq(self, value: impl Into<String>, timeout: std::time::Duration) -> bool {
        self.app
            .wait_for_value_eq(self.selector.clone(), value, timeout)
    }

    pub fn tap(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.tap_node(element.id())
    }

    pub fn focus(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.focus_node(element.id())
    }

    pub fn set_text(self, value: impl Into<String>) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.set_text_node(element.id(), value)
    }

    pub fn increment(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.increment_node(element.id())
    }

    pub fn decrement(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.decrement_node(element.id())
    }

    pub fn scroll_down(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.scroll_down_node(element.id())
    }

    pub fn hover(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.hover(self.app)
    }

    pub fn hover_at(self, normalized_x: f32, normalized_y: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.hover_at(self.app, normalized_x, normalized_y)
    }

    pub fn tap_at(self, normalized_x: f32, normalized_y: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.tap_at(self.app, normalized_x, normalized_y)
    }

    pub fn drag_by(self, dx: f32, dy: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.drag_by(self.app, dx, dy)
    }

    pub fn drag_between(self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.drag_between(self.app, from_x, from_y, to_x, to_y)
    }

    pub fn magnify(self, factor: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.magnify(self.app, factor)
    }
}
