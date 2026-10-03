use super::*;

#[cfg(feature = "accessibility")]
use std::borrow::Cow;
#[cfg(feature = "accessibility")]
use std::collections::{BTreeSet, VecDeque};
#[cfg(feature = "accessibility")]
use std::ops::RangeInclusive;
#[cfg(feature = "accessibility")]
use waterui_backend_core::widget::InteractionFocusBinding;
#[cfg(feature = "accessibility")]
use waterui_form::picker::date::{DatePickerType, DateTime};

#[cfg(feature = "accessibility")]
#[derive(Clone)]
pub(crate) struct ScopedAccessibilityIdentifier {
    identifier: AccessibilityIdentifier,
    identity: Rc<()>,
}

#[cfg(feature = "accessibility")]
impl MetadataKey for ScopedAccessibilityIdentifier {}

#[cfg(feature = "accessibility")]
impl ScopedAccessibilityIdentifier {
    pub(crate) fn new(identifier: AccessibilityIdentifier) -> Self {
        Self {
            identifier,
            identity: Rc::new(()),
        }
    }

    fn key(&self) -> usize {
        Rc::as_ptr(&self.identity) as usize
    }

    fn value(&self) -> &AccessibilityIdentifier {
        &self.identifier
    }
}

/// The identity of one `.a11y_label()` / `.a11y_role()` scope.
///
/// Naming metadata is nearest-consumer: whichever node ends up *representing* the
/// wrapped view owns its role and label, and nothing below may say the same thing
/// again. A control registers its own node and so claims the scope; a composed
/// container has no such node and synthesizes one instead (see
/// [`HydrolysisRenderer::begin_accessibility_container`]). The claim is what tells
/// those two apart — both see the identical environment, so the container can only
/// know whether it is the view's representative by asking whether anything above it
/// already was.
#[cfg(feature = "accessibility")]
#[derive(Clone)]
pub(crate) struct ScopedAccessibilitySemantics {
    identity: Rc<()>,
    /// The `Activate` a representative silenced by this scope's claim delegates
    /// to the claiming node, with the interaction owner's hit region it was
    /// captured at. Shared across clones — `Environment` clones share
    /// their `Rc` values and [`crate::renderer::restore_a11y_naming_scope`]
    /// clones the scope itself — so a donation always lands in the slot the
    /// claimer drains.
    delegated_activation: Rc<RefCell<Option<DelegatedActivation>>>,
    /// Text a descendant donated to the claim's accessible name (see
    /// [`AccessibilityNameFromContents`]). `Some` once any leaf donated — even
    /// if the joined string is empty — so the claimer can tell "text was
    /// consumed" apart from "there were no text descendants".
    donated_text: Rc<RefCell<Option<String>>>,
}

#[cfg(feature = "accessibility")]
impl MetadataKey for ScopedAccessibilitySemantics {}

#[cfg(feature = "accessibility")]
impl ScopedAccessibilitySemantics {
    pub(crate) fn new() -> Self {
        Self {
            identity: Rc::new(()),
            delegated_activation: Rc::new(RefCell::new(None)),
            donated_text: Rc::new(RefCell::new(None)),
        }
    }

    fn key(&self) -> usize {
        Rc::as_ptr(&self.identity) as usize
    }

    /// Deposit `activation` for the node claiming this scope — called by a tap
    /// gesture whose own element the claim silences. `interaction` is the
    /// donor's hit region and effective clip in window hit-test space, the
    /// placement [`SemanticCore::accessibility_activation_point`] projects
    /// (water-rs/waterui#1323 §5); `None` on the semantic walk, which has no
    /// geometry to carry. The first donor wins: the claim stands in for the
    /// nearest actionable representative.
    pub(crate) fn delegate_activation(
        &self,
        activation: AccessibilityActivation,
        interaction: Option<NodePlacement>,
    ) {
        let mut slot = self.delegated_activation.borrow_mut();
        if slot.is_none() {
            *slot = Some(DelegatedActivation {
                activation,
                interaction,
            });
        }
    }

    /// Take the activation silenced representatives delegated, if any — the
    /// claiming node drains it after its subtree has been walked.
    pub(crate) fn take_delegated_activation(&self) -> Option<DelegatedActivation> {
        self.delegated_activation.borrow_mut().take()
    }

    /// Deposit text a descendant emitted no node for: the claim computes its
    /// accessible name from these contents (water-rs/hydrolysis#229).
    pub(crate) fn donate_text(&self, text: &str) {
        let mut slot = self.donated_text.borrow_mut();
        match slot.as_mut() {
            Some(name) => {
                if !name.is_empty() {
                    name.push(' ');
                }
                name.push_str(text);
            }
            None => *slot = Some(text.to_owned()),
        }
    }

    /// Take the text descendants donated to this claim's name — `Some` when
    /// any leaf was consumed, even if the joined string is empty.
    pub(crate) fn take_donated_text(&self) -> Option<String> {
        self.donated_text.borrow_mut().take()
    }
}

/// Marks a subtree whose text feeds the enclosing claim's accessible name.
///
/// Installed by [`accessibility_container_child_environment`] when the
/// claiming container's role computes its name from descendant content — the
/// platform names a tab after its text, so a `Label` node repeating those
/// words is read twice (water-rs/hydrolysis#229). A text leaf under the
/// marker donates its string to the claim's [`ScopedAccessibilitySemantics`]
/// instead of registering a node — unless the leaf names itself: its own role
/// or label is a nearer claim and wins. A nested claim boundary re-derives
/// the marker, so consumption always belongs to the nearest claiming element.
#[cfg(feature = "accessibility")]
#[derive(Clone)]
pub(crate) struct AccessibilityNameFromContents;

#[cfg(feature = "accessibility")]
impl MetadataKey for AccessibilityNameFromContents {}

/// Whether an element of `role` takes its accessible name from descendant
/// text — the ARIA "name from content" roles. Those containers consume their
/// text descendants; containers whose role names the grouping itself (a tab
/// list, a navigation landmark) leave them emitting `Label` nodes as before.
#[cfg(feature = "accessibility")]
fn accessibility_role_names_from_contents(role: &AccessibilityRole) -> bool {
    matches!(
        role,
        AccessibilityRole::Button
            | AccessibilityRole::Link
            | AccessibilityRole::Checkbox
            | AccessibilityRole::RadioButton
            | AccessibilityRole::Switch
            | AccessibilityRole::MenuItem
            | AccessibilityRole::MenuItemCheckbox
            | AccessibilityRole::MenuItemRadio
            | AccessibilityRole::Option
            | AccessibilityRole::Tab
            | AccessibilityRole::Header
            | AccessibilityRole::Text
    )
}

/// The `f32`-representable points inside the half-open range `[lo, hi)`, as
/// `(nearest, farthest)`.
///
/// Pointer input is delivered in `f32`, so an activation point survives only
/// if the narrowing keeps it inside the fragment: `hi` yields the largest
/// `f32` strictly below it, `lo` the smallest `f32` at or above it. `None`
/// when no `f32` lands inside — a sliver thinner than one `f32` ulp is as
/// unreachable as an empty fragment (water-rs/hydrolysis#27).
#[cfg(feature = "accessibility")]
fn f32_interior_range(lo: f64, hi: f64) -> Option<(f32, f32)> {
    let lo = match lo as f32 {
        narrowed if f64::from(narrowed) >= lo => narrowed,
        narrowed => narrowed.next_up(),
    };
    let hi = match hi as f32 {
        narrowed if f64::from(narrowed) < hi => narrowed,
        narrowed => narrowed.next_down(),
    };
    (lo <= hi).then_some((lo, hi))
}

#[cfg(feature = "accessibility")]
pub(crate) const ACCESSIBILITY_ROOT_NODE_ID: AccessibilityNodeId = AccessibilityNodeId(0);
#[cfg(feature = "accessibility")]
pub(crate) const ACCESSIBILITY_FIRST_NODE_ID: u64 = 1;

#[cfg(feature = "accessibility")]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AccessibilityNodeKey {
    owner: RetainedIdentity,
    local: AccessibilityLocalNodeKey,
}

#[cfg(feature = "accessibility")]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AccessibilityLocalNodeKey {
    Ordinal(u64),
    Semantic(i64),
}

/// A semantic activation handler: the closure a widget's accessibility node
/// runs for `Click`, independent of geometry or pointer input.
#[cfg(feature = "accessibility")]
pub(crate) type AccessibilityActivation =
    Rc<RefCell<dyn FnMut(&mut SemanticCore, &Environment) -> bool>>;

/// Where an emitted accessibility node sits in window hit-test space: its
/// logical bounds — the rectangle the tree reports — and the effective clip
/// in force when it registered.
///
/// The clip stays out of the reported bounds (water-rs/waterui#1323 §4): it
/// is kept beside the node so
/// [`SemanticCore::accessibility_activation_point`] can project the logical
/// rectangle into the fragment a pointer can actually reach.
#[cfg(feature = "accessibility")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct NodePlacement {
    pub(crate) bounds: kurbo::Rect,
    pub(crate) clip: Option<kurbo::Rect>,
}

/// An activation a silenced interaction owner donated to a claiming node,
/// with the hit region the donation was captured at.
///
/// Pointer hit-testing belongs to the interaction owner — its explicit hit
/// region, transform and clips — not to the node that ends up answering for
/// it (water-rs/waterui#1323 §5). [`SemanticCore::accessibility_activation_point`]
/// therefore projects a delegated node's point through `interaction` rather
/// than the node's logical rectangle: a `List` row straddling a clip can
/// leave a visible sliver where its silenced `on_tap` strip is already fully
/// clipped, and only the region projection tells those apart
/// (water-rs/hydrolysis#27).
#[cfg(feature = "accessibility")]
pub(crate) struct DelegatedActivation {
    pub(crate) activation: AccessibilityActivation,
    /// The donating gesture's hit region and effective clip, in window
    /// hit-test space. `None` on the semantic walk — it carries no geometry,
    /// so its nodes carry no point at all.
    pub(crate) interaction: Option<NodePlacement>,
}

