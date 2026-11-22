//! Shared harness for Hydrolysis's own integration tests.
//!
//! These tests exercise Hydrolysis's public runtime surface —
//! [`SemanticRuntime`] and [`HeadlessRuntime`] — the same way a host does:
//! input goes in through `push_input_event`, frames advance through
//! `pump_at`, and observations come back through `accessibility_tree`,
//! `accessibility_activation_point` and captured snapshots. This module is
//! the smallest mount/query/input vocabulary those tests speak; it
//! implements nothing Hydrolysis does not already expose.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, Node, NodeId, Rect, Role,
    TreeId as AccessibilityTreeId, TreeUpdate as AccessibilityTreeUpdate,
};
use hydrolysis::testing::TestStyle;
use hydrolysis::{
    FontFamilyResolution, HeadlessRuntime, HeadlessSnapshot, InputEvent, KeyCode, KeyState,
    Modifiers, PointerButton, PointerKind, SemanticRuntime, Style, keyboard_types,
};
use waterui::View;
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;

/// The virtual frame interval every pump advances: 60 Hz.
pub const VIRTUAL_FRAME: Duration = Duration::from_micros(16_667);

/// Narrows a finite `f64` coordinate to `f32`, rounding to nearest.
#[expect(
    clippy::cast_possible_truncation,
    reason = "test layout coordinates are finite and well inside f32 range"
)]
pub const fn f64_as_f32(v: f64) -> f32 {
    v as f32
}

/// The frame instant a pump clock starts from: a fixed epoch so a test's
/// first pump is deterministic.
fn epoch() -> Instant {
    Instant::now()
}

/// Mounts `view` on the semantic runtime at `width`x`height` — the unstyled
/// pipeline a test drives when its subject carries no layout.
pub fn mount_semantic(
    env: Environment,
    view: impl View + Clone + 'static,
    width: u32,
    height: u32,
) -> SemanticApp {
    let content = AnyViewBuilder::new(move || AnyView::new(view.clone()));
    let runtime = SemanticRuntime::new(env, content, width, height, FontFamilyResolution::Strict);
    SemanticApp::new(runtime)
}

/// Mounts `view` on the rendered runtime under [`TestStyle`] at
/// `width`x`height` — the pipeline whose frames carry layout, hit and
/// accessibility geometry.
pub fn mount_rendered(
    env: Environment,
    view: impl View + Clone + 'static,
    width: u32,
    height: u32,
) -> RenderedApp {
    mount_rendered_with(env, view, width, height, TestStyle::new())
}

/// [`mount_rendered`] with an explicit style — for tests that probe the
/// token layer through `TestStyle::with_token_installer` or another local
/// `Style`.
pub fn mount_rendered_with(
    env: Environment,
    view: impl View + Clone + 'static,
    width: u32,
    height: u32,
    style: impl Style,
) -> RenderedApp {
    let content = AnyViewBuilder::new(move || AnyView::new(view.clone()));
    let runtime = HeadlessRuntime::new_for_tests(env, content, width, height, style);
    RenderedApp::new(runtime)
}

/// Finds the node carrying `label`, panicking with the tree's labels when
/// none matches.
#[track_caller]
pub fn node_id(tree: &AccessibilityTreeUpdate, label: &str) -> NodeId {
    tree.nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .map_or_else(
            || {
                let labels: Vec<_> = tree
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| node.label())
                    .collect();
                panic!("no accessibility node labelled {label}; tree labels: {labels:?}")
            },
            |(id, _)| *id,
        )
}

/// Finds the node carrying `label` inside `role`, panicking otherwise.
#[track_caller]
pub fn node_id_with_role(tree: &AccessibilityTreeUpdate, role: Role, label: &str) -> NodeId {
    tree.nodes
        .iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .map_or_else(
            || panic!("no {role:?} node labelled {label}"),
            |(id, _)| *id,
        )
}

