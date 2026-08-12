//! [`NodeSubView`]: the [`SubView`] adapter that lets a real [`Layout`]
//! measure retained child [`RenderNode`]s on demand, with the resolved-text
//! shaping fast path.

use super::*;

/// A [`SubView`] adapter that measures a child [`RenderNode`] **on demand** at
/// whatever proposal the real [`Layout`] passes — the node analogue of
/// [`HydroSubview`]. A fixed pre-measured size is wrong: layouts re-probe children
/// at refined proposals (e.g. `GridLayout` asks each cell its height at the column
/// width; a `FrameLayout` with no height constraint would otherwise fill the whole
/// proposed height), so the adapter must forward the live proposal.
///
/// Like [`HydroSubview`], a text-leaf subtree is resolved at construction (reading
/// its signals and theme) into a shaping input, and `measure` then shapes it
/// through the content-keyed `TextMeasureService` directly. Every other node
/// measures by recursing through the renderer's `HydroState`, shared across one
/// container level's children through a `RefCell`.
pub(super) struct NodeSubView<'a> {
    node: MainThreadBound<&'a RenderNode>,
    state: MainThreadBound<&'a RefCell<&'a mut HydroState>>,
    env: MainThreadBound<Environment>,
    stretch: StretchAxis,
    priority: i32,
    /// Per-proposal memo for this layout pass (containers probe children with
    /// repeated proposals). Only the recursion path caches here; the text path
    /// memoizes in the content-keyed `TextMeasureService` instead.
    measure_cache: MainThreadBound<RefCell<Vec<(ProposalSize, ViewDimensions)>>>,
    /// Present when this child is a resolved text-leaf subtree, letting `measure`
    /// shape it directly instead of recursing.
    resolved_text: Option<ResolvedNodeTextMeasure>,
}

/// A text-leaf node resolved on the main thread, ready to be shaped on any thread.
struct ResolvedNodeTextMeasure {
    input: ResolvedTextLayoutInput,
    service: std::sync::Arc<TextMeasureService>,
}

/// Resolve a node that measures as a pure text leaf into a `Send` shaping input,
/// or `None` for anything else (which then measures on the main thread). Mirrors
/// the layout-transparent delegation arms of [`RenderNode::measure`] so the
/// worker-thread fast path and the main-thread recursion agree on which
/// environment scopes the text.
fn try_resolve_node_text_leaf(
    node: &RenderNode,
    env: &Environment,
) -> Option<ResolvedTextLayoutInput> {
    match node {
        RenderNode::Text(text) => Some(resolve_text_layout_input(
            &text.content.get(),
            text.alignment.get(),
            env,
        )),
        RenderNode::Opacity(node) => try_resolve_node_text_leaf(&node.child, env),
        RenderNode::Scale(node) => try_resolve_node_text_leaf(&node.child, env),
        RenderNode::Rotation(node) => try_resolve_node_text_leaf(&node.child, env),
        RenderNode::Offset(node) => try_resolve_node_text_leaf(&node.child, env),
        RenderNode::Retain(node) => try_resolve_node_text_leaf(&node.child, env),
        RenderNode::Dynamic(node) => try_resolve_node_text_leaf(&node.child, env),
        RenderNode::Env(node) => try_resolve_node_text_leaf(&node.child, &node.env),
        RenderNode::Wrapper(node) => try_resolve_node_text_leaf(&node.child, &node.env),
        _ => None,
    }
}

impl<'a> NodeSubView<'a> {
    pub(super) fn new(
        node: &'a RenderNode,
        state: &'a RefCell<&'a mut HydroState>,
        env: &'a Environment,
    ) -> Self {
        let resolved_text =
            try_resolve_node_text_leaf(node, env).map(|input| ResolvedNodeTextMeasure {
                input,
                service: std::sync::Arc::clone(&state.borrow().text),
            });
        Self {
            stretch: node.stretch(),
            priority: node.priority(),
            node: MainThreadBound::new(node),
            state: MainThreadBound::new(state),
            env: MainThreadBound::new(env.clone()),
            measure_cache: MainThreadBound::new(RefCell::new(Vec::new())),
            resolved_text,
        }
    }

    /// Apply this child's stretch axis to a measured size against the proposal,
    /// matching [`HydroSubview::apply_stretch`] so both paths agree.
    fn apply_stretch(
        &self,
        mut dimensions: ViewDimensions,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        if self.stretch.stretches_horizontal()
            && let Some(width) = proposal.width
        {
            dimensions.size.width = dimensions.size.width.max(width);
        }
        if self.stretch.stretches_vertical()
            && let Some(height) = proposal.height
        {
            dimensions.size.height = dimensions.size.height.max(height);
        }
        dimensions
    }
}

impl SubView for NodeSubView<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        // Worker-safe path: shaping a resolved text leaf touches no
        // `MainThreadBound` state, so it may run on any thread.
        if let Some(resolved) = &self.resolved_text {
            let layout = resolved.service.shape(&resolved.input, proposal.width);
            let dimensions = text_dimensions_from_layout(&layout, None);
            return self.apply_stretch(dimensions, proposal);
        }
        if let Some((_, dimensions)) = self
            .measure_cache
            .borrow()
            .iter()
            .find(|(cached, _)| *cached == proposal)
        {
            return dimensions.clone();
        }
        let dimensions = {
            let mut state = self.state.borrow_mut();
            self.node.measure(&mut state, &self.env, proposal)
        };
        let dimensions = self.apply_stretch(dimensions, proposal);
        self.measure_cache
            .borrow_mut()
            .push((proposal, dimensions.clone()));
        dimensions
    }
    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }
    fn priority(&self) -> i32 {
        self.priority
    }
}
