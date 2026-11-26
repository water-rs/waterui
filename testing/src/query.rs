use core::borrow::Borrow;
use core::marker::PhantomData;
use core::time::Duration;

use accesskit::{Action as AccessibilityAction, ActionData as AccessibilityActionData};
use hydrolysis::HeadlessRuntime;
use waterui::component::input::ContentType;

use crate::app::{DragOptions, SemanticApp};
use crate::driver::RuntimeDriver;
use crate::selector::{ElementRef, ElementSet, Selector, define_selector_filters};
use crate::semantics::{NodeId, Role};
use crate::wait::{Expectation, WaitOptions, WaitResult};

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
    #[track_caller]
    fn resolve_elements<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementSet<R>;
    #[doc(hidden)]
    #[track_caller]
    fn resolve_single<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementRef<R>;
    #[doc(hidden)]
    #[track_caller]
    fn assert_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector);
    #[doc(hidden)]
    #[track_caller]
    fn assert_not_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector);
    #[doc(hidden)]
    #[track_caller]
    fn assert_ui_focus<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector);
    #[doc(hidden)]
    #[track_caller]
    fn dispatch_action<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    );
}

impl QueryMode for Settled {
    #[track_caller]
    fn resolve_elements<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementSet<R> {
        app.resolve_elements(selector)
    }

    #[track_caller]
    fn resolve_single<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementRef<R> {
        app.resolve_single(selector)
    }

    #[track_caller]
    fn assert_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_exists(selector);
    }

    #[track_caller]
    fn assert_not_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_not_exists(selector);
    }

    #[track_caller]
    fn assert_ui_focus<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_ui_focus(selector);
    }

    #[track_caller]
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
    #[track_caller]
    fn resolve_elements<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementSet<R> {
        app.resolve_elements_unsynced(selector)
    }

    #[track_caller]
    fn resolve_single<R: RuntimeDriver>(
        app: &mut SemanticApp<R>,
        selector: &Selector,
    ) -> ElementRef<R> {
        app.resolve_single_unsynced(selector)
    }

    #[track_caller]
    fn assert_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_exists_unsynced(selector);
    }

    #[track_caller]
    fn assert_not_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_not_exists_unsynced(selector);
    }

    #[track_caller]
    fn assert_ui_focus<R: RuntimeDriver>(app: &mut SemanticApp<R>, selector: &Selector) {
        app.assert_ui_focus_unsynced(selector);
    }

    #[track_caller]
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
    #[track_caller]
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
    #[track_caller]
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
    #[track_caller]
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

    #[track_caller]
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
    #[track_caller]
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

    #[track_caller]
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
            selector: Selector::new(),
            _mode: PhantomData,
        }
    }
}

impl<'a, R> Query<'a, R, Queued> {
    pub(crate) fn queued(app: &'a mut SemanticApp<R>) -> Self {
        Self {
            app,
            selector: Selector::new(),
            _mode: PhantomData,
        }
    }
}

impl<R: RuntimeDriver, M: QueryMode> Query<'_, R, M> {
    define_selector_filters!(@query self);

    /// Resolves all matching elements.
    #[must_use]
    #[track_caller]
    pub fn all(self) -> ElementSet<R> {
        M::resolve_elements(self.app, &self.selector)
    }

    /// Resolves zero or one matching element.
    ///
    /// # Panics
    ///
    /// Panics if the selector resolves more than one element.
    #[must_use]
    #[track_caller]
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
    #[track_caller]
    pub fn single(self) -> ElementRef<R> {
        M::resolve_single(self.app, &self.selector)
    }

    /// Returns whether at least one matching element exists.
    #[must_use]
    #[track_caller]
    pub fn exists(self) -> bool {
        !self.all().is_empty()
    }

    /// Asserts that at least one matching element exists.
    #[track_caller]
    pub fn assert_exists(self) {
        M::assert_exists(self.app, &self.selector);
    }

    /// Asserts that no matching element exists.
    #[track_caller]
    pub fn assert_not_exists(self) {
        M::assert_not_exists(self.app, &self.selector);
    }

    /// Asserts that the matching element owns `WaterUI` UI focus.
    #[track_caller]
    pub fn assert_ui_focus(self) {
        M::assert_ui_focus(self.app, &self.selector);
    }

    /// Performs a tap on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the click action.
    #[track_caller]
    pub fn tap(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Click, None);
    }

    /// Requests accessibility focus on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the focus action.
    #[track_caller]
    pub fn focus(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Focus, None);
    }

    /// Sets text on the matching editable element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the set-value action.
    #[track_caller]
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
    #[track_caller]
    pub fn increment(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Increment, None);
    }

    /// Performs a decrement action on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the decrement action.
    #[track_caller]
    pub fn decrement(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Decrement, None);
    }

    /// Performs a scroll-down action on the matching element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the scroll-down action.
    #[track_caller]
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
    #[track_caller]
    pub fn expand(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Expand, None);
    }

    /// Collapses the matching expanded element.
    ///
    /// # Panics
    ///
    /// Panics if the element does not handle the collapse action.
    #[track_caller]
    pub fn collapse(self) {
        let element = M::resolve_single(self.app, &self.selector);
        M::dispatch_action(self.app, element.id(), AccessibilityAction::Collapse, None);
    }
}

