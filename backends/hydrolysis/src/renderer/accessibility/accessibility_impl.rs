use super::*;

#[cfg(feature = "accessibility")]
use std::borrow::Cow;
#[cfg(feature = "accessibility")]
use std::collections::VecDeque;
#[cfg(feature = "accessibility")]
use std::ops::RangeInclusive;
#[cfg(feature = "accessibility")]
use waterui_form::picker::date::{DatePickerType, DateTime};

#[cfg(feature = "accessibility")]
pub(crate) const ACCESSIBILITY_ROOT_NODE_ID: AccessibilityNodeId = AccessibilityNodeId(0);
#[cfg(feature = "accessibility")]
pub(crate) const ACCESSIBILITY_FIRST_NODE_ID: u64 = 1;

#[cfg(feature = "accessibility")]
#[derive(Clone)]
pub(crate) enum AccessibilityActionTarget {
    PointerPrimaryClick {
        point: vello::kurbo::Point,
    },
    Toggle {
        binding: nami::Binding<bool>,
    },
    Slider {
        value: nami::Binding<f64>,
        range: RangeInclusive<f64>,
        step: f64,
    },
    Stepper {
        value: nami::Binding<i32>,
        step: nami::Computed<i32>,
        range: RangeInclusive<i32>,
    },
    DatePicker {
        value: nami::Binding<DateTime>,
        range: RangeInclusive<DateTime>,
        ty: DatePickerType,
        origin: LayoutPoint,
    },
    TextField {
        value: nami::Binding<StyledStr>,
        line_limit: Option<usize>,
    },
    SecureField {
        value: nami::Binding<FormSecure>,
    },
    PickerSelect {
        selection: nami::Binding<waterui_core::id::Id>,
        target: waterui_core::id::Id,
    },
    Scroll {
        handle: ScrollHandle,
        axis: ScrollAxis,
    },
}

#[cfg(feature = "accessibility")]
pub(crate) struct AccessibilityBuilder {
    pub(crate) nodes: Vec<(AccessibilityNodeId, AccessibilityNode)>,
    pub(crate) root_children: Vec<AccessibilityNodeId>,
    pub(crate) actions: BTreeMap<AccessibilityNodeId, AccessibilityActionTarget>,
    pub(crate) next_node_id: u64,
    pub(crate) root_bounds: vello::kurbo::Rect,
    pub(crate) root_label: String,
    pub(crate) focus: AccessibilityNodeId,
    pub(crate) pending_text_input_nodes: VecDeque<AccessibilityNodeId>,
    pub(crate) parent_stack: Vec<AccessibilityNodeId>,
    pub(crate) suppression_depth: usize,
    pub(crate) pending_tree_update: Option<AccessibilityTreeUpdate>,
}

#[cfg(feature = "accessibility")]
impl Default for AccessibilityBuilder {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            root_children: Vec::new(),
            actions: BTreeMap::new(),
            next_node_id: ACCESSIBILITY_FIRST_NODE_ID,
            root_bounds: vello::kurbo::Rect::ZERO,
            root_label: String::from("WaterUI Window"),
            focus: ACCESSIBILITY_ROOT_NODE_ID,
            pending_text_input_nodes: VecDeque::new(),
            parent_stack: Vec::new(),
            suppression_depth: 0,
            pending_tree_update: None,
        }
    }
}

