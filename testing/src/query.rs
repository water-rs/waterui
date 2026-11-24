use core::marker::PhantomData;

use accesskit::{Action as AccessibilityAction, ActionData as AccessibilityActionData};
use hydrolysis::HeadlessRuntime;
use waterui::component::input::ContentType;

use crate::app::{DragOptions, SemanticApp};
use crate::driver::RuntimeDriver;
use crate::selector::{ElementRef, ElementSet, Selector};
use crate::semantics::{NodeId, Role};

/// Dispatch mode that settles pending runtime work immediately after each
/// interaction terminal on [`Query`].
#[derive(Debug, Clone, Copy)]
pub struct Settled;

/// Dispatch mode that queues interactions for the next measured frame without
/// settling, returned by [`PerfRun::query`](crate::PerfRun::query).
#[derive(Debug, Clone, Copy)]
pub struct Queued;

mod sealed {
    use super::{Queued, Settled};

    pub trait Sealed {}
    impl Sealed for Settled {}
    impl Sealed for Queued {}
}

/// Controls whether a [`Query`] syncs before resolving and settles after
/// dispatching an interaction.
pub trait QueryMode: sealed::Sealed {
    #[doc(hidden)]
    fn resolve_elements<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementSet<R>;
    #[doc(hidden)]
    fn resolve_single<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementRef<R>;
    #[doc(hidden)]
    fn assert_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector);
    #[doc(hidden)]
    fn assert_not_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector);
    #[doc(hidden)]
    fn assert_ui_focus<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector);
    #[doc(hidden)]
    fn dispatch_action<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    );
}

impl QueryMode for Settled {
    fn resolve_elements<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementSet<R> {
        app.resolve_elements(selector)
    }

    fn resolve_single<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementRef<R> {
        app.resolve_single(selector)
    }

    fn assert_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_exists(selector);
    }

    fn assert_not_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_not_exists(selector);
    }

    fn assert_ui_focus<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_ui_focus(selector);
    }

    fn dispatch_action<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) {
        app.perform_action_expect(node_id, action, data);
    }
}

impl QueryMode for Queued {
    fn resolve_elements<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementSet<R> {
        app.resolve_elements_unsynced(selector)
    }

    fn resolve_single<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementRef<R> {
        app.resolve_single_unsynced(selector)
    }

    fn assert_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_exists_unsynced(selector);
    }

    fn assert_not_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_not_exists_unsynced(selector);
    }

    fn assert_ui_focus<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_ui_focus_unsynced(selector);
    }

    fn dispatch_action<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) {
        app.queue_action_expect(node_id, action, data);
    }
}

/// Geometry dispatch behavior for [`Query<'_, HeadlessRuntime, M>`].
pub trait RenderedQueryMode: QueryMode {
    #[doc(hidden)]
    fn pointer_point(
        element: &ElementRef<HeadlessRuntime>,
        app: &SemanticApp<HeadlessRuntime>,
        op: &str,
        normalized_x: f32,
        normalized_y: f32,
    ) -> (f32, f32);
    #[doc(hidden)]
    fn hover_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32);
    #[doc(hidden)]
    fn tap_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32);
    #[doc(hidden)]
    fn drag_from_to_with(
        app: &mut SemanticApp<HeadlessRuntime>,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        options: DragOptions,
    );
    #[doc(hidden)]
    fn magnify_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32, factor: f32);
}

impl RenderedQueryMode for Settled {
    fn pointer_point(
        element: &ElementRef<HeadlessRuntime>,
        app: &SemanticApp<HeadlessRuntime>,
        op: &str,
        normalized_x: f32,
        normalized_y: f32,
    ) -> (f32, f32) {
        element.pointer_point(app, op, normalized_x, normalized_y)
    }

    fn hover_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32) {
        app.hover_at(x, y);
    }

    fn tap_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32) {
        app.tap_at(x, y);
    }

    fn drag_from_to_with(
        app: &mut SemanticApp<HeadlessRuntime>,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        options: DragOptions,
    ) {
        app.drag_from_to_with(from_x, from_y, to_x, to_y, options);
    }

    fn magnify_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32, factor: f32) {
        app.magnify_at(x, y, factor);
    }
}

impl RenderedQueryMode for Queued {
    fn pointer_point(
        element: &ElementRef<HeadlessRuntime>,
        app: &SemanticApp<HeadlessRuntime>,
        op: &str,
        normalized_x: f32,
        normalized_y: f32,
    ) -> (f32, f32) {
        element.pointer_point(app, op, normalized_x, normalized_y)
    }