#[cfg(feature = "accessibility")]
#[derive(Clone)]
pub(crate) enum AccessibilityActionTarget {
    /// Direct semantic activation: `Click` invokes the widget's own activation
    /// handler, no pointer and no coordinates.
    Activate {
        action: AccessibilityActivation,
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
        /// The popup's window anchor when the node was emitted by a rendered
        /// frame; `None` in the semantic tree, where activation mounts the
        /// same window with no placement at all.
        origin: Option<LayoutPoint>,
        /// The node's own environment — its popup opens inside it
        /// (water-rs/hydrolysis#140).
        env: Environment,
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
    /// A `List` row. `ScrollIntoView` reveals the row's measured span through
    /// the list's scroll handle; `Click` resolves the activation a pointer
    /// click on the row's centre would run — the topmost gesture recognizer
    /// or pointer target there, or, where the frame emits no pointer
    /// machinery, the innermost `Click`-advertising node inside the row;
    /// `Focus` lands on the row like on every focusable node. Arrow-key
    /// navigation between rows is built on these actions
    /// (water-rs/waterui#1223).
    ListRow {
        index: usize,
        handle: ScrollHandle,
        extents: Rc<RefCell<crate::renderer::lazy::VirtualExtentIndex>>,
        /// The row's erased collection id — the value the list's selection
        /// binding is keyed by.
        id: crate::widgets::layout::list::ListItemId,
        /// The list's row-selection state when the list is selectable: `Click`
        /// then writes the binding like a pointer click on the row, and
        /// arrow-key navigation moves the selection along with the focus
        /// (water-rs/waterui#1226).
        selection: Option<Rc<crate::widgets::layout::list::ListRowSelection>>,
    },
}

#[cfg(feature = "accessibility")]
pub(crate) struct AccessibilityBuilder {
    pub(crate) nodes: Vec<(AccessibilityNodeId, AccessibilityNode)>,
    pub(crate) root_children: Vec<AccessibilityNodeId>,
    pub(crate) actions: BTreeMap<AccessibilityNodeId, AccessibilityActionTarget>,
    /// The [`InteractionFocusBinding`] a node was emitted under, keyed by node
    /// — the same modifier state the pointer path reads for press slots, so a
    /// keyboard-focused node writes its author binding regardless of whether a
    /// pointer target exists.
    pub(crate) focus_bindings: BTreeMap<AccessibilityNodeId, InteractionFocusBinding>,
    /// The accessibility node each interaction identity emitted this frame —
    /// the pointer machinery's anchor into the semantic tree: a press resolves
    /// the node keyboard focus lands on, and a widget's focus-ring state reads
    /// the focused node back through its interaction key. Cleared at
    /// `reset_scene` so mid-flush reads only ever see this flush's links: a
    /// key whose widget emitted no node must not resolve to the id a previous
    /// flush registered — `render_list_parts` puts the result on the parent
    /// stack, where an unregistered id panics the first child attach.
    /// `finalize_tree_update` prunes links stamped for suppressed nodes.
    pub(crate) interaction_nodes: BTreeMap<InteractionKey, AccessibilityNodeId>,
    /// Activations a silenced interaction owner delegated into a node, each
    /// with the hit region it was captured at — a `List` row's `Click`
    /// dispatches the tap gesture its content silenced into the row's naming
    /// scope (water-rs/hydrolysis#27), and
    /// [`SemanticCore::accessibility_activation_point`] projects the node's
    /// point through that interaction owner's region, not the node's logical
    /// rectangle (water-rs/waterui#1323 §5). A donation never replaces the
    /// node's own action target: it rides alongside for the targets that
    /// dispatch retained actions directly.
    pub(crate) delegated_activations: BTreeMap<AccessibilityNodeId, DelegatedActivation>,
    /// The effective clip — the top of the hit clip stack — in effect when
    /// each bounded node registered, in window hit-test space. Reported node
    /// bounds stay the logical rectangle; this is how
    /// [`SemanticCore::accessibility_activation_point`] projects them onto the
    /// fragment a pointer can actually reach (water-rs/hydrolysis#27,
    /// water-rs/waterui#1323 §4).
    pub(crate) node_clips: BTreeMap<AccessibilityNodeId, kurbo::Rect>,
    /// The `OnKeyPress` scope chain each focusable interaction identity was
    /// registered under — how a bubble resolves the handlers of a focused
    /// node whose registration emitted no live target this frame (the
    /// semantic walk registers none).
    pub(crate) focus_key_handlers:
        BTreeMap<InteractionKey, Option<Rc<crate::renderer::KeyHandlerNode>>>,
    pub(crate) next_node_id: u64,
    node_ids: BTreeMap<AccessibilityNodeKey, AccessibilityNodeId>,
    active_node_keys: BTreeSet<AccessibilityNodeKey>,
    owner_ordinals: BTreeMap<RetainedIdentity, u64>,
    owner_stack: Vec<RetainedIdentity>,
    fallback_owner: Rc<()>,
    pub(crate) root_bounds: kurbo::Rect,
    pub(crate) root_label: String,
    pub(crate) focus: AccessibilityNodeId,
    pub(crate) pending_text_input_nodes: VecDeque<AccessibilityNodeId>,
    pub(crate) parent_stack: Vec<AccessibilityNodeId>,
    pub(crate) suppression_depth: usize,
    pub(crate) consumed_identifier_scopes: BTreeSet<usize>,
    /// Naming scopes ([`ScopedAccessibilitySemantics`]) already claimed by a node
    /// this flush, keyed by scope identity.
    consumed_semantics_scopes: BTreeSet<usize>,
    /// The bounds a suppressed decorative graphics leaf would have published,
    /// keyed by the innermost container node above it. A naming container
    /// whose children all turned out decorative adopts them as its own bounds —
    /// the element box rather than the frame it was stretched into — the same
    /// contract `collapse_single_child_container` keeps when a real child
    /// exists.
    suppressed_leaf_bounds: BTreeMap<AccessibilityNodeId, kurbo::Rect>,
    pub(crate) pending_tree_update: Option<AccessibilityTreeUpdate>,
}

#[cfg(feature = "accessibility")]
impl Default for AccessibilityBuilder {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            root_children: Vec::new(),
            actions: BTreeMap::new(),
            focus_bindings: BTreeMap::new(),
            interaction_nodes: BTreeMap::new(),
            delegated_activations: BTreeMap::new(),
            node_clips: BTreeMap::new(),
            focus_key_handlers: BTreeMap::new(),
            next_node_id: ACCESSIBILITY_FIRST_NODE_ID,
            node_ids: BTreeMap::new(),
            active_node_keys: BTreeSet::new(),
            owner_ordinals: BTreeMap::new(),
            owner_stack: Vec::new(),
            fallback_owner: Rc::new(()),
            root_bounds: kurbo::Rect::ZERO,
            root_label: String::from("WaterUI Window"),
            focus: ACCESSIBILITY_ROOT_NODE_ID,
            pending_text_input_nodes: VecDeque::new(),
            parent_stack: Vec::new(),
            suppression_depth: 0,
            consumed_identifier_scopes: BTreeSet::new(),
            consumed_semantics_scopes: BTreeSet::new(),
            suppressed_leaf_bounds: BTreeMap::new(),
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
    ///
    /// The focus-link maps clear here as well: every emission path resets
    /// first, so a link a subtree did not re-stamp this flush belongs to a
    /// node that is gone — keeping it would let a mid-flush lookup resolve a
    /// key to an unregistered id (the `.visible(false)` `List` row panic).
    /// Between flushes the maps hold exactly the last flush's links, which is
    /// what the event-time readers (`set_keyboard_focus`, the key-bubble
    /// handler lookup) need.
    pub(crate) fn reset_scene(&mut self) {
        self.pending_tree_update = None;
        self.nodes.clear();
        self.root_children.clear();
        self.actions.clear();
        self.focus_bindings.clear();
        self.interaction_nodes.clear();
        self.delegated_activations.clear();
        self.node_clips.clear();
        self.focus_key_handlers.clear();
        self.active_node_keys.clear();
        self.owner_ordinals.clear();
        self.owner_stack.clear();
        self.pending_text_input_nodes.clear();
        self.parent_stack.clear();
        self.suppression_depth = 0;
        self.consumed_identifier_scopes.clear();
        self.consumed_semantics_scopes.clear();
        self.suppressed_leaf_bounds.clear();
    }

    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.reset_scene();
    }

    /// The most specific node covering `point` that `include` admits, in
    /// window coordinates.
    ///
    /// Nodes are registered as the tree is walked, so a child is always pushed
    /// after the parent that contains it and the last match is the innermost
    /// one — the element a user pointing at that spot means.
    pub(crate) fn node_at_point_where(
        &self,
        point: kurbo::Point,
        include: impl Fn(&AccessibilityNode) -> bool,
    ) -> Option<AccessibilityNodeId> {
        self.nodes
            .iter()
            .rev()
            .find(|(_, node)| {
                node.bounds().is_some_and(|bounds| {
                    point.x >= bounds.x0
                        && point.x < bounds.x1
                        && point.y >= bounds.y0
                        && point.y < bounds.y1
                }) && include(node)
            })
            .map(|(id, _)| *id)
    }

    /// The most specific node covering `point`, in window coordinates.
    ///
    /// Only "inspect this element" asks, and a browser page hosts no inspector
    /// endpoint to reveal the answer in. The press path's focus lookup goes
    /// through `node_at_point_where` instead, which compiles everywhere the
    /// accessibility feature does.
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn node_at_point(&self, point: kurbo::Point) -> Option<AccessibilityNodeId> {
        self.node_at_point_where(point, |_| true)
    }

