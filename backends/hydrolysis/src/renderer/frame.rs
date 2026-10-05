//! Frame lifecycle: scene reset, rebuild/redraw frame boundaries, layer
//! stack management, frame triggers, and per-frame statistics.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use kurbo::Shape as _;

/// The two transforms a clip layer is pushed under: `paint` positions the
/// scene layer's clip, `hit` positions the matching hit-test clip — they diverge
/// where paint and hit spaces differ (e.g. a filter-atlas capture paints into
/// slot space but keeps window hit space).
#[derive(Clone, Copy)]
pub struct LayerTransforms {
    pub(crate) paint: kurbo::Affine,
    pub hit: kurbo::Affine,
}

/// What one frame's window pass was made of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderLayerStats {
    /// Layers the compositor drew, which is every layer unless the window pass
    /// was handed to a GPU surface outright.
    pub(crate) composited_scene: u32,
    /// Composited layers that were recorded scene segments.
    pub(crate) scene_segments: u32,
    /// Composited layers that were embedded GPU content mounts.
    pub(crate) gpu_content: u32,
    /// Composited layers that were filtered subtrees.
    pub filtered_subtrees: u32,
}

pub fn duration_micros_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Whether a recording encodes any visible content.
pub const fn scene_has_content(scene: &Recording) -> bool {
    !scene.is_empty()
}

impl SemanticCore {
    /// Whether the persistent render tree has been built. The view tree's `body()`
    /// is dispatched recursively exactly once — on the first frame, when this is
    /// `false`. Afterwards every change (reactive value, structural patch, scroll,
    /// resize, interaction) is reflected by refreshing this retained tree, so the
    /// runner routes any later rebuild request through the refresh pump instead of
    /// re-running `build_content`.
    #[must_use]
    pub const fn has_render_tree(&self) -> bool {
        self.render_tree.is_some()
    }

    #[must_use]
    pub const fn state(&self) -> &HydroState {
        &self.state
    }

    pub const fn state_mut(&mut self) -> &mut HydroState {
        &mut self.state
    }

    /// Drop cached `Dynamic` measurements whose node has left the retained tree.
    ///
    /// Shared by the two frames that can remove nodes: the one-time build and any
    /// refresh that applied a structural patch.
    pub(crate) fn prune_dynamic_measurements(&mut self, live: &FxHashSet<usize>) {
        self.state
            .measurement
            .retain_dynamic_identities(|identity| live.contains(&identity));
    }

    /// Drop text-input focus / the selection drag when the target they name is
    /// no longer emitted. Targets are pure-emission (rebuilt in flush order every
    /// frame), so after any flush — rebuild or refresh — a previously focused
    /// field may be gone. Both are held by stable identity, so this only asks
    /// whether that identity still resolves; it can never mistake a different
    /// field that moved into the old position for the focused one. Shared by both
    /// frame paths.
    pub(crate) fn validate_focused_text_input_after_flush(&mut self) {
        let modal_active = self.modal_shield_active();
        let focus_is_live = !self.text_editing.has_focus()
            || self
                .text_editing
                .focused_target()
                .is_some_and(|target| !modal_active || target.modal);
        if !focus_is_live {
            self.set_focused_text_input_key(None);
        }
        // The semantic focus node must still be emitted and hittable: a node
        // that went hidden or disabled — itself or through an ancestor —
        // releases focus rather than keeping it on an inert view.
        #[cfg(feature = "accessibility")]
        {
            if !self
                .keyboard_focus_node()
                .is_none_or(|node| self.emitted_node_is_live(node))
            {
                self.hit_test.focus_dropped_this_frame = true;
                self.set_keyboard_focus_node(None, false);
            }
        }
        let keyboard_focus_is_live = self.hit_test.keyboard_focus.as_ref().is_none_or(|focused| {
            self.interaction_key_is_live(focused) || {
                // A key resolved through the semantic focus link stays
                // live while its node emits — the semantic walk emits
                // no pointer machinery to back a key. In the rendered
                // runtime a key backed by no target is dead: its widget
                // went non-hittable and the emitted node cannot
                // resurrect it.
                #[cfg(feature = "accessibility")]
                {
                    self.semantic_walk
                        && self
                            .focus_node_for_key(focused)
                            .is_some_and(|node| self.emitted_node_is_live(node))
                }
                #[cfg(not(feature = "accessibility"))]
                {
                    false
                }
            }
        });
        if !keyboard_focus_is_live {
            self.hit_test.focus_dropped_this_frame = true;
            self.set_keyboard_focus(None, false);
        }
        let selection_drag_is_live = self
            .text_editing
            .active_text_selection_drag
            .as_ref()
            .is_none_or(|drag| self.text_editing.index_of(&drag.target).is_some());
        if !selection_drag_is_live {
            self.text_editing.active_text_selection_drag = None;
        }
    }

