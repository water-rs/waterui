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
    },
    TextField {
        value: nami::Binding<StyledStr>,
        line_limit: Option<usize>,
    },
    SecureField {
        value: nami::Binding<FormSecure>,
    },
    PickerCycle {
        selection: nami::Binding<waterui_core::id::Id>,
        ids: Vec<waterui_core::id::Id>,
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
    pub(crate) suppression_depth: usize,
    pub(crate) pending_tree_update: Option<AccessibilityTreeUpdate>,
}

#[cfg(not(feature = "accessibility"))]
#[derive(Default)]
pub(crate) struct AccessibilityBuilder;

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
            suppression_depth: 0,
            pending_tree_update: None,
        }
    }
}

#[cfg(feature = "accessibility")]
impl AccessibilityBuilder {
    pub(crate) fn reset_scene(&mut self) {
        self.pending_tree_update = None;
        self.pending_text_input_nodes.clear();
    }

    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.nodes.clear();
        self.root_children.clear();
        self.actions.clear();
        self.next_node_id = ACCESSIBILITY_FIRST_NODE_ID;
        self.pending_text_input_nodes.clear();
        self.suppression_depth = 0;
    }

    pub(crate) fn swap_render_state(
        &mut self,
        subtree_nodes: &mut Vec<(AccessibilityNodeId, AccessibilityNode)>,
        subtree_root_children: &mut Vec<AccessibilityNodeId>,
        subtree_actions: &mut BTreeMap<AccessibilityNodeId, AccessibilityActionTarget>,
    ) {
        core::mem::swap(&mut self.nodes, subtree_nodes);
        core::mem::swap(&mut self.root_children, subtree_root_children);
        core::mem::swap(&mut self.actions, subtree_actions);
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
        let Some(state) = env.get::<AccessibilityState>() else {
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
        if attach_to_root {
            self.root_children.push(node_id);
        }
        self.nodes.push((node_id, node));
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

impl HydrolysisRenderer {
    #[cfg(feature = "accessibility")]
    pub(crate) fn next_accessibility_node_id(&mut self) -> AccessibilityNodeId {
        self.accessibility.next_node_id()
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn replay_dynamic_accessibility_subtree(
        &mut self,
        transform: vello::kurbo::Affine,
        subtree: &DynamicAccessibilitySubtree,
    ) -> AccessibilityNodeIdRemap {
        if self.accessibility.suppression_depth > 0 {
            return AccessibilityNodeIdRemap::new(self.accessibility.next_node_id);
        }

        let id_map = AccessibilityNodeIdRemap::new(self.accessibility.next_node_id);
        let mut root_child_cursor = 0usize;

        for (local_id, node) in &subtree.nodes {
            let mapped_id = self.next_accessibility_node_id();
            assert!(
                mapped_id == id_map.map(*local_id),
                "hydrolysis dynamic accessibility node ids must be replayed in local id order"
            );
            let mut mapped_node = node.clone();
            remap_accessibility_node_references(&mut mapped_node, id_map);
            if let Some(bounds) = mapped_node.bounds() {
                let bounds = transformed_rect(transform, accesskit_rect_to_kurbo_rect(bounds));
                mapped_node.set_bounds(kurbo_rect_to_accesskit_rect(bounds));
            }
            while subtree
                .root_children
                .as_slice()
                .get(root_child_cursor)
                .is_some_and(|root_child| root_child.0 < local_id.0)
            {
                root_child_cursor = root_child_cursor
                    .checked_add(1)
                    .expect("hydrolysis dynamic accessibility root child cursor overflow");
            }
            if subtree
                .root_children
                .as_slice()
                .get(root_child_cursor)
                .is_some_and(|root_child| root_child == local_id)
            {
                self.accessibility.root_children.push(mapped_id);
            }
            self.accessibility.nodes.push((mapped_id, mapped_node));
            if let Some(action_target) = subtree.actions.get(local_id) {
                self.accessibility.actions.insert(
                    mapped_id,
                    transform_accessibility_action_target(action_target, transform),
                );
            }
        }

        id_map
    }

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
        let target = self
            .accessibility
            .actions
            .get(&target_node)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "hydrolysis accessibility action {:?} targets unmapped node {:?}",
                    action, target_node
                )
            });
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
            AccessibilityActionTarget::DatePicker { value, range, ty } => {
                handle_accessibility_date_picker_action(&value, &range, ty, action, action_data)
            }
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
            AccessibilityActionTarget::PickerCycle { selection, ids } => {
                handle_accessibility_picker_cycle_action(&selection, &ids, action)
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
    pub(crate) fn register_accessibility_node(
        &mut self,
        node: AccessibilityNode,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
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
        self.accessibility
            .register_node_internal(node, bounds, env, action_target, false)
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
        let resolved = label.semantic_text().clone().resolve(env);
        let plain = resolved.content.get().to_plain();
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
pub(crate) fn register_accessibility_text_run_node(
    renderer: &mut HydrolysisRenderer,
    value: &str,
    bounds: vello::kurbo::Rect,
    env: &Environment,
) -> Option<AccessibilityNodeId> {
    let mut text_run = AccessibilityNode::new(AccessibilityNodeRole::TextRun);
    text_run.set_value(value.to_owned());
    text_run.set_character_lengths(accessibility_character_lengths(value));
    renderer.register_accessibility_child_node(text_run, bounds, env, None)
}

#[cfg(feature = "accessibility")]
fn accessibility_character_lengths(value: &str) -> Vec<u8> {
    value
        .chars()
        .map(|ch| {
            u8::try_from(ch.len_utf8())
                .expect("hydrolysis accessibility text run utf8 length must fit into u8")
        })
        .collect()
}

#[cfg(feature = "accessibility")]
pub(crate) fn collapsed_accessibility_text_selection(
    node_id: AccessibilityNodeId,
    character_index: usize,
) -> AccessibilityTextSelection {
    AccessibilityTextSelection {
        anchor: AccessibilityTextPosition {
            node: node_id,
            character_index,
        },
        focus: AccessibilityTextPosition {
            node: node_id,
            character_index,
        },
    }
}

#[cfg(feature = "accessibility")]
fn transform_accessibility_action_target(
    target: &AccessibilityActionTarget,
    transform: vello::kurbo::Affine,
) -> AccessibilityActionTarget {
    match target {
        AccessibilityActionTarget::PointerPrimaryClick { point } => {
            AccessibilityActionTarget::PointerPrimaryClick {
                point: transform * *point,
            }
        }
        AccessibilityActionTarget::Toggle { binding } => AccessibilityActionTarget::Toggle {
            binding: binding.clone(),
        },
        AccessibilityActionTarget::Slider { value, range, step } => {
            AccessibilityActionTarget::Slider {
                value: value.clone(),
                range: range.clone(),
                step: *step,
            }
        }
        AccessibilityActionTarget::Stepper { value, step, range } => {
            AccessibilityActionTarget::Stepper {
                value: value.clone(),
                step: step.clone(),
                range: range.clone(),
            }
        }
        AccessibilityActionTarget::DatePicker { value, range, ty } => {
            AccessibilityActionTarget::DatePicker {
                value: value.clone(),
                range: range.clone(),
                ty: *ty,
            }
        }
        AccessibilityActionTarget::TextField { value, line_limit } => {
            AccessibilityActionTarget::TextField {
                value: value.clone(),
                line_limit: *line_limit,
            }
        }
        AccessibilityActionTarget::SecureField { value } => {
            AccessibilityActionTarget::SecureField {
                value: value.clone(),
            }
        }
        AccessibilityActionTarget::PickerCycle { selection, ids } => {
            AccessibilityActionTarget::PickerCycle {
                selection: selection.clone(),
                ids: ids.clone(),
            }
        }
        AccessibilityActionTarget::PickerSelect { selection, target } => {
            AccessibilityActionTarget::PickerSelect {
                selection: selection.clone(),
                target: *target,
            }
        }
        AccessibilityActionTarget::Scroll { handle, axis } => AccessibilityActionTarget::Scroll {
            handle: handle.clone(),
            axis: *axis,
        },
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
        AccessibilityAction::Focus => {
            renderer.handle_pointer_down(x, y, PointerButton::Primary, env)
        }
        _ => panic!(
            "hydrolysis accessibility pointer target does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_scroll_action(
    handle: &ScrollHandle,
    axis: ScrollAxis,
    action: AccessibilityAction,
) -> bool {
    match action {
        AccessibilityAction::ScrollLeft => match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => handle.apply_scroll_delta(1.0, 0.0, true),
            ScrollAxis::Vertical => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollRight => match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => handle.apply_scroll_delta(-1.0, 0.0, true),
            ScrollAxis::Vertical => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollUp => match axis {
            ScrollAxis::Vertical | ScrollAxis::All => handle.apply_scroll_delta(0.0, 1.0, true),
            ScrollAxis::Horizontal => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollDown => match axis {
            ScrollAxis::Vertical | ScrollAxis::All => handle.apply_scroll_delta(0.0, -1.0, true),
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
fn handle_accessibility_date_picker_action(
    value: &nami::Binding<DateTime>,
    range: &RangeInclusive<DateTime>,
    ty: DatePickerType,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => true,
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
fn handle_accessibility_picker_cycle_action(
    selection: &nami::Binding<waterui_core::id::Id>,
    ids: &[waterui_core::id::Id],
    action: AccessibilityAction,
) -> bool {
    match action {
        AccessibilityAction::Click => {
            assert!(
                !(ids.is_empty()),
                "hydrolysis accessibility picker cycle requires non-empty options"
            );
            let current = selection.get();
            let index = ids.iter().position(|id| *id == current).unwrap_or_else(|| {
                panic!("hydrolysis accessibility picker selection is not present in options")
            });
            let next = ids[(index + 1) % ids.len()];
            if next == current {
                return false;
            }
            selection.set(next);
            true
        }
        AccessibilityAction::Focus => true,
        _ => panic!(
            "hydrolysis accessibility picker cycle does not support action {:?}",
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