/// Expectation and wait terminals available only on settled test queries.
impl<R: RuntimeDriver> Query<'_, R, Settled> {
    #[track_caller]
    fn wait_expectation_within(
        self,
        make: impl FnOnce(Selector) -> Expectation,
        timeout: Duration,
    ) -> bool {
        let expectation = make(self.selector);
        self.app.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    #[track_caller]
    fn wait_expectation_within_frames(
        self,
        make: impl FnOnce(Selector) -> Expectation,
        frame_budget: usize,
    ) -> bool {
        let expectation = make(self.selector);
        self.app.wait_for_frames(&[expectation], frame_budget) == WaitResult::Completed
    }

    #[track_caller]
    fn assert_expectation_within(
        self,
        make: impl FnOnce(Selector) -> Expectation,
        diagnostic_selector: &Selector,
        what: &str,
        timeout: Duration,
    ) {
        let description = diagnostic_selector.describe();
        let expectation = make(self.selector);
        let stats = self
            .app
            .wait_for_with_stats(&[expectation], WaitOptions::new(timeout));
        if stats.result != WaitResult::Completed {
            let nearest = self.app.tree.closest_candidates(diagnostic_selector, 3);
            panic!(
                "waterui-testing expected {what} for selector {description} within {timeout:?}, got {:?}; nearest: {nearest}; waited {} frames ({:?})",
                stats.result, stats.frames, stats.elapsed
            );
        }
    }

    #[track_caller]
    fn assert_expectation_within_frames(
        self,
        make: impl FnOnce(Selector) -> Expectation,
        diagnostic_selector: &Selector,
        what: &str,
        frame_budget: usize,
    ) {
        let description = diagnostic_selector.describe();
        let expectation = make(self.selector);
        let stats = self
            .app
            .wait_for_frames_with_stats(&[expectation], frame_budget);
        if stats.result != WaitResult::Completed {
            let nearest = self.app.tree.closest_candidates(diagnostic_selector, 3);
            panic!(
                "waterui-testing expected {what} for selector {description} within {frame_budget} frames, got {:?}; nearest: {nearest}; waited {} frames ({:?})",
                stats.result, stats.frames, stats.elapsed
            );
        }
    }

    /// Turns this query into an existence [`Expectation`].
    #[must_use]
    #[track_caller]
    pub fn expect_exists(self) -> Expectation {
        Expectation::exists(self.selector)
    }

    /// Turns this query into a non-existence [`Expectation`].
    #[must_use]
    #[track_caller]
    pub fn expect_not_exists(self) -> Expectation {
        Expectation::not_exists(self.selector)
    }

    /// Turns this query into a value-equality [`Expectation`].
    #[must_use]
    #[track_caller]
    pub fn expect_value_eq(self, value: impl Into<String>) -> Expectation {
        Expectation::value_eq(self.selector, value)
    }

    /// Turns this query into a UI-focus [`Expectation`].
    #[must_use]
    #[track_caller]
    pub fn expect_ui_focus(self) -> Expectation {
        Expectation::ui_focus(self.selector)
    }

    /// Waits until at least one matching element exists.
    #[must_use]
    #[track_caller]
    pub fn wait_for_existence(self, timeout: Duration) -> bool {
        self.wait_expectation_within(Expectation::exists, timeout)
    }

    /// Waits until no matching element exists.
    #[must_use]
    #[track_caller]
    pub fn wait_for_nonexistence(self, timeout: Duration) -> bool {
        self.wait_expectation_within(Expectation::not_exists, timeout)
    }

    /// Waits until one matching element has the expected value.
    #[must_use]
    #[track_caller]
    pub fn wait_for_value_eq(self, value: impl Into<String>, timeout: Duration) -> bool {
        let value = value.into();
        self.wait_expectation_within(|s| Expectation::value_eq(s, value), timeout)
    }

    /// Waits until the matching element owns `WaterUI` UI focus.
    #[must_use]
    #[track_caller]
    pub fn wait_for_ui_focus(self, timeout: Duration) -> bool {
        self.wait_expectation_within(Expectation::ui_focus, timeout)
    }

    /// Waits within `frame_budget` pumped frames until at least one matching
    /// element exists.
    #[must_use]
    #[track_caller]
    pub fn wait_for_existence_within_frames(self, frame_budget: usize) -> bool {
        self.wait_expectation_within_frames(Expectation::exists, frame_budget)
    }

    /// Waits within `frame_budget` pumped frames until no matching element
    /// exists.
    #[must_use]
    #[track_caller]
    pub fn wait_for_nonexistence_within_frames(self, frame_budget: usize) -> bool {
        self.wait_expectation_within_frames(Expectation::not_exists, frame_budget)
    }

    /// Waits within `frame_budget` pumped frames until one matching element has
    /// the expected value.
    #[must_use]
    #[track_caller]
    pub fn wait_for_value_eq_within_frames(
        self,
        value: impl Into<String>,
        frame_budget: usize,
    ) -> bool {
        let value = value.into();
        self.wait_expectation_within_frames(|s| Expectation::value_eq(s, value), frame_budget)
    }

    /// Waits within `frame_budget` pumped frames until the matching element
    /// owns `WaterUI` UI focus.
    #[must_use]
    #[track_caller]
    pub fn wait_for_ui_focus_within_frames(self, frame_budget: usize) -> bool {
        self.wait_expectation_within_frames(Expectation::ui_focus, frame_budget)
    }

    /// Asserts that at least one matching element exists within `timeout`.
    #[track_caller]
    pub fn assert_exists_within(self, timeout: Duration) {
        let diag = self.selector.clone();
        self.assert_expectation_within(Expectation::exists, &diag, "existence", timeout);
    }

    /// Asserts that no matching element exists within `timeout`.
    #[track_caller]
    pub fn assert_not_exists_within(self, timeout: Duration) {
        let diag = self.selector.clone();
        self.assert_expectation_within(Expectation::not_exists, &diag, "nonexistence", timeout);
    }

    /// Asserts that one matching element reaches `value` within `timeout`.
    #[track_caller]
    pub fn assert_value_eq_within(self, value: impl Into<String>, timeout: Duration) {
        let value = value.into();
        let diag = self.selector.clone().with_expected_value(value.clone());
        self.assert_expectation_within(
            |s| Expectation::value_eq(s, value),
            &diag,
            "value equality",
            timeout,
        );
    }

    /// Asserts that the matching element owns `WaterUI` UI focus within
    /// `timeout`.
    #[track_caller]
    pub fn assert_ui_focus_within(self, timeout: Duration) {
        let diag = self.selector.clone();
        self.assert_expectation_within(Expectation::ui_focus, &diag, "UI focus", timeout);
    }

    /// Asserts that at least one matching element exists within `frame_budget`
    /// pumped frames.
    #[track_caller]
    pub fn assert_exists_within_frames(self, frame_budget: usize) {
        let diag = self.selector.clone();
        self.assert_expectation_within_frames(
            Expectation::exists,
            &diag,
            "existence",
            frame_budget,
        );
    }

    /// Asserts that no matching element exists within `frame_budget` pumped
    /// frames.
    #[track_caller]
    pub fn assert_not_exists_within_frames(self, frame_budget: usize) {
        let diag = self.selector.clone();
        self.assert_expectation_within_frames(
            Expectation::not_exists,
            &diag,
            "nonexistence",
            frame_budget,
        );
    }

    /// Asserts that one matching element reaches `value` within `frame_budget`
    /// pumped frames.
    #[track_caller]
    pub fn assert_value_eq_within_frames(self, value: impl Into<String>, frame_budget: usize) {
        let value = value.into();
        let diag = self.selector.clone().with_expected_value(value.clone());
        self.assert_expectation_within_frames(
            |s| Expectation::value_eq(s, value),
            &diag,
            "value equality",
            frame_budget,
        );
    }

    /// Asserts that the matching element owns `WaterUI` UI focus within
    /// `frame_budget` pumped frames.
    #[track_caller]
    pub fn assert_ui_focus_within_frames(self, frame_budget: usize) {
        let diag = self.selector.clone();
        self.assert_expectation_within_frames(
            Expectation::ui_focus,
            &diag,
            "UI focus",
            frame_budget,
        );
    }
}