    /// Relocates focus that lost its view to this frame's flush: a hidden
    /// or unmounted view cannot keep focus, and dropping it on the floor
    /// leaves every keystroke dead until a pointer press re-grants it —
    /// the #126 regression. Focus relocates to the next focusable after the
    /// freed slot, in the emitted tree order `traversal_anchor` tracks —
    /// the same move Tab traversal performs. A `.focused(binding)` grant
    /// that landed in the same frame wins over the relocation, as does any
    /// focusable the transition already handed focus to.
    pub(crate) fn relocate_dropped_focus(&mut self) {
        if !self.hit_test.focus_dropped_this_frame {
            return;
        }
        self.hit_test.focus_dropped_this_frame = false;
        let focus_alive = self.hit_test.focused_embedded_key.is_some()
            || self.hit_test.keyboard_focus.is_some()
            || {
                #[cfg(feature = "accessibility")]
                {
                    self.keyboard_focus_node().is_some()
                }
                #[cfg(not(feature = "accessibility"))]
                {
                    false
                }
            };
        if !focus_alive {
            self.move_keyboard_focus(false);
        }
    }

    /// Whether `node` was emitted this frame and still accepts focus: not
    /// hidden and not disabled, itself or through an ancestor's state.
    #[cfg(feature = "accessibility")]
    pub(crate) fn emitted_node_is_live(&self, node: AccessibilityNodeId) -> bool {
        self.accessibility
            .nodes
            .iter()
            .any(|(id, emitted)| *id == node && !emitted.is_hidden() && !emitted.is_disabled())
    }

    /// The shared frame-trigger handle for closures that outlive a borrow of
    /// the renderer (navigation controllers, GPU-surface invalidators, …).
    pub(crate) fn frame_signals(&self) -> FrameSignals {
        self.signals.clone()
    }

    pub fn request_redraw(&self) {
        self.signals.request_redraw();
    }

    /// Schedules a full frame: every awake frame re-reads signals, runs
    /// layout, and re-encodes the retained tree. Reactive updates and visual
    /// values outside the reactive graph (a scroll offset, a scrollbar drag)
    /// share this one path.
    pub fn request_refresh(&self) {
        self.signals.request_refresh();
    }

    pub fn take_redraw_request(&mut self) -> bool {
        let requested = self.signals.take_redraw_request();
        if requested {
            self.state.counters.host_wakeups += 1;
        }
        requested
    }

    pub fn request_rebuild(&self) {
        self.signals.request_rebuild();
    }

    #[must_use]
    pub fn has_rebuild_request(&self) -> bool {
        self.signals.has_rebuild_request()
    }

    pub fn request_next_frame_rebuild(&self) {
        self.signals.request_next_frame_rebuild();
    }

    pub fn take_rebuild_request(&mut self) -> bool {
        let requested = self.signals.take_rebuild_request();
        if requested {
            self.state.counters.host_wakeups += 1;
        }
        requested
    }

    #[must_use]
    pub fn has_patch_request(&self) -> bool {
        self.signals.has_patch_request()
    }

    pub fn take_patch_request(&mut self) -> bool {
        let requested = self.signals.take_patch_request();
        if requested {
            self.state.counters.host_wakeups += 1;
        }
        requested
    }

    pub fn take_next_frame_rebuild_request(&mut self) -> bool {
        let requested = self.signals.take_next_frame_rebuild_request();
        if requested {
            self.state.counters.host_wakeups += 1;
        }
        requested
    }

