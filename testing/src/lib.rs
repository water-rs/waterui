//! Headless rendering and accessibility-first test utilities for WaterUI.

use core::ops::Index;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, Node as AccessibilityNode,
    NodeId as AccessibilityNodeId, Role as AccessibilityRole, Toggled as AccessibilityToggled,
    TreeId as AccessibilityTreeId, TreeUpdate as AccessibilityTreeUpdate,
};
use hydrolysis::{
    HydrolysisRenderer, HydrolysisViewRenderer, OffscreenWindow, PlatformWindow, SurfaceProvider,
};
use waterui::component::table::TableConfig;
use waterui::graphics::SceneViewMergeToParent;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::view::Hook;
use waterui_core::{AnyView, Environment, Native, View};

/// Internal async bridge used by `#[waterui::test(...)]` expansion.
pub fn block_on<F>(future: F) -> F::Output
where
    F: core::future::Future,
{
    pollster::block_on(future)
}

/// RGBA8 frame captured from a headless hydrolysis render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel data in RGBA8 row-major order.
    pub rgba8: Vec<u8>,
}

/// Headless host that renders WaterUI views into an offscreen texture.
#[derive(Debug)]
pub struct TestHost {
    env: Environment,
    width: u32,
    height: u32,
}

impl TestHost {
    /// Creates a test host with a fixed render size.
    #[must_use]
    pub const fn new(env: Environment, width: u32, height: u32) -> Self {
        Self { env, width, height }
    }

    /// Renders a view and returns the captured RGBA8 snapshot.
    pub fn render<V: View>(&self, view: V) -> Snapshot {
        let mut platform = OffscreenWindow::new(
            self.width.max(1),
            self.height.max(1),
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        let bounds = vello::kurbo::Rect::new(
            0.0,
            0.0,
            f64::from(self.width.max(1)),
            f64::from(self.height.max(1)),
        );

        let surface = platform.surface();
        renderer.set_frame_resources(surface.device(), surface.queue());
        renderer.reset_scene();
        renderer.begin_rebuild_frame();
        renderer.dispatch(view, &self.env, bounds);
        renderer.finish_rebuild_frame();

        let frame = surface
            .acquire()
            .expect("waterui-testing failed to acquire offscreen frame");
        renderer.render_scene_to_texture(
            surface.device(),
            surface.queue(),
            frame.view(),
            self.width.max(1),
            self.height.max(1),
            vello::peniko::Color::TRANSPARENT,
        );
        let rgba8 = readback_texture_rgba8(
            surface.device(),
            surface.queue(),
            frame.texture(),
            self.width.max(1),
            self.height.max(1),
        );
        renderer.clear_frame_resources();
        surface.present(frame);

        Snapshot {
            width: self.width.max(1),
            height: self.height.max(1),
            rgba8,
        }
    }
}

fn readback_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    const BYTES_PER_PIXEL: u32 = 4;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("waterui-testing-readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("waterui-testing-readback-encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender
            .send(result)
            .expect("waterui-testing readback channel receiver dropped");
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver
        .recv()
        .expect("waterui-testing readback callback dropped")
        .expect("waterui-testing failed to map readback buffer");

    let mapped = slice.get_mapped_range();
    let mut pixels = vec![0u8; (width * height * BYTES_PER_PIXEL) as usize];
    for row in 0..height as usize {
        let source_start = row * padded_bytes_per_row as usize;
        let source_end = source_start + unpadded_bytes_per_row as usize;
        let destination_start = row * unpadded_bytes_per_row as usize;
        let destination_end = destination_start + unpadded_bytes_per_row as usize;
        pixels[destination_start..destination_end]
            .copy_from_slice(&mapped[source_start..source_end]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}

/// Stable role wrapper exposed by the testing API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Role(AccessibilityRole);

impl Role {
    pub const BUTTON: Self = Self(AccessibilityRole::Button);
    pub const LABEL: Self = Self(AccessibilityRole::Label);
    pub const TEXT_INPUT: Self = Self(AccessibilityRole::TextInput);
    pub const CHECKBOX: Self = Self(AccessibilityRole::CheckBox);
    pub const SWITCH: Self = Self(AccessibilityRole::Switch);
    pub const SLIDER: Self = Self(AccessibilityRole::Slider);
    pub const LIST: Self = Self(AccessibilityRole::List);
    pub const LIST_ITEM: Self = Self(AccessibilityRole::ListItem);
    pub const COMBOBOX: Self = Self(AccessibilityRole::ComboBox);
    pub const OPTION: Self = Self(AccessibilityRole::ListBoxOption);

    const fn as_accesskit(self) -> AccessibilityRole {
        self.0
    }
}

/// Stable node identifier wrapper exposed by the testing API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(AccessibilityNodeId);

impl NodeId {
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0.0
    }

    const fn as_accesskit(self) -> AccessibilityNodeId {
        self.0
    }
}

impl From<AccessibilityNodeId> for NodeId {
    fn from(value: AccessibilityNodeId) -> Self {
        Self(value)
    }
}

/// Immutable accessibility node snapshot used by assertions and queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSnapshot {
    id: NodeId,
    role: Role,
    label: Option<String>,
    value: Option<String>,
    enabled: bool,
    selected: bool,
    checked: Option<bool>,
    expanded: Option<bool>,
    hidden: bool,
    children: Vec<NodeId>,
}

impl NodeSnapshot {
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }

    #[must_use]
    pub const fn checked(&self) -> Option<bool> {
        self.checked
    }

    #[must_use]
    pub const fn expanded(&self) -> Option<bool> {
        self.expanded
    }

    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    fn from_accesskit(id: AccessibilityNodeId, node: &AccessibilityNode) -> Self {
        let checked = match node.toggled() {
            Some(AccessibilityToggled::True) => Some(true),
            Some(AccessibilityToggled::False) => Some(false),
            Some(AccessibilityToggled::Mixed) | None => None,
        };
        let expanded = node.is_expanded();

        let value = node
            .value()
            .map(ToOwned::to_owned)
            .or_else(|| node.numeric_value().map(|v| v.to_string()));

        Self {
            id: NodeId::from(id),
            role: Role(node.role()),
            label: node.label().map(ToOwned::to_owned),
            value,
            enabled: !node.is_disabled(),
            selected: node.is_selected().unwrap_or(false),
            checked,
            expanded,
            hidden: node.is_hidden(),
            children: node.children().iter().copied().map(NodeId::from).collect(),
        }
    }
}

/// Immutable accessibility tree snapshot.
#[derive(Debug, Clone)]
pub struct TreeSnapshot {
    revision: u64,
    root: NodeId,
    focus: NodeId,
    nodes: BTreeMap<NodeId, NodeSnapshot>,
}

impl TreeSnapshot {
    fn empty() -> Self {
        let root = NodeId::from(AccessibilityNodeId(0));
        Self {
            revision: 0,
            root,
            focus: root,
            nodes: BTreeMap::new(),
        }
    }

    fn from_update(revision: u64, update: AccessibilityTreeUpdate) -> Self {
        let root = update
            .tree
            .as_ref()
            .map(|tree| NodeId::from(tree.root))
            .unwrap_or_else(|| NodeId::from(AccessibilityNodeId(0)));
        let focus = NodeId::from(update.focus);
        let mut nodes = BTreeMap::new();
        for (id, node) in update.nodes {
            let stable_id = NodeId::from(id);
            nodes.insert(stable_id, NodeSnapshot::from_accesskit(id, &node));
        }

        Self {
            revision,
            root,
            focus,
            nodes,
        }
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub const fn focus(&self) -> NodeId {
        self.focus
    }

    #[must_use]
    pub fn nodes(&self) -> &BTreeMap<NodeId, NodeSnapshot> {
        &self.nodes
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&NodeSnapshot> {
        self.nodes.get(&id)
    }

    fn matching(&self, selector: &Selector) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter_map(|(id, node)| selector.matches(node).then_some(*id))
            .collect()
    }
}

impl Index<NodeId> for TreeSnapshot {
    type Output = NodeSnapshot;

    fn index(&self, index: NodeId) -> &Self::Output {
        self.nodes.get(&index).unwrap_or_else(|| {
            panic!(
                "waterui-testing tree index missing node id {} (revision {})",
                index.as_u64(),
                self.revision
            )
        })
    }
}

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

    fn matches(&self, node: &NodeSnapshot) -> bool {
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
    node_id: NodeId,
    node: NodeSnapshot,
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

    /// Performs a click/tap action.
    pub fn tap(&self, app: &mut MountedApp) -> bool {
        app.perform_action(self.node_id, AccessibilityAction::Click, None)
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
    fn new(elements: Vec<ElementRef>) -> Self {
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

/// XCTest-like expectation descriptor.
#[derive(Debug, Clone)]
pub struct Expectation {
    kind: ExpectationKind,
    inverted: bool,
}

impl Expectation {
    #[must_use]
    pub const fn inverted(mut self) -> Self {
        self.inverted = true;
        self
    }
}

#[derive(Debug, Clone)]
enum ExpectationKind {
    Exists(Selector),
    NotExists(Selector),
    ValueEquals { selector: Selector, value: String },
}

/// Wait behavior configuration.
#[derive(Debug, Clone, Copy)]
pub struct WaitOptions {
    pub timeout: Duration,
    pub enforce_order: bool,
}

impl WaitOptions {
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            enforce_order: false,
        }
    }

    #[must_use]
    pub const fn enforce_order(mut self, enforce_order: bool) -> Self {
        self.enforce_order = enforce_order;
        self
    }
}

impl Default for WaitOptions {
    fn default() -> Self {
        Self::new(Duration::from_secs(2))
    }
}

/// XCTest-like wait result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitResult {
    Completed,
    TimedOut,
    IncorrectOrder,
    InvertedFulfillment,
    Interrupted,
}

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
        let mut env = Environment::new().extending(SceneViewMergeToParent);
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
    env: Environment,
    content: AnyViewBuilder<AnyView>,
    driver: Box<dyn A11yDriver>,
    tree: TreeSnapshot,
    revision: u64,
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