/// The geometry terminals a query reaches only on the rendered runtime.
impl<M: RenderedQueryMode> Query<'_, HeadlessRuntime, M> {
    /// Moves hover to the matching element center.
    #[track_caller]
    pub fn hover(self) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "hover", 0.5, 0.5);
        M::hover_at(self.app, x, y);
    }

    /// Moves hover to a normalized point inside the matching element.
    #[track_caller]
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
    #[track_caller]
    pub fn tap_at(self, normalized_x: f32, normalized_y: f32) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "tap_at", normalized_x, normalized_y);
        M::tap_at(self.app, x, y);
    }

    /// Drags from the matching element center by a delta.
    #[track_caller]
    pub fn drag_by(self, dx: f32, dy: f32) {
        self.drag_by_with(dx, dy, DragOptions::default());
    }

    /// Drags from the matching element center by a delta with step/timing control.
    #[track_caller]
    pub fn drag_by_with(self, dx: f32, dy: f32, options: DragOptions) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "drag_by", 0.5, 0.5);
        M::drag_from_to_with(self.app, x, y, x + dx, y + dy, options);
    }

    /// Drags between normalized points inside the matching element.
    #[track_caller]
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
    #[track_caller]
    pub fn magnify(self, factor: f32) {
        let element = M::resolve_single(self.app, &self.selector);
        let (x, y) = M::pointer_point(&element, self.app, "magnify", 0.5, 0.5);
        M::magnify_at(self.app, x, y, factor);
    }
}