    /// Whether a state change has already been requested but not yet applied,
    /// so the semantics the last flush produced are stale.
    ///
    /// This is the *unapplied* half of [`Self::has_scheduled_semantic_work`]:
    /// a signal fired and asked for a patch or a rebuild, and the next flush
    /// will show a different tree. It deliberately excludes work that merely
    /// continues over future frames — animations, gesture deadlines, gliding
    /// scrolls — because those never stop asking, so a caller that waits on
    /// them waits forever. An observer that needs to see the current state
    /// waits on this; one that needs the app to come fully to rest waits on
    /// `has_scheduled_semantic_work`.
    #[must_use]
    pub fn has_pending_semantic_update(&self) -> bool {
        self.signals.has_patch_request()
            || self.signals.has_rebuild_request()
            || self.signals.has_next_frame_rebuild_request()
    }

    /// Whether the renderer has scheduled work that will still change layout,
    /// semantics, or reactive state on a future frame: pending patches or
    /// rebuilds, active animations, armed gesture deadlines, gliding smooth
    /// scrolls, or an OS file drop awaiting its delivering drain.
    ///
    /// Visual-only redraw requests (caret blink, the visible-window present
    /// cadence) are deliberately excluded: they repaint pixels without moving
    /// semantic state, and a focused text caret blinks forever — including it
    /// would make an app with a focused field never count as settled.
    #[must_use]
    pub fn has_scheduled_semantic_work(&self) -> bool {
        self.has_pending_semantic_update()
            || self.os_file_drop_pending()
            || self.animations_active()
            || self.next_gesture_deadline().is_some()
            || self.has_gliding_smooth_scrolls()
            || self.has_active_touch_fling()
    }

    pub(crate) const fn measurement_cache_stats(&self) -> (u32, u32) {
        self.state.measurement.stats()
    }
}

impl HydrolysisRenderer {
    /// Records the window's logical bounds and the root transform that maps
    /// them onto the target's physical pixel grid.
    ///
    /// Both halves are needed together: the bounds alone say how big the window
    /// is in layout units, and only the transform says which device pixels that
    /// covers — which is the rectangle a full-window GPU surface has to match to
    /// be rendered straight into the target.
    pub(crate) fn set_window_viewport(
        &mut self,
        bounds: kurbo::Rect,
        root_transform: kurbo::Affine,
    ) {
        self.window_bounds = bounds;
        self.window_root_transform = root_transform;
        // The activation-point projection intersects node bounds with the
        // window bounds alongside the node's clip chain — both in the same
        // window hit-test space the hit clip stack uses.
        self.hit_test.window_bounds = bounds;
    }

    /// The window's bounds in hit-test (window logical) space — the rect the
    /// safe-area band math measures a node's frame against.
    pub(crate) const fn window_bounds(&self) -> kurbo::Rect {
        self.window_bounds
    }

    pub(crate) const fn state_and_scene_mut(&mut self) -> (&mut HydroState, &mut Recording) {
        (&mut self.core.state, &mut self.scene)
    }

    /// The per-frame work counters of the last rendered pump.
    /// `HydroState::counters` is private to `crate::renderer`, so callers
    /// outside the renderer reach it through here.
    #[must_use]
    pub const fn frame_work_counters(&self) -> FrameWorkCounters {
        self.core.state.counters
    }

    /// Mutable access for the runner's host-side wakeup and submission
    /// sites (the ones that happen outside a renderer method).
    pub const fn frame_work_counters_mut(&mut self) -> &mut FrameWorkCounters {
        &mut self.core.state.counters
    }

    #[must_use]
    /// The frame's recorded scene.
    pub const fn scene(&self) -> &Recording {
        &self.scene
    }

    /// The recordings the last flush committed to the compositor, in painter's
    /// order — `self.scene` itself is only the scratch tail that has not been
    /// drained yet. Tests assert on painted geometry through this.
    #[cfg(test)]
    pub(crate) fn painted_recordings(&self) -> impl Iterator<Item = &Recording> {
        self.compositor
            .render_layers
            .iter()
            .filter_map(|layer| match layer {
                RenderLayer::Scene(scene) => Some(scene),
                _ => None,
            })
    }