/// Finds the node whose label contains `needle`, panicking otherwise.
#[track_caller]
pub fn node_id_contains(tree: &AccessibilityTreeUpdate, needle: &str) -> NodeId {
    tree.nodes
        .iter()
        .find(|(_, node)| node.label().is_some_and(|label| label.contains(needle)))
        .map_or_else(
            || panic!("no accessibility node whose label contains {needle}"),
            |(id, _)| *id,
        )
}

/// Whether any node's label contains `needle`.
pub fn node_label_exists(tree: &AccessibilityTreeUpdate, needle: &str) -> bool {
    tree.nodes
        .iter()
        .any(|(_, node)| node.label() == Some(needle))
}

/// How many nodes carry `role` and `label`.
pub fn count_role_label(tree: &AccessibilityTreeUpdate, role: Role, label: &str) -> usize {
    tree.nodes
        .iter()
        .filter(|(_, node)| node.role() == role && node.label() == Some(label))
        .count()
}

/// Whether any node carries `role` and `label`.
pub fn node_exists(tree: &AccessibilityTreeUpdate, role: Role, label: &str) -> bool {
    tree.nodes
        .iter()
        .any(|(_, node)| node.role() == role && node.label() == Some(label))
}

/// The label-labelled node's bounds, panicking when it reports none.
#[track_caller]
pub fn node_bounds(tree: &AccessibilityTreeUpdate, label: &str) -> Rect {
    node(tree, node_id(tree, label))
        .bounds()
        .unwrap_or_else(|| panic!("node labelled {label} reports no bounds"))
}

/// The `role`+`label` node's bounds, panicking when it reports none.
#[track_caller]
pub fn node_bounds_with_role(tree: &AccessibilityTreeUpdate, role: Role, label: &str) -> Rect {
    node(tree, node_id_with_role(tree, role, label))
        .bounds()
        .unwrap_or_else(|| panic!("{role:?} node labelled {label} reports no bounds"))
}

/// The bounds of the update's only `role` node, panicking when there is not
/// exactly one.
#[track_caller]
pub fn node_bounds_only_role(tree: &AccessibilityTreeUpdate, role: Role) -> Rect {
    let mut found = nodes(tree).filter(|(_, node)| node.role() == role);
    let (id, node) = found
        .next()
        .unwrap_or_else(|| panic!("no {role:?} node in the accessibility tree"));
    assert!(
        found.next().is_none(),
        "more than one {role:?} node in the accessibility tree"
    );
    node.bounds()
        .unwrap_or_else(|| panic!("the {role:?} node {id:?} reports no bounds"))
}

/// The node record for `id`.
#[track_caller]
pub fn node(tree: &AccessibilityTreeUpdate, id: NodeId) -> &Node {
    tree.nodes
        .iter()
        .find_map(|(node_id, node)| (*node_id == id).then_some(node))
        .unwrap_or_else(|| panic!("no accessibility node with id {id:?}"))
}

/// Every `(id, node)` pair in the update, for tree-shape assertions.
pub fn nodes(tree: &AccessibilityTreeUpdate) -> impl Iterator<Item = (NodeId, &Node)> {
    tree.nodes.iter().map(|(id, node)| (*id, node))
}

/// A `AccessibilityActionRequest` on `node`.
pub const fn action_request(
    node: NodeId,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> AccessibilityActionRequest {
    AccessibilityActionRequest {
        action,
        target_tree: AccessibilityTreeId::ROOT,
        target_node: node,
        data,
    }
}

/// The pointer down/up event pair a primary press at `(x, y)` makes.
pub const fn tap_events(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    ]
}

/// A wheel/trackpad scroll event at `(x, y)`.
pub const fn scroll_event(x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) -> InputEvent {
    InputEvent::Scroll {
        x,
        y,
        dx,
        dy,
        is_line_delta,
    }
}

/// A named-key down event ("Escape", "Return", ...).
pub fn key_press_event(named: &str) -> InputEvent {
    key_event(named, KeyState::Pressed)
}

/// A named-key up event matching [`key_press_event`].
pub fn key_release_event(named: &str) -> InputEvent {
    key_event(named, KeyState::Released)
}

