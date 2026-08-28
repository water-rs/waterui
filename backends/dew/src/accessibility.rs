//! Platform-neutral accessibility output for embedded boards.
//!
//! Dew has no operating-system accessibility service to target directly. It
//! therefore publishes an AccessKit tree through [`Board`](crate::Board): a
//! board with an assistive interface forwards updates to that interface and
//! queues incoming actions back into the runtime. Display-only boards use the
//! trait defaults without paying for a platform adapter.

use std::collections::BTreeMap;

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use kurbo::Rect;
use nami::{Binding, Computed, Signal};
use waterui_core::id::Id;

use crate::pointer::PointerTargetHandle;

pub use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};

const ROOT_ID: NodeId = NodeId(0);
const FIRST_NODE_ID: u64 = 1;

#[derive(Clone)]
pub(crate) enum ActionTarget {
    Pointer {
        handler: PointerTargetHandle,
        bounds: Rect,
    },
    Toggle(Binding<bool>),
    Slider {
        value: Binding<f64>,
        range: core::ops::RangeInclusive<f64>,
        step: f64,
    },
    Stepper {
        value: Binding<i32>,
        step: Computed<i32>,
        range: core::ops::RangeInclusive<i32>,
    },
    Select {
        selection: Binding<Id>,
        value: Id,
    },
}

pub(crate) struct AccessibilityBuilder {
    nodes: Vec<(NodeId, Node)>,
    roots: Vec<NodeId>,
    parents: Vec<NodeId>,
    actions: BTreeMap<NodeId, ActionTarget>,
    next_id: u64,
    focus: NodeId,
    root_bounds: Rect,
    pending_update: Option<TreeUpdate>,
    suppression_depth: usize,
}

impl Default for AccessibilityBuilder {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            roots: Vec::new(),
            parents: Vec::new(),
            actions: BTreeMap::new(),
            next_id: FIRST_NODE_ID,
            focus: ROOT_ID,
            root_bounds: Rect::ZERO,
            pending_update: None,
            suppression_depth: 0,
        }
    }
}

impl AccessibilityBuilder {
    pub(crate) const fn allocate_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("dew accessibility node ID overflow");
        id
    }

    pub(crate) fn begin_frame(&mut self, root_bounds: Rect) {
        self.nodes.clear();
        self.roots.clear();
        self.parents.clear();
        self.actions.clear();
        self.pending_update = None;
        self.suppression_depth = 0;
        self.root_bounds = root_bounds;
    }

    pub(crate) fn register(
        &mut self,
        id: NodeId,
        mut node: Node,
        bounds: Rect,
        target: Option<ActionTarget>,
    ) {
        if self.suppression_depth > 0 || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        node.set_bounds(accesskit::Rect {
            x0: bounds.x0,
            y0: bounds.y0,
            x1: bounds.x1,
            y1: bounds.y1,
        });
        self.nodes.push((id, node));
        if let Some(parent_id) = self.parents.last().copied() {
            self.node_mut(parent_id).push_child(id);
        } else {
            self.roots.push(id);
        }
        if let Some(target) = target {
            self.actions.insert(id, target);
        }
    }

    pub(crate) fn push_parent(&mut self, id: NodeId) {
        assert!(
            self.nodes.iter().any(|(node_id, _)| *node_id == id),
            "dew accessibility parent must be registered before its children"
        );
        self.parents.push(id);
    }

    pub(crate) fn pop_parent(&mut self) {
        self.parents
            .pop()
            .expect("dew accessibility parent stack underflow");
    }

    pub(crate) const fn push_suppression(&mut self) {
        self.suppression_depth = self
            .suppression_depth
            .checked_add(1)
            .expect("dew accessibility suppression depth overflow");
    }

    pub(crate) const fn pop_suppression(&mut self) {
        self.suppression_depth = self
            .suppression_depth
            .checked_sub(1)
            .expect("dew accessibility suppression stack underflow");
    }

    pub(crate) fn finish_frame(&mut self) {
        assert!(
            self.parents.is_empty(),
            "dew accessibility parent stack must balance within a frame"
        );
        assert_eq!(
            self.suppression_depth, 0,
            "dew accessibility suppression stack must balance within a frame"
        );
        let mut root = Node::new(Role::Window);
        root.set_label("WaterUI Window");
        root.set_bounds(accesskit::Rect {
            x0: self.root_bounds.x0,
            y0: self.root_bounds.y0,
            x1: self.root_bounds.x1,
            y1: self.root_bounds.y1,
        });
        root.set_children(self.roots.clone());
        let mut nodes = Vec::with_capacity(self.nodes.len() + 1);
        nodes.push((ROOT_ID, root));
        nodes.extend(self.nodes.iter().cloned());
        if !nodes.iter().any(|(id, _)| *id == self.focus) {
            self.focus = ROOT_ID;
        }
        self.pending_update = Some(TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT_ID)),
            tree_id: TreeId::ROOT,
            focus: self.focus,
        });
    }

    pub(crate) const fn take_update(&mut self) -> TreeUpdate {
        self.pending_update
            .take()
            .expect("dew accessibility update requires a completed render frame")
    }

    pub(crate) fn handle_action(&mut self, request: &ActionRequest) -> bool {
        assert_eq!(
            request.target_tree,
            TreeId::ROOT,
            "dew accessibility action targets an unknown tree"
        );
        if request.target_node == ROOT_ID {
            assert_eq!(
                request.action,
                Action::Focus,
                "dew accessibility root only supports Focus"
            );
            let changed = self.focus != ROOT_ID;
            self.focus = ROOT_ID;
            return changed;
        }
        let node = self
            .nodes
            .iter()
            .find_map(|(id, node)| (*id == request.target_node).then_some(node))
            .unwrap_or_else(|| {
                panic!(
                    "dew accessibility action targets unknown node {:?}",
                    request.target_node
                )
            });
        assert!(
            node.supports_action(request.action),
            "dew accessibility node {:?} does not support {:?}",
            request.target_node,
            request.action
        );
        if request.action == Action::Focus {
            let changed = self.focus != request.target_node;
            self.focus = request.target_node;
            return changed;
        }
        let target = self
            .actions
            .get(&request.target_node)
            .cloned()
            .expect("dew accessibility action has no registered target");
        let changed = match target {
            ActionTarget::Pointer { handler, bounds } => match request.action {
                Action::Click => handler.activate(bounds),
                action => panic!("dew pointer accessibility target does not support {action:?}"),
            },
            ActionTarget::Toggle(binding) => match request.action {
                Action::Click => {
                    binding.set(!binding.get());
                    true
                }
                action => panic!("dew toggle accessibility target does not support {action:?}"),
            },
            ActionTarget::Slider { value, range, step } => {
                numeric_f64_action(&value, range, step, request.action, request.data.as_ref())
            }
            ActionTarget::Stepper { value, step, range } => {
                numeric_i32_action(&value, &step, range, request.action, request.data.as_ref())
            }
            ActionTarget::Select { selection, value } => match request.action {
                Action::Click => {
                    if selection.get() == value {
                        false
                    } else {
                        selection.set(value);
                        true
                    }
                }
                action => panic!("dew selection accessibility target does not support {action:?}"),
            },
        };
        if changed {
            self.focus = request.target_node;
        }
        changed
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.nodes
            .iter_mut()
            .find_map(|(node_id, node)| (*node_id == id).then_some(node))
            .expect("dew accessibility parent stack contains an unknown node")
    }
}