    /// Drops the recorded scene and the hit-test state derived from it.
    pub fn reset_scene(&mut self) {
        self.hit_test.reset_scene();
        self.gesture_engine.clear_targets();
        self.text_editing.text_input_targets.clear();
        self.scene.reset();
        self.compositor.render_layers.clear();
        self.compositor.active_scene_layers.clear();
        self.state.measurement.reset_counters();
        self.state.counters.reset_frame();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_filtered_count = 0;
        #[cfg(feature = "accessibility")]
        self.accessibility.reset_scene();
    }

    /// Marks the start of a full rebuild frame.
    pub fn begin_rebuild_frame(&mut self) {
        // A full rebuild re-dispatches every Dynamic node, so any pending isolated
        // reactive patch is subsumed by it.
        self.signals.begin_rebuild();
        self.state.measurement.begin_frame();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_filtered_count = 0;
        self.lifecycle.begin_rebuild_frame();
        self.hit_test.begin_rebuild_frame();
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.animation_controller.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        self.compositor.render_layers.clear();
        self.compositor.active_scene_layers.clear();
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
    }

    pub(crate) fn begin_redraw_frame(&mut self) {
        // Clear the per-frame `stable_ptr`-keyed view-dimension cache, not just the
        // counters: the refresh path now runs full layout every frame, so it measures
        // `RetainedSubview`/widget content through that cache. Its keys are view heap
        // addresses, unique only within a frame (a freed view's address is reused next
        // frame), so a stale entry would otherwise be read as a different view's size.
        // The persistent, content-keyed text-shaping cache is untouched and keeps full
        // layout cheap.
        self.state.measurement.begin_frame();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_filtered_count = 0;
    }

    /// Ends the rebuild pass, asserting the scene-layer stack drained, and
    /// flushes the frame's last open layer.
    ///
    /// # Panics
    /// Panics when a frame ends with scene layers still open — the
    /// tracked-stack invariant the flush asserts.
    pub fn finish_rebuild_frame(&mut self) {
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis renderer: scene layer stack must be empty at end of rebuild (len={})",
            self.compositor.active_scene_layers.len()
        );
        self.flush_scene_layer();
        self.lifecycle.finish_rebuild_frame();
        // Prune the measure-path `Dynamic` dimension cache down to the identities
        // still present in the retained render tree. The cache is read by
        // `measure_dynamic` when a `Dynamic` leaf is measured after its content was
        // handed to a `DynamicHostNode`; the live `DynamicHostNode`s in `render_tree`
        // are exactly the alive identities now that the dispatch path is gone.
        let live_dynamics = self
            .render_tree
            .as_ref()
            .map(RenderNode::collect_dynamic_identities)
            .unwrap_or_default();
        self.prune_dynamic_measurements(&live_dynamics);

        self.validate_focused_text_input_after_flush();