fn key_event(named: &str, state: KeyState) -> InputEvent {
    let key = KeyCode::Named(named.to_owned());
    InputEvent::Key {
        logical_key: key.to_w3c_key(),
        physical_code: keyboard_types::Code::Unidentified,
        repeat: false,
        key,
        state,
        modifiers: Modifiers::default(),
    }
}

/// A mounted semantic-runtime app: pumps on a virtual clock, reads the
/// merged accessibility tree, dispatches keyboard and accessibility
/// actions.
pub struct SemanticApp {
    runtime: SemanticRuntime,
    at: Instant,
    tree: Option<AccessibilityTreeUpdate>,
}

impl SemanticApp {
    fn new(runtime: SemanticRuntime) -> Self {
        Self {
            runtime,
            at: epoch(),
            tree: None,
        }
    }

    /// The runtime, for APIs this wrapper does not cover.
    pub const fn runtime(&self) -> &SemanticRuntime {
        &self.runtime
    }

    /// The mutable runtime, for APIs this wrapper does not cover.
    pub const fn runtime_mut(&mut self) -> &mut SemanticRuntime {
        &mut self.runtime
    }

    /// Pumps one virtual frame.
    pub fn pump(&mut self) {
        self.at += VIRTUAL_FRAME;
        let result = self.runtime.pump_at(self.at);
        if let Some(update) = result.into_tree_update() {
            self.tree = Some(update);
        }
    }

    /// Pumps until the runtime reports quiescence or the pump budget runs
    /// out.
    pub fn settle(&mut self) {
        for _ in 0..240 {
            self.pump();
            if self.runtime.is_settled() {
                return;
            }
        }
        panic!("the semantic runtime did not settle");
    }

    /// Pumps `duration` of virtual time.
    pub fn pump_for(&mut self, duration: Duration) {
        let mut remaining = duration;
        while !remaining.is_zero() {
            let step = VIRTUAL_FRAME.min(remaining);
            self.at += step;
            let result = self.runtime.pump_at(self.at);
            if let Some(update) = result.into_tree_update() {
                self.tree = Some(update);
            }
            remaining -= step;
        }
    }

    /// The latest merged accessibility tree — refreshed on demand when the
    /// runtime has newer state than the last pump emitted, pumping the first
    /// frame when nothing has run yet.
    pub fn tree(&mut self) -> &AccessibilityTreeUpdate {
        if self.tree.is_none() {
            self.pump();
        }
        if let Some(update) = self.runtime.accessibility_tree() {
            self.tree = Some(update);
        }
        self.tree
            .as_ref()
            .expect("a mounted window emits an accessibility tree")
    }

    /// Dispatches a named key press+release through the input path.
    pub fn press_key(&mut self, named: &str) {
        self.runtime.push_input_event(key_press_event(named));
        self.runtime.push_input_event(key_release_event(named));
        self.settle();
    }

    /// Performs an accessibility action, asserting the runtime handled it.
    pub fn perform_action(
        &mut self,
        node: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) {
        assert!(
            self.runtime
                .perform_accessibility_action(action_request(node, action, data)),
            "accessibility action {action:?} on {node:?} was not handled"
        );
        self.settle();
    }

    /// `Click` on `node` — the retained-activation path a screen reader
    /// takes.
    pub fn click(&mut self, node: NodeId) {
        self.perform_action(node, AccessibilityAction::Click, None);
    }

    /// `Focus` on `node` — text-input focus.
    pub fn focus(&mut self, node: NodeId) {
        self.perform_action(node, AccessibilityAction::Focus, None);
    }

    /// `SetValue` on `node` — the write a text field's a11y action takes.
    pub fn set_text(&mut self, node: NodeId, value: impl Into<String>) {
        self.perform_action(
            node,
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        );
    }

    /// `ScrollDown` on `node`.
    pub fn scroll_down(&mut self, node: NodeId) {
        self.perform_action(node, AccessibilityAction::ScrollDown, None);
    }
}

