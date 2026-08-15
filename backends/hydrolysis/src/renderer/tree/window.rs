//! The window-level entry points: build-or-patch the retained window tree
//! ([`HydrolysisRenderer::capture_window_tree`]) and the per-frame pass
//! ([`HydrolysisRenderer::flush_window_tree`]), plus [`RenderNode::patch`].

use super::*;

impl RenderNode {
    /// Apply pending reactive `Dynamic` content changes by rebuilding only the
    /// affected child subtree — no whole-window re-dispatch. Returns whether
    /// anything changed; the caller relays the whole (retained, cheap) tree out
    /// when so, which lets a size-changing swap reflow its ancestors without
    /// resetting the scene and re-dispatching, which is visible as a flash.
    /// Walks the whole tree.
    pub(crate) fn patch(&mut self, renderer: &mut HydrolysisRenderer) -> bool {
        // No environment is threaded through: a rebuild uses the node's own captured
        // environment (`Dynamic`/`Collection`/`Env` carry it), so the walk only needs
        // the renderer.
        match self {
            RenderNode::Dynamic(node) => {
                let pending = node.pending.borrow_mut().take();
                if let Some(content) = pending {
                    let node_env = node.env.clone();
                    node.child = RenderNode::build(content, &node_env, renderer);
                    true
                } else {
                    node.child.patch(renderer)
                }
            }
            RenderNode::Container(container) => {
                let mut changed = false;
                for child in &mut container.children {
                    changed |= child.patch(renderer);
                }
                changed
            }
            RenderNode::Opacity(node) => node.child.patch(renderer),
            RenderNode::Scale(node) => node.child.patch(renderer),
            RenderNode::Rotation(node) => node.child.patch(renderer),
            RenderNode::Offset(node) => node.child.patch(renderer),
            RenderNode::Retain(node) => node.child.patch(renderer),
            RenderNode::Env(node) => node.child.patch(renderer),
            RenderNode::Wrapper(node) => node.child.patch(renderer),
            RenderNode::Collection(node) => {
                // Reconcile membership first (keeps surviving items' nodes and,
                // with a transition, starts enters/exits), then advance the
                // transition clock — settling finished phases and resolving this
                // frame's presence factors — then always patch every entry so
                // surviving items' nested reactive content (e.g. an
                // active-indicator `.background(Computed)`) updates.
                let membership_changed = node.dirty.replace(false);
                if membership_changed {
                    node.reconcile(renderer);
                }
                let mut changed = membership_changed | node.advance_transitions(renderer);
                for entry in &mut node.entries {
                    changed |= entry.node.patch(renderer);
                }
                changed
            }
            RenderNode::Scroll(node) => node.child.patch(renderer),
            // A ViewEffect and an AppliedFilter wrap a child render node whose
            // reactive descendants must keep patching, so the walk recurses into
            // them (the effect itself owns its runtime, with no structural patch).
            RenderNode::ViewEffect(node) => node.child.borrow_mut().patch(renderer),
            RenderNode::AppliedFilter(node) => node.child.patch(renderer),
            RenderNode::Color(_)
            | RenderNode::Text(_)
            | RenderNode::SceneView(_)
            // A GpuSurface owns its runtime and re-renders every flush; like a
            // self-drawn scene it has no structural patch.
            | RenderNode::GpuSurface(_)
            // A widget leaf re-dispatches from its live config every flush, so it
            // needs no structural patch.
            | RenderNode::Widget(_) => false,
            // A lazy stack keeps only visible item subtrees. Patch those retained
            // items before parent layout so a Dynamic row-height change updates the
            // scroll extent in the same refresh instead of one frame later.
            RenderNode::LazyStack(node) => node.patch_visible(renderer),
        }
    }

    /// Collect the identities of every live `DynamicHostNode` in this retained
    /// subtree, so the measure-path dynamic dimension cache can be pruned to the
    /// `Dynamic`s still present in the tree. Walks the same child-bearing variants
    /// as [`RenderNode::patch`]. A set, not a list: the prune tests every cached
    /// identity against it, which is quadratic over a linear scan.
    pub(crate) fn collect_dynamic_identities(&self) -> FxHashSet<usize> {
        let mut out = FxHashSet::default();
        self.collect_dynamic_identities_into(&mut out);
        out
    }