fn numeric_f64_action(
    binding: &Binding<f64>,
    range: core::ops::RangeInclusive<f64>,
    step: f64,
    action: Action,
    data: Option<&ActionData>,
) -> bool {
    assert!(
        step > 0.0,
        "dew accessibility slider requires a positive step"
    );
    let start = *range.start();
    let end = *range.end();
    let previous = binding.get().clamp(start, end);
    let next = match action {
        Action::Increment => (previous + step).min(end),
        Action::Decrement => (previous - step).max(start),
        Action::SetValue => match data {
            Some(ActionData::NumericValue(value)) => value.clamp(start, end),
            _ => panic!("dew accessibility slider SetValue requires NumericValue data"),
        },
        action => panic!("dew slider accessibility target does not support {action:?}"),
    };
    if (next - previous).abs() <= f64::EPSILON {
        return false;
    }
    binding.set(next);
    true
}

fn numeric_i32_action(
    binding: &Binding<i32>,
    step: &Computed<i32>,
    range: core::ops::RangeInclusive<i32>,
    action: Action,
    data: Option<&ActionData>,
) -> bool {
    let step = step.get();
    assert!(
        step > 0,
        "dew accessibility stepper requires a positive step"
    );
    let start = *range.start();
    let end = *range.end();
    let previous = binding.get().clamp(start, end);
    let next = match action {
        Action::Increment => previous.saturating_add(step).min(end),
        Action::Decrement => previous.saturating_sub(step).max(start),
        Action::SetValue => match data {
            Some(ActionData::NumericValue(value)) => {
                let rounded = value.round().clamp(f64::from(start), f64::from(end));
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "the rounded value is clamped to the binding's i32 range"
                )]
                let rounded = rounded as i32;
                rounded
            }
            Some(ActionData::Value(value)) => value
                .parse::<i32>()
                .expect("dew accessibility stepper value must parse as i32")
                .clamp(start, end),
            _ => panic!("dew accessibility stepper SetValue requires numeric data"),
        },
        action => panic!("dew stepper accessibility target does not support {action:?}"),
    };
    if next == previous {
        return false;
    }
    binding.set(next);
    true
}
