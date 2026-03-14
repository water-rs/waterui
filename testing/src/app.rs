use std::{
    future::Future,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, TreeId as AccessibilityTreeId,
};
use executor_core::{
    LocalExecutor,
    async_task::{AsyncTask, Runnable},
    try_init_local_executor,
};
use hydrolysis::HydrolysisViewRenderer;
use waterui::graphics::SceneViewMergeToParent;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment, View};

use crate::artifacts::{CapturedSnapshot, TestArtifacts};
use crate::driver::{
    A11yDriver, DriverPumpResult, HydrolysisA11yDriver, install_native_component_hooks,
};
use crate::query::Query;
use crate::selector::{ElementRef, ElementSet, Selector};
use crate::semantics::{NodeId, TreeSnapshot};
use crate::snapshot::Snapshot;
use crate::wait::{Expectation, ExpectationKind, WaitOptions, WaitResult};

#[derive(Clone, Debug)]
struct TestLocalExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    runnable_rx: Rc<mpsc::Receiver<Runnable>>,
}

impl Default for TestLocalExecutor {
    fn default() -> Self {
        let (runnable_tx, runnable_rx) = mpsc::channel();
        Self {
            runnable_tx,
            runnable_rx: Rc::new(runnable_rx),
        }
    }
}

impl TestLocalExecutor {
    fn drain(&self) -> bool {
        let mut ran = false;
        loop {
            let Ok(runnable) = self.runnable_rx.try_recv() else {
                return ran;
            };
            ran = true;
            runnable.run();
        }
    }
}

impl LocalExecutor for TestLocalExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            let _ = runnable_tx.send(runnable);
        });
        runnable.schedule();
        AsyncTask::from(task)
    }
}

/// Runtime test host and configuration.
#[derive(Debug)]
pub struct UiTest {
    env: Environment,
    width: u32,
    height: u32,
    local_executor: TestLocalExecutor,
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
        let mut env = Environment::new().extending(SceneViewMergeToParent);
        install_native_component_hooks(&mut env);
        env.insert(waterui_core::ViewRenderer::new(
            HydrolysisViewRenderer::default(),
        ));
        let local_executor = TestLocalExecutor::default();
        let _ = try_init_local_executor(waterui::task::monitored_local_executor(
            local_executor.clone(),
        ));

