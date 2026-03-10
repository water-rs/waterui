use std::time::{Duration, Instant};

use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, TreeId as AccessibilityTreeId,
};
use hydrolysis::HydrolysisViewRenderer;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment, View};

use crate::driver::{A11yDriver, DriverPumpResult, HydrolysisA11yDriver, install_native_component_hooks};
use crate::selector::{ElementRef, ElementSet, Selector};
use crate::semantics::{NodeId, Role, TreeSnapshot};
use crate::snapshot::Snapshot;
use crate::wait::{Expectation, ExpectationKind, WaitOptions, WaitResult};

/// Runtime test host and configuration.
#[derive(Debug)]
pub struct UiTest {
    env: Environment,
    width: u32,
    height: u32,
}

impl Default for UiTest {
    fn default() -> Self {
        Self::new()
    }
}

impl UiTest {
    /// Creates a default UI test runtime (390x844 viewport).
    #[must_use]
    pub fn new() -> Self {
        let mut env = Environment::new();
        install_native_component_hooks(&mut env);
        env.insert(waterui_core::ViewRenderer::new(
            HydrolysisViewRenderer::default(),
        ));

        Self {
            env,
            width: 390,
            height: 844,
        }
    }

    /// Overrides the logical viewport size used by the mounted app.
    #[must_use]
    pub const fn viewport(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Mounts a no-arg view builder and returns a semantic testing session.
    pub fn mount<V, F>(self, view_fn: F) -> MountedApp
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        let builder = AnyViewBuilder::new(move || AnyView::new(view_fn()));
        let mut app = MountedApp {
            env: self.env,
            content: builder,
            driver: Box::new(HydrolysisA11yDriver::new(self.width, self.height)),
            tree: TreeSnapshot::empty(),
            revision: 1,
        };
        let rebuilt = app.pump_once();
        assert!(
            !(!rebuilt),
            "waterui-testing initial mount did not produce a frame"
        );
        app
    }
}

/// Mounted semantic app session used in `#[waterui::test(...)]`.
pub struct MountedApp {
    pub(crate) env: Environment,
    pub(crate) content: AnyViewBuilder<AnyView>,
    pub(crate) driver: Box<dyn A11yDriver>,
    pub(crate) tree: TreeSnapshot,
    pub(crate) revision: u64,
}

impl core::fmt::Debug for MountedApp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MountedApp")
            .field("revision", &self.tree.revision())
            .field("nodes", &self.tree.nodes().len())
            .finish()
    }
}

impl MountedApp {
    /// Returns the latest accessibility tree snapshot.
    #[must_use]
    pub fn tree(&self) -> &TreeSnapshot {
        &self.tree
    }

    /// Captures the latest RGBA snapshot from the offscreen renderer.
    pub fn snapshot(&mut self) -> Snapshot {
        let outcome = self.driver.pump(self.content.build(), &self.env, true);
        self.apply_pump_result(outcome)
            .unwrap_or_else(|| panic!("waterui-testing driver did not produce a snapshot"))
    }

