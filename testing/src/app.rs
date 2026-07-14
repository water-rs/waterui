use std::time::{Duration, Instant};

use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, TreeId as AccessibilityTreeId,
};
use hydrolysis::{KeyCode, Modifiers};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment, View};

use crate::artifacts::{CapturedSnapshot, TestArtifacts};
use crate::driver::{A11yDriver, DriverPumpResult, HydrolysisA11yDriver};
use crate::perf::{PerfApp, PerfConfig, PerfReport};
use crate::query::Query;
use crate::selector::{ElementRef, ElementSet, Selector};
use crate::semantics::{NodeId, TreeSnapshot};
use crate::snapshot::Snapshot;
use crate::wait::{Expectation, ExpectationKind, WaitOptions, WaitResult};

/// Installs a theme package into a test environment before mounting a view.
pub trait ThemeInstaller: Clone + 'static {
    /// Installs theme tokens, renderers, and package-specific hooks into the environment.
    fn install(&self, env: &mut Environment);
}

impl<F> ThemeInstaller for F
where
    F: Fn(&mut Environment) + Clone + 'static,
{
    fn install(&self, env: &mut Environment) {
        self(env);
    }
}

/// Creates a typed UI test builder.
#[must_use]
pub fn ui() -> UiBuilder<()> {
    UiBuilder::new()
}

/// Runtime test host and configuration.
#[derive(Clone, Debug)]
pub struct UiBuilder<T = ()> {
    env: Environment,
    width: u32,
    height: u32,
    theme: T,
    perf_config: PerfConfig,
}

impl Default for UiBuilder<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl UiBuilder<()> {
    /// Creates a default semantic UI test runtime (390x844 viewport).
    #[must_use]
    pub fn new() -> Self {
        let mut env = Environment::new();
        hydrolysis::testing::install_theme(&mut env);
        Self {
            env,
            width: 390,
            height: 844,
            theme: (),
            perf_config: PerfConfig::default(),
        }
    }

    /// Mounts a no-arg view builder and returns a semantic testing session.
    ///
    /// # Panics
    ///
    /// Panics if the initial Hydrolysis semantic pass does not produce an accessibility tree.
    pub fn mount<V, F>(self, view_fn: F) -> SemanticApp
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        mount_app(
            self.env,
            self.width,
            self.height,
            view_fn,
            DriverMode::Semantic,
        )
    }
}

impl<T> UiBuilder<T> {
    /// Overrides the environment used by the mounted app.
    #[must_use]
    pub fn environment(mut self, env: Environment) -> Self {
        self.env = env;
        self
    }

    /// Overrides the logical viewport size used by the mounted app.
    #[must_use]
    pub const fn viewport(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Installs a theme package and enables offscreen rendering APIs.
    #[must_use]
    pub fn theme<U: ThemeInstaller>(self, theme: U) -> UiBuilder<U> {
        UiBuilder {
            env: self.env,
            width: self.width,
            height: self.height,
            theme,
            perf_config: self.perf_config,
        }
    }

    /// Configures repeated offscreen performance measurement defaults.
    #[must_use]
    pub const fn perf_config(mut self, config: PerfConfig) -> Self {
        self.perf_config = config;
        self
    }
}

impl<T: ThemeInstaller> UiBuilder<T> {
    /// Mounts a no-arg view builder and returns an offscreen GPU-backed testing session.
    ///
    /// # Panics
    ///
    /// Panics if the initial Hydrolysis offscreen frame does not produce an accessibility tree.
    pub fn mount<V, F>(self, view_fn: F) -> OffscreenApp
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        let mut env = self.env;
        self.theme.install(&mut env);
        OffscreenApp {
            app: mount_app(env, self.width, self.height, view_fn, DriverMode::Offscreen),
        }
    }

    /// Measures steady-state offscreen frames for a view with the default `steady` scenario.
    pub fn perf<V, F>(self, view_fn: F) -> PerfReport
    where
        V: View + 'static,
        F: Fn() -> V + Clone + 'static,
    {
        self.perf_with(view_fn, |perf| {
            perf.measure("steady-redraw", |run| {
                run.redraw();
            });
        })
    }