    fn hover_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32) {
        app.queue_hover_at(x, y);
    }

    fn tap_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32) {
        app.queue_tap_at(x, y);
    }

    fn drag_from_to_with(
        app: &mut SemanticApp<HeadlessRuntime>,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        options: DragOptions,
    ) {
        assert!(
            !options.frame_per_step(),
            "waterui-testing queued perf drags must not pump intermediate frames (`frame_per_step = true`)"
        );
        app.queue_drag_from_to_with(from_x, from_y, to_x, to_y, options);
    }

    fn magnify_at(app: &mut SemanticApp<HeadlessRuntime>, x: f32, y: f32, factor: f32) {
        app.queue_magnify_at(x, y, factor);
    }
}

/// Chainable query builder bound to a mounted app session.
///
/// `R` is the session's runtime — `SemanticRuntime` on a style-free
/// [`SemanticApp`], `HeadlessRuntime` on a rendered one. `M` selects whether
/// interaction terminals settle immediately ([`Settled`], the default for tests)
/// or queue for the next measured frame ([`Queued`], returned by
/// [`PerfRun::query`](crate::PerfRun::query)). The semantic terminals exist on
/// every query; the geometry terminals — `hover`, `tap_at`, `drag_by`,
/// `magnify` — exist only on `Query<'_, HeadlessRuntime, M>`, so a semantic
/// query has no way to reach for pixels.
#[derive(Debug)]
#[must_use = "a query does nothing until a terminal call such as `tap`, `assert_exists`, or `wait_for_existence`"]
pub struct Query<'a, R = hydrolysis::SemanticRuntime, M = Settled> {
    pub(crate) app: &'a mut SemanticApp<R>,
    pub(crate) selector: Selector,
    _mode: PhantomData<fn() -> M>,
}

impl<'a, R> Query<'a, R, Settled> {
    pub(crate) fn settled(app: &'a mut SemanticApp<R>) -> Self {
        Self {
            app,
            selector: Selector::default(),
            _mode: PhantomData,
        }
    }
}

impl<'a, R> Query<'a, R, Queued> {
    pub(crate) fn queued(app: &'a mut SemanticApp<R>) -> Self {
        Self {
            app,
            selector: Selector::default(),
            _mode: PhantomData,
        }
    }
}

impl<R: RuntimeDriver, M: QueryMode> Query<'_, R, M> {
    /// Restricts the query to nodes with the given accessibility role.
    pub fn role(mut self, role: Role) -> Self {
        self.selector = self.selector.role(role);
        self
    }

    /// Restricts the query to nodes with the given automation identifier.
    pub fn identifier(mut self, identifier: impl Into<String>) -> Self {
        self.selector = self.selector.identifier(identifier);
        self
    }

    /// Restricts the query to nodes with exactly matching labels.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.selector = self.selector.label(label);
        self
    }

    /// Restricts the query to nodes whose labels contain the provided text.
    pub fn label_contains(mut self, label: impl Into<String>) -> Self {
        self.selector = self.selector.label_contains(label);
        self
    }

    /// Restricts the query to descendants of an element.
    pub fn within(mut self, handle: &ElementRef<R>) -> Self {
        self.selector = self.selector.within(handle.clone());
        self
    }

    /// Restricts the query to direct children of an element.
    pub fn children_of(mut self, handle: &ElementRef<R>) -> Self {
        self.selector = self.selector.children_of(handle.clone());
        self
    }

    /// Restricts the query to nodes with the requested enabled state.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.selector = self.selector.enabled(enabled);
        self
    }

    /// Restricts the query to nodes with the requested selected state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selector = self.selector.selected(selected);
        self
    }

    /// Restricts the query to nodes with the requested checked state.
    pub fn checked(mut self, checked: bool) -> Self {
        self.selector = self.selector.checked(checked);
        self
    }

    /// Restricts the query to nodes with an indeterminate checked state.
    pub fn mixed(mut self) -> Self {
        self.selector = self.selector.mixed();
        self
    }

    /// Restricts the query to nodes with the requested expanded state.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.selector = self.selector.expanded(expanded);
        self
    }

    /// Restricts the query to nodes with the requested busy state.
    pub fn busy(mut self, busy: bool) -> Self {
        self.selector = self.selector.busy(busy);
        self
    }

    /// Restricts the query to nodes with exactly matching values.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.selector = self.selector.value(value);
        self
    }

    /// Restricts the query to nodes whose values contain the provided text.
    pub fn value_contains(mut self, value: impl Into<String>) -> Self {
        self.selector = self.selector.value_contains(value);
        self
    }

    /// Restricts the query to nodes with exactly matching placeholders.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.selector = self.selector.placeholder(placeholder);
        self
    }

    /// Restricts the query to nodes whose placeholders contain the provided text.
    pub fn placeholder_contains(mut self, placeholder: impl Into<String>) -> Self {
        self.selector = self.selector.placeholder_contains(placeholder);
        self
    }

    /// Includes or excludes hidden nodes.
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.selector = self.selector.hidden(hidden);
        self
    }

    /// Restricts the query to nodes declaring the given content type.
    pub fn content_type(mut self, content_type: ContentType) -> Self {
        self.selector = self.selector.content_type(content_type);
        self
    }

    /// Resolves all matching elements.
    #[must_use]
    pub fn all(self) -> ElementSet<R> {
        M::resolve_elements(self.app, &self.selector)
    }

    /// Resolves zero or one matching element.
    ///
    /// # Panics
    ///
    /// Panics if the selector resolves more than one element.
    #[must_use]
    pub fn optional(self) -> Option<ElementRef<R>> {
        let all = M::resolve_elements(self.app, &self.selector);
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
    pub fn single(self) -> ElementRef<R> {
        M::resolve_single(self.app, &self.selector)
    }

    /// Returns whether at least one matching element exists.
    #[must_use]
    pub fn exists(self) -> bool {
        !self.all().is_empty()
    }

    /// Asserts that at least one matching element exists.
    pub fn assert_exists(self) {
        M::assert_exists(self.app, &self.selector);
    }

    /// Asserts that no matching element exists.
    pub fn assert_not_exists(self) {
        M::assert_not_exists(self.app, &self.selector);
    }

    /// Asserts that the matching element owns `WaterUI` UI focus.
    pub fn assert_ui_focus(self) {
        M::assert_ui_focus(self.app, &self.selector);
    }

    /// Performs a tap on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the click action.
    pub fn tap(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Click, None);
    }

    /// Requests accessibility focus on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the focus action.
    pub fn focus(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Focus, None);
    }

    /// Sets text on the matching editable element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the set-value action.
    pub fn set_text(self, value: impl Into<String>) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(
            self.app,
            element.id(),
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        );
    }

    /// Performs an increment action on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the increment action.
    pub fn increment(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Increment, None);
    }

    /// Performs a decrement action on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the decrement action.
    pub fn decrement(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Decrement, None);
    }

    /// Performs a scroll-down action on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the scroll-down action.
    pub fn scroll_down(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(
            self.app,
            element.id(),
            AccessibilityAction::ScrollDown,
            None,
        );
    }

    /// Expands the matching collapsible element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the expand action.
    pub fn expand(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Expand, None);
    }

    /// Collapses the matching expanded element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the collapse action.
    pub fn collapse(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Collapse, None);
    }
}

