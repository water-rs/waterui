//! Frame lifecycle: scene reset, rebuild/redraw frame boundaries, layer
//! stack management, frame triggers, and per-frame statistics.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use kurbo::Shape as _;

pub fn duration_micros_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
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
            .node(node)
            .is_some_and(|emitted| !emitted.is_hidden() && !emitted.is_disabled())
    }

    pub fn request_redraw(&self) {
        self.signals.request_redraw();
    }

    pub fn take_redraw_request(&mut self) -> bool {
        let requested = self.signals.take_redraw_request();
        if requested {
            self.state.counters.host_wakeups += 1;
        }
        requested
    }

    /// Whether a state change has already been marked but not yet applied,
    /// so the semantics the last flush produced are stale.
    ///
    /// This is the *unapplied* half of [`Self::has_scheduled_semantic_work`]:
    /// a signal fired and marked its owner, and the next flush will show a
    /// different tree. It deliberately excludes work that merely continues
    /// over future frames — animations, gesture deadlines, gliding scrolls —
    /// because those never stop asking, so a caller that waits on them waits
    /// forever. An observer that needs to see the current state waits on
    /// this; one that needs the app to come fully to rest waits on
    /// `has_scheduled_semantic_work`.
    #[must_use]
    pub fn has_pending_semantic_update(&self) -> bool {
        self.root_is_dirty()
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
        self.window_display_transform = root_transform;
        // The activation-point projection intersects node bounds with the
        // window bounds alongside the node's clip chain — both in the same
        // window hit-test space the hit clip stack uses.
        self.hit_test.window_bounds = bounds;
    }

    /// The renderer state and the recording node's trailing run, split so
    /// text shaping can read the one while drawing into the other.
    pub(crate) fn state_and_run_mut(&mut self) -> (&mut HydroState, &mut Recording) {
        let run = self
            .program
            .last_mut()
            .expect("hydrolysis renderer: drawing with no recording node")
            .run();
        (&mut self.core.state, run)
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

    /// Drops the recorded scene and the hit-test state derived from it.
    ///
    /// The hit registries are retained: clearing them would lose what the
    /// kept nodes own — materialization rebuilds the flat lists, so only
    /// the gesture engine's per-frame target list drains here.
    pub fn reset_scene(&mut self) {
        self.gesture_engine.clear_targets();
        self.text_editing.text_input_targets.clear();
        self.state.measurement.reset_counters();
        self.state.counters.reset_frame();
        #[cfg(feature = "accessibility")]
        self.accessibility.reset_scene();
    }

    /// Marks the start of a full rebuild frame.
    pub fn begin_rebuild_frame(&mut self) {
        // A full rebuild re-dispatches every Dynamic node, so any pending isolated
        // reactive patch is subsumed by it.
        self.begin_rebuild();
        self.core.begin_outside_read_frame();
        self.state.measurement.begin_frame();
        self.hit_test.begin_rebuild_frame();
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.begin_animation_rebuild();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
        // The frame's window-level record: registrations emitted with no
        // enclosing node — test-side binds and transient passes alike —
        // attribute to the root's own record, closed in
        // `finish_rebuild_frame`.
        let root = self.core.root_core.clone();
        self.core.enter_reader(&root, ReaderPhase::Record, true);
        self.program
            .push(mount::ProgramBuilder::new(Rc::clone(&root.cell)));
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
    }

    /// Ends the rebuild pass and stores the root's program for the commit.
    ///
    /// # Panics
    /// Panics when a node program is left open at the end of the frame.
    pub fn finish_rebuild_frame(&mut self) {
        let program = self
            .program
            .pop()
            .expect("hydrolysis renderer: finish_rebuild_frame without its root program")
            .finish();
        let root = self.core.root_core.clone();
        root.retained.stage(program);
        root.cell.mark_quiet(mount::Dirty::COMMIT);
        assert!(
            self.program.is_empty(),
            "hydrolysis renderer: a node program is still open at the end of a rebuild"
        );
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

        // The frame's window-level record closes before materialization:
        // the leave retires every subtree the root record left unplaced,
        // so the flat lists rebuild against the post-retire registries.
        self.core.leave_reader(None);

        self.core.registries();
        // Same frame-end record `flush_window_tree` runs: the first-frame
        // build ends here, not in the flush tail, and the placements it
        // registered publish only through this drain.
        self.core.record_platform_views();
        self.validate_focused_text_input_after_flush();

        self.core.retire_unbound_animation_slots();
        self.core
            .hit_test
            .finish_rebuild_frame(&self.core.text_editing.text_input_targets);
        self.relocate_dropped_focus();
        self.clear_focused_fields();
        self.core.navigation.finish_rebuild_frame();
        self.core.finish_outside_read_frame();
        self.core.finish_rebuild();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
    }

    /// The recording node's trailing run (§C): drawing lands in it.
    ///
    /// # Panics
    /// Panics when no node is recording.
    pub fn scene_mut(&mut self) -> &mut Recording {
        self.program().run()
    }

    /// The recording node's program builder.
    pub(crate) fn program(&mut self) -> &mut mount::ProgramBuilder {
        self.program
            .last_mut()
            .expect("hydrolysis renderer: drawing with no recording node")
    }

    /// The opacity the open programs apply at the current record point —
    /// whether content recorded here can show at all.
    pub(crate) fn record_alpha(&self) -> f32 {
        self.program
            .iter()
            .map(mount::ProgramBuilder::alpha)
            .product()
    }

    /// `local` — record space of the recording node — in window space,
    /// through the node's structural placement chain.
    pub(crate) fn record_world(&self, local: kurbo::Affine) -> kurbo::Affine {
        let program = self
            .program
            .last()
            .expect("hydrolysis renderer: record_world with no recording node");
        program.cell().placement().resolved_transform(true) * local
    }

    pub(crate) fn draw_context(
        &mut self,
        ctx: RenderContext,
        body: impl FnOnce(&mut waterui_graphics::draw::Recorder),
    ) {
        self.scene_mut().record_picture(ctx.local, body);
    }

    /// Records `core`'s node into its own program (§C): the parent's list
    /// (or the window when none records) gets an `Item::Node`, the node's
    /// frame anchors on the placement that list resolves against, and the
    /// node draws from its frame origin with `local = IDENTITY`.
    pub(crate) fn record_layer_node(
        &mut self,
        core: &NodeCore,
        bounds: kurbo::Rect,
        record: impl FnOnce(&mut Self, RenderContext),
    ) {
        self.record_layer_node_in(core, bounds, true, record);
    }

    /// Records a layer node; `listed: false` leaves it out of its parent's
    /// program, for a caller that lists it elsewhere and sets its anchor.
    pub(crate) fn record_layer_node_in(
        &mut self,
        core: &NodeCore,
        bounds: kurbo::Rect,
        listed: bool,
        record: impl FnOnce(&mut Self, RenderContext),
    ) {
        let cell = Rc::clone(&core.cell);
        if listed {
            let anchor = if self.program.is_empty() {
                self.current_placement()
            } else {
                let parent = self.program();
                parent.push_node(Rc::clone(&cell));
                parent.anchor()
            };
            *core.retained.anchor.borrow_mut() = Some(anchor);
        }
        let retained = Rc::clone(&core.retained);
        self.with_reader(core, ReaderPhase::Record, |renderer| {
            renderer
                .program
                .push(mount::ProgramBuilder::new(Rc::clone(&cell)));
            record(
                renderer,
                RenderContext {
                    local: kurbo::Affine::IDENTITY,
                    bounds,
                },
            );
            let program = renderer
                .program
                .pop()
                .expect("hydrolysis renderer: a node's program left the stack during its record")
                .finish();
            retained.stage(program);
            cell.mark_quiet(mount::Dirty::COMMIT);
        });
    }

    /// Records a presentation host's program (§D): the host's reader and
    /// its own program, anchored on the window.
    pub(crate) fn record_host(&mut self, host: &NodeCore, record: impl FnOnce(&mut Self)) {
        let anchor = host
            .cell
            .placement()
            .parent()
            .expect("hydrolysis renderer: a presentation host is not under the window");
        *host.retained.anchor.borrow_mut() = Some(anchor);
        // The root records into the frame's window-level program, which
        // `finish_rebuild_frame` stores.
        if let [frame] = self.program.as_slice()
            && Rc::ptr_eq(frame.cell(), &host.cell)
        {
            self.with_reader(host, ReaderPhase::Record, record);
            return;
        }
        let retained = Rc::clone(&host.retained);
        let saved = core::mem::take(&mut self.program);
        self.with_reader(host, ReaderPhase::Record, |renderer| {
            renderer
                .program
                .push(mount::ProgramBuilder::new(Rc::clone(&host.cell)));
            record(renderer);
            let program = renderer
                .program
                .pop()
                .expect("hydrolysis renderer: a host's program left the stack during its record")
                .finish();
            retained.stage(program);
            host.cell.mark_quiet(mount::Dirty::COMMIT);
        });
        self.program = saved;
    }

    /// Opens the named scope `key` on the recording node's program (§C):
    /// its layer groups at `alpha` under `clip`, and a placement scope with
    /// the caller's record-space `scope` delta resolves the registrations
    /// inside. `paint` is the transform `clip` is given under, in the
    /// recording node's space; the clip is stored in the scope's own space.
    /// The layer sits at the scope's record-space delta.
    pub(crate) fn open_scope(
        &mut self,
        key: mount::ScopeKey,
        alpha: f32,
        paint: kurbo::Affine,
        clip: &ScopeClip,
        scope: crate::renderer::ScopeDelta,
    ) {
        self.open_scope_with(key, scope.transform, alpha, Some(clip), paint, scope, true);
    }

    /// [`Self::open_scope`] with the scope layer's own transform `layer`
    /// (in the enclosing scope's space), an optional clip given under
    /// `clip_space`, and the hit state: `hit: false` keeps the scope's
    /// content painted while nothing inside it takes input. The placement
    /// scope still takes the caller's explicit `scope` delta.
    #[expect(
        clippy::too_many_arguments,
        reason = "a scope's layer transform, clip and placement delta are independent inputs"
    )]
    pub(crate) fn open_scope_with(
        &mut self,
        key: mount::ScopeKey,
        layer: kurbo::Affine,
        alpha: f32,
        clip: Option<&ScopeClip>,
        clip_space: kurbo::Affine,
        scope: crate::renderer::ScopeDelta,
        hit: bool,
    ) {
        let space = self.program().origin() * layer;
        let shape = clip.map(|clip| clip.in_space(space.inverse() * clip_space));
        self.push_placement_scope(scope, clip.map(ScopeClip::hit_bounds));
        if !hit {
            let gate = crate::renderer::HitGate::Inactive;
            let placement = self.current_placement();
            placement.set_removes(gate.removes());
            placement.set_alpha(gate.alpha());
        }
        let placement = self.current_placement();
        self.program().open_scope(
            key,
            mount::ScopeProps {
                transform: layer,
                clip: shape,
                alpha,
            },
            placement,
        );
    }

    /// Closes the scope [`Self::open_scope`] opened.
    pub(crate) fn close_scope(&mut self) {
        self.program().close_scope();
        self.pop_placement_scope();
    }

    /// Runs `f` inside the named scope `key` — the lexical pairing of
    /// [`Self::open_scope`] and [`Self::close_scope`].
    pub(crate) fn with_scope(
        &mut self,
        key: mount::ScopeKey,
        alpha: f32,
        paint: kurbo::Affine,
        clip: &ScopeClip,
        scope: crate::renderer::ScopeDelta,
        f: impl FnOnce(&mut Self),
    ) {
        self.open_scope(key, alpha, paint, clip, scope);
        f(self);
        self.close_scope();
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

    /// The engine work the last commit did.
    pub(crate) const fn mount_stats(&self) -> mount::MountStats {
        self.last_mount_stats
    }
}