    /// Measures custom offscreen scenarios using a closure-driven automation API.
    pub fn perf_with<V, F, A>(self, view_fn: F, automation: A) -> PerfReport
    where
        V: View + 'static,
        F: Fn() -> V + Clone + 'static,
        A: FnOnce(&mut PerfApp<T, F, V>),
    {
        let config = self.perf_config;
        let mut app = PerfApp::new(self, view_fn, config);
        automation(&mut app);
        app.finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverMode {
    Semantic,
    Offscreen,
}

fn mount_app<V, F>(
    env: Environment,
    width: u32,
    height: u32,
    view_fn: F,
    mode: DriverMode,
) -> SemanticApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let builder = AnyViewBuilder::new(move || AnyView::new(view_fn()));
    let mut app = SemanticApp {
        env,
        content: builder,
        driver: Box::new(HydrolysisA11yDriver::new(width, height, mode)),
        tree: TreeSnapshot::empty(),
        ui_focus: None,
        revision: 1,
    };
    let rebuilt = app.pump_once();
    assert!(
        rebuilt,
        "waterui-testing initial mount did not produce a semantic tree"
    );
    app
}

/// Offscreen GPU-backed app session with snapshot and performance hooks.
#[derive(Debug)]
pub struct OffscreenApp {
    pub(crate) app: SemanticApp,
}

impl OffscreenApp {
    /// Returns the semantic app API shared with non-rendering tests.
    #[must_use]
    pub const fn semantic(&self) -> &SemanticApp {
        &self.app
    }

    /// Returns the mutable semantic app API shared with non-rendering tests.
    #[must_use]
    pub const fn semantic_mut(&mut self) -> &mut SemanticApp {
        &mut self.app
    }

    /// Captures the latest RGBA snapshot from the offscreen renderer.
    ///
    /// # Panics
    ///
    /// Panics if the offscreen driver does not produce a snapshot.
    pub fn snapshot(&mut self) -> Snapshot {
        let outcome = self.app.driver.pump(&self.app.content, &self.app.env, true);

        self.app
            .apply_pump_result(outcome)
            .unwrap_or_else(|| panic!("waterui-testing driver did not produce a snapshot"))
    }

    /// Captures a snapshot and stores it in `WaterUI`'s canonical artifact layout.
    pub fn capture_snapshot(
        &mut self,
        suite: impl AsRef<str>,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        let artifacts = self.app.artifacts(suite);
        artifacts.capture_snapshot(case, stage, self.snapshot())
    }

    /// Queues a primary pointer-down without the semantic settle used by
    /// [`SemanticApp::pointer_down_at`]. The event is processed by the next
    /// pump (e.g. [`Self::snapshot`]), so visual stage tests can capture
    /// animation phases that begin at the event — the settle would otherwise
    /// pump frames for its full timeout and skip past short transients such
    /// as the Material ripple growth.
    pub fn queue_pointer_down(&mut self, x: f32, y: f32) {
        let _ = self.app.driver.pointer_down(x, y, &self.app.env);
    }

    /// Queues a primary pointer-up without the semantic settle; see
    /// [`Self::queue_pointer_down`].
    pub fn queue_pointer_up(&mut self, x: f32, y: f32) {
        let _ = self.app.driver.pointer_up(x, y, &self.app.env);
    }

    /// Queues a pointer move without the semantic settle; see
    /// [`Self::queue_pointer_down`]. Visual stage tests use this to park the
    /// pointer away from a widget so idle captures are free of hover state.
    pub fn queue_pointer_move(&mut self, x: f32, y: f32) {
        let _ = self.app.driver.pointer_move(x, y, &self.app.env);
    }
}

impl core::ops::Deref for OffscreenApp {
    type Target = SemanticApp;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl core::ops::DerefMut for OffscreenApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

/// Mounted semantic app session used in `#[waterui::test(...)]`.
pub struct SemanticApp {
    pub(crate) env: Environment,
    pub(crate) content: AnyViewBuilder<AnyView>,
    pub(crate) driver: Box<dyn A11yDriver>,
    pub(crate) tree: TreeSnapshot,
    pub(crate) ui_focus: Option<NodeId>,
    pub(crate) revision: u64,
}

impl core::fmt::Debug for SemanticApp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SemanticApp")
            .field("revision", &self.tree.revision())
            .field("nodes", &self.tree.nodes().len())
            .finish_non_exhaustive()
    }
}