    pub(crate) fn next_node_id(&mut self) -> AccessibilityNodeId {
        let node_id = AccessibilityNodeId(self.next_node_id);
        self.next_node_id = self
            .next_node_id
            .checked_add(1)
            .expect("hydrolysis accessibility node ID overflow");
        node_id
    }

    pub(crate) fn push_owner(&mut self, owner: &Rc<()>) {
        self.owner_stack.push(RetainedIdentity::for_rc(owner));
    }

    pub(crate) fn pop_owner(&mut self) {
        self.owner_stack
            .pop()
            .expect("hydrolysis accessibility owner stack underflow");
    }

    fn stable_node_id(&mut self, semantic_key: Option<i64>) -> AccessibilityNodeId {
        let owner = self
            .owner_stack
            .last()
            .cloned()
            .unwrap_or_else(|| RetainedIdentity::for_rc(&self.fallback_owner));
        let local = semantic_key.map_or_else(
            || {
                let ordinal = self.owner_ordinals.entry(owner.clone()).or_default();
                let current = *ordinal;
                *ordinal = ordinal
                    .checked_add(1)
                    .expect("hydrolysis accessibility owner-local node index overflow");
                AccessibilityLocalNodeKey::Ordinal(current)
            },
            AccessibilityLocalNodeKey::Semantic,
        );
        let key = AccessibilityNodeKey { owner, local };
        self.active_node_keys.insert(key.clone());
        if let Some(node_id) = self.node_ids.get(&key) {
            *node_id
        } else {
            let node_id = self.next_node_id();
            self.node_ids.insert(key, node_id);
            node_id
        }
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

    pub(crate) fn apply_state(env: &Environment, node: &mut AccessibilityNode) {
        // The tree build stores accessibility state as a live signal (a static
        // state is a constant signal), so a reactive state — e.g. a filter chip's
        // selected binding — is resolved fresh on every emission.
        let Some(state) = env
            .get::<AccessibilityStateSignal>()
            .map(|signal| signal.state().snapshot())
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
            node.set_toggled(match checked {
                waterui::accessibility::AccessibilityChecked::False => AccessibilityToggled::False,
                waterui::accessibility::AccessibilityChecked::True => AccessibilityToggled::True,
                waterui::accessibility::AccessibilityChecked::Mixed => AccessibilityToggled::Mixed,
            });
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

    /// Registers `node` and returns its stable id.
    ///
    /// `placement` is the flushed hit rect and the effective clip — `Some`
    /// for the rendered runtime, `None` for the semantic runtime, whose nodes
    /// carry no geometry at all. A `Some` rect with non-positive extent is not
    /// an element and registers nothing, matching the layout-driven emission's
    /// contract.
    pub(crate) fn register_node_internal(
        &mut self,
        mut node: AccessibilityNode,
        placement: Option<NodePlacement>,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
        attach_to_root: bool,
        semantic_key: Option<i64>,
    ) -> Option<AccessibilityNodeId> {
        if self.suppression_depth > 0 {
            return None;
        }
        if placement.is_some_and(|placement| {
            placement.bounds.width() <= 0.0 || placement.bounds.height() <= 0.0
        }) {
            return None;
        }
        // This node represents the view the enclosing naming scope wraps, so it
        // owns that role and label; a container below must not emit a second node
        // repeating them.
        if let Some(scope) = env.get::<ScopedAccessibilitySemantics>() {
            self.consumed_semantics_scopes.insert(scope.key());
        }
        Self::apply_state(env, &mut node);
        node.set_text_direction(
            if waterui_core::layout::layout_direction(env)
                .snapshot()
                .is_right_to_left()
            {
                AccessibilityTextDirection::RightToLeft
            } else {
                AccessibilityTextDirection::LeftToRight
            },
        );
        // Nearest-consumer automation identifier: a leaf that already carries
        // an author id (an explicit backend decision) keeps it.
        if node.author_id().is_none()
            && let Some(scope) = env.get::<ScopedAccessibilityIdentifier>()
            && self.consumed_identifier_scopes.insert(scope.key())
        {
            node.set_author_id(scope.value().as_str().to_string());
        }
        let node_id = self.stable_node_id(semantic_key);
        if let Some(placement) = placement {
            node.set_bounds(kurbo_rect_to_accesskit_rect(placement.bounds));
            if let Some(clip) = placement.clip {
                self.node_clips.insert(node_id, clip);
            }
        }
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
        if let Some(binding) = env.get::<InteractionFocusBinding>() {
            self.focus_bindings.insert(node_id, binding.clone());
        }
        Some(node_id)
    }

    /// Whether the naming scope `env` sits in was already claimed this flush by a
    /// node representing the wrapped view.
    fn semantics_scope_is_claimed(&self, env: &Environment) -> bool {
        env.get::<ScopedAccessibilitySemantics>()
            .is_some_and(|scope| self.consumed_semantics_scopes.contains(&scope.key()))
    }

    /// Bind `node_id`'s `Click` to the donated `activation`, advertising the
    /// `Focus` and `Click` actions a tap gesture's own node would have
    /// carried.
    ///
    /// Called with the activation a silenced tap delegated to this node's
    /// naming scope — the node stands in for that gesture, so it must be no
    /// less activatable than the view it represents. A node that already has
    /// an action target keeps it — a donation never overrides a real action —
    /// but a target that dispatches retained activations (a `List` row's
    /// `Click`) still resolves the donation, so it is retained on the side.
    /// The donor's hit region is kept either way: it is the interaction
    /// owner's geometry the activation-point query projects through
    /// (water-rs/waterui#1323 §5). A disabled node advertises no actions and
    /// takes neither.
    fn attach_delegated_activation(
        &mut self,
        node_id: AccessibilityNodeId,
        donation: DelegatedActivation,
    ) {
        if self.actions.contains_key(&node_id) {
            self.delegated_activations
                .entry(node_id)
                .and_modify(|stored| {
                    // The semantic walk's donation carries no geometry and
                    // lands first; the rendered walk's carries the
                    // interaction owner's real placement. Geometry upgrades,
                    // never downgrades — the first donor's placement stands.
                    if stored.interaction.is_none() {
                        stored.interaction = donation.interaction;
                    }
                })
                .or_insert(donation);
            return;
        }
        let Some(node) = self
            .nodes
            .iter_mut()
            .find_map(|(id, node)| (*id == node_id).then_some(node))
        else {
            return;
        };
        if node.is_disabled() {
            return;
        }
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        self.actions.insert(
            node_id,
            AccessibilityActionTarget::Activate {
                action: Rc::clone(&donation.activation),
            },
        );
        self.delegated_activations.insert(node_id, donation);
    }

    /// Name `node_id` from the descendant text its claim consumed (see
    /// [`SemanticCore::consume_accessibility_descendant_text`]), when the
    /// claim carries no explicit label. An empty donation names nothing.
    fn name_claim_from_contents(&mut self, node_id: AccessibilityNodeId, name: String) {
        if name.is_empty() {
            return;
        }
        if let Some((_, node)) = self.nodes.iter_mut().find(|(id, _)| *id == node_id)
            && node.label().is_none()
        {
            node.set_label(name);
        }
    }

    /// Collapses a synthesized naming container around exactly one semantic
    /// node into that node.
    ///
    /// `.a11y_label(..)` on a view with a single accessibility element means
    /// "this is that element's name" on every platform — `SwiftUI`'s
    /// `accessibilityLabel` overrides the element rather than wrapping it — so
    /// a padding or frame between the metadata and a lone text must not turn
    /// the override into a `Group("name")` around a `Label(content)`. The child
    /// keeps its identity and its actions; its bounds become the naming
    /// container's resolved extent when one was recorded at layout — the frame
    /// the parent assigned to the labelled view routinely exceeds it under the
    /// placement contract (a window's overlay places its base over the whole
    /// bounds, so a root `view.size(8, 8)` is laid out in the window while the
    /// element sits in the resolved 8x8 box). The child takes the scope's
    /// label, the scope's automation id, and the scope's explicit role. With
    /// zero or several children the container stands: it is then the only node
    /// that can say the parts belong together.
    fn collapse_single_child_container(
        &mut self,
        container_id: AccessibilityNodeId,
        resolved_bounds: Option<kurbo::Rect>,
    ) {
        let container_index = self
            .nodes
            .iter()
            .position(|(id, _)| *id == container_id)
            .expect("hydrolysis accessibility container to collapse is not registered");
        let container = &self.nodes[container_index].1;
        let [child_id] = *container.children() else {
            return;
        };
        // A container carrying semantic state of its own is a distinct
        // element, not a naming wrapper: dissolving it into the child would
        // drop the state and overwrite the child's own label and role.
        if container.is_expanded().is_some()
            || container.is_selected().is_some()
            || container.toggled().is_some()
            || container.is_disabled()
            || container.is_busy()
        {
            return;
        }
        let label = container.label().map(str::to_owned);
        let author_id = container.author_id().map(str::to_owned);
        let role = container.role();
        let click = container.supports_action(AccessibilityAction::Click);
        let focus = container.supports_action(AccessibilityAction::Focus);
        let child = self
            .nodes
            .iter_mut()
            .find_map(|(id, node)| (*id == child_id).then_some(node))
            .expect("hydrolysis accessibility container child is not registered");
        if let Some(label) = label {
            child.set_label(label);
        }
        // The container's advertised actions and action target — the activation
        // a silenced tap delegated to it — move to the surviving node; a real
        // action the child registered itself always wins.
        if click {
            child.add_action(AccessibilityAction::Click);
        }
        if focus {
            child.add_action(AccessibilityAction::Focus);
        }
        if let Some(target) = self.actions.remove(&container_id) {
            self.actions.entry(child_id).or_insert(target);
        }
        // A delegated activation rides with its target: the surviving child
        // dispatches it and projects its point through the donor's region.
        if let Some(donation) = self.delegated_activations.remove(&container_id) {
            self.delegated_activations
                .entry(child_id)
                .or_insert(donation);
        }
        // The scope's automation id was claimed by the container, so it would
        // vanish with it. A child that carries its own id keeps it, matching
        // the nearest-consumer rule that let it claim one in the first place.
        if let Some(author_id) = author_id
            && child.author_id().is_none()
        {
            child.set_author_id(author_id);
        }
        if role != AccessibilityNodeRole::Group {
            child.set_role(role);
        }
        // The element reports the scope owner's resolved extent — the size the
        // container answered to the placement proposal, centred on the
        // assigned frame — not the assigned frame nor the child's own
        // placement (water-rs/hydrolysis#51).
        if let Some(resolved) = resolved_bounds
            && resolved.width() > 0.0
            && resolved.height() > 0.0
        {
            child.set_bounds(kurbo_rect_to_accesskit_rect(resolved));
        }
        // The child takes the container's place under its parent.
        if let Some(slot) = self
            .root_children
            .iter_mut()
            .find(|slot| **slot == container_id)
        {
            *slot = child_id;
        } else {
            let slot = self
                .nodes
                .iter_mut()
                .filter(|(id, _)| *id != container_id)
                .find_map(|(_, node)| {
                    node.children()
                        .iter()
                        .position(|child| *child == container_id)
                        .map(|position| (node, position))
                });
            let (parent, position) =
                slot.expect("hydrolysis accessibility container to collapse has no parent");
            let mut children = parent.children().to_vec();
            children[position] = child_id;
            parent.set_children(children);
        }
        self.nodes.remove(container_index);
        if self.focus == container_id {
            self.focus = child_id;
        }
    }

    /// The update `finalize_tree_update` publishes: the synthesized window
    /// root over the registered nodes, with the current focus. Factored so
    /// the merged multi-window path can re-emit a quiet window's last state
    /// from the live registry instead of cloning a stored update.
    fn assembled_tree_update(&self) -> AccessibilityTreeUpdate {
        let mut root = AccessibilityNode::new(AccessibilityNodeRole::Window);
        root.set_label(self.root_label.clone());
        if self.root_bounds.width() > 0.0 && self.root_bounds.height() > 0.0 {
            root.set_bounds(kurbo_rect_to_accesskit_rect(self.root_bounds));
        }
        root.set_children(self.root_children.clone());
        let mut nodes = Vec::with_capacity(self.nodes.len() + 1);
        nodes.push((ACCESSIBILITY_ROOT_NODE_ID, root));
        nodes.extend(self.nodes.iter().cloned());
        AccessibilityTreeUpdate {
            nodes,
            tree: Some(AccessibilityTree::new(ACCESSIBILITY_ROOT_NODE_ID)),
            tree_id: AccessibilityTreeId::ROOT,
            focus: self.focus,
        }
    }

    pub(crate) fn finalize_tree_update(&mut self) {
        self.node_ids
            .retain(|key, _| self.active_node_keys.contains(key));
        if !self.nodes.iter().any(|(id, _)| *id == self.focus) {
            self.focus = ACCESSIBILITY_ROOT_NODE_ID;
        }
        let live = self
            .nodes
            .iter()
            .map(|(id, _)| *id)
            .collect::<BTreeSet<_>>();
        self.interaction_nodes.retain(|_, node| live.contains(node));
        let interaction_nodes = &self.interaction_nodes;
        self.focus_key_handlers
            .retain(|key, _| interaction_nodes.contains_key(key));
        self.pending_tree_update = Some(self.assembled_tree_update());
    }
}

#[cfg(feature = "accessibility")]
pub(crate) struct AccessibilityContainerScope {
    parent_pushed: bool,
    suppression_pushed: bool,
    /// The node this scope synthesized for the container, when it did.
    container_node: Option<AccessibilityNodeId>,
    /// The extent the container resolved at layout — the size it answered to
    /// the placement proposal, centred on the assigned frame — kept so the
    /// element the scope collapses onto reports it instead of the assigned
    /// frame or the child's (water-rs/hydrolysis#51).
    resolved_bounds: Option<kurbo::Rect>,
    /// The naming scope that node's registration claimed — the channel
    /// silenced representatives (a tap gesture whose own node was suppressed)
    /// delegate their activation through, drained when the scope ends.
    naming_scope: Option<ScopedAccessibilitySemantics>,
}

#[cfg(feature = "accessibility")]
impl AccessibilityContainerScope {
    /// A scope that emitted no node and suppressed nothing: the container is not
    /// this view's representative, so it only shields its children.
    const INERT: Self = Self {
        parent_pushed: false,
        suppression_pushed: false,
        container_node: None,
        resolved_bounds: None,
        naming_scope: None,
    };
}

/// The environment a container hands its children when the container itself
/// carries accessibility naming metadata.
///
/// `Some` means the container is a candidate for its own node: the semantics name
/// the container, not each leaf inside it, so they are stripped from the child
/// environment. Without that strip a `.a11y_label("Navigation")` on a bar reached
/// every tab under it and the tree announced "Navigation" once per tab instead of
/// once for the bar.
#[cfg(feature = "accessibility")]
pub(crate) fn accessibility_container_child_environment(env: &Environment) -> Option<Environment> {
    // A role or a label names the container. Identifier/hidden/state metadata are
    // subtree-scoped and name nothing on their own, so they never make a bare
    // container into a semantic node.
    if env.get::<AccessibilityRole>().is_none() && env.get::<AccessibilityLabel>().is_none() {
        return None;
    }

    let mut child_env = env.clone();
    child_env.remove::<ScopedAccessibilityIdentifier>();
    child_env.remove::<AccessibilityLabel>();
    child_env.remove::<AccessibilityRole>();
    child_env.remove::<AccessibilityHidden>();
    child_env.remove::<AccessibilityChildren>();
    child_env.remove::<AccessibilityState>();
    child_env.remove::<AccessibilityStateSignal>();
    // Every claim boundary re-derives text consumption: an enclosing
    // consuming claim's marker is dropped here so a nested claim decides for
    // its own subtree — the nearer claim owns the text.
    child_env.remove::<AccessibilityNameFromContents>();
    if env
        .get::<AccessibilityRole>()
        .is_some_and(accessibility_role_names_from_contents)
    {
        child_env.insert(AccessibilityNameFromContents);
    }
    // `ScopedAccessibilitySemantics` deliberately stays: it no longer names
    // anything — the claim consumed it — but it is still the claim's identity,
    // which is how a silenced representative inside (a tap gesture) sees the
    // scope is spoken for and delegates its activation to the claiming node.
    Some(child_env)
}

impl SemanticCore {
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

    /// The tree this window would publish if it emitted now: the last
    /// emitted node set with the current focus, rebuilt from the live
    /// registry. A clean frame has nothing new to say, but the merged
    /// multi-window update must still describe the window — a sibling popup
    /// emitting alone would otherwise drop it from the host's tree (the main
    /// root's children list is republished whole each merge).
    #[cfg(feature = "accessibility")]
    fn current_accessibility_tree_update(&self) -> Option<AccessibilityTreeUpdate> {
        (!self.accessibility.nodes.is_empty()).then(|| self.accessibility.assembled_tree_update())
    }

    /// The accessibility tree of every open window, merged into one update —
    /// or `None` when no window emitted this frame.
    ///
    /// Publishing is driven by pending updates: the merged update exists
    /// exactly when at least one core — the main window or any popup — has
    /// one. A core that emitted contributes its pending update; a clean core
    /// re-emits its live registry so the merged tree still describes it — a
    /// popup publishing alone would otherwise drop the main window from the
    /// host's tree (the main root's children list is republished whole each
    /// merge). Each popup's ids shift into a per-window range (node ids are
    /// unique per core), and each popup root attaches to the main root's
    /// children so the merged tree stays one tree. Actions addressed at a
    /// shifted id demultiplex back to the owning window's core by the same
    /// stride — see the runtime's `perform_accessibility_action`.
    ///
    /// A window contributes nothing only when it has never emitted: there is
    /// no published tree to describe.
    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn take_merged_accessibility_tree_update<'a>(
        &mut self,
        popups: impl IntoIterator<Item = &'a mut SemanticCore>,
    ) -> Option<AccessibilityTreeUpdate> {
        self.merged_accessibility_tree_update(popups, false, |core| {
            core.take_accessibility_tree_update()
        })
    }

    /// The merged tree as of now — a read-only query: it consumes and clears
    /// nothing, so pending per-window updates stay pending for the next pump
    /// to publish. Each window contributes its pending update when one is
    /// held, else the live registry a publish would describe; popups get the
    /// same id stride the published merge assigns. `None` when the main
    /// window has never produced a tree — popups merge into it, so without
    /// it there is nothing to return.
    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn accessibility_tree<'a>(
        &mut self,
        popups: impl IntoIterator<Item = &'a mut SemanticCore>,
    ) -> Option<AccessibilityTreeUpdate> {
        self.merged_accessibility_tree_update(popups, true, |core| {
            core.peek_accessibility_tree_update().cloned()
        })
    }