    /// Starts a chainable semantic query.
    #[must_use]
    pub fn query(&mut self) -> Query<'_> {
        Query {
            app: self,
            selector: Selector::default(),
        }
    }

    /// Convenience existence assertion.
    pub fn assert_exists(&mut self, selector: Selector) {
        let count = self.matching_ids(&selector).len();
        assert!(
            !(count == 0),
            "waterui-testing assertion failed: selector expected to exist but matched 0 nodes"
        );
    }

    /// Convenience non-existence assertion.
    pub fn assert_not_exists(&mut self, selector: Selector) {
        let count = self.matching_ids(&selector).len();
        assert!(
            !(count != 0),
            "waterui-testing assertion failed: selector expected to be absent but matched {count} nodes"
        );
    }

    /// Creates an existence expectation.
    #[must_use]
    pub fn expect_exists(&self, selector: Selector) -> Expectation {
        Expectation {
            kind: ExpectationKind::Exists(selector),
            inverted: false,
        }
    }

    /// Creates a non-existence expectation.
    #[must_use]
    pub fn expect_not_exists(&self, selector: Selector) -> Expectation {
        Expectation {
            kind: ExpectationKind::NotExists(selector),
            inverted: false,
        }
    }

    /// Creates a value-equality expectation.
    #[must_use]
    pub fn expect_value_eq(&self, selector: Selector, value: impl Into<String>) -> Expectation {
        Expectation {
            kind: ExpectationKind::ValueEquals {
                selector,
                value: value.into(),
            },
            inverted: false,
        }
    }

    /// Waits for expectations using XCTest-like semantics.
    pub fn wait_for(&mut self, expectations: &[Expectation], options: WaitOptions) -> WaitResult {
        const MIN_IDLE_BACKOFF: Duration = Duration::from_millis(1);
        const MAX_IDLE_BACKOFF: Duration = Duration::from_millis(16);

        assert!(
            !(expectations.is_empty()),
            "waterui-testing wait_for requires at least one expectation"
        );

        let has_inverted = expectations.iter().any(|e| e.inverted);
        let mut fulfilled = vec![false; expectations.len()];
        let mut next_order_index = 0usize;
        let deadline = Instant::now() + options.timeout;
        let mut idle_backoff = Duration::ZERO;

        loop {
            for (idx, expectation) in expectations.iter().enumerate() {
                let condition = self.evaluate_expectation(expectation);
                if expectation.inverted {
                    if condition {
                        return WaitResult::InvertedFulfillment;
                    }
                    continue;
                }

                if fulfilled[idx] {
                    continue;
                }

                if condition {
                    if options.enforce_order && idx != next_order_index {
                        return WaitResult::IncorrectOrder;
                    }
                    fulfilled[idx] = true;
                    if options.enforce_order {
                        next_order_index += 1;
                    }
                } else if options.enforce_order && idx > next_order_index {
                    let later_fulfilled = fulfilled
                        .iter()
                        .enumerate()
                        .skip(idx + 1)
                        .any(|(_, done)| *done);
                    if later_fulfilled {
                        return WaitResult::IncorrectOrder;
                    }
                }
            }

            let all_non_inverted = expectations
                .iter()
                .enumerate()
                .all(|(idx, expectation)| expectation.inverted || fulfilled[idx]);

            if all_non_inverted && !has_inverted {
                return WaitResult::Completed;
            }

            let now = Instant::now();
            if now >= deadline {
                return if all_non_inverted {
                    WaitResult::Completed
                } else {
                    WaitResult::TimedOut
                };
            }

            let previous_revision = self.tree.revision();
            let rebuilt = self.pump_once();
            let progressed = rebuilt || self.tree.revision() != previous_revision;
            if progressed {
                idle_backoff = Duration::ZERO;
                continue;
            }

            let next_backoff = if idle_backoff.is_zero() {
                MIN_IDLE_BACKOFF
            } else {
                idle_backoff.saturating_mul(2).min(MAX_IDLE_BACKOFF)
            };
            idle_backoff = next_backoff;

            let now = Instant::now();
            if now >= deadline {
                continue;
            }
            let remaining = deadline.saturating_duration_since(now);
            let sleep_for = next_backoff.min(remaining);
            if !sleep_for.is_zero() {
                std::thread::sleep(sleep_for);
            }
        }
    }

    /// Convenience API mirroring XCTest `waitForExistence`.
    pub fn wait_for_existence(&mut self, selector: Selector, timeout: Duration) -> bool {
        let expectation = self.expect_exists(selector);
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Convenience API mirroring XCTest `waitForNonexistence`.
    pub fn wait_for_nonexistence(&mut self, selector: Selector, timeout: Duration) -> bool {
        let expectation = self.expect_not_exists(selector);
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Waits for one node's value to equal the expected value.
    pub fn wait_for_value_eq(
        &mut self,
        selector: Selector,
        value: impl Into<String>,
        timeout: Duration,
    ) -> bool {
        let expectation = self.expect_value_eq(selector, value);
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    fn evaluate_expectation(&mut self, expectation: &Expectation) -> bool {
        match &expectation.kind {
            ExpectationKind::Exists(selector) => !self.matching_ids(selector).is_empty(),
            ExpectationKind::NotExists(selector) => self.matching_ids(selector).is_empty(),
            ExpectationKind::ValueEquals { selector, value } => {
                let ids = self.matching_ids(selector);
                if ids.len() != 1 {
                    return false;
                }
                self.tree[ids[0]].value() == Some(value.as_str())
            }
        }
    }

    fn matching_ids(&mut self, selector: &Selector) -> Vec<NodeId> {
        self.tree.matching(selector)
    }

    fn resolve_elements(&mut self, selector: &Selector) -> ElementSet {
        let ids = self.matching_ids(selector);
        let elements = ids
            .into_iter()
            .map(|id| ElementRef {
                node_id: id,
                node: self.tree[id].clone(),
            })
            .collect();
        ElementSet::new(elements)
    }

    fn resolve_single(&mut self, selector: &Selector) -> ElementRef {
        let results = self.resolve_elements(selector);
        match results.len() {
            1 => results[0].clone(),
            0 => panic!("waterui-testing selector resolved 0 nodes, expected exactly 1"),
            n => panic!("waterui-testing selector resolved {n} nodes, expected exactly 1"),
        }
    }

    pub(crate) fn perform_action(
        &mut self,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) -> bool {
        let request = AccessibilityActionRequest {
            target_tree: AccessibilityTreeId::ROOT,
            target_node: node_id.as_accesskit(),
            action,
            data,
        };
        let changed = self.driver.perform_action(request, &self.env);
        self.settle_after_change(changed)
    }

    pub(crate) fn hover_at(&mut self, x: f32, y: f32) -> bool {
        let changed = self.driver.hover_at(x, y, &self.env);
        self.settle_after_change(changed)
    }

    pub(crate) fn drag_from_to(&mut self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
        const STEPS: usize = 6;

        let mut changed = self.driver.pointer_down(from_x, from_y, &self.env);
        for step in 1..=STEPS {
            let t = step as f32 / STEPS as f32;
            let x = from_x + (to_x - from_x) * t;
            let y = from_y + (to_y - from_y) * t;
            changed |= self.driver.pointer_move(x, y, &self.env);
        }
        changed |= self.driver.pointer_up(to_x, to_y, &self.env);
        self.settle_after_change(changed)
    }

    pub(crate) fn magnify_at(&mut self, x: f32, y: f32, factor: f32) -> bool {
        let changed = self.driver.magnify_at(x, y, factor, &self.env);
        self.settle_after_change(changed)
    }

    fn settle_after_change(&mut self, changed: bool) -> bool {
        if !changed {
            return false;
        }
        self.settle(Duration::from_millis(200));
        true
    }

    fn settle(&mut self, timeout: Duration) {
        const MIN_IDLE_BACKOFF: Duration = Duration::from_millis(1);
        const MAX_IDLE_BACKOFF: Duration = Duration::from_millis(16);

        let deadline = Instant::now() + timeout;
        let mut idle_backoff = Duration::ZERO;
        loop {
            let previous_revision = self.tree.revision();
            let rebuilt = self.pump_once();
            let progressed = rebuilt || self.tree.revision() != previous_revision;
            if progressed {
                idle_backoff = Duration::ZERO;
                if Instant::now() >= deadline {
                    return;
                }
                continue;
            }
            if Instant::now() >= deadline {
                return;
            }
            let next_backoff = if idle_backoff.is_zero() {
                MIN_IDLE_BACKOFF
            } else {
                idle_backoff.saturating_mul(2).min(MAX_IDLE_BACKOFF)
            };
            idle_backoff = next_backoff;
            std::thread::sleep(next_backoff.min(deadline.saturating_duration_since(Instant::now())));
        }
    }

    fn apply_pump_result(&mut self, outcome: DriverPumpResult) -> Option<Snapshot> {
        if let Some(update) = outcome.tree_update {
            self.tree = TreeSnapshot::from_update(self.revision, update);
            self.revision = self
                .revision
                .checked_add(1)
                .expect("waterui-testing tree revision overflow");
        } else {
            assert!(
                !self.tree.nodes().is_empty(),
                "waterui-testing did not receive an accessibility tree update after mount"
            );
        }
        outcome.snapshot
    }

    fn pump_once(&mut self) -> bool {
        let outcome = self.driver.pump(self.content.build(), &self.env, false);
        let rebuilt = outcome.rebuilt;
        let _ = self.apply_pump_result(outcome);
        rebuilt
    }
}

/// Chainable query builder bound to a mounted app session.
pub struct Query<'a> {
    app: &'a mut MountedApp,
    selector: Selector,
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
            !(all.len() > 1),
            "waterui-testing selector resolved {} nodes, expected at most 1",
            all.len()
        );
        Some(all[0].clone())
    }

    #[must_use]
    pub fn single(self) -> ElementRef {
        self.app.resolve_single(&self.selector)
    }

    pub fn tap(self) -> bool {
        let element = self.app.resolve_single(&self.selector);
        self.app.tap_node(element.id())
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

    pub fn drag_by(self, dx: f32, dy: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.drag_by(self.app, dx, dy)
    }

    pub fn magnify(self, factor: f32) -> bool {
        let element = self.app.resolve_single(&self.selector);
        element.magnify(self.app, factor)
    }
}

impl MountedApp {
    fn tap_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::Click, None)
    }

    fn set_text_node(&mut self, node_id: NodeId, value: impl Into<String>) -> bool {
        self.perform_action(
            node_id,
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        )
    }

    fn increment_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::Increment, None)
    }

    fn decrement_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::Decrement, None)
    }

    fn scroll_down_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::ScrollDown, None)
    }
}