        self.core
            .animation_controller
            .finish_rebuild_frame_with_inactive_slot_retention(false);
        self.core
            .hit_test
            .finish_rebuild_frame(&self.core.text_editing.text_input_targets);
        self.relocate_dropped_focus();
        self.core.navigation.finish_rebuild_frame();
        self.core.signals.finish_rebuild();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
    }

    /// The frame's recorded scene, mutably.
    pub const fn scene_mut(&mut self) -> &mut Recording {
        &mut self.scene
    }

    pub(crate) fn draw_context(
        &mut self,
        ctx: RenderContext,
        body: impl FnOnce(&mut waterui_graphics::draw::Recorder),
    ) {
        self.scene.record_picture(ctx.transform, body);
    }

    pub(crate) fn push_layer_rect(
        &mut self,
        alpha: f32,
        transforms: LayerTransforms,
        rect: kurbo::Rect,
    ) {
        self.record_clip_layer_push();
        self.scene.push_group(
            peniko::Fill::NonZero,
            peniko::BlendMode::default(),
            alpha,
            transforms.paint,
            &rect,
        );
        // The same clip paint uses bounds the hit regions flushed inside the
        // layer: a row straddling a scroll viewport keeps only the part of its
        // hit bounds that is actually painted (water-rs/hydrolysis#252).
        self.hit_test
            .push_hit_clip(transformed_rect(transforms.hit, rect));
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform: transforms.paint,
            shape: LayerShape::Rect(rect),
        });
    }

    pub(super) fn push_layer_path(
        &mut self,
        alpha: f32,
        transforms: LayerTransforms,
        path: kurbo::BezPath,
    ) {
        self.record_clip_layer_push();
        self.scene.push_group(
            peniko::Fill::NonZero,
            peniko::BlendMode::default(),
            alpha,
            transforms.paint,
            &path,
        );
        self.hit_test
            .push_hit_clip(transformed_rect(transforms.hit, path.bounding_box()));
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform: transforms.paint,
            shape: LayerShape::Path(path),
        });
    }

    pub(super) fn push_layer_rounded_rect(
        &mut self,
        alpha: f32,
        transforms: LayerTransforms,
        path: kurbo::BezPath,
        rect: kurbo::Rect,
        corner_width: f64,
        corner_height: f64,
    ) {
        self.record_clip_layer_push();
        self.scene.push_group(
            peniko::Fill::NonZero,
            peniko::BlendMode::default(),
            alpha,
            transforms.paint,
            &path,
        );
        self.hit_test
            .push_hit_clip(transformed_rect(transforms.hit, rect));
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform: transforms.paint,
            shape: LayerShape::RoundedRect {
                path,
                rect,
                corner_width,
                corner_height,
            },
        });
    }

    pub(crate) fn pop_layer(&mut self) {
        self.scene.pop_scope();
        self.compositor
            .active_scene_layers
            .pop()
            .expect("hydrolysis renderer: pop_layer underflow");
        self.hit_test.pop_hit_clip();
    }

    /// Opens a rect clip/opacity scope on the recording, runs `f` inside it,
    /// then closes it. The lexical pairing every traversal helper uses, so an
    /// unclosed or misplaced scope is a type error — and the cutover has one
    /// defined place to substitute retained group layers (water-rs/hydrolysis#205).
    pub(crate) fn with_clip_rect_scope(
        &mut self,
        alpha: f32,
        transforms: LayerTransforms,
        rect: kurbo::Rect,
        f: impl FnOnce(&mut Self),
    ) {
        self.push_layer_rect(alpha, transforms, rect);
        f(self);
        self.pop_layer();
    }

    /// The [`Self::with_clip_rect_scope`] pairing for an arbitrary clip path.
    pub(super) fn with_clip_path_scope(
        &mut self,
        alpha: f32,
        transforms: LayerTransforms,
        path: kurbo::BezPath,
        f: impl FnOnce(&mut Self),
    ) {
        self.push_layer_path(alpha, transforms, path);
        f(self);
        self.pop_layer();
    }

    /// The [`Self::with_clip_rect_scope`] pairing for a rounded-rect clip.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn with_clip_rounded_rect_scope(
        &mut self,
        alpha: f32,
        transforms: LayerTransforms,
        path: kurbo::BezPath,
        rect: kurbo::Rect,
        corner_width: f64,
        corner_height: f64,
        f: impl FnOnce(&mut Self),
    ) {
        self.push_layer_rounded_rect(alpha, transforms, path, rect, corner_width, corner_height);
        f(self);
        self.pop_layer();
    }

    pub(super) fn record_clip_layer_push(&mut self) {
        self.frame_clip_layers = self
            .frame_clip_layers
            .checked_add(1)
            .expect("hydrolysis frame clip layer counter overflow");
        let depth =
            u32::try_from(self.captured_clip_depth + self.compositor.active_scene_layers.len() + 1)
                .expect("hydrolysis active scene layer depth exceeds u32");
        self.frame_max_clip_depth = self.frame_max_clip_depth.max(depth);
    }

    /// Records what `record` flushes as [`CapturedLayers`] in the subtree's
    /// own space instead of presenting it: the frame's scene and its open
    /// clip/opacity scopes are set aside, so the subtree records from an empty
    /// scope stack, and every layer it produces — its drawing and any keyed
    /// layer — is taken out of the frame. Hit targets and accessibility nodes
    /// the flush registers are live, as for any other flush.
    pub(crate) fn capture_layers(&mut self, record: impl FnOnce(&mut Self)) -> CapturedLayers {
        let outer_scene = core::mem::replace(&mut self.scene, Recording::new());
        let outer_ancestry = core::mem::take(&mut self.compositor.active_scene_layers);
        self.captured_clip_depth += outer_ancestry.len();
        let start = self.compositor.render_layers.len();
        record(self);
        self.flush_scene_layer();
        let layers = self.compositor.render_layers.split_off(start);
        self.captured_clip_depth -= outer_ancestry.len();
        self.scene = outer_scene;
        self.compositor.active_scene_layers = outer_ancestry;
        CapturedLayers(layers)
    }

    /// Presents `layers` at `transform` under the frame's open scopes: scene
    /// segments are drawn into the current scene, and each keyed layer is
    /// placed by `transform` and shown under the open ancestry, in order.
    pub(crate) fn present_layers(&mut self, layers: &CapturedLayers, transform: kurbo::Affine) {
        for layer in &layers.0 {
            if let RenderLayer::Scene(recording) = layer {
                self.scene.append(recording, transform);
            } else {
                self.flush_scene_layer();
                let placed = layer.placed(transform, &self.compositor.active_scene_layers);
                self.compositor.render_layers.push(placed);
            }
        }
    }

    pub(super) fn flush_scene_layer(&mut self) {
        assert!(
            (self.scene.open_clip_count() as usize) == self.compositor.active_scene_layers.len(),
            "hydrolysis renderer: scene clip count {} does not match tracked scene layers {}",
            self.scene.open_clip_count(),
            self.compositor.active_scene_layers.len()
        );

        for _ in 0..self.compositor.active_scene_layers.len() {
            self.scene.pop_scope();
        }

        if self.scene.is_empty() {
            for layer in &self.compositor.active_scene_layers {
                layer.push_to_scene(&mut self.scene);
            }
            return;
        }
        let scene = core::mem::take(&mut self.scene);
        self.compositor
            .render_layers
            .push(RenderLayer::Scene(scene));

        for layer in &self.compositor.active_scene_layers {
            layer.push_to_scene(&mut self.scene);
        }
    }

    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn record_native_view_layer(
        &mut self,
        view: objc2::rc::Retained<objc2_web_kit::WKWebView>,
        transform: kurbo::Affine,
        bounds: kurbo::Rect,
        occlusion: Rc<RefCell<Vec<kurbo::Rect>>>,
    ) {
        self.flush_scene_layer();
        self.compositor
            .render_layers
            .push(RenderLayer::NativeView(NativeViewLayer {
                view,
                transform,
                bounds,
                active_layers: self.compositor.active_scene_layers.clone(),
                occlusion,
            }));
    }

    pub(crate) fn set_host_redraw_handle(&mut self, handle: RedrawHandle) {
        self.host_redraw_handle = Some(handle);
    }

    /// The engine's scheduling answer from the last presented frame, drained
    /// once so one pump consumes it: `Next::At` asks the host for the frame
    /// an in-flight animation needs, `Next::Idle` parks the display link.
    pub(crate) const fn take_engine_next(&mut self) -> Option<cherenkov::Next> {
        self.engine_next.take()
    }

    pub(crate) fn render_layer_stats(&self) -> RenderLayerStats {
        let scene_layers = u32::try_from(self.compositor.render_layers.len())
            .expect("hydrolysis render layer count exceeds u32");
        let scene_segments = u32::try_from(
            self.compositor
                .render_layers
                .iter()
                .filter(|layer| matches!(layer, RenderLayer::Scene(_)))
                .count(),
        )
        .expect("hydrolysis scene segment layer count exceeds u32");
        let gpu_content = u32::try_from(
            self.compositor
                .render_layers
                .iter()
                .filter(|layer| matches!(layer, RenderLayer::GpuContent(_)))
                .count(),
        )
        .expect("hydrolysis GPU content layer count exceeds u32");
        let filtered_subtrees = u32::try_from(
            self.compositor
                .render_layers
                .iter()
                .filter(|layer| matches!(layer, RenderLayer::Filtered(_)))
                .count(),
        )
        .expect("hydrolysis filtered layer count exceeds u32");
        RenderLayerStats {
            composited_scene: scene_layers,
            scene_segments,
            gpu_content,
            filtered_subtrees,
        }
    }

    pub(crate) const fn clip_layer_stats(&self) -> (u32, u32) {
        (self.frame_clip_layers, self.frame_max_clip_depth)
    }
}