#[allow(
    clippy::missing_panics_doc,
    reason = "assertion helpers intentionally panic with WaterUI-specific diagnostics"
)]
impl SemanticApp {
    /// Returns the latest accessibility tree snapshot.
    #[must_use]
    pub const fn tree(&self) -> &TreeSnapshot {
        &self.tree
    }

    /// Returns the latest UI focus target tracked by Hydrolysis.
    #[must_use]
    pub const fn ui_focus(&self) -> Option<NodeId> {
        self.ui_focus
    }

    /// Creates a canonical artifact helper rooted at the provided suite.
    #[must_use]
    pub fn artifacts(&self, suite: impl AsRef<str>) -> TestArtifacts {
        TestArtifacts::new(suite.as_ref())
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
    pub fn assert_exists(&mut self, selector: &Selector) {
        let results = self.resolve_elements(selector);
        let count = results.len();
        assert!(
            (count != 0),
            "waterui-testing assertion failed: selector {} expected to exist but matched 0 nodes on revision {}",
            selector.describe(),
            self.tree.revision()
        );
    }

    /// Convenience non-existence assertion.
    pub fn assert_not_exists(&mut self, selector: &Selector) {
        let results = self.resolve_elements(selector);
        let count = results.len();
        assert!(
            (count == 0),
            "waterui-testing assertion failed: selector {} expected to be absent but matched {count} nodes; candidates: {}",
            selector.describe(),
            results.debug_summary(3)
        );
    }

    /// Asserts that the selector resolves to the current UI-focused element.
    pub fn assert_ui_focus(&mut self, selector: &Selector) {
        let element = self.resolve_single(selector);
        assert!(
            self.ui_focus == Some(element.id()),
            "waterui-testing assertion failed: selector was not the current UI-focused element"
        );
    }

    /// Asserts that the selector resolves to exactly one node with the expected value.
    pub fn assert_value_eq(&mut self, selector: &Selector, value: impl Into<String>) {
        let expected = value.into();
        let element = self.resolve_single(selector);
        let actual = element.node().value();
        assert!(
            actual == Some(expected.as_str()),
            "waterui-testing assertion failed: selector value mismatch (expected {expected:?}, got {actual:?})"
        );
    }

    /// Creates an existence expectation.
    #[must_use]
    pub const fn expect_exists(&self, selector: Selector) -> Expectation {
        Expectation {
            kind: ExpectationKind::Exists(selector),
            inverted: false,
        }
    }

    /// Creates a non-existence expectation.
    #[must_use]
    pub const fn expect_not_exists(&self, selector: Selector) -> Expectation {
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

    /// Convenience API mirroring `XCTest` `waitForExistence`.
    pub fn wait_for_existence(&mut self, selector: &Selector, timeout: Duration) -> bool {
        let expectation = self.expect_exists(selector.clone());
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Convenience API mirroring `XCTest` `waitForNonexistence`.
    pub fn wait_for_nonexistence(&mut self, selector: &Selector, timeout: Duration) -> bool {
        let expectation = self.expect_not_exists(selector.clone());
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Waits for one node's value to equal the expected value.
    pub fn wait_for_value_eq(
        &mut self,
        selector: &Selector,
        value: impl Into<String>,
        timeout: Duration,
    ) -> bool {
        let expectation = self.expect_value_eq(selector.clone(), value);
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Waits until the selector resolves to the current UI-focused element.
    pub fn wait_for_ui_focus(&mut self, selector: &Selector, timeout: Duration) -> bool {
        const MIN_IDLE_BACKOFF: Duration = Duration::from_millis(1);
        const MAX_IDLE_BACKOFF: Duration = Duration::from_millis(16);

        let deadline = Instant::now() + timeout;
        let mut idle_backoff = Duration::ZERO;
        loop {
            if self.matches_ui_focus(selector) {
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

    fn evaluate_expectation(&self, expectation: &Expectation) -> bool {
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

    fn matching_ids(&self, selector: &Selector) -> Vec<NodeId> {
        self.validate_selector_scope(selector);
        self.tree.matching(selector)
    }

    pub(crate) fn resolve_elements(&self, selector: &Selector) -> ElementSet {
        let ids = self.matching_ids(selector);
        let elements = ids
            .into_iter()
            .map(|id| ElementRef {
                node_id: id,
                node: self.tree[id].clone(),
                revision: self.tree.revision(),
            })
            .collect();
        ElementSet::new(elements, self.tree.revision())
    }

    pub(crate) fn resolve_single(&self, selector: &Selector) -> ElementRef {
        let results = self.resolve_elements(selector);
        match results.len() {
            1 => results[0].clone(),
            0 => panic!(
                "waterui-testing selector {} resolved 0 nodes, expected exactly 1 on revision {}",
                selector.describe(),
                self.tree.revision()
            ),
            n => panic!(
                "waterui-testing selector {} resolved {n} nodes, expected exactly 1; candidates: {}",
                selector.describe(),
                results.debug_summary(3)
            ),
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

    /// Dispatches a pointer tap at viewport coordinates and settles resulting updates.
    pub fn tap_at(&mut self, x: f32, y: f32) -> bool {
        let mut changed = self.driver.pointer_down(x, y, &self.env);
        changed |= self.driver.pointer_up(x, y, &self.env);
        self.settle_after_change(changed)
    }

    /// Dispatches a primary pointer-down event at viewport coordinates.
    pub fn pointer_down_at(&mut self, x: f32, y: f32) -> bool {
        let changed = self.driver.pointer_down(x, y, &self.env);
        self.settle_after_change(changed)
    }

    /// Dispatches a primary pointer-up event at viewport coordinates.
    pub fn pointer_up_at(&mut self, x: f32, y: f32) -> bool {
        let changed = self.driver.pointer_up(x, y, &self.env);
        self.settle_after_change(changed)
    }

    pub(crate) fn drag_from_to(&mut self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
        const STEPS: u16 = 6;

        let mut changed = self.driver.pointer_down(from_x, from_y, &self.env);
        for step in 1..=STEPS {
            let t = f32::from(step) / f32::from(STEPS);
            let x = (to_x - from_x).mul_add(t, from_x);
            let y = (to_y - from_y).mul_add(t, from_y);
            changed |= self.driver.pointer_move(x, y, &self.env);
        }
        changed |= self.driver.pointer_up(to_x, to_y, &self.env);
        self.settle_after_change(changed)
    }

    /// Dispatches a wheel/trackpad scroll at viewport coordinates and settles resulting updates.
    pub fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) -> bool {
        let changed = self
            .driver
            .scroll_at(x, y, dx, dy, is_line_delta, &self.env);
        self.settle_after_change(changed)
    }

    /// Dispatches committed text through the Hydrolysis text input path.
    pub fn text_input(&mut self, text: impl Into<String>) -> bool {
        let changed = self.driver.text_input(text.into(), &self.env);
        self.settle_after_change(changed)
    }

    /// Dispatches a named keyboard key such as `Backspace`, `Delete`, or `ArrowLeft`.
    pub fn press_named_key(&mut self, key: impl Into<String>) -> bool {
        let changed =
            self.driver
                .key_press(KeyCode::Named(key.into()), Modifiers::default(), &self.env);
        self.settle_after_change(changed)
    }

    /// Dispatches a character keyboard key without text-input synthesis.
    pub fn press_character_key(&mut self, key: impl Into<String>) -> bool {
        let changed = self.driver.key_press(
            KeyCode::Character(key.into()),
            Modifiers::default(),
            &self.env,
        );
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
        let outcome = self.driver.pump(&self.content, &self.env, false);
        let rebuilt = outcome.rebuilt;
        let _ = self.apply_pump_result(outcome);
        rebuilt
    }

    fn matches_ui_focus(&self, selector: &Selector) -> bool {
        let ids = self.matching_ids(selector);
        ids.len() == 1 && self.ui_focus == Some(ids[0])
    }

    pub(crate) fn assert_current_element(&self, element: &ElementRef, context: &str) {
        assert!(
            element.revision() == self.tree.revision(),
            "waterui-testing stale element handle during {context}: handle revision {} does not match current tree revision {}; re-query the element before interacting. handle={}",
            element.revision(),
            self.tree.revision(),
            element.debug_summary()
        );
        assert!(
            self.tree.node(element.id()).is_some(),
            "waterui-testing missing current node for handle during {context}: handle={} is not present in revision {}",
            element.debug_summary(),
            self.tree.revision()
        );
    }

    fn validate_selector_scope(&self, selector: &Selector) {
        if let Some(scope) = selector.scope() {
            self.assert_current_element(scope.handle(), "scoped query");
        }
    }
}

impl SemanticApp {
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