/// A scope's clip silhouette, given in the recording node's space under the
/// paint transform [`HydrolysisRenderer::open_scope`] receives.
pub enum ScopeClip {
    Rect(kurbo::Rect),
    Path(kurbo::BezPath),
    /// A rounded-rect clip: `path` is its silhouette, `rect` its hit bounds.
    RoundedRect {
        path: kurbo::BezPath,
        rect: kurbo::Rect,
    },
}

impl ScopeClip {
    pub(crate) fn in_space(&self, transform: kurbo::Affine) -> waterui_graphics::draw::ShapeData {
        if transform == kurbo::Affine::IDENTITY {
            return match self {
                Self::Rect(rect) => waterui_graphics::draw::ShapeData::of(rect),
                Self::Path(path) | Self::RoundedRect { path, .. } => {
                    waterui_graphics::draw::ShapeData::of(path)
                }
            };
        }
        let mut path = match self {
            Self::Rect(rect) => rect.to_path(waterui_graphics::draw::PATH_TOLERANCE),
            Self::Path(path) | Self::RoundedRect { path, .. } => path.clone(),
        };
        path.apply_affine(transform);
        waterui_graphics::draw::ShapeData::of(&path)
    }

    /// The hit clip in the placement scope's space: the clip's bounds as the
    /// caller gave them.
    pub(crate) fn hit_bounds(&self) -> kurbo::Rect {
        match self {
            Self::Rect(rect) | Self::RoundedRect { rect, .. } => *rect,
            Self::Path(path) => path.bounding_box(),
        }
    }
}