    /// One merge behind the publish and read paths. `pending` yields a
    /// window's pending update (taken when publishing, peeked when reading).
    /// Publishing emits only when some window had one; `when_quiet` makes the
    /// read path describe the tree regardless. A window without a pending
    /// update contributes its live registry, rebuilt only once the merge is
    /// known to emit, so a quiet publish costs no tree construction.
    #[cfg(feature = "accessibility")]
    fn merged_accessibility_tree_update<'a>(
        &mut self,
        popups: impl IntoIterator<Item = &'a mut SemanticCore>,
        when_quiet: bool,
        mut pending: impl FnMut(&mut SemanticCore) -> Option<AccessibilityTreeUpdate>,
    ) -> Option<AccessibilityTreeUpdate> {
        use accesskit::NodeId as AccessibilityNodeId;

        /// Node ids are unique per core, so each window gets its own range.
        /// The action-dispatch side indexes popups by `id / STRIDE - 1`, so
        /// this stride is part of the merged tree's contract.
        const WINDOW_ID_STRIDE: u64 = 1 << 32;
        const ROOT: AccessibilityNodeId = AccessibilityNodeId(0);

        let main_pending = pending(self);
        let popups: Vec<(&mut SemanticCore, Option<AccessibilityTreeUpdate>)> = popups
            .into_iter()
            .map(|popup| {
                let update = pending(popup);
                (popup, update)
            })
            .collect();
        if !when_quiet
            && main_pending.is_none()
            && popups.iter().all(|(_, pending)| pending.is_none())
        {
            return None;
        }

        let mut merged = main_pending.or_else(|| self.current_accessibility_tree_update())?;
        if popups.is_empty() {
            return Some(merged);
        }

        let mut root_children = merged
            .nodes
            .iter()
            .find(|(id, _)| *id == ROOT)
            .map_or_else(Vec::new, |(_, node)| node.children().to_vec());

        // The deepest popup holding real (non-root) focus owns the merged
        // tree's focus — iterating in z-order, the last such popup wins.
        // Every other case leaves the merged focus on the main tree.
        let mut focused_popup = None;
        for (index, (popup, pending)) in popups.into_iter().enumerate() {
            let Some(update) = pending.or_else(|| popup.current_accessibility_tree_update()) else {
                continue;
            };
            let offset = (index as u64 + 1) * WINDOW_ID_STRIDE;
            if update.focus != ROOT {
                focused_popup = Some(AccessibilityNodeId(update.focus.0 + offset));
            }
            for (id, mut node) in update.nodes {
                let children: Vec<_> = node
                    .children()
                    .iter()
                    .map(|child| AccessibilityNodeId(child.0 + offset))
                    .collect();
                node.set_children(children);
                let shifted = AccessibilityNodeId(id.0 + offset);
                if id == ROOT {
                    root_children.push(shifted);
                }
                merged.nodes.push((shifted, node));
            }
        }

        if let Some((_, root)) = merged.nodes.iter_mut().find(|(id, _)| *id == ROOT) {
            root.set_children(root_children);
        }
        if let Some(focus) = focused_popup {
            merged.focus = focus;
        }
        Some(merged)
    }

    /// Borrows the pending tree update without consuming it.
    ///
    /// The platform accessibility bridge is the update's real consumer; an
    /// observer such as the inspector must not take it out from under them.
    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn peek_accessibility_tree_update(&self) -> Option<&AccessibilityTreeUpdate> {
        self.accessibility.pending_tree_update.as_ref()
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
                AccessibilityAction::Focus => self.set_keyboard_focus_node(None, false),
                AccessibilityAction::Click => false,
                _ => panic!("hydrolysis accessibility root does not support action {action:?}"),
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
                        "hydrolysis accessibility action {action:?} targets unknown node {target_node:?}"
                    )
                });
            if action == AccessibilityAction::Focus
                && node.supports_action(AccessibilityAction::Focus)
            {
                return self.set_keyboard_focus_node(Some(target_node), false);
            }
            assert!(
                !node.supports_action(action),
                "hydrolysis accessibility node {target_node:?} advertises action {action:?} but registered no action target"
            );
            return false;
        };
        let changed = match target {
            AccessibilityActionTarget::Activate { action: activation } => match action {
                AccessibilityAction::Click => (activation.borrow_mut())(self, env),
                AccessibilityAction::Focus => true,
                _ => {
                    panic!("hydrolysis accessibility activation does not support action {action:?}")
                }
            },
            AccessibilityActionTarget::Toggle { binding } => match action {
                AccessibilityAction::Click => {
                    let next = !binding.snapshot();
                    binding.set(next);
                    true
                }
                AccessibilityAction::Focus => true,
                _ => panic!("hydrolysis accessibility toggle does not support action {action:?}"),
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
                env: picker_env,
            } => handle_accessibility_date_picker_action(
                self,
                &value,
                &range,
                ty,
                origin,
                &picker_env,
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
            AccessibilityActionTarget::ListRow {
                index,
                handle,
                extents,
                id,
                selection,
            } => match action {
                AccessibilityAction::Focus => {
                    Self::scroll_list_row_into_view(index, &handle, &extents);
                    true
                }
                AccessibilityAction::Click => {
                    // The row's activation resolves exactly as a click on its
                    // centre would, then the Select action writes the binding
                    // like the row's own press slot — plain, toggle and range
                    // semantics ride the held modifiers.
                    let clicked = self.click_list_row(target_node, env);

                    if let Some(selection) = selection {
                        selection.write(index, id, self.hit_test.modifiers);
                        true
                    } else {
                        clicked
                    }
                }
                AccessibilityAction::ScrollIntoView => {
                    Self::scroll_list_row_into_view(index, &handle, &extents)
                }
                _ => panic!("hydrolysis accessibility list row does not support action {action:?}"),
            },
        };
        if changed && focus_action {
            self.set_keyboard_focus_node(Some(target_node), false);
        }
        changed
    }

    /// Links the widget's interaction identity to the accessibility node it
    /// just emitted. Pointer paths hold interaction keys, not node ids — this
    /// map is how a pointer press resolves the node keyboard focus lands on,
    /// and how a bound widget reads the focused node back through its own key.
    /// The map exists only where the semantic tree does — under the
    /// accessibility feature.
    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_focus_link(
        &mut self,
        key: &crate::renderer::InteractionKey,
        node_id: AccessibilityNodeId,
    ) {
        let chain = self.snapshot_key_handlers();
        self.accessibility
            .focus_key_handlers
            .insert(key.clone(), chain);
        self.accessibility
            .interaction_nodes
            .insert(key.clone(), node_id);
    }

    /// `Click` on a `List` row dispatches the activation the row's content
    /// retained for it — Enter/Space on a focused row lands here, so a tap
    /// target, an inner button, or nothing at all behave exactly as they do
    /// under the pointer (water-rs/waterui#1223).
    ///
    /// No pointer press is synthesized at the bounds' centre: a row
    /// straddling a clip has its centre where no pointer can hit, and the
    /// synthesized press dead-clicked on just such a row
    /// (water-rs/hydrolysis#27, water-rs/waterui#1323 §5). The row's silenced
    /// tap gesture delegates its activation into the row's naming scope at
    /// flush, which the row drains here. A row with no retained tap resolves
    /// the node a pointer at its centre would hit in the accessibility
    /// tree's own terms — the innermost descendant advertising `Click`.
    #[cfg(feature = "accessibility")]
    pub(crate) fn click_list_row(
        &mut self,
        row_node: AccessibilityNodeId,
        env: &Environment,
    ) -> bool {
        if let Some(activation) = self
            .accessibility
            .delegated_activations
            .get(&row_node)
            .map(|donation| Rc::clone(&donation.activation))
        {
            return (activation.borrow_mut())(self, env);
        }
        if let Some(dest) = self.clickable_descendant(row_node) {
            return self.handle_accessibility_action(
                AccessibilityActionRequest {
                    action: AccessibilityAction::Click,
                    target_node: dest,
                    target_tree: AccessibilityTreeId::ROOT,
                    data: None,
                },
                env,
            );
        }
        false
    }

    /// The node a pointer at the `List` row's centre would hit, in the
    /// accessibility tree's own terms: the innermost descendant of
    /// `row_node` advertising `Click` — nodes emit parent-first, so the
    /// last matching node emitted is the innermost. `None` on a row with
    /// nothing clickable inside.
    #[cfg(feature = "accessibility")]
    fn clickable_descendant(&self, row_node: AccessibilityNodeId) -> Option<AccessibilityNodeId> {
        let mut inside: std::collections::BTreeSet<AccessibilityNodeId> =
            std::collections::BTreeSet::new();
        let mut stack: Vec<AccessibilityNodeId> = self
            .accessibility
            .nodes
            .iter()
            .find_map(|(id, node)| (*id == row_node).then(|| node.children().to_vec()))
            .unwrap_or_default();
        while let Some(id) = stack.pop() {
            if inside.insert(id)
                && let Some((_, node)) = self
                    .accessibility
                    .nodes
                    .iter()
                    .find(|(other, _)| *other == id)
            {
                stack.extend_from_slice(node.children());
            }
        }
        self.accessibility
            .nodes
            .iter()
            .rev()
            .find(|(id, node)| {
                inside.contains(id) && node.supports_action(AccessibilityAction::Click)
            })
            .map(|(id, _)| *id)
    }

    /// Resolves the point a pointer could actually reach inside `node_id`'s
    /// accessibility bounds.
    ///
    /// A node's reported bounds are its logical rectangle — visibility is a
    /// projection concern resolved here, at the point of use
    /// (water-rs/waterui#1323 §4). What projects is the region a pointer can
    /// activate: for a node whose `Click` was delegated by a silenced
    /// interaction owner (a `List` row standing in for its `on_tap` strip),
    /// that owner's own hit region and clip; otherwise the node's logical
    /// rectangle intersected with the clip chain in effect when it
    /// registered (the same [`HitTestState::hit_clip_stack`] data the pointer
    /// path clips hit regions with, water-rs/hydrolysis#252) — either way,
    /// intersected with the window bounds (water-rs/waterui#1323 §5). The
    /// callers that must produce a real point — a testing `tap_at`, an
    /// automation `pointer tap` — resolve through this query instead of the
    /// bounds' centre, which may sit inside a clipped region where nothing
    /// can be hit (water-rs/hydrolysis#27).
    ///
    /// `(x_fraction, y_fraction)` pick a spot inside the projected region —
    /// `0.5, 0.5` is its centre; the result is clamped into the visible
    /// fragment, so a spot clipped away lands on the nearest point a pointer
    /// can reach. An element with no visible fragment fails loudly with
    /// [`AccessibilityActivationPointError::EmptyFragment`] — an off-screen
    /// point is never returned.
    #[cfg(feature = "accessibility")]
    pub fn accessibility_activation_point(
        &self,
        node_id: AccessibilityNodeId,
        x_fraction: f64,
        y_fraction: f64,
    ) -> Result<kurbo::Point, AccessibilityActivationPointError> {
        let Some((_, node)) = self
            .accessibility
            .nodes
            .iter()
            .find(|(id, _)| *id == node_id)
        else {
            return Err(AccessibilityActivationPointError::NoNode);
        };
        // A node whose activation was delegated resolves against the
        // interaction owner's hit region — the gesture's own bounds and
        // effective clip — not the node's logical rectangle: a `List` row's
        // sliver can outlive the silenced `on_tap` strip that answers for it
        // (water-rs/waterui#1323 §5). Otherwise the node's own placement
        // stands in: the tree it registered is what a pointer sees.
        let (bounds, clip) = match self
            .accessibility
            .delegated_activations
            .get(&node_id)
            .and_then(|donation| donation.interaction)
        {
            Some(placement) => (placement.bounds, placement.clip),
            None => {
                let Some(bounds) = node.bounds() else {
                    // The semantic walk emits nodes with no geometry at all —
                    // there is no rectangle to project.
                    return Err(AccessibilityActivationPointError::NoBounds);
                };
                (
                    kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1),
                    self.accessibility.node_clips.get(&node_id).copied(),
                )
            }
        };
        let mut fragment = bounds;
        if let Some(clip) = clip {
            fragment = fragment.intersect(clip);
        }
        fragment = fragment.intersect(self.hit_test.window_bounds);
        if fragment.width() <= 0.0 || fragment.height() <= 0.0 {
            return Err(AccessibilityActivationPointError::EmptyFragment);
        }
        // Hit-test space is half-open and pointer input arrives in `f32`, so
        // the point must come from the range an `f32` event can still land
        // inside: clamping to `x1 - ε` in `f64` narrows back onto the excluded
        // edge whenever ε is below the `f32` ulp at that magnitude.
        let Some((x0, x1)) = f32_interior_range(fragment.x0, fragment.x1) else {
            return Err(AccessibilityActivationPointError::EmptyFragment);
        };
        let Some((y0, y1)) = f32_interior_range(fragment.y0, fragment.y1) else {
            return Err(AccessibilityActivationPointError::EmptyFragment);
        };
        let requested = kurbo::Point::new(
            bounds.x0 + bounds.width() * x_fraction,
            bounds.y0 + bounds.height() * y_fraction,
        );
        Ok(kurbo::Point::new(
            f64::from((requested.x as f32).clamp(x0, x1)),
            f64::from((requested.y as f32).clamp(y0, y1)),
        ))
    }

    /// Reveal row `index` of the list `handle` scrolls: the minimum scroll
    /// that puts the row's measured span inside the viewport, the same
    /// reveal `ScrollIntoView` performs on any scrollable container. The row
    /// shares the list's extent index, so semantic lists reveal in row
    /// units and rendered lists in pixels. Reports the action handled.
    #[cfg(feature = "accessibility")]
    pub(crate) fn scroll_list_row_into_view(
        index: usize,
        handle: &ScrollHandle,
        extents: &Rc<RefCell<crate::renderer::lazy::VirtualExtentIndex>>,
    ) -> bool {
        let metrics = handle.metrics();
        let extents = extents.borrow();
        let row_start = extents.offset_of(index);
        let row_end = extents.offset_of(index + 1);
        let viewport_end = metrics.offset_y + metrics.viewport_height;
        let target = if row_start < metrics.offset_y {
            row_start
        } else if row_end > viewport_end {
            (row_end - metrics.viewport_height).min(row_start)
        } else {
            return true;
        };
        let _ = handle.scroll_to(metrics.offset_x, target);
        true
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

    /// Parents the nodes registered until the matching pop to `node_id`.
    ///
    /// A scroll region's content are its descendants on every platform, so a
    /// widget that already registered its own node uses this to gather what it
    /// contains, rather than letting that content land beside it.
    #[cfg(feature = "accessibility")]
    pub(crate) fn push_accessibility_parent(&mut self, node_id: AccessibilityNodeId) {
        self.accessibility.parent_stack.push(node_id);
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn pop_accessibility_parent(&mut self) {
        self.accessibility
            .parent_stack
            .pop()
            .expect("hydrolysis accessibility parent stack underflow");
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn push_accessibility_owner(&mut self, owner: &Rc<()>) {
        self.accessibility.push_owner(owner);
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn pop_accessibility_owner(&mut self) {
        self.accessibility.pop_owner();
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn pop_accessibility_suppression(&mut self) {
        self.accessibility.pop_suppression();
    }

    /// Open the accessibility scope of a container view that carries naming
    /// metadata, emitting the node that represents the container itself.
    ///
    /// A composed container — a navigation bar's stack, a card, a toolbar — has no
    /// leaf standing for the whole, so `.a11y_role(TabList)` / `.a11y_label(..)` on
    /// it would otherwise reach only the leaves inside and the container would
    /// announce nothing. Assistive technology then cannot tell that the tabs belong
    /// together, which is exactly what a tab list exists to say.
    ///
    /// The caller must have determined the container carries those semantics
    /// ([`accessibility_container_child_environment`] returned `Some`), and must
    /// flush its children under that returned environment. `resolved_bounds`
    /// is the extent the container resolved at layout — the size it answered
    /// to the placement proposal, centred on the assigned frame — the
    /// bounds the element reports when the scope collapses onto its single
    /// semantic child (water-rs/hydrolysis#51).
    #[cfg(feature = "accessibility")]
    pub(crate) fn begin_accessibility_container(
        &mut self,
        bounds: kurbo::Rect,
        resolved_bounds: Option<kurbo::Rect>,
        env: &Environment,
    ) -> AccessibilityContainerScope {
        self.begin_accessibility_container_inner(Some(bounds), resolved_bounds, env)
    }

    /// The semantic counterpart of [`Self::begin_accessibility_container`]:
    /// the same scope logic with no rect — a semantic container has no
    /// zero-extent case to suppress, and its node carries no bounds.
    #[cfg(feature = "accessibility")]
    pub(crate) fn begin_accessibility_container_semantic(
        &mut self,
        env: &Environment,
    ) -> AccessibilityContainerScope {
        self.begin_accessibility_container_inner(None, None, env)
    }

    #[cfg(feature = "accessibility")]
    fn begin_accessibility_container_inner(
        &mut self,
        bounds: Option<kurbo::Rect>,
        resolved_bounds: Option<kurbo::Rect>,
        env: &Environment,
    ) -> AccessibilityContainerScope {
        debug_assert!(
            accessibility_container_child_environment(env).is_some(),
            "hydrolysis accessibility container scope requires a role or a label"
        );

        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            self.push_accessibility_suppression();
            return AccessibilityContainerScope {
                parent_pushed: false,
                suppression_pushed: true,
                container_node: None,
                resolved_bounds: None,
                naming_scope: None,
            };
        }

        // A control that already registered its own node under this scope is the
        // view's representative and owns the role and label. The containers
        // composing its chrome sit below and must stay silent, or every button
        // would answer to its own name twice.
        if self.accessibility.semantics_scope_is_claimed(env) {
            return AccessibilityContainerScope::INERT;
        }

        let mut node = AccessibilityNode::new(Self::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::Group,
        ));
        if let Some(label) = self.resolve_accessibility_label(env, None) {
            node.set_label(label);
        }
        let state_hidden = env
            .get::<AccessibilityStateSignal>()
            .is_some_and(|signal| signal.state().snapshot().is_hidden());
        let excludes_descendants = env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants);
        if bounds.is_some_and(|bounds| bounds.width() <= 0.0 || bounds.height() <= 0.0) {
            self.push_accessibility_suppression();
            return AccessibilityContainerScope {
                parent_pushed: false,
                suppression_pushed: true,
                container_node: None,
                resolved_bounds: None,
                naming_scope: None,
            };
        }
        self.watch_accessibility_state(env);
        let placement = bounds.map(|bounds| NodePlacement {
            bounds,
            clip: self.hit_test.hit_clip_stack.last().copied(),
        });
        let Some(node_id) = self
            .accessibility
            .register_node_internal(node, placement, env, None, true, None)
        else {
            // Bounds are positive (or semantic `None`) and the scope is unclaimed,
            // so registration can only have declined because the whole subtree is
            // suppressed — where the children emit nothing either, leaving no
            // node to parent them to.
            assert!(
                self.accessibility.suppression_depth > 0,
                "hydrolysis accessibility container at positive bounds must register a node"
            );
            return AccessibilityContainerScope::INERT;
        };
        self.accessibility.parent_stack.push(node_id);
        let suppression_pushed = state_hidden || excludes_descendants;
        if suppression_pushed {
            self.push_accessibility_suppression();
        }
        AccessibilityContainerScope {
            parent_pushed: true,
            suppression_pushed,
            container_node: Some(node_id),
            resolved_bounds,
            naming_scope: env.get::<ScopedAccessibilitySemantics>().cloned(),
        }
    }

    /// Whether the naming scope `env` sits in was already claimed this flush —
    /// for gesture observers, which delegate their activation to the claimer
    /// instead of registering a silenced second element.
    #[cfg(feature = "accessibility")]
    pub(crate) fn accessibility_scope_is_claimed(&self, env: &Environment) -> bool {
        self.accessibility.semantics_scope_is_claimed(env)
    }

    /// Consume `text` into the enclosing name-from-contents claim, if `env`
    /// sits under one this leaf doesn't name itself. Returns true when the
    /// leaf must emit no node of its own — its string is already part of the
    /// claiming element's name, so a `Label` node for it is read twice
    /// (water-rs/hydrolysis#229).
    ///
    /// A leaf carrying its own role or label is a nearer claim and keeps its
    /// node; a suppressed subtree (hidden, `ExcludeDescendants`, or a widget's
    /// own merged label) names nothing and donates nothing.
    #[cfg(feature = "accessibility")]
    pub(crate) fn consume_accessibility_descendant_text(
        &self,
        env: &Environment,
        text: &str,
    ) -> bool {
        if env.get::<AccessibilityNameFromContents>().is_none()
            || env.get::<AccessibilityRole>().is_some()
            || env.get::<AccessibilityLabel>().is_some()
            || self.accessibility.suppression_depth > 0
        {
            return false;
        }
        let Some(scope) = env.get::<ScopedAccessibilitySemantics>() else {
            return false;
        };
        scope.donate_text(text);
        true
    }

    /// Drain every channel `scope` collected from the subtree that produced it
    /// — the activation a silenced representative delegated (see
    /// [`ScopedAccessibilitySemantics::delegate_activation`]) and the text
    /// name-from-contents descendants donated (see
    /// [`SemanticCore::consume_accessibility_descendant_text`]) — onto
    /// `node_id`, the node that claimed the scope. Called once the claimer's
    /// subtree has been walked, when every donation has already landed.
    ///
    /// Returns whether descendant text was consumed (`Some` was donated — even
    /// an empty join means the claim held silenced text descendants), so the
    /// caller can tell a composite apart from a wrapper around a single child.
    #[cfg(feature = "accessibility")]
    fn drain_scope_donations(
        &mut self,
        scope: &ScopedAccessibilitySemantics,
        node_id: AccessibilityNodeId,
    ) -> bool {
        if let Some(activation) = scope.take_delegated_activation() {
            self.accessibility
                .attach_delegated_activation(node_id, activation);
        }
        match scope.take_donated_text() {
            Some(name) => {
                self.accessibility.name_claim_from_contents(node_id, name);
                true
            }
            None => false,
        }
    }

    /// [`SemanticCore::drain_scope_donations`] for a claim reached through
    /// `env` rather than a container's stored naming scope — a tap gesture's
    /// own node drains once its content has been rendered.
    #[cfg(feature = "accessibility")]
    pub(crate) fn drain_claim_scope(
        &mut self,
        node_id: AccessibilityNodeId,
        env: &Environment,
    ) -> bool {
        let Some(scope) = env.get::<ScopedAccessibilitySemantics>() else {
            return false;
        };
        self.drain_scope_donations(scope, node_id)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn end_accessibility_container(&mut self, scope: AccessibilityContainerScope) {
        if scope.suppression_pushed {
            self.pop_accessibility_suppression();
        }
        if scope.parent_pushed {
            self.accessibility
                .parent_stack
                .pop()
                .expect("hydrolysis accessibility container parent stack underflow");
        }
        if let Some(container_id) = scope.container_node {
            // Donations land in the claim's scope during the subtree walk and
            // are drained once it ends: a tap gesture the claim silenced
            // delegates its activation through the scope (the container node
            // stands in for it, so it must stay activatable), and a
            // name-from-contents role collects the text of the leaves it
            // silenced as the container's own name.
            let consumed_text = if let Some(naming_scope) = scope.naming_scope {
                self.drain_scope_donations(&naming_scope, container_id)
            } else {
                false
            };
            // A container that consumed descendant text is a composite — it
            // holds its own name — so it must not dissolve into a surviving
            // control child: the close button would answer as the tab.
            if !consumed_text {
                self.accessibility
                    .collapse_single_child_container(container_id, scope.resolved_bounds);
            }
            // No real child ever registered under the container, but suppressed
            // decorative leaves beneath it still placed their boxes: the
            // container's node reports the resolved extent its layout recorded
            // — or, when none was, the element box they left — not the frame
            // the container itself was stretched into — and, as
            // `collapse_single_child_container` keeps the child's role when the
            // container names nothing but `Group`, a suppressed graphics leaf
            // (always `Image`-roled) lends its role too.
            if let Some(bounds) = self
                .accessibility
                .suppressed_leaf_bounds
                .remove(&container_id)
                && self
                    .accessibility
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == container_id)
                    .is_some_and(|(_, node)| node.children().is_empty())
                && let Some((_, node)) = self
                    .accessibility
                    .nodes
                    .iter_mut()
                    .find(|(id, _)| *id == container_id)
            {
                let element = scope
                    .resolved_bounds
                    .filter(|resolved| resolved.width() > 0.0 && resolved.height() > 0.0)
                    .unwrap_or(bounds);
                node.set_bounds(kurbo_rect_to_accesskit_rect(element));
                if node.role() == AccessibilityNodeRole::Group {
                    node.set_role(AccessibilityNodeRole::Image);
                }
            }
        }
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_node(
        &mut self,
        node: AccessibilityNode,
        bounds: kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        let placement = NodePlacement {
            bounds,
            clip: self.hit_test.hit_clip_stack.last().copied(),
        };
        self.accessibility.register_node_internal(
            node,
            Some(placement),
            env,
            action_target,
            true,
            None,
        )
    }

    /// Registers a leaf accessibility node in either runtime: bounds from
    /// `ctx` when a rendered frame supplies one, or none when the semantic
    /// walk emits the same node without layout.
    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_leaf(
        &mut self,
        ctx: Option<crate::renderer::RenderContext>,
        node: AccessibilityNode,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        match ctx {
            Some(ctx) => self.register_accessibility_node(
                node,
                crate::renderer::transformed_rect(ctx.hit_transform, ctx.bounds),
                env,
                action_target,
            ),
            None => self.register_accessibility_node_semantic(node, env, action_target),
        }
    }

    /// Notes the bounds a decorative graphics leaf would have published had it
    /// carried semantics. The innermost container node may still need them:
    /// when every child under a naming container is decorative, the container's
    /// synthesized node adopts these bounds — the placed element's box, not the
    /// frame the parent assigned — the contract
    /// `collapse_single_child_container` keeps when a real child exists.
    ///
    /// A no-op on the semantic walk (no `ctx` to take a rect from), inside a
    /// suppressed subtree, and above every container node — a leaf outside all
    /// of them has nothing to lend bounds to.
    #[cfg(feature = "accessibility")]
    pub(crate) fn note_suppressed_graphics_leaf(
        &mut self,
        ctx: Option<crate::renderer::RenderContext>,
    ) {
        let Some(ctx) = ctx else {
            return;
        };
        if self.accessibility.suppression_depth > 0 {
            return;
        }
        let Some(&parent) = self.accessibility.parent_stack.last() else {
            return;
        };
        let bounds = crate::renderer::transformed_rect(ctx.hit_transform, ctx.bounds);
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        self.accessibility
            .suppressed_leaf_bounds
            .entry(parent)
            .and_modify(|acc| *acc = acc.union(bounds))
            .or_insert(bounds);
    }

    /// The semantic counterpart of [`Self::register_accessibility_node`]: the
    /// same node and action target, with no bounds — the semantic runtime has
    /// no layout to take a rect from.
    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_node_semantic(
        &mut self,
        node: AccessibilityNode,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        self.accessibility
            .register_node_internal(node, None, env, action_target, true, None)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_child_node(
        &mut self,
        node: AccessibilityNode,
        bounds: kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        let placement = NodePlacement {
            bounds,
            clip: self.hit_test.hit_clip_stack.last().copied(),
        };
        self.accessibility.register_node_internal(
            node,
            Some(placement),
            env,
            action_target,
            false,
            None,
        )
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_child_node_semantic(
        &mut self,
        node: AccessibilityNode,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        self.accessibility
            .register_node_internal(node, None, env, action_target, false, None)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_child_node_with_key(
        &mut self,
        semantic_key: i64,
        node: AccessibilityNode,
        bounds: kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        let placement = NodePlacement {
            bounds,
            clip: self.hit_test.hit_clip_stack.last().copied(),
        };
        self.accessibility.register_node_internal(
            node,
            Some(placement),
            env,
            action_target,
            false,
            Some(semantic_key),
        )
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn register_accessibility_child_node_with_key_semantic(
        &mut self,
        semantic_key: i64,
        node: AccessibilityNode,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.watch_accessibility_state(env);
        self.accessibility.register_node_internal(
            node,
            None,
            env,
            action_target,
            false,
            Some(semantic_key),
        )
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
        label: &waterui_controls::label::Label,
        env: &Environment,
    ) -> Option<String> {
        use waterui_core::Signal;
        let plain = label
            .clone()
            .resolve(env)
            .accessibility_label()
            .snapshot()
            .to_semantic();
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
            return Some(styled.to_semantic().to_string());
        }
        if let Some(text) = view.downcast_ref::<waterui_text::Text>() {
            let styled = self.read_signal(&text.resolve(&scoped_env).content);
            return Some(styled.to_semantic().to_string());
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
            return Some(styled.to_semantic().to_string());
        }
        if let Some(icon) = view.downcast_ref::<Native<SystemIcon>>() {
            return Some(icon.as_inner().name.as_str().to_owned());
        }
        // A control's `label` is the same `Label` a composite button's name
        // derives from, so a container holding a bare control derives the
        // control's own name rather than dropping it — e.g. a `List` row of a
        // single `toggle("Wi-Fi")` announces "Wi-Fi".
        macro_rules! label_from_native_config {
            ($($config:ty),+ $(,)?) => {$(
                if let Some(native) = view.downcast_ref::<Native<$config>>() {
                    return Self::accessibility_label_from_label(
                        &native.as_inner().label,
                        &scoped_env,
                    );
                }
            )+};
        }
        label_from_native_config!(
            waterui_controls::toggle::ToggleConfig,
            waterui_controls::slider::SliderConfig,
            waterui_controls::stepper::StepperConfig,
            waterui_controls::button::ButtonConfig,
            waterui_controls::text_field::ResolvedTextFieldConfig,
            waterui_form::secure::SecureFieldConfig,
            waterui_form::picker::PickerConfig,
            waterui_form::picker::date::DatePickerConfig,
            waterui_form::picker::color::ColorPickerConfig,
        );
        if let Some(menu) = view.downcast_ref::<Native<waterui_controls::menu::ResolvedMenu>>() {
            let styled = self.read_signal(&menu.as_inner().accessibility_label);
            return Some(styled.to_semantic().to_string());
        }
        if let Some(progress) =
            view.downcast_ref::<Native<waterui::component::progress::ProgressConfig>>()
        {
            return self
                .accessibility_label_from_view_with_budget(
                    &progress.as_inner().label,
                    &scoped_env,
                    remaining - 1,
                )
                .or_else(|| {
                    self.accessibility_label_from_view_with_budget(
                        &progress.as_inner().value_label,
                        &scoped_env,
                        remaining - 1,
                    )
                });
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
        // Reading the label through `read_signal` subscribes it, so a reactive
        // label ("3 unread messages") republishes without a subtree rebuild.
        let signal = env
            .get::<AccessibilityLabel>()
            .map(|label| label.signal().clone());
        signal
            .map(|signal| self.read_signal(&signal).as_str().to_owned())
            .or(default_label)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn resolve_accessibility_value(
        &mut self,
        env: &Environment,
        default_value: Option<String>,
    ) -> Option<String> {
        // Same subscription as the label: a reactive value republishes without
        // a subtree rebuild.
        let signal = env
            .get::<AccessibilityValue>()
            .map(|value| value.signal().clone());
        signal
            .map(|signal| self.read_signal(&signal).as_str().to_owned())
            .or(default_value)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn resolve_accessibility_role(
        env: &Environment,
        default_role: AccessibilityNodeRole,
    ) -> AccessibilityNodeRole {
        env.get::<AccessibilityRole>().map_or(default_role, |role| {
            accessibility_role_to_accesskit_role(role.clone())
        })
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn finalize_accessibility_tree_update(&mut self) {
        self.accessibility.finalize_tree_update();
    }
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
        AccessibilityRole::Dialog => AccessibilityNodeRole::Dialog,
        _ => panic!("hydrolysis accessibility role variant is not implemented"),
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
    if matches!(action, AccessibilityAction::Focus) {
        return true;
    }
    let step = ACCESSIBILITY_SCROLL_STEP;
    // `Axis` is `#[non_exhaustive]`: a variant hydrolysis does not know is a
    // framework bug and must panic; a direction a known axis does not serve
    // is a declined action and reports `false`. A supported direction is
    // handled whether or not the scroll could still move.
    let delta = match axis {
        ScrollAxis::Horizontal => match action {
            AccessibilityAction::ScrollLeft => Some((step, 0.0)),
            AccessibilityAction::ScrollRight => Some((-step, 0.0)),
            _ => None,
        },
        ScrollAxis::Vertical => match action {
            AccessibilityAction::ScrollUp => Some((0.0, step)),
            AccessibilityAction::ScrollDown => Some((0.0, -step)),
            _ => None,
        },
        ScrollAxis::All => match action {
            AccessibilityAction::ScrollLeft => Some((step, 0.0)),
            AccessibilityAction::ScrollRight => Some((-step, 0.0)),
            AccessibilityAction::ScrollUp => Some((0.0, step)),
            AccessibilityAction::ScrollDown => Some((0.0, -step)),
            _ => None,
        },
        _ => panic!("scroll axis variant is not supported by hydrolysis"),
    };
    match delta {
        Some((dx, dy)) => {
            let _ = handle.apply_scroll_delta(dx, dy, false);
            true
        }
        None => false,
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
    let previous = value.snapshot().clamp(start, end);
    let next = match action {
        AccessibilityAction::Increment => (previous + step).min(end),
        AccessibilityAction::Decrement => (previous - step).max(start),
        AccessibilityAction::SetValue => match data {
            Some(AccessibilityActionData::NumericValue(target)) => target.clamp(start, end),
            _ => {
                panic!("hydrolysis accessibility slider SetValue requires NumericValue data")
            }
        },
        _ => panic!("hydrolysis accessibility slider does not support action {action:?}"),
    };
    if (next - previous).abs() > f64::EPSILON {
        value.set(next);
    }
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
    let step_value = step.snapshot();
    assert!(
        (step_value > 0),
        "hydrolysis accessibility stepper requires positive step"
    );
    let previous = value.snapshot().clamp(start, end);
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
        _ => panic!("hydrolysis accessibility stepper does not support action {action:?}"),
    };
    if next != previous {
        value.set(next);
    }
    true
}

#[cfg(feature = "accessibility")]
#[allow(
    clippy::too_many_arguments,
    reason = "threads the full accessibility-action context; grouping into a struct would not improve clarity"
)]
fn handle_accessibility_date_picker_action(
    renderer: &mut SemanticCore,
    value: &nami::Binding<DateTime>,
    range: &RangeInclusive<DateTime>,
    ty: DatePickerType,
    origin: Option<LayoutPoint>,
    picker_env: &Environment,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
    env: &Environment,
) -> bool {
    match action {
        // A rendered node carries its trigger anchor; a semantic node carries
        // none and mounts the same window with no placement at all.
        AccessibilityAction::Click => {
            // The picker's own environment layers over the dispatch's
            // (water-rs/hydrolysis#140).
            let env = picker_env.layered_on(env);
            match origin {
                Some(origin) => {
                    renderer.show_date_picker(value.clone(), range.clone(), ty, origin, &env);
                }
                None => {
                    renderer.activate_date_picker(value.clone(), range.clone(), ty, &env);
                }
            }
            true
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
            let previous = value.snapshot().clamp(*range.start(), *range.end());
            let next = parsed.clamp(*range.start(), *range.end());
            if next != previous {
                value.set(next);
            }
            true
        }
        _ => panic!("hydrolysis accessibility date picker does not support action {action:?}"),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_text_field_action(
    renderer: &mut SemanticCore,
    node_id: AccessibilityNodeId,
    value: &nami::Binding<StyledStr>,
    line_limit: Option<usize>,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            renderer.focus_text_input_for_accessibility_node(node_id);
            true
        }
        AccessibilityAction::SetValue => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!("hydrolysis accessibility text field SetValue requires Value data");
            };
            let normalized = normalized_insert_text(text.as_ref(), line_limit);
            assert!(
                !(exceeds_line_limit(normalized.as_str(), line_limit)),
                "hydrolysis accessibility text field SetValue exceeds line_limit {line_limit:?}"
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
            let mut plain = value.snapshot().to_plain().to_string();
            assert!(
                apply_text_insert(&mut plain, normalized.as_str(), line_limit),
                "hydrolysis accessibility text field ReplaceSelectedText exceeds line_limit {line_limit:?}"
            );
            value.set(StyledStr::plain(plain));
            true
        }
        _ => panic!("hydrolysis accessibility text field does not support action {action:?}"),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_secure_field_action(
    renderer: &mut SemanticCore,
    node_id: AccessibilityNodeId,
    value: &nami::Binding<FormSecure>,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            renderer.focus_text_input_for_accessibility_node(node_id);
            true
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
            let mut plain = value.snapshot().expose().to_owned();
            assert!(
                apply_text_insert(&mut plain, text.as_ref(), Some(1)),
                "hydrolysis accessibility secure field ReplaceSelectedText exceeds line_limit 1"
            );
            let mut next = FormSecure::default();
            next.set(plain);
            value.set(next);
            true
        }
        _ => panic!("hydrolysis accessibility secure field does not support action {action:?}"),
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
            if selection.snapshot() != target {
                selection.set(target);
            }
            true
        }
        _ => panic!("hydrolysis accessibility picker select does not support action {action:?}"),
    }
}

#[cfg(all(test, feature = "accessibility"))]
mod inspect_tests {
    use super::*;
    use kurbo::{Point, Rect};

    /// Registers a node covering `bounds` and returns its id.
    fn push(builder: &mut AccessibilityBuilder, bounds: Rect) -> AccessibilityNodeId {
        let id = builder.next_node_id();
        let mut node = AccessibilityNode::new(accesskit::Role::Label);
        node.set_bounds(kurbo_rect_to_accesskit_rect(bounds));
        builder.nodes.push((id, node));
        id
    }

    /// "Inspect element" has to mean the element under the pointer, which is the
    /// innermost one, not the container that happens to contain it.
    #[test]
    fn the_innermost_node_covering_a_point_wins() {
        let mut builder = AccessibilityBuilder::default();
        // A container, then a child inside it: registration order is tree order.
        let container = push(&mut builder, Rect::new(0.0, 0.0, 100.0, 100.0));
        let child = push(&mut builder, Rect::new(10.0, 10.0, 40.0, 40.0));

        assert_eq!(builder.node_at_point(Point::new(20.0, 20.0)), Some(child));
        assert_eq!(
            builder.node_at_point(Point::new(80.0, 80.0)),
            Some(container),
            "a point outside the child still belongs to the container"
        );
    }

    /// A point on nothing names nothing, so no menu entry is offered there.
    #[test]
    fn a_point_outside_every_node_names_none() {
        let mut builder = AccessibilityBuilder::default();
        push(&mut builder, Rect::new(0.0, 0.0, 10.0, 10.0));

        assert_eq!(builder.node_at_point(Point::new(50.0, 50.0)), None);
    }

    /// Bounds are half-open, so two nodes sharing an edge do not both claim it.
    #[test]
    fn an_edge_belongs_to_exactly_one_node() {
        let mut builder = AccessibilityBuilder::default();
        let left = push(&mut builder, Rect::new(0.0, 0.0, 50.0, 50.0));
        let right = push(&mut builder, Rect::new(50.0, 0.0, 100.0, 50.0));

        assert_eq!(builder.node_at_point(Point::new(49.9, 10.0)), Some(left));
        assert_eq!(
            builder.node_at_point(Point::new(50.0, 10.0)),
            Some(right),
            "the shared edge belongs to the node that starts there"
        );
    }
}