#[cfg(feature = "accessibility")]
impl AccessibilityBuilder {
    /// Reset the per-flush accessibility emission state. Called before EVERY flush
    /// (rebuild and refresh both go through `HydrolysisRenderer::reset_scene`), so
    /// the full a11y tree it re-emits starts clean — crucially resetting
    /// `next_node_id` so a node keeps a stable id across frames (a `Cell`-stable id
    /// is what `ui_focus`/`tree.focus()` compare against). Previously this only
    /// cleared the pending update, so on a geometry-static refresh flush the node
    /// list accumulated and ids drifted, desyncing UI focus.
    pub(crate) fn reset_scene(&mut self) {
        self.pending_tree_update = None;
        self.nodes.clear();
        self.root_children.clear();
        self.actions.clear();
        self.next_node_id = ACCESSIBILITY_FIRST_NODE_ID;
        self.pending_text_input_nodes.clear();
        self.parent_stack.clear();
        self.suppression_depth = 0;
    }

    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.reset_scene();
    }

    pub(crate) fn next_node_id(&mut self) -> AccessibilityNodeId {
        let node_id = AccessibilityNodeId(self.next_node_id);
        self.next_node_id = self
            .next_node_id
            .checked_add(1)
            .expect("hydrolysis accessibility node ID overflow");
        node_id
    }

    pub(crate) fn push_pending_text_input_node(&mut self, node_id: AccessibilityNodeId) {
        self.pending_text_input_nodes.push_back(node_id);
    }

    pub(crate) fn take_pending_text_input_node(&mut self) -> Option<AccessibilityNodeId> {
        self.pending_text_input_nodes.pop_front()
    }

    pub(crate) fn push_suppression(&mut self) {
        self.suppression_depth = self
            .suppression_depth
            .checked_add(1)
            .expect("hydrolysis accessibility suppression depth overflow");
    }

    pub(crate) fn pop_suppression(&mut self) {
        self.suppression_depth = self
            .suppression_depth
            .checked_sub(1)
            .expect("hydrolysis accessibility suppression underflow");
    }

    pub(crate) fn apply_state(&self, env: &Environment, node: &mut AccessibilityNode) {
        // The tree build stores accessibility state as a live signal (a static
        // state is a constant signal), so a reactive state — e.g. a filter chip's
        // selected binding — is resolved fresh on every emission.
        let Some(state) = env
            .get::<AccessibilityStateSignal>()
            .map(|signal| signal.state().get())
        else {
            return;
        };
        if state.is_disabled() {
            node.set_disabled();
        }
        if state.is_selected() {
            node.set_selected(true);
        }
        if let Some(checked) = state.checked_state() {
            node.set_toggled(AccessibilityToggled::from(checked));
        }
        if let Some(expanded) = state.expanded_state() {
            node.set_expanded(expanded);
        }
        if state.is_busy() {
            node.set_busy();
        }
        if state.is_hidden() {
            node.set_hidden();
        }
    }

    pub(crate) fn register_node_internal(
        &mut self,
        mut node: AccessibilityNode,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
        attach_to_root: bool,
    ) -> Option<AccessibilityNodeId> {
        if self.suppression_depth > 0 {
            return None;
        }
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return None;
        }
        self.apply_state(env, &mut node);
        let node_id = self.next_node_id();
        node.set_bounds(kurbo_rect_to_accesskit_rect(bounds));
        self.nodes.push((node_id, node));
        if attach_to_root {
            if let Some(parent_id) = self.parent_stack.last().copied() {
                let parent = self
                    .nodes
                    .iter_mut()
                    .find_map(|(id, node)| (*id == parent_id).then_some(node))
                    .expect("hydrolysis accessibility parent stack contains an unknown node");
                parent.push_child(node_id);
            } else {
                self.root_children.push(node_id);
            }
        }
        if let Some(target) = action_target {
            self.actions.insert(node_id, target);
        }
        Some(node_id)
    }

    pub(crate) fn finalize_tree_update(&mut self) {
        let mut root = AccessibilityNode::new(AccessibilityNodeRole::Window);
        root.set_label(self.root_label.clone());
        root.set_bounds(kurbo_rect_to_accesskit_rect(self.root_bounds));
        root.set_children(self.root_children.clone());
        let mut nodes = Vec::with_capacity(self.nodes.len() + 1);
        nodes.push((ACCESSIBILITY_ROOT_NODE_ID, root));
        nodes.extend(self.nodes.iter().cloned());
        if !self.nodes.iter().any(|(id, _)| *id == self.focus) {
            self.focus = ACCESSIBILITY_ROOT_NODE_ID;
        }
        self.pending_tree_update = Some(AccessibilityTreeUpdate {
            nodes,
            tree: Some(AccessibilityTree::new(ACCESSIBILITY_ROOT_NODE_ID)),
            tree_id: AccessibilityTreeId::ROOT,
            focus: self.focus,
        });
    }
}