    fn perform_action(
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
        if changed {
            let _ = self.wait_for(
                &[self.expect_exists(Selector::default())],
                WaitOptions::new(Duration::from_millis(200)),
            );
        }
        changed
    }

    fn pump_once(&mut self) -> bool {
        let outcome = self.driver.pump(self.content.build(), &self.env);
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
        outcome.rebuilt
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

trait A11yDriver {
    fn pump(&mut self, content: AnyView, env: &Environment) -> DriverPumpResult;
    fn perform_action(&mut self, request: AccessibilityActionRequest, env: &Environment) -> bool;
}

#[derive(Debug)]
struct DriverPumpResult {
    rebuilt: bool,
    tree_update: Option<AccessibilityTreeUpdate>,
}

#[derive(Debug)]
struct HydrolysisA11yDriver {
    platform: OffscreenWindow,
    renderer: HydrolysisRenderer,
    needs_rebuild: bool,
}

impl HydrolysisA11yDriver {
    fn new(width: u32, height: u32) -> Self {
        let mut platform =
            OffscreenWindow::new(width.max(1), height.max(1), wgpu::TextureFormat::Rgba8Unorm);
        let renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };

        Self {
            platform,
            renderer,
            needs_rebuild: true,
        }
    }
}

impl A11yDriver for HydrolysisA11yDriver {
    fn pump(&mut self, content: AnyView, env: &Environment) -> DriverPumpResult {
        let surface = self.platform.surface();
        let (width, height) = surface.size();
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));

        self.renderer
            .set_frame_resources(surface.device(), surface.queue());

        let animation_dirty = self.renderer.advance_animations();
        if animation_dirty {
            self.needs_rebuild = true;
        }

        let should_rebuild = self.needs_rebuild || self.renderer.take_rebuild_request();
        if should_rebuild {
            self.renderer.reset_scene();
            self.renderer.begin_rebuild_frame();
            self.renderer.dispatch(content, env, bounds);
            self.renderer.finish_rebuild_frame();
            self.needs_rebuild = false;
        }

        let frame = surface
            .acquire()
            .expect("waterui-testing driver failed to acquire offscreen frame");
        self.renderer.render_scene_to_texture(
            surface.device(),
            surface.queue(),
            frame.view(),
            width,
            height,
            vello::peniko::Color::TRANSPARENT,
        );
        self.renderer.clear_frame_resources();
        surface.present(frame);

        if self.renderer.take_rebuild_request() {
            self.needs_rebuild = true;
        }

        DriverPumpResult {
            rebuilt: should_rebuild || animation_dirty,
            tree_update: self.renderer.take_accessibility_tree_update(),
        }
    }

    fn perform_action(&mut self, request: AccessibilityActionRequest, env: &Environment) -> bool {
        let changed = self.renderer.handle_accessibility_action(request, env);
        if changed {
            self.needs_rebuild = true;
        }
        changed
    }
}