/// A mounted rendered-runtime app under [`TestStyle`]: everything
/// [`SemanticApp`] carries plus pointer input, bounds-backed queries and
/// frame snapshots.
pub struct RenderedApp {
    runtime: HeadlessRuntime,
    at: Instant,
    tree: Option<AccessibilityTreeUpdate>,
    snapshot: Option<HeadlessSnapshot>,
}

impl RenderedApp {
    fn new(runtime: HeadlessRuntime) -> Self {
        Self {
            runtime,
            at: epoch(),
            tree: None,
            snapshot: None,
        }
    }

    /// The runtime, for APIs this wrapper does not cover.
    pub const fn runtime(&self) -> &HeadlessRuntime {
        &self.runtime
    }

    /// The mutable runtime, for APIs this wrapper does not cover.
    pub const fn runtime_mut(&mut self) -> &mut HeadlessRuntime {
        &mut self.runtime
    }

    /// Pumps one virtual frame.
    pub fn pump(&mut self) {
        self.at += VIRTUAL_FRAME;
        let result = self.runtime.pump_at(false, self.at);
        if let Some(update) = result.into_tree_update() {
            self.tree = Some(update);
        }
    }

    /// Pumps until the runtime reports quiescence or the pump budget runs
    /// out.
    pub fn settle(&mut self) {
        for _ in 0..240 {
            self.pump();
            if self.runtime.is_settled() {
                return;
            }
        }
        panic!("the headless runtime did not settle");
    }

    /// Pumps `duration` of virtual time.
    pub fn pump_for(&mut self, duration: Duration) {
        let mut remaining = duration;
        while !remaining.is_zero() {
            let step = VIRTUAL_FRAME.min(remaining);
            self.at += step;
            let result = self.runtime.pump_at(false, self.at);
            if let Some(update) = result.into_tree_update() {
                self.tree = Some(update);
            }
            remaining -= step;
        }
    }

    /// Captures a frame snapshot on the next pump and returns it.
    pub fn snapshot(&mut self) -> &HeadlessSnapshot {
        self.at += VIRTUAL_FRAME;
        let mut result = self.runtime.pump_at(true, self.at);
        self.snapshot = result.take_snapshot();
        if let Some(update) = result.into_tree_update() {
            self.tree = Some(update);
        }
        self.snapshot
            .as_ref()
            .expect("a snapshot pump produces a frame")
    }