        Self {
            env,
            width: 390,
            height: 844,
            local_executor,
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
            ui_focus: None,
            revision: 1,
            local_executor: self.local_executor,
        };
        let rebuilt = app.pump_once();
        assert!(
            rebuilt,
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
    pub(crate) ui_focus: Option<NodeId>,
    pub(crate) revision: u64,
    local_executor: TestLocalExecutor,
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

    /// Returns the latest UI focus target tracked by Hydrolysis.
    #[must_use]
    pub const fn ui_focus(&self) -> Option<NodeId> {
        self.ui_focus
    }

    /// Captures the latest RGBA snapshot from the offscreen renderer.
    pub fn snapshot(&mut self) -> Snapshot {
        let _ = self.local_executor.drain();
        let outcome = self.driver.pump(&self.content, &self.env, true);
        let snapshot = self
            .apply_pump_result(outcome)
            .unwrap_or_else(|| panic!("waterui-testing driver did not produce a snapshot"));
        let _ = self.local_executor.drain();
        snapshot
    }

    /// Creates a canonical artifact helper rooted at the provided suite.
    #[must_use]
    pub fn artifacts(&self, suite: impl AsRef<str>) -> TestArtifacts {
        TestArtifacts::new(suite.as_ref())
    }

    /// Captures a snapshot and stores it in WaterUI's canonical artifact layout.
    pub fn capture_snapshot(
        &mut self,
        suite: impl AsRef<str>,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        let artifacts = self.artifacts(suite);
        artifacts.capture_snapshot(case, stage, self.snapshot())
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

    /// Asserts that the selector resolves to the current UI-focused element.
    pub fn assert_ui_focus(&mut self, selector: Selector) {
        let element = self.resolve_single(&selector);
        assert!(
            self.ui_focus == Some(element.id()),
            "waterui-testing assertion failed: selector was not the current UI-focused element"
        );
    }

    /// Asserts that the selector resolves to exactly one node with the expected value.
    pub fn assert_value_eq(&mut self, selector: Selector, value: impl Into<String>) {
        let expected = value.into();
        let element = self.resolve_single(&selector);
        let actual = element.node().value();
        assert!(
            actual == Some(expected.as_str()),
            "waterui-testing assertion failed: selector value mismatch (expected {:?}, got {:?})",
            expected,
            actual
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

    /// Waits until the selector resolves to the current UI-focused element.
    pub fn wait_for_ui_focus(&mut self, selector: Selector, timeout: Duration) -> bool {
        const MIN_IDLE_BACKOFF: Duration = Duration::from_millis(1);
        const MAX_IDLE_BACKOFF: Duration = Duration::from_millis(16);

        let deadline = Instant::now() + timeout;
        let mut idle_backoff = Duration::ZERO;
        loop {
            if self.matches_ui_focus(&selector) {
                return true;
            }

            if Instant::now() >= deadline {
                return false;
            }

            let previous_revision = self.tree.revision();
            let previous_ui_focus = self.ui_focus;
            let rebuilt = self.pump_once();
            let progressed = rebuilt
                || self.tree.revision() != previous_revision
                || self.ui_focus != previous_ui_focus;
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
            std::thread::sleep(
                next_backoff.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
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

    pub(crate) fn resolve_elements(&mut self, selector: &Selector) -> ElementSet {
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

    pub(crate) fn resolve_single(&mut self, selector: &Selector) -> ElementRef {
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

    /// Clears the latest Hydrolysis-managed UI focus target.
    pub fn clear_ui_focus(&mut self) -> bool {
        let changed = self.driver.clear_ui_focus(&self.env);
        self.settle_after_change(changed)
    }

    pub(crate) fn hover_at(&mut self, x: f32, y: f32) -> bool {
        let changed = self.driver.hover_at(x, y, &self.env);
        self.settle_after_change(changed)
    }

    pub(crate) fn tap_at(&mut self, x: f32, y: f32) -> bool {
        let mut changed = self.driver.pointer_down(x, y, &self.env);
        changed |= self.driver.pointer_up(x, y, &self.env);
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
            let previous_ui_focus = self.ui_focus;
            let rebuilt = self.pump_once();
            let progressed = rebuilt
                || self.tree.revision() != previous_revision
                || self.ui_focus != previous_ui_focus;
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
            std::thread::sleep(
                next_backoff.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn apply_pump_result(&mut self, outcome: DriverPumpResult) -> Option<Snapshot> {
        self.ui_focus = outcome.ui_focus;
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
        let drained_before = self.local_executor.drain();
        let outcome = self.driver.pump(&self.content, &self.env, false);
        let rebuilt = outcome.rebuilt;
        let _ = self.apply_pump_result(outcome);
        let drained_after = self.local_executor.drain();
        rebuilt || drained_before || drained_after
    }

    fn matches_ui_focus(&mut self, selector: &Selector) -> bool {
        let ids = self.matching_ids(selector);
        ids.len() == 1 && self.ui_focus == Some(ids[0])
    }
}

impl MountedApp {
    pub(crate) fn tap_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::Click, None)
    }

    pub(crate) fn focus_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::Focus, None)
    }

    pub(crate) fn set_text_node(&mut self, node_id: NodeId, value: impl Into<String>) -> bool {
        self.perform_action(
            node_id,
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        )
    }

    pub(crate) fn increment_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::Increment, None)
    }

    pub(crate) fn decrement_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::Decrement, None)
    }

    pub(crate) fn scroll_down_node(&mut self, node_id: NodeId) -> bool {
        self.perform_action(node_id, AccessibilityAction::ScrollDown, None)
    }
}