#[cfg(feature = "accessibility")]
pub(crate) struct AccessibilityGroupScope {
    parent_pushed: bool,
    suppression_pushed: bool,
}

#[cfg(feature = "accessibility")]
pub(crate) fn accessibility_group_child_environment(env: &Environment) -> Option<Environment> {
    if !matches!(
        env.get::<AccessibilityRole>(),
        Some(AccessibilityRole::Group)
    ) {
        return None;
    }

    let mut child_env = env.clone();
    child_env.remove::<AccessibilityLabel>();
    child_env.remove::<AccessibilityRole>();
    child_env.remove::<AccessibilityHidden>();
    child_env.remove::<AccessibilityChildren>();
    child_env.remove::<AccessibilityState>();
    child_env.remove::<AccessibilityStateSignal>();
    Some(child_env)
}

impl HydrolysisRenderer {
    #[cfg(feature = "accessibility")]
    pub fn set_accessibility_root_label(&mut self, label: &str) {
        self.accessibility.root_label.clear();
        self.accessibility.root_label.push_str(label);
    }

    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn take_accessibility_tree_update(&mut self) -> Option<AccessibilityTreeUpdate> {
        self.accessibility.pending_tree_update.take()
    }

    #[cfg(feature = "accessibility")]
    pub fn handle_accessibility_action(
        &mut self,
        request: AccessibilityActionRequest,
        env: &Environment,
    ) -> bool {
        let action = request.action;
        let action_data = request.data;
        let target_node = request.target_node;
        let focus_action = matches!(
            action,
            AccessibilityAction::Focus | AccessibilityAction::Click
        );
        if target_node == ACCESSIBILITY_ROOT_NODE_ID {
            return match action {
                AccessibilityAction::Focus => {
                    let changed = self.accessibility.focus != ACCESSIBILITY_ROOT_NODE_ID;
                    self.accessibility.focus = ACCESSIBILITY_ROOT_NODE_ID;
                    changed
                }
                AccessibilityAction::Click => false,
                _ => panic!(
                    "hydrolysis accessibility root does not support action {:?}",
                    action
                ),
            };
        }
        let Some(target) = self.accessibility.actions.get(&target_node).cloned() else {
            // A node may legitimately register no action target: a disabled
            // control stays in the tree (focusable, announced as disabled)
            // but exposes no activate/value actions, and static content never
            // had any. Focus still lands when advertised; any action the node
            // does not advertise is rejected as unhandled. An *advertised*
            // action without a registered target is a widget bug.
            let node = self
                .accessibility
                .nodes
                .iter()
                .find_map(|(id, node)| (*id == target_node).then_some(node))
                .unwrap_or_else(|| {
                    panic!(
                        "hydrolysis accessibility action {:?} targets unknown node {:?}",
                        action, target_node
                    )
                });
            if action == AccessibilityAction::Focus
                && node.supports_action(AccessibilityAction::Focus)
            {
                let changed = self.accessibility.focus != target_node;
                self.accessibility.focus = target_node;
                return changed;
            }
            assert!(
                !node.supports_action(action),
                "hydrolysis accessibility node {target_node:?} advertises action {action:?} but registered no action target"
            );
            return false;
        };
        let changed = match target {
            AccessibilityActionTarget::PointerPrimaryClick { point } => {
                handle_accessibility_pointer_action(self, action, point, env)
            }
            AccessibilityActionTarget::Toggle { binding } => match action {
                AccessibilityAction::Click => {
                    let next = !binding.get();
                    binding.set(next);
                    true
                }
                AccessibilityAction::Focus => true,
                _ => panic!(
                    "hydrolysis accessibility toggle does not support action {:?}",
                    action
                ),
            },
            AccessibilityActionTarget::Slider { value, range, step } => {
                handle_accessibility_slider_action(
                    &value,
                    *range.start(),
                    *range.end(),
                    step,
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::Stepper { value, step, range } => {
                handle_accessibility_stepper_action(
                    &value,
                    &step,
                    *range.start(),
                    *range.end(),
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::DatePicker {
                value,
                range,
                ty,
                origin,
            } => handle_accessibility_date_picker_action(
                self,
                &value,
                &range,
                ty,
                origin,
                action,
                action_data,
                env,
            ),
            AccessibilityActionTarget::TextField { value, line_limit } => {
                handle_accessibility_text_field_action(
                    self,
                    target_node,
                    &value,
                    line_limit,
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::SecureField { value } => {
                handle_accessibility_secure_field_action(
                    self,
                    target_node,
                    &value,
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::PickerSelect { selection, target } => {
                handle_accessibility_picker_select_action(&selection, target, action)
            }
            AccessibilityActionTarget::Scroll { handle, axis } => {
                handle_accessibility_scroll_action(&handle, axis, action)
            }
        };
        if changed && focus_action {
            self.accessibility.focus = target_node;
        }
        changed
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn push_pending_text_input_accessibility_node(
        &mut self,
        node_id: AccessibilityNodeId,
    ) {
        self.accessibility.push_pending_text_input_node(node_id);
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn take_pending_text_input_accessibility_node(
        &mut self,
    ) -> Option<AccessibilityNodeId> {
        self.accessibility.take_pending_text_input_node()
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn push_accessibility_suppression(&mut self) {
        self.accessibility.push_suppression();
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn pop_accessibility_suppression(&mut self) {
        self.accessibility.pop_suppression();
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn begin_accessibility_group(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
    ) -> AccessibilityGroupScope {
        assert!(
            matches!(
                env.get::<AccessibilityRole>(),
                Some(AccessibilityRole::Group)
            ),
            "hydrolysis accessibility group scope requires the Group role"
        );

        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            self.push_accessibility_suppression();
            return AccessibilityGroupScope {
                parent_pushed: false,
                suppression_pushed: true,
            };
        }

        let mut node = AccessibilityNode::new(AccessibilityNodeRole::Group);
        if let Some(label) = self.resolve_accessibility_label(env, None) {
            node.set_label(label);
        }
        let state_hidden = env
            .get::<AccessibilityStateSignal>()
            .is_some_and(|signal| signal.state().get().is_hidden());
        let excludes_descendants = env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants);
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            self.push_accessibility_suppression();
            return AccessibilityGroupScope {
                parent_pushed: false,
                suppression_pushed: true,
            };
        }
        let node_id = self
            .register_accessibility_node(node, bounds, env, None)
            .expect("positive accessibility group bounds must register a node");
        self.accessibility.parent_stack.push(node_id);
        let suppression_pushed = state_hidden || excludes_descendants;
        if suppression_pushed {
            self.push_accessibility_suppression();
        }
        AccessibilityGroupScope {
            parent_pushed: true,
            suppression_pushed,
        }
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn end_accessibility_group(&mut self, scope: AccessibilityGroupScope) {
        if scope.suppression_pushed {
            self.pop_accessibility_suppression();
        }
        if scope.parent_pushed {
            self.accessibility
                .parent_stack
                .pop()
                .expect("hydrolysis accessibility group parent stack underflow");
        }
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_node(
        &mut self,
        node: AccessibilityNode,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        self.accessibility
            .register_node_internal(node, bounds, env, action_target, true)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_child_node(
        &mut self,
        node: AccessibilityNode,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        self.accessibility
            .register_node_internal(node, bounds, env, action_target, false)
    }

    /// Subscribes the scoped accessibility-state signal (if any) to the refresh
    /// pump, so a state change — a chip toggling selected, a row expanding —
    /// re-flushes the tree and re-emits the node with the current state.
    #[cfg(feature = "accessibility")]
    fn watch_accessibility_state(&mut self, env: &Environment) {
        if let Some(signal) = env.get::<AccessibilityStateSignal>() {
            self.watch_signal(signal.state());
        }
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn accessibility_label_from_view(
        &mut self,
        view: &AnyView,
        env: &Environment,
    ) -> Option<String> {
        self.accessibility_label_from_view_with_budget(view, env, 32)
    }

    /// Resolves the spoken accessibility text directly from a typed
    /// [`Label`](waterui_controls::label::Label) without any view-tree
    /// traversal. Backends should prefer this over `accessibility_label_from_view`
    /// when the source is already a known label, so that `LabelDisplayMode::Hidden`
    /// labels still surface their semantic text in the accessibility tree.
    #[cfg(feature = "accessibility")]
    pub(crate) fn accessibility_label_from_label(
        &self,
        label: &waterui_controls::label::Label,
        env: &Environment,
    ) -> Option<String> {
        use waterui_core::Signal;
        let plain = label
            .clone()
            .resolve(env)
            .accessibility_label()
            .get()
            .to_plain();
        let trimmed = plain.as_str().trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(String::from(trimmed))
        }
    }

    #[cfg(feature = "accessibility")]
    fn accessibility_label_from_view_with_budget(
        &mut self,
        view: &AnyView,
        env: &Environment,
        remaining: usize,
    ) -> Option<String> {
        assert!(
            (remaining != 0),
            "hydrolysis accessibility label extraction exceeded recursion budget for {}",
            view.name()
        );
        let (view, scoped_env) = flatten_environment_metadata_ref(view, env);
        if let Some(content) = passthrough_content(view) {
            return self.accessibility_label_from_view_with_budget(
                content,
                &scoped_env,
                remaining - 1,
            );
        }
        if let Some(label) = view.downcast_ref::<SemanticLabel>() {
            let styled = self.read_signal(&label.semantic_text().resolve(&scoped_env).content);
            return Some(styled.to_plain().to_string());
        }
        if let Some(label) = view.downcast_ref::<Str>() {
            return Some(label.as_str().to_owned());
        }
        if let Some(label) = view.downcast_ref::<&'static str>() {
            let body = AnyView::new((*label).body(&scoped_env));
            return self.accessibility_label_from_view_with_budget(
                &body,
                &scoped_env,
                remaining - 1,
            );
        }
        if let Some(label) = view.downcast_ref::<String>() {
            let body = AnyView::new(label.clone().body(&scoped_env));
            return self.accessibility_label_from_view_with_budget(
                &body,
                &scoped_env,
                remaining - 1,
            );
        }
        if let Some(label) = view.downcast_ref::<Cow<'static, str>>() {
            let body = AnyView::new(label.clone().body(&scoped_env));
            return self.accessibility_label_from_view_with_budget(
                &body,
                &scoped_env,
                remaining - 1,
            );
        }
        if let Some(text) = view.downcast_ref::<Native<TextConfig>>() {
            let styled = self.read_signal(&text.as_inner().content);
            return Some(styled.to_plain().to_string());
        }
        if let Some(icon) = view.downcast_ref::<Native<SystemIcon>>() {
            return Some(icon.as_inner().name.as_str().to_owned());
        }
        if let Some(container) = view.downcast_ref::<Native<FixedContainer>>() {
            let (_, children) = container.as_inner().as_parts();
            let labels = children
                .iter()
                .filter_map(|child| {
                    self.accessibility_label_from_view_with_budget(
                        child,
                        &scoped_env,
                        remaining - 1,
                    )
                    .map(|label| label.trim().to_owned())
                    .filter(|label| !label.is_empty())
                })
                .collect::<Vec<_>>();
            if !labels.is_empty() {
                return Some(labels.join(" "));
            }
        }
        None
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn resolve_accessibility_label(
        &mut self,
        env: &Environment,
        default_label: Option<String>,
    ) -> Option<String> {
        env.get::<AccessibilityLabel>()
            .map(|label| label.as_str().as_str().to_owned())
            .or(default_label)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn resolve_accessibility_role(
        &self,
        env: &Environment,
        default_role: AccessibilityNodeRole,
    ) -> AccessibilityNodeRole {
        env.get::<AccessibilityRole>()
            .map(|role| accessibility_role_to_accesskit_role(role.clone()))
            .unwrap_or(default_role)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn finalize_accessibility_tree_update(&mut self) {
        self.accessibility.finalize_tree_update();
    }
}

#[cfg(feature = "accessibility")]
pub(crate) fn accessibility_activation_point(bounds: vello::kurbo::Rect) -> vello::kurbo::Point {
    vello::kurbo::Point::new((bounds.x0 + bounds.x1) * 0.5, (bounds.y0 + bounds.y1) * 0.5)
}

#[cfg(feature = "accessibility")]
fn accessibility_role_to_accesskit_role(role: AccessibilityRole) -> AccessibilityNodeRole {
    match role {
        AccessibilityRole::Button => AccessibilityNodeRole::Button,
        AccessibilityRole::Link => AccessibilityNodeRole::Link,
        AccessibilityRole::Image => AccessibilityNodeRole::Image,
        AccessibilityRole::Text => AccessibilityNodeRole::Label,
        AccessibilityRole::Header => AccessibilityNodeRole::Header,
        AccessibilityRole::Footer => AccessibilityNodeRole::Footer,
        AccessibilityRole::Navigation => AccessibilityNodeRole::Navigation,
        AccessibilityRole::Main => AccessibilityNodeRole::Main,
        AccessibilityRole::Search => AccessibilityNodeRole::Search,
        AccessibilityRole::Article => AccessibilityNodeRole::Article,
        AccessibilityRole::Section => AccessibilityNodeRole::Section,
        AccessibilityRole::List => AccessibilityNodeRole::List,
        AccessibilityRole::ListItem => AccessibilityNodeRole::ListItem,
        AccessibilityRole::Checkbox => AccessibilityNodeRole::CheckBox,
        AccessibilityRole::RadioButton => AccessibilityNodeRole::RadioButton,
        AccessibilityRole::Switch => AccessibilityNodeRole::Switch,
        AccessibilityRole::Slider => AccessibilityNodeRole::Slider,
        AccessibilityRole::ProgressBar => AccessibilityNodeRole::ProgressIndicator,
        AccessibilityRole::Tab => AccessibilityNodeRole::Tab,
        AccessibilityRole::TabList => AccessibilityNodeRole::TabList,
        AccessibilityRole::TabPanel => AccessibilityNodeRole::TabPanel,
        AccessibilityRole::Menu => AccessibilityNodeRole::Menu,
        AccessibilityRole::MenuItem => AccessibilityNodeRole::MenuItem,
        AccessibilityRole::MenuBar => AccessibilityNodeRole::MenuBar,
        AccessibilityRole::MenuItemCheckbox => AccessibilityNodeRole::MenuItemCheckBox,
        AccessibilityRole::MenuItemRadio => AccessibilityNodeRole::MenuItemRadio,
        AccessibilityRole::Combobox => AccessibilityNodeRole::ComboBox,
        AccessibilityRole::Option => AccessibilityNodeRole::ListBoxOption,
        AccessibilityRole::Group => AccessibilityNodeRole::Group,
        _ => panic!("hydrolysis accessibility role variant is not implemented"),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_pointer_action(
    renderer: &mut HydrolysisRenderer,
    action: AccessibilityAction,
    point: vello::kurbo::Point,
    env: &Environment,
) -> bool {
    let x = point.x as f32;
    let y = point.y as f32;
    match action {
        AccessibilityAction::Click => {
            let mut changed = renderer.handle_pointer_down(x, y, PointerButton::Primary, env);
            changed |= renderer.handle_pointer_up(x, y, PointerButton::Primary, env);
            changed
        }
        // Accessibility focus moves only the accessibility focus ring; it must
        // not synthesize a pointer press, which would steal the text-input UI
        // focus (UI focus and accessibility focus are independent).
        AccessibilityAction::Focus => true,
        _ => panic!(
            "hydrolysis accessibility pointer target does not support action {:?}",
            action
        ),
    }
}

/// One accessibility scroll step in logical pixels. Assistive-tech scroll
/// actions are programmatic: they move the offset immediately (the pixel
/// path) instead of gliding through the smoothed-wheel animation, so the
/// result is deterministic for the caller.
#[cfg(feature = "accessibility")]
const ACCESSIBILITY_SCROLL_STEP: f32 = crate::scroll::SCROLL_LINE_STEP as f32;

#[cfg(feature = "accessibility")]
fn handle_accessibility_scroll_action(
    handle: &ScrollHandle,
    axis: ScrollAxis,
    action: AccessibilityAction,
) -> bool {
    let step = ACCESSIBILITY_SCROLL_STEP;
    match action {
        AccessibilityAction::ScrollLeft => match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => handle.apply_scroll_delta(step, 0.0, false),
            ScrollAxis::Vertical => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollRight => match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => {
                handle.apply_scroll_delta(-step, 0.0, false)
            }
            ScrollAxis::Vertical => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollUp => match axis {
            ScrollAxis::Vertical | ScrollAxis::All => handle.apply_scroll_delta(0.0, step, false),
            ScrollAxis::Horizontal => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollDown => match axis {
            ScrollAxis::Vertical | ScrollAxis::All => handle.apply_scroll_delta(0.0, -step, false),
            ScrollAxis::Horizontal => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        _ => false,
    }
}

#[cfg(feature = "accessibility")]
pub(crate) fn slider_step_for_range(range: RangeInclusive<f64>) -> f64 {
    let start = *range.start();
    let end = *range.end();
    let span = end - start;
    assert!(
        span > 0.0,
        "hydrolysis accessibility slider requires range start < end"
    );
    span / 100.0
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_slider_action(
    value: &nami::Binding<f64>,
    start: f64,
    end: f64,
    step: f64,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    if matches!(action, AccessibilityAction::Focus) {
        return true;
    }
    assert!(
        step > 0.0,
        "hydrolysis accessibility slider requires positive step"
    );
    let previous = value.get().clamp(start, end);
    let next = match action {
        AccessibilityAction::Increment => (previous + step).min(end),
        AccessibilityAction::Decrement => (previous - step).max(start),
        AccessibilityAction::SetValue => match data {
            Some(AccessibilityActionData::NumericValue(target)) => target.clamp(start, end),
            _ => {
                panic!("hydrolysis accessibility slider SetValue requires NumericValue data")
            }
        },
        _ => panic!(
            "hydrolysis accessibility slider does not support action {:?}",
            action
        ),
    };
    if (next - previous).abs() <= f64::EPSILON {
        return false;
    }
    value.set(next);
    true
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_stepper_action(
    value: &nami::Binding<i32>,
    step: &nami::Computed<i32>,
    start: i32,
    end: i32,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    if matches!(action, AccessibilityAction::Focus) {
        return true;
    }
    let step_value = step.get();
    assert!(
        (step_value > 0),
        "hydrolysis accessibility stepper requires positive step"
    );
    let previous = value.get().clamp(start, end);
    let next = match action {
        AccessibilityAction::Increment => previous.saturating_add(step_value).min(end),
        AccessibilityAction::Decrement => previous.saturating_sub(step_value).max(start),
        AccessibilityAction::SetValue => match data {
            Some(AccessibilityActionData::NumericValue(target)) => {
                let rounded = target.round() as i32;
                rounded.clamp(start, end)
            }
            Some(AccessibilityActionData::Value(ref text)) => {
                let parsed = text
                    .parse::<i32>()
                    .expect("hydrolysis accessibility stepper SetValue text must parse as i32");
                parsed.clamp(start, end)
            }
            _ => panic!("hydrolysis accessibility stepper SetValue requires numeric data"),
        },
        _ => panic!(
            "hydrolysis accessibility stepper does not support action {:?}",
            action
        ),
    };
    if next == previous {
        return false;
    }
    value.set(next);
    true
}

#[cfg(feature = "accessibility")]
#[allow(
    clippy::too_many_arguments,
    reason = "threads the full accessibility-action context; grouping into a struct would not improve clarity"
)]
fn handle_accessibility_date_picker_action(
    renderer: &mut HydrolysisRenderer,
    value: &nami::Binding<DateTime>,
    range: &RangeInclusive<DateTime>,
    ty: DatePickerType,
    origin: LayoutPoint,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
    env: &Environment,
) -> bool {
    match action {
        AccessibilityAction::Click => {
            renderer.show_date_picker(value.clone(), range.clone(), ty, origin, env)
        }
        AccessibilityAction::Focus => true,
        AccessibilityAction::SetValue => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!("hydrolysis accessibility date picker SetValue requires Value data");
            };
            let parsed = ty.parse_value(text.as_ref()).unwrap_or_else(|error| {
                panic!(
                    "hydrolysis accessibility date picker could not parse value {:?} with format {}: {error}",
                    text,
                    ty.format_string(),
                )
            });
            let previous = value.get().clamp(*range.start(), *range.end());
            let next = parsed.clamp(*range.start(), *range.end());
            if next == previous {
                return false;
            }
            value.set(next);
            true
        }
        _ => panic!(
            "hydrolysis accessibility date picker does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_text_field_action(
    renderer: &mut HydrolysisRenderer,
    node_id: AccessibilityNodeId,
    value: &nami::Binding<StyledStr>,
    line_limit: Option<usize>,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            renderer.focus_text_input_for_accessibility_node(node_id)
        }
        AccessibilityAction::SetValue => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!("hydrolysis accessibility text field SetValue requires Value data");
            };
            let normalized = normalized_insert_text(text.as_ref(), line_limit);
            assert!(
                !(exceeds_line_limit(normalized.as_str(), line_limit)),
                "hydrolysis accessibility text field SetValue exceeds line_limit {:?}",
                line_limit
            );
            value.set(StyledStr::plain(normalized));
            true
        }
        AccessibilityAction::ReplaceSelectedText => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!(
                    "hydrolysis accessibility text field ReplaceSelectedText requires Value data"
                );
            };
            let normalized = normalized_insert_text(text.as_ref(), line_limit);
            let mut plain = value.get().to_plain().to_string();
            assert!(
                apply_text_insert(&mut plain, normalized.as_str(), line_limit),
                "hydrolysis accessibility text field ReplaceSelectedText exceeds line_limit {:?}",
                line_limit
            );
            value.set(StyledStr::plain(plain));
            true
        }
        _ => panic!(
            "hydrolysis accessibility text field does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_secure_field_action(
    renderer: &mut HydrolysisRenderer,
    node_id: AccessibilityNodeId,
    value: &nami::Binding<FormSecure>,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            renderer.focus_text_input_for_accessibility_node(node_id)
        }
        AccessibilityAction::SetValue => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!("hydrolysis accessibility secure field SetValue requires Value data");
            };
            let normalized = normalized_insert_text(text.as_ref(), Some(1));
            let mut next = FormSecure::default();
            next.set(normalized);
            value.set(next);
            true
        }
        AccessibilityAction::ReplaceSelectedText => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!(
                    "hydrolysis accessibility secure field ReplaceSelectedText requires Value data"
                );
            };
            let mut plain = value.get().expose().to_owned();
            assert!(
                apply_text_insert(&mut plain, text.as_ref(), Some(1)),
                "hydrolysis accessibility secure field ReplaceSelectedText exceeds line_limit 1"
            );
            let mut next = FormSecure::default();
            next.set(plain);
            value.set(next);
            true
        }
        _ => panic!(
            "hydrolysis accessibility secure field does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_picker_select_action(
    selection: &nami::Binding<waterui_core::id::Id>,
    target: waterui_core::id::Id,
    action: AccessibilityAction,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            if selection.get() == target {
                return false;
            }
            selection.set(target);
            true
        }
        _ => panic!(
            "hydrolysis accessibility picker select does not support action {:?}",
            action
        ),
    }
}