/// Wait terminals available only on settled test queries.
impl<R: RuntimeDriver> Query<'_, R, Settled> {
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
}

/// The geometry terminals a query reaches only on the rendered runtime.
impl<M: RenderedQueryMode> Query<'_, HeadlessRuntime, M> {
    /// Moves hover to the matching element center.
    pub fn hover(self) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "hover", 0.5, 0.5);
        M::hover_at(self.app, x, y);
    }

    /// Moves hover to a normalized point inside the matching element.
    pub fn hover_at(self, normalized_x: f32, normalized_y: f32) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "hover_at", normalized_x, normalized_y);
        M::hover_at(self.app, x, y);
    }

    /// Performs a tap at a normalized point inside the matching element.
    ///
    /// # Panics
    ///
    /// Panics when the element has no visible fragment a pointer can reach —
    /// a fully clipped element reports not visible rather than tapping into
    /// whatever lies under the clip.
    pub fn tap_at(self, normalized_x: f32, normalized_y: f32) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "tap_at", normalized_x, normalized_y);
        M::tap_at(self.app, x, y);
    }

    /// Drags from the matching element center by a delta.
    pub fn drag_by(self, dx: f32, dy: f32) {
        self.drag_by_with(dx, dy, DragOptions::default());
    }

    /// Drags from the matching element center by a delta with step/timing control.
    pub fn drag_by_with(self, dx: f32, dy: f32, options: DragOptions) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "drag_by", 0.5, 0.5);
        M::drag_from_to_with(self.app, x, y, x + dx, y + dy, options);
    }

    /// Drags between normalized points inside the matching element.
    pub fn drag_between(self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) {
        let element = M::resolve_single(self.app, &self.selector);
        let (start_x, start_y) =
            M::pointer_point(&element, self.app, "drag_between", from_x, from_y);
        let (end_x, end_y) = M::pointer_point(&element, self.app, "drag_between", to_x, to_y);
        M::drag_from_to_with(
            self.app,
            start_x,
            start_y,
            end_x,
            end_y,
            DragOptions::default(),
        );
    }

    /// Applies a magnification gesture to the matching element.
    pub fn magnify(self, factor: f32) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "magnify", 0.5, 0.5);
        M::magnify_at(self.app, x, y, factor);
    }
}
