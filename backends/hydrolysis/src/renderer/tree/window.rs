//! The window-level entry points: build-or-patch the retained window tree
//! ([`HydrolysisRenderer::capture_window_tree`]) and the per-frame refresh
//! pump ([`HydrolysisRenderer::flush_window_tree`]), plus [`RenderNode::patch`].

use super::*;

impl RenderNode {
    /// Apply pending reactive `Dynamic` content changes by rebuilding only the
    /// affected child subtree — no whole-window re-dispatch. Returns whether
    /// anything changed; the caller relays the whole (retained, cheap) tree out
    /// when so, which lets a size-changing swap reflow its ancestors (e.g. the
    /// chart's spacers) without the legacy reset-and-redispatch flash (Bug 2).
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
            // A lazy stack re-reads its collection length and items every flush, so a
            // membership change needs no structural patch — only a scheduled refresh,
            // which its watcher requests. A widget leaf likewise re-dispatches from
            // its live config every flush, so it needs no structural patch.
            | RenderNode::LazyStack(_)
            | RenderNode::Widget(_) => false,
        }
    }

    /// Collect the identities of every live `DynamicHostNode` in this retained
    /// subtree, so the measure-path dynamic dimension cache can be pruned to the
    /// `Dynamic`s still present in the tree. Walks the same child-bearing variants
    /// as [`RenderNode::patch`].
    pub(crate) fn collect_dynamic_identities(&self) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_dynamic_identities_into(&mut out);
        out
    }

    fn collect_dynamic_identities_into(&self, out: &mut Vec<usize>) {
        match self {
            RenderNode::Dynamic(node) => {
                out.push(node.source.identity());
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
            | RenderNode::LazyStack(_)
            | RenderNode::Widget(_) => {}
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
            self.refresh_content_size_limits(&tree, env);
            tree.flush(self, ctx, env);
            self.render_tree = Some(tree);
            return;
        }
        self.render_depth = 0;
        let mut node = RenderNode::build(content, env, self);
        node.layout(self, env, size);
        self.refresh_content_size_limits(&node, env);
        node.flush(self, ctx, env);
        self.render_tree = Some(node);
    }

    /// Re-flush the retained window tree without rebuilding it — the cheap
    /// per-frame path for a geometry-static frame (animation, scroll, re-present).
    /// Layout is reused from the last build. Returns `false` if no tree is built.
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
        // Track the live window bounds every flush: resize reaches this path (the
        // build path only runs once at startup), and text-context-menu clamping /
        // effect-rect checks read the stored bounds.
        self.set_window_bounds(bounds);
        // Roll over this frame's Retain watcher guards exactly like the rebuild path:
        // a full re-flush re-reads (re-subscribes) every reactive input, so last
        // frame's guards must move current -> previous (dropped next frame) instead of
        // accumulating. `patch` may build new `Dynamic` subtrees that subscribe, so
        // this precedes it.
        self.lifecycle.begin_rebuild_frame();
        // Reset frame-bound input registrations. Scroll, list, and table state
        // are owned by their semantic retained nodes.
        self.hit_test.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        // Apply any pending reactive Dynamic content changes incrementally
        // (rebuild only the affected child subtrees). `structural_change` is true
        // when a `Dynamic`/collection added or removed a subtree this frame — the
        // only time the signal/node-identity-keyed animation slots or the
        // `Dynamic`-dimension measurement cache can hold entries for now-gone nodes,
        // so prune them only then (clear-active before the flush re-binds the live
        // ones, drop-unbound after). A geometry-static or pure-value refresh removes
        // nothing, so it skips this and leaves in-flight animations untouched.
        // Fold in a structural patch a widget-owned sub-view applied during the
        // previous frame's flush (mid-flush, past that frame's bookkeeping
        // window): this frame runs the prune cycle for its dropped subtrees.
        let structural_change = self.take_subview_structural_change() | tree.patch(self);
        if structural_change {
            self.animation_controller.begin_rebuild_frame();
        }
        // Then run FULL layout every frame. Layout is cheap by construction: the only
        // heavy work — text shaping — is memoized in the persistent, content-keyed
        // text cache, and containers just recompute `place()` arithmetic over cached
        // child measurements. Always relaying out lets a reactive value change that
        // alters a leaf's size (text content, a widget's intrinsic size) reflow its
        // ancestors through this same cheap pump, with no `body()` rebuild and no
        // reset-and-redispatch flash. `begin_redraw_frame` (below) cleared the
        // per-frame `stable_ptr` view-dimension cache so this measure pass is sound.
        self.reset_scene();
        self.begin_redraw_frame();
        let size = Size::new(bounds.width() as f32, bounds.height() as f32);
        tree.layout(self, env, size);
        self.refresh_content_size_limits(&tree, env);
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        tree.flush(self, ctx, env);
        // The overlay-mode text context menu re-encodes with the frame it floats
        // over; only drawing it on the one-time build path left it visible for a
        // single frame.
        self.render_active_text_context_menu_overlay(env, transform);
        self.flush_vello_scene_layer();
        self.hit_test.finish_rebuild_frame();
        self.navigation.finish_rebuild_frame();
        if structural_change {
            // Drop animation slots + `Dynamic`-dimension cache entries for subtrees
            // the patch removed: the flush above re-bound every live animation, so
            // anything still unbound belongs to a gone node.
            self.animation_controller
                .finish_rebuild_frame_with_inactive_slot_retention(false);
            let live_dynamics = tree.collect_dynamic_identities();
            self.state
                .measurement
                .retain_dynamic_identities(|identity| live_dynamics.contains(&identity));
        }
        self.lifecycle.finish_rebuild_frame();
        // A reactive value change can remove the focused field; drop focus/drag that
        // now points past the re-emitted text-input targets, then republish the a11y
        // tree so a value/label change is reflected on this cheap path (parity with
        // the rebuild path's `finish_rebuild_frame`).
        self.validate_focused_text_input_after_flush();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
        self.render_tree = Some(tree);
        true
    }

    /// Re-measures the window content's per-axis minimum and maximum sizes.
    ///
    /// Each axis is negotiated independently: a zero proposal asks for the hard
    /// minimum and an infinite proposal asks for the hard maximum. The other axis
    /// stays unspecified so cross-axis layout does not turn one dimension's
    /// constraint into the other dimension's result.
    fn refresh_content_size_limits(&mut self, tree: &RenderNode, env: &Environment) {
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
        if let Some(maximum) = maximum {
            assert!(
                maximum.width >= minimum.width && maximum.height >= minimum.height,
                "hydrolysis window layout reported maximum {maximum:?} below minimum {minimum:?}"
            );
        }
        self.content_size_limits = Some(ContentSizeLimits { minimum, maximum });
    }

    /// The window content's size limits from the latest layout pass, or `None`
    /// before the first build.
    #[must_use]
    pub(crate) fn content_size_limits(&self) -> Option<ContentSizeLimits> {
        self.content_size_limits
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