    /// Sets the display scale the runtime renders at.
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.runtime.set_scale_factor(scale_factor);
    }

    /// Captures a frame and writes it as a PNG under
    /// `<artifacts>/<suite>/<case>/<stage>.png` — the layout
    /// `WATERUI_TEST_ARTIFACTS_DIR` uploads in CI.
    pub fn capture_snapshot(&mut self, suite: &str, case: &str, stage: &str) {
        let snapshot = self.snapshot();
        let path = hydrolysis::testing::artifact_snapshot_path(suite, case, stage);
        std::fs::create_dir_all(path.parent().expect("a snapshot path has a case directory"))
            .expect("the artifact directory must be creatable");
        image::RgbaImage::from_raw(
            snapshot.width(),
            snapshot.height(),
            snapshot.rgba8().to_vec(),
        )
        .expect("snapshot dimensions must match the rgba buffer")
        .save(&path)
        .expect("the snapshot png must be writable");
    }

    /// The latest merged accessibility tree, pumping the first frame when
    /// nothing has run yet.
    pub fn tree(&mut self) -> &AccessibilityTreeUpdate {
        if self.tree.is_none() {
            self.pump();
        }
        if let Some(update) = self.runtime.accessibility_tree() {
            self.tree = Some(update);
        }
        self.tree
            .as_ref()
            .expect("a mounted window emits an accessibility tree")
    }

    /// Dispatches a named key press+release through the input path.
    pub fn press_key(&mut self, named: &str) {
        self.runtime.push_input_event(key_press_event(named));
        self.runtime.push_input_event(key_release_event(named));
        self.settle();
    }

    /// Performs an accessibility action, asserting the runtime handled it.
    pub fn perform_action(
        &mut self,
        node: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) {
        assert!(
            self.runtime
                .perform_accessibility_action(action_request(node, action, data)),
            "accessibility action {action:?} on {node:?} was not handled"
        );
        self.settle();
    }

    /// `Focus` on `node` without settling: a focused field's caret animates
    /// on a cycle, so the runtime never reports quiescence afterward —
    /// callers advance time with [`Self::pump_for`] instead.
    pub fn focus_quiet(&mut self, node: NodeId) {
        assert!(
            self.runtime.perform_accessibility_action(action_request(
                node,
                AccessibilityAction::Focus,
                None
            )),
            "focus on {node:?} was not handled"
        );
    }

    /// `Click` on `node`.
    pub fn click(&mut self, node: NodeId) {
        self.perform_action(node, AccessibilityAction::Click, None);
    }

    /// `Focus` on `node` — the write a text field's a11y focus takes.
    pub fn focus(&mut self, node: NodeId) {
        self.perform_action(node, AccessibilityAction::Focus, None);
    }

    /// `SetValue` on `node`.
    pub fn set_text(&mut self, node: NodeId, value: impl Into<String>) {
        self.perform_action(
            node,
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        );
    }

    /// `ScrollDown` on `node`.
    pub fn scroll_down(&mut self, node: NodeId) {
        self.perform_action(node, AccessibilityAction::ScrollDown, None);
    }

    /// A primary pointer press+release at `(x, y)` through the real input
    /// path.
    pub fn tap_at(&mut self, x: f32, y: f32) {
        for event in tap_events(x, y) {
            self.runtime.push_input_event(event);
        }
        self.settle();
    }

    /// A secondary-button press+release at `(x, y)` — a right-click, what a
    /// context menu opens on.
    pub fn secondary_click_at(&mut self, x: f32, y: f32) {
        self.runtime.push_input_event(InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        });
        self.runtime.push_input_event(InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        });
        self.settle();
    }

    /// Queues a primary pointer press at `(x, y)` without pumping — pair
    /// with [`Self::queue_pointer_up`] when the events must land inside the
    /// same pumped clock window.
    pub fn queue_pointer_down(&mut self, x: f32, y: f32) {
        self.runtime.push_input_event(InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    /// Queues the release matching [`Self::queue_pointer_down`] without
    /// pumping.
    pub fn queue_pointer_up(&mut self, x: f32, y: f32) {
        self.runtime.push_input_event(InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    /// A pointer press at `(x, y)` that stays held — pair with
    /// [`Self::pointer_up_at`].
    pub fn pointer_down_at(&mut self, x: f32, y: f32) {
        self.runtime.push_input_event(InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    /// Releases the pointer held by [`Self::pointer_down_at`].
    pub fn pointer_up_at(&mut self, x: f32, y: f32) {
        self.runtime.push_input_event(InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
        self.settle();
    }

    /// A wheel/trackpad scroll at `(x, y)`.
    pub fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) {
        self.runtime
            .push_input_event(scroll_event(x, y, dx, dy, is_line_delta));
        self.settle();
    }

    /// Queues a wheel/trackpad scroll without settling — the scroll's
    /// transient stays observable to [`Self::pump_for`].
    pub fn queue_scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) {
        self.runtime
            .push_input_event(scroll_event(x, y, dx, dy, is_line_delta));
    }

    /// A primary-pointer drag between two viewport points in `steps`
    /// segments.
    #[expect(clippy::cast_precision_loss, reason = "step counts are far below 2^24")]
    pub fn drag_from_to(&mut self, from_x: f32, from_y: f32, to_x: f32, to_y: f32, steps: u32) {
        let steps = steps.max(1);
        self.runtime.push_input_event(InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x: from_x,
            y: from_y,
            button: PointerButton::Primary,
        });
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let x = (to_x - from_x).mul_add(t, from_x);
            let y = (to_y - from_y).mul_add(t, from_y);
            self.runtime.push_input_event(InputEvent::PointerMove {
                id: 1,
                kind: PointerKind::Mouse,
                x,
                y,
            });
        }
        self.runtime.push_input_event(InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x: to_x,
            y: to_y,
            button: PointerButton::Primary,
        });
        self.settle();
    }

    /// Supplies the touch-gesture parameters a real platform's window
    /// reports (Android's `ViewConfiguration` values through
    /// `PlatformWindow::touch_scroll_config`), enabling
    /// `PointerKind::Touch` drags to claim scroll views the way they do on
    /// device.
    pub fn set_touch_scroll_config(&mut self, config: hydrolysis::TouchScrollConfig) {
        self.runtime.set_touch_scroll_config(config);
    }

    /// Queues a drag between viewport coordinates without the final settle:
    /// the events — and the per-step pumps when `frame_per_step` is set —
    /// are processed immediately, so a release-triggered transient stays
    /// observable to [`Self::pump_for`] and [`Self::snapshot`]. `kind` is the
    /// pointer kind every event carries; `Touch` is what a real device's
    /// finger produces.
    #[expect(clippy::cast_precision_loss, reason = "step counts are far below 2^24")]
    #[expect(
        clippy::too_many_arguments,
        reason = "a drag is four coordinates plus its step count, pacing and pointer kind"
    )]
    pub fn queue_drag_from_to_with(
        &mut self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        steps: u32,
        frame_per_step: bool,
        kind: PointerKind,
    ) {
        let steps = steps.max(1);
        self.runtime.push_input_event(InputEvent::PointerDown {
            id: 1,
            kind,
            x: from_x,
            y: from_y,
            button: PointerButton::Primary,
        });
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            self.runtime.push_input_event(InputEvent::PointerMove {
                id: 1,
                kind,
                x: (to_x - from_x).mul_add(t, from_x),
                y: (to_y - from_y).mul_add(t, from_y),
            });
            if frame_per_step {
                self.pump();
            }
        }
        self.runtime.push_input_event(InputEvent::PointerUp {
            id: 1,
            kind,
            x: to_x,
            y: to_y,
            button: PointerButton::Primary,
        });
    }

    /// A primary-pointer drag between two viewport points, touch-kind for
    /// the touch-scroll path.
    #[expect(clippy::cast_precision_loss, reason = "step counts are far below 2^24")]
    pub fn touch_drag_from_to(
        &mut self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        steps: u32,
    ) {
        let steps = steps.max(1);
        self.runtime.push_input_event(InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Touch,
            x: from_x,
            y: from_y,
            button: PointerButton::Primary,
        });
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let x = (to_x - from_x).mul_add(t, from_x);
            let y = (to_y - from_y).mul_add(t, from_y);
            self.runtime.push_input_event(InputEvent::PointerMove {
                id: 1,
                kind: PointerKind::Touch,
                x,
                y,
            });
        }
        self.runtime.push_input_event(InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Touch,
            x: to_x,
            y: to_y,
            button: PointerButton::Primary,
        });
        self.settle();
    }

    /// A tap routed through [`HeadlessRuntime::accessibility_activation_point`]:
    /// the point the node can actually be hit at, inside its visible
    /// fragment.
    pub fn tap_node(&mut self, tree_node: NodeId, x_fraction: f64, y_fraction: f64) {
        let point = self
            .runtime
            .accessibility_activation_point(tree_node, x_fraction, y_fraction)
            .expect("the node must resolve a hittable point");
        self.tap_at(f64_as_f32(point.x), f64_as_f32(point.y));
    }

    /// The activation point `node` resolves — for tests that assert where
    /// it lands.
    pub fn activation_point(
        &self,
        tree_node: NodeId,
        x_fraction: f64,
        y_fraction: f64,
    ) -> Result<kurbo::Point, hydrolysis::AccessibilityActivationPointError> {
        self.runtime
            .accessibility_activation_point(tree_node, x_fraction, y_fraction)
    }
}