fn install_native_component_hooks(env: &mut Environment) {
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct NoopDriver;

    impl A11yDriver for NoopDriver {
        fn pump(&mut self, _content: AnyView, _env: &Environment) -> DriverPumpResult {
            DriverPumpResult {
                rebuilt: false,
                tree_update: None,
            }
        }

        fn perform_action(
            &mut self,
            _request: AccessibilityActionRequest,
            _env: &Environment,
        ) -> bool {
            false
        }
    }

    fn node_id(raw: u64) -> NodeId {
        NodeId::from(AccessibilityNodeId(raw))
    }

    fn node(
        id: u64,
        role: Role,
        label: Option<&str>,
        value: Option<&str>,
        enabled: bool,
    ) -> NodeSnapshot {
        NodeSnapshot {
            id: node_id(id),
            role,
            label: label.map(ToOwned::to_owned),
            value: value.map(ToOwned::to_owned),
            enabled,
            selected: false,
            checked: None,
            expanded: None,
            hidden: false,
            children: Vec::new(),
        }
    }

    fn tree(nodes: Vec<NodeSnapshot>) -> TreeSnapshot {
        let Some(root) = nodes.first().map(NodeSnapshot::id) else {
            panic!("test tree helper requires at least one node");
        };
        let nodes = nodes.into_iter().map(|node| (node.id(), node)).collect();
        TreeSnapshot {
            revision: 1,
            root,
            focus: root,
            nodes,
        }
    }

    fn mounted(tree: TreeSnapshot) -> MountedApp {
        MountedApp {
            env: Environment::new(),
            content: AnyViewBuilder::new(|| AnyView::new(())),
            driver: Box::new(NoopDriver),
            tree,
            revision: 2,
        }
    }

    #[test]
    fn smoke_snapshot_size_matches_target() {
        let host = TestHost::new(Environment::new(), 64, 48);
        let snapshot = host.render(());
        assert_eq!(snapshot.width, 64);
        assert_eq!(snapshot.height, 48);
        assert_eq!(snapshot.rgba8.len(), 64 * 48 * 4);
    }

    #[test]
    fn query_chain_and_index_are_type_safe() {
        let mut app = mounted(tree(vec![
            node(1, Role::LIST, Some("root"), None, true),
            node(2, Role::BUTTON, Some("Save changes"), None, true),
            node(3, Role::BUTTON, Some("Save draft"), None, false),
        ]));

        let results = app
            .query()
            .role(Role::BUTTON)
            .label_contains("Save")
            .enabled(true)
            .all();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id().as_u64(), 2);
        assert_eq!(
            results[results[0].id()].node().label(),
            Some("Save changes")
        );
        assert_eq!(app.tree()[node_id(2)].label(), Some("Save changes"));
    }

    #[test]
    fn wait_for_existence_and_nonexistence_complete_immediately() {
        let mut app = mounted(tree(vec![
            node(1, Role::LIST, Some("root"), None, true),
            node(2, Role::LABEL, Some("status"), Some("ready"), true),
        ]));

        assert!(app.wait_for_existence(
            Selector::default().role(Role::LABEL).label("status"),
            Duration::from_millis(50),
        ));
        assert!(app.wait_for_nonexistence(
            Selector::default().role(Role::BUTTON).label("missing"),
            Duration::from_millis(50),
        ));
        assert!(app.wait_for_value_eq(
            Selector::default().role(Role::LABEL).label("status"),
            "ready",
            Duration::from_millis(50),
        ));
    }

    #[test]
    fn wait_for_inverted_reports_fulfillment() {
        let mut app = mounted(tree(vec![
            node(1, Role::LIST, Some("root"), None, true),
            node(2, Role::BUTTON, Some("Delete"), None, true),
        ]));

        let expectation = app
            .expect_exists(Selector::default().role(Role::BUTTON).label("Delete"))
            .inverted();
        let result = app.wait_for(&[expectation], WaitOptions::new(Duration::from_millis(10)));
        assert_eq!(result, WaitResult::InvertedFulfillment);
    }

    #[test]
    fn wait_for_times_out_when_condition_never_matches() {
        let mut app = mounted(tree(vec![node(1, Role::LIST, Some("root"), None, true)]));

        let expectation = app.expect_exists(Selector::default().role(Role::BUTTON).label("never"));
        let result = app.wait_for(&[expectation], WaitOptions::new(Duration::from_millis(10)));
        assert_eq!(result, WaitResult::TimedOut);
    }

    #[test]
    fn wait_for_panics_on_empty_expectations() {
        let mut app = mounted(tree(vec![node(1, Role::LIST, Some("root"), None, true)]));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app.wait_for(&[], WaitOptions::default());
        }));
        assert!(outcome.is_err());
    }

    #[test]
    fn query_optional_panics_on_multiple_matches() {
        let mut app = mounted(tree(vec![
            node(1, Role::LIST, Some("root"), None, true),
            node(2, Role::BUTTON, Some("A"), None, true),
            node(3, Role::BUTTON, Some("A"), None, true),
        ]));

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = app.query().role(Role::BUTTON).label("A").optional();
        }));
        assert!(outcome.is_err());
    }

    #[test]
    fn element_set_index_by_node_id_panics_when_missing() {
        let mut app = mounted(tree(vec![
            node(1, Role::LIST, Some("root"), None, true),
            node(2, Role::BUTTON, Some("A"), None, true),
        ]));
        let set = app.query().role(Role::BUTTON).all();

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = &set[node_id(99)];
        }));
        assert!(outcome.is_err());
    }
}