    pub(super) fn collect_dynamic_identities_into(&self, out: &mut FxHashSet<usize>) {
        match self {
            RenderNode::Dynamic(node) => {
                out.insert(node.source.identity());
                node.child.collect_dynamic_identities_into(out);
            }
            RenderNode::Container(container) => {
                for child in &container.children {
                    child.collect_dynamic_identities_into(out);
                }
            }
            RenderNode::Opacity(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Scale(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Rotation(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Offset(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Retain(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Env(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Wrapper(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Collection(node) => {
                for entry in &node.entries {
                    entry.node.collect_dynamic_identities_into(out);
                }
            }
            RenderNode::Scroll(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::ViewEffect(node) => {
                node.child.borrow().collect_dynamic_identities_into(out);
            }
            RenderNode::AppliedFilter(node) => node.child.collect_dynamic_identities_into(out),
            RenderNode::Color(_)
            | RenderNode::Text(_)
            | RenderNode::SceneView(_)
            | RenderNode::GpuSurface(_)
            | RenderNode::Widget(_) => {}
            RenderNode::LazyStack(node) => node
                .item_cache
                .borrow()
                .collect_dynamic_identities_into(out),
        }
    }
}

impl HydrolysisRenderer {
    /// Build the retained tree before its first sized frame. Embedded GPU hosts
    /// use this during async setup so every statically reachable `GpuSurface`
    /// can finish its own setup before the first render target is presented.
    pub(crate) fn prepare_window_tree(&mut self, content: AnyView, env: &Environment) {
        assert!(
            self.render_tree.is_none(),
            "hydrolysis renderer: window tree prepared more than once"
        );
        self.begin_rebuild_frame();
        self.render_depth = 0;
        let tree = RenderNode::build(content, env, self);
        self.render_tree = Some(tree);
        self.finish_rebuild_frame();
    }

    /// Build the window render tree from `content`, lay it out at `bounds`, and
    /// flush it into the scene — the render-tree analogue of
    /// [`HydrolysisRenderer::capture_window_scene`]. The built tree is retained in
    /// `render_tree` for subsequent per-frame flushes.
    pub fn capture_window_tree(
        &mut self,
        content: AnyView,
        env: &Environment,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) {
        let size = Size::new(bounds.width() as f32, bounds.height() as f32);
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        // The tree is built once and persists. A later "rebuild" request reuses
        // it — applying pending Dynamic patches, relaying out, and re-flushing —
        // rather than rebuilding (which would re-connect each `Dynamic`, and a
        // `Dynamic` can only connect once). Called within a begin/finish rebuild
        // frame, so scene/layer flushing is handled by the caller.
        if let Some(mut tree) = self.render_tree.take() {
            tree.patch(self);
            tree.layout(self, env, size);
            tree.flush(self, ctx, env);
            self.render_tree = Some(tree);
            return;
        }
        self.render_depth = 0;
        let mut node = RenderNode::build(content, env, self);
        node.layout(self, env, size);
        node.flush(self, ctx, env);
        self.render_tree = Some(node);
    }

    /// Apply pending structural changes, run layout, and re-encode the retained
    /// window tree without rebuilding it. Returns `false` if no tree is built.
    /// This is the one per-frame pass: every awake frame patches, lays out, and
    /// re-encodes, so the presented scene can never go stale against layout.
    pub fn flush_window_tree(
        &mut self,
        env: &Environment,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) -> bool {
        let Some(mut tree) = self.render_tree.take() else {
            return false;
        };
        // Track the live window bounds every frame: text-context-menu clamping and
        // effect-rect checks read the stored bounds.
        self.set_window_bounds(bounds);
        // Roll over this frame's Retain watcher guards exactly like the build path:
        // every re-encode re-reads and re-subscribes reactive visual inputs.
        self.lifecycle.begin_rebuild_frame();
        // Reset frame-bound input registrations. Scroll, list, and table state are
        // owned by their semantic retained nodes.
        self.hit_test.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        // Fold in a structural patch a widget-owned sub-view applied during
        // the previous frame's flush (mid-flush, past that frame's
        // bookkeeping window).
        let structural_change = self.take_subview_structural_change() | tree.patch(self);
        if structural_change {
            self.animation_controller.begin_rebuild_frame();
        }
        self.reset_scene();
        self.begin_redraw_frame();
        // Layout runs every frame: geometry can never go stale against the
        // scene encoded right after it.
        let size = Size::new(bounds.width() as f32, bounds.height() as f32);
        tree.layout(self, env, size);
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        tree.flush(self, ctx, env);
        // The overlay-mode text context menu re-encodes with the frame it floats
        // over; drawing it only on the one-time build path would leave it visible
        // for a single frame.
        self.render_active_text_context_menu_overlay(env, transform);
        self.flush_vello_scene_layer();
        self.hit_test.finish_rebuild_frame();
        self.navigation.finish_rebuild_frame();
        if structural_change {
            // The flush re-bound every live animation. Drop slots and cached
            // Dynamic measurements belonging to subtrees removed by the patch.
            self.animation_controller
                .finish_rebuild_frame_with_inactive_slot_retention(false);
            self.prune_dynamic_measurements(&tree.collect_dynamic_identities());
        }
        self.lifecycle.finish_rebuild_frame();
        // Drop focus or drag targets that are no longer emitted, then publish the
        // refreshed accessibility tree.
        self.validate_focused_text_input_after_flush();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
        self.render_tree = Some(tree);
        true
    }

    /// Measures the window content's per-axis minimum and maximum sizes, or
    /// `None` before the tree is built.
    ///
    /// This is four whole-tree measure passes at proposals the frame's own
    /// layout never uses, so it is demand-driven rather than run on every
    /// refresh: only the runner calls it, and only once it knows the answer will
    /// reach a window that acts on it (see `apply_window_size_limits`).
    pub(crate) fn measure_content_size_limits(
        &mut self,
        env: &Environment,
    ) -> Option<ContentSizeLimits> {
        let tree = self.render_tree.take()?;
        let limits = self.content_size_limits_of(&tree, env);
        self.render_tree = Some(tree);
        Some(limits)
    }

    /// Each axis is negotiated independently: a zero proposal asks for the hard
    /// minimum and an infinite proposal asks for the hard maximum. The other axis
    /// stays unspecified so cross-axis layout does not turn one dimension's
    /// constraint into the other dimension's result.
    fn content_size_limits_of(
        &mut self,
        tree: &RenderNode,
        env: &Environment,
    ) -> ContentSizeLimits {
        let min_width = tree
            .measure(&mut self.state, env, ProposalSize::new(Some(0.0), None))
            .size
            .width;
        let min_height = tree
            .measure(&mut self.state, env, ProposalSize::new(None, Some(0.0)))
            .size
            .height;
        let max_width = tree
            .measure(
                &mut self.state,
                env,
                ProposalSize::new(Some(f32::INFINITY), None),
            )
            .size
            .width;
        let max_height = tree
            .measure(
                &mut self.state,
                env,
                ProposalSize::new(None, Some(f32::INFINITY)),
            )
            .size
            .height;

        let minimum = Size::new(
            validated_minimum_axis(min_width, "width"),
            validated_minimum_axis(min_height, "height"),
        );
        let maximum = content_maximum_size(max_width, max_height);
        if let Some(maximum) = maximum
            && !(maximum.width >= minimum.width && maximum.height >= minimum.height)
        {
            // The root's two numbers say the tree contradicted itself, but not
            // which view did. Walk it and let the offending nodes name themselves,
            // otherwise this is only reproducible by guesswork.
            let culprits = probe_contract_violations(tree, &mut self.state, env);
            panic!(
                "hydrolysis window layout reported maximum {maximum:?} below minimum \
                 {minimum:?}.\nA view answered a larger proposal with a smaller size. \
                 Offending nodes (deepest first):\n{culprits}"
            );
        }
        ContentSizeLimits { minimum, maximum }
    }
}

fn validated_minimum_axis(value: f32, axis: &str) -> f32 {
    assert!(
        value.is_finite() && value >= 0.0,
        "hydrolysis window layout reported invalid minimum {axis}: {value}"
    );
    value
}

fn validated_maximum_axis(value: f32, axis: &str) -> Option<f32> {
    assert!(
        !value.is_nan() && value >= 0.0,
        "hydrolysis window layout reported invalid maximum {axis}: {value}"
    );
    value.is_finite().then_some(value)
}

fn content_maximum_size(width: f32, height: f32) -> Option<Size> {
    let width = validated_maximum_axis(width, "width");
    let height = validated_maximum_axis(height, "height");
    if width.is_none() && height.is_none() {
        return None;
    }
    Some(Size::new(
        width.unwrap_or(f32::MAX),
        height.unwrap_or(f32::MAX),
    ))
}

/// Reports every node whose own probe answers contradict each other, deepest
/// first, so the innermost cause is read before the containers that inherited it.
///
/// Only runs when the window's own check has already failed, so the cost of
/// re-measuring the tree four times per node does not matter.
fn probe_contract_violations(
    tree: &RenderNode,
    state: &mut HydroState,
    env: &Environment,
) -> String {
    let mut report = String::new();
    walk_probe_contract(tree, state, env, 0, &mut report);
    if report.is_empty() {
        report.push_str(
            "  (no single node contradicts itself; the disagreement is produced by a              container combining its children)\n",
        );
    }
    report
}

fn walk_probe_contract(
    node: &RenderNode,
    state: &mut HydroState,
    env: &Environment,
    depth: usize,
    report: &mut String,
) {
    for child in node.child_nodes() {
        walk_probe_contract(child, state, env, depth + 1, report);
    }

    let min_width = node
        .measure(state, env, ProposalSize::new(Some(0.0), None))
        .size
        .width;
    let max_width = node
        .measure(state, env, ProposalSize::new(Some(f32::INFINITY), None))
        .size
        .width;
    let min_height = node
        .measure(state, env, ProposalSize::new(None, Some(0.0)))
        .size
        .height;
    let max_height = node
        .measure(state, env, ProposalSize::new(None, Some(f32::INFINITY)))
        .size
        .height;

    for (axis, min, max) in [
        ("width", min_width, max_width),
        ("height", min_height, max_height),
    ] {
        if min > max {
            use core::fmt::Write as _;
            let _ = writeln!(
                report,
                "  {:indent$}{} {axis}: minimum {min} exceeds maximum {max}",
                "",
                node.kind(),
                indent = depth * 2
            );
        }
    }
}
