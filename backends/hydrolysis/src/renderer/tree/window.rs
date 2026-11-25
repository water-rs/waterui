//! The window-level entry points: build-or-patch the retained window tree
//! ([`HydrolysisRenderer::capture_window_tree`]) and the per-frame pass
//! ([`HydrolysisRenderer::flush_window_tree`]), plus [`RenderNode::patch`].

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::platform::{WindowKeyboardArea, WindowSafeArea};
use waterui_core::contract_violation;
use waterui_layout::padding::EdgeInsets;

/// The window's container-region insets in logical units, read through the
/// renderer so the frame subscribes to them — a host write on the
/// [`WindowSafeArea`] binding re-lays out through the ordinary input path with
/// no explicit refresh request. An environment that carries none reports zero
/// insets, which is also what the whole window pipeline saw before.
pub(super) fn window_container_insets<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    env: &Environment,
) -> EdgeInsets {
    env.get::<WindowSafeArea>()
        .map_or_default(|area| renderer.read_signal(&area.0))
}

/// The window's keyboard-region insets in logical units — §7.1's keyboard
/// region: the software keyboard's own cover of the window, tracked
/// frame-by-frame as the host animates it, which never resizes the window
/// or the rendering surface.
pub(super) fn window_keyboard_insets<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    env: &Environment,
) -> EdgeInsets {
    env.get::<WindowKeyboardArea>()
        .map_or_default(|area| renderer.read_signal(&area.0))
}

/// `bounds` shrunk by the deeper of the two regions on each edge — §7.1's
/// "a view is laid out clear of the regions it does not ignore", applied
/// at the window root. Clamps per axis so insets larger than the window
/// collapse to the origin rather than inverting the rect.
/// [`ReleasedRegions::unreleased_depth`](safe_area::ReleasedRegions::unreleased_depth)
/// owns the `max(container, keyboard)` resolution — the one place it lives.
/// `direction` is the publish direction the insets' leading and trailing
/// resolve under (water-rs/waterui#1827).
pub fn window_content_rect(
    bounds: kurbo::Rect,
    container: &EdgeInsets,
    keyboard: &EdgeInsets,
    direction: waterui_core::layout::LayoutDirection,
) -> kurbo::Rect {
    let none = safe_area::ReleasedRegions::default();
    let x0 =
        bounds.x0 + none.unreleased_depth(safe_area::Edge::Leading, container, keyboard, direction);
    let y0 =
        bounds.y0 + none.unreleased_depth(safe_area::Edge::Top, container, keyboard, direction);
    let x1 = (bounds.x1
        - none.unreleased_depth(safe_area::Edge::Trailing, container, keyboard, direction))
    .max(x0);
    let y1 = (bounds.y1
        - none.unreleased_depth(safe_area::Edge::Bottom, container, keyboard, direction))
    .max(y0);
    kurbo::Rect::new(x0, y0, x1, y1)
}

/// The root layout inputs for one window frame: the laid-out content rect
/// (the window shrunk by the deeper region on each edge), its `Size`, and
/// the §7.1 [`safe_area::SafeAreaLayout`] context the tree's root node lays out
/// against — computed once per frame so the root `RenderContext` and the
/// context's frame agree exactly. `transform` places the window in device
/// pixels: the touch tolerance derives from its per-axis scale, never a
/// literal.
pub fn window_root_layout<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    env: &Environment,
    bounds: kurbo::Rect,
    transform: kurbo::Affine,
) -> (kurbo::Rect, Size, safe_area::SafeAreaLayout) {
    let container = window_container_insets(renderer, env);
    let keyboard = window_keyboard_insets(renderer, env);
    // The direction the regions' leading and trailing resolve under —
    // read as a signal so a direction change re-lays the window out
    // (water-rs/waterui#1827).
    let direction = renderer.read_signal(&waterui_layout::layout_direction(env));
    let content = window_content_rect(bounds, &container, &keyboard, direction);
    let size = Size::new(
        crate::num_cast::f64_as_f32(content.width()),
        crate::num_cast::f64_as_f32(content.height()),
    );
    let frame = kurbo::Rect::new(
        content.x0,
        content.y0,
        content.x0 + f64::from(size.width),
        content.y0 + f64::from(size.height),
    );
    let [xx, yx, xy, yy, _, _] = transform.as_coeffs();
    (
        content,
        size,
        safe_area::SafeAreaLayout::root(
            bounds,
            container,
            keyboard,
            frame,
            xx.hypot(yx),
            xy.hypot(yy),
            direction,
        ),
    )
}

/// The [`RenderContext`] the window tree flushes under: `bounds` inset to the
/// safe area, with the content rect's origin folded into the transforms so
/// placement, drawing and hit-testing all agree the window begins at (0, 0).
/// Returns the [`RenderContext`] plus `hit_delta`, the root node's
/// placement transform in hit space: `hit_transform * shift`, the hit-space
/// composition of the content-rect origin the paint transform folds in.
fn safe_area_context(
    content: kurbo::Rect,
    hit_transform: kurbo::Affine,
) -> (RenderContext, kurbo::Affine) {
    let shift = kurbo::Affine::translate((content.x0, content.y0));
    (
        RenderContext::new(
            shift,
            kurbo::Rect::new(0.0, 0.0, content.width(), content.height()),
        ),
        hit_transform * shift,
    )
}

/// Result of [`RenderNode::patch`]: whether structural teardown/animation-slot
/// retirement is required (`structural`) and whether any node's geometry inputs
/// mutated (`layout`). Viewport-only row materialization in a [`LazyStackNode`]
/// updates mounted row subtrees (`structural`) without changing the stack's
/// estimated extent (`!layout`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PatchOutcome {
    pub(crate) structural: bool,
    pub(crate) layout: bool,
}

impl PatchOutcome {
    pub(crate) const NONE: Self = Self {
        structural: false,
        layout: false,
    };

    pub(crate) const fn both(changed: bool) -> Self {
        Self {
            structural: changed,
            layout: changed,
        }
    }
}

impl core::ops::BitOr for PatchOutcome {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            structural: self.structural | rhs.structural,
            layout: self.layout | rhs.layout,
        }
    }
}

impl core::ops::BitOrAssign for PatchOutcome {
    fn bitor_assign(&mut self, rhs: Self) {
        self.structural |= rhs.structural;
        self.layout |= rhs.layout;
    }
}

/// Cached root layout state and layout-pass signal dependencies for the
/// window's retained [`RenderNode`] tree.
pub struct WindowLayoutState<E: crate::text::TextEngine> {
    laid_out: Size,
    laid_out_proposal: Option<ProposalSize>,
    laid_out_area: Option<Box<safe_area::SafeAreaLayout>>,
    needs_layout: bool,
    dependencies: Option<LayoutDependencies<E>>,
}

impl<E: crate::text::TextEngine> WindowLayoutState<E> {
    pub(crate) fn new(root: Weak<NodeCell<E>>) -> Self {
        Self {
            laid_out: Size::zero(),
            laid_out_proposal: None,
            laid_out_area: None,
            needs_layout: true,
            dependencies: Some(LayoutDependencies::new(root)),
        }
    }

    #[cfg(any(test, hydrolysis_cherenkov))]
    pub(crate) fn invalidate(&mut self) {
        self.laid_out_proposal = None;
        self.laid_out_area = None;
        self.needs_layout = true;
    }
}

impl<E: crate::text::TextEngine> RenderNode<E> {
    /// Apply pending reactive `Dynamic` content changes by rebuilding only the
    /// affected child subtree — no whole-window re-dispatch. Returns which
    /// structural and layout boundaries moved; the caller re-runs layout when
    /// `layout` is set, letting a size-changing swap reflow its ancestors
    /// without resetting the scene and re-dispatching.
    pub(crate) fn patch(&mut self, renderer: &mut SemanticCore<E>) -> PatchOutcome {
        // No environment is threaded through: a rebuild uses the node's own captured
        // environment (`Dynamic`/`Collection`/`Env` carry it), so the walk only needs
        // the renderer.
        match self {
            Self::Dynamic(node) => {
                // Apply the swap first (detaching an outgoing instance into
                // its ghost and starting an incoming instance's clock), then
                // advance every clock this host owns.
                let changed = if node.apply_pending(renderer) {
                    PatchOutcome::both(true)
                } else {
                    node.child.borrow_mut().patch(renderer)
                };
                PatchOutcome::both(node.advance_transitions(renderer)) | changed
            }
            Self::Container(container) => {
                let mut changed = PatchOutcome::NONE;
                for child in &mut container.children {
                    changed |= child.patch(renderer);
                }
                changed
            }
            Self::Opacity(node) => node.child.patch(renderer),
            Self::Scale(node) => node.child.patch(renderer),
            Self::Rotation(node) => node.child.patch(renderer),
            Self::Offset(node) => node.child.patch(renderer),
            Self::Retain(node) => node.child.patch(renderer),
            Self::Env(node) => node.child.patch(renderer),
            Self::Wrapper(node) => node.child.patch(renderer),
            Self::Collection(node) => {
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
                let mut changed =
                    PatchOutcome::both(membership_changed | node.advance_transitions(renderer));
                for entry in &mut node.entries {
                    changed |= entry.node.patch(renderer);
                }
                changed
            }
            Self::Scroll(node) => {
                // A patched subtree may measure differently: drop the cached
                // `0`-probe floor so the next window-minimum pass re-measures
                // it on a frame that already does layout work.
                let changed = node.child.patch(renderer);
                if changed.layout {
                    node.non_scrolling_minimum.set(None);
                }
                changed
            }
            // A filtered view wraps a child render node whose reactive
            // descendants must keep patching, so the walk recurses into it
            // (the filter itself owns its runtime, with no structural patch).
            #[cfg(waterui_feature = "effects")]
            Self::Filtered(node) => node.child.patch(renderer),
            // A paint marker only: its child patches as on its own.
            Self::Fill(node) => node.child.patch(renderer),
            Self::Color(_) | Self::Text(_) | Self::Widget(_) => PatchOutcome::NONE,
            #[cfg(hydrolysis_scene_view)]
            Self::SceneView(_) => PatchOutcome::NONE,
            #[cfg(hydrolysis_gpu)]
            Self::GpuContent(_) | Self::ExternalFrame(_) => PatchOutcome::NONE,
            // A lazy stack keeps only visible item subtrees. Patch those retained
            // items before parent layout so a Dynamic row-height change updates the
            // scroll extent in the same refresh instead of one frame later.
            Self::LazyStack(node) => node.patch_visible(renderer),
        }
    }

    /// Consume the subtree's layout-invalidated marks: `true` when any
    /// `FixedContainer` in the subtree had a `Layout::watch_invalidation`
    /// subscription fire, or any text leaf's content or alignment changed, since
    /// the last consume, meaning `place` must re-run even where an outer
    /// `RetainedSubview` reports an unchanged rect and proposal. Walks the same
    /// child-bearing variants as [`Self::patch`]. Every visited mark is
    /// cleared, so a stale mark cannot force a second relayout.
    pub(super) fn take_layout_dirty(&mut self) -> bool {
        match self {
            Self::Container(node) => {
                let own = node.layout_dirty.replace(false);
                node.children
                    .iter_mut()
                    .fold(own, |dirty, child| child.take_layout_dirty() | dirty)
            }
            Self::Opacity(node) => node.child.take_layout_dirty(),
            Self::Scale(node) => node.child.take_layout_dirty(),
            Self::Rotation(node) => node.child.take_layout_dirty(),
            Self::Offset(node) => node.child.take_layout_dirty(),
            Self::Retain(node) => node.child.take_layout_dirty(),
            Self::Env(node) => node.child.take_layout_dirty(),
            Self::Wrapper(node) => node.child.take_layout_dirty(),
            Self::Dynamic(node) => {
                node.layout_dirty.replace(false) | node.child.borrow_mut().take_layout_dirty()
            }
            Self::Scroll(node) => {
                let dirty = node.child.take_layout_dirty();
                if dirty {
                    node.non_scrolling_minimum.set(None);
                }
                dirty
            }
            #[cfg(waterui_feature = "effects")]
            Self::Filtered(node) => node.child.take_layout_dirty(),
            Self::Fill(node) => node.child.take_layout_dirty(),
            Self::Collection(node) => node
                .entries
                .iter_mut()
                .fold(false, |dirty, entry| entry.node.take_layout_dirty() | dirty),
            Self::LazyStack(node) => node.item_cache.borrow_mut().take_layout_dirty(),
            Self::Text(node) => node.layout_dirty.replace(false),
            // These leaves' measured size can change after mount — a
            // reloaded scene's natural size, a stream learning its size
            // once decoding starts — and they hold no consumable mark;
            // `size_drifted` re-probes the last proposal and answers the
            // drift, which the next `measure` clears by rewriting
            // `measured` (water-rs/waterui#1957).
            #[cfg(hydrolysis_scene_view)]
            Self::SceneView(node) => node.size_drifted(),
            #[cfg(hydrolysis_gpu)]
            Self::GpuContent(node) => node.size_drifted(),
            #[cfg(hydrolysis_gpu)]
            Self::ExternalFrame(node) => node.size_drifted(),
            // A widget's measured size is a function of the
            // `RetainedSubview`s it owns (labels, nav bars, badges):
            // their marks are its marks — each sub-view keeps a consumed
            // mark in its own `needs_layout`, so reporting it here does
            // not starve the sub-view's own flush.
            Self::Widget(node) => node.behavior.take_layout_dirty(),
            Self::Color(_) => false,
        }
    }
}

impl<E: crate::text::TextEngine> SemanticCore<E> {
    /// The emit-pass equivalent of the `SemanticCore` half of
    /// `HydrolysisRenderer::reset_scene`: clears every pure-emission registry
    /// the accessibility walk re-pushes — input targets, gesture targets,
    /// text-input targets and the accessibility builder's node/action
    /// state — so each walk re-registers exactly what is live. The rendered
    /// pump runs this after the structural patch, so state the patch orphaned
    /// drops in the renderer's teardown order rather than ahead of it.
    #[cfg(hydrolysis_cherenkov)]
    fn reset_semantic_scene(&mut self) {
        self.gesture_engine.clear_targets();
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        self.text_editing.text_input_targets.clear();
        self.state.measurement.reset_counters();
        #[cfg(feature = "accessibility")]
        self.accessibility.reset_scene();
    }

    /// The emit-pass equivalent of the subsystem frame opens in
    /// `HydrolysisRenderer::flush_window_tree` — the Retain watcher rollover
    /// plus the input, lazy and navigation registries' begins. The scene
    /// clears stay in `reset_semantic_scene`, which the pump runs after the
    /// patch like the rendered path.
    #[cfg(hydrolysis_cherenkov)]
    fn begin_semantic_emit_frame(&mut self) {
        self.hit_test.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        #[cfg(feature = "navigation")]
        self.navigation.begin_rebuild_frame();
    }

    /// The emit-pass equivalent of the `SemanticCore` half of
    /// `HydrolysisRenderer::begin_rebuild_frame` — the build path's frame
    /// opens, including the animation and accessibility registries the pump
    /// path leaves to `reset_semantic_scene`.
    /// `signals.begin_rebuild` stays with the caller — only a build enters one.
    #[cfg(hydrolysis_cherenkov)]
    fn begin_semantic_rebuild_frame(&mut self) {
        self.state.measurement.begin_frame();
        self.hit_test.begin_rebuild_frame();
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.begin_animation_rebuild();
        self.lazy.begin_rebuild_frame();
        #[cfg(feature = "navigation")]
        self.navigation.begin_rebuild_frame();
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
    }

    /// The emit-pass equivalent of the non-scene half of
    /// `HydrolysisRenderer::finish_rebuild_frame` — the build path: Retain
    /// watcher rollover, the animation-slot prune, focus validation, and the
    /// accessibility tree's publication.
    /// `signals.finish_rebuild` stays with the caller — only a build entered
    /// one.
    #[cfg(hydrolysis_cherenkov)]
    fn finish_semantic_rebuild_frame(&mut self) {
        self.validate_focused_text_input_after_flush();
        self.retire_unbound_animation_slots();
        self.hit_test.finish_rebuild_frame(
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
            &self.text_editing.text_input_targets,
        );
        self.relocate_dropped_focus();
        #[cfg(feature = "navigation")]
        self.navigation.finish_rebuild_frame();
        self.finish_outside_read_frame();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
    }

    /// The emit-pass equivalent of `HydrolysisRenderer::flush_window_tree`'s
    /// finish ordering — the pump path: the input and navigation teardowns run
    /// while the frame's retained subscriptions are still held, then the
    /// structural-change prunes, then the Retain watcher rollover last, then
    /// focus validation/relocation and the accessibility tree's publication.
    /// Releasing in the renderer's order is what exposes same-manager watcher
    /// re-entrancy (water-rs/waterui#1213) to `#[waterui::test]`.
    #[cfg(hydrolysis_cherenkov)]
    fn finish_semantic_emit_frame(&mut self, structural_change: bool) {
        self.frame_registries();
        self.hit_test.finish_rebuild_frame(
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
            &self.text_editing.text_input_targets,
        );
        #[cfg(feature = "navigation")]
        self.navigation.finish_rebuild_frame();
        self.finish_outside_read_frame();
        if structural_change {
            // The emit re-bound every live animation slot; drop the slots
            // belonging to subtrees the patch removed, before the frame's
            // retained subscriptions release.
            self.retire_unbound_animation_slots();
        }
        self.validate_focused_text_input_after_flush();
        self.relocate_dropped_focus();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
    }

    /// Build the retained window tree from `content` and emit its
    /// accessibility tree — the semantic analogue of
    /// [`HydrolysisRenderer::capture_window_tree`]: dispatch and emission only,
    /// with no layout, no encode and no theme.
    ///
    /// Like the rendered path, a call made with a tree already built applies
    /// the pending patch and re-emits instead of re-dispatching.
    #[cfg(hydrolysis_cherenkov)]
    pub(crate) fn capture_window_semantics(&mut self, content: impl View, env: &Environment) {
        if self.render_tree.is_some() {
            assert!(
                self.flush_window_semantics(env),
                "hydrolysis renderer: retained window tree vanished during semantics capture"
            );
            return;
        }
        // Mirror `build_window_scene`: `reset_scene` runs before the frame
        // opens on the rendered build path.
        self.reset_semantic_scene();
        self.app_frame_rate_vote = None;
        self.begin_rebuild();
        self.begin_semantic_rebuild_frame();
        self.render_depth = 0;
        let tree = RenderNode::build(content, env, self);
        // Marks on the window tree propagate to the window's root cell,
        // which the pump polls.
        tree.core().cell.set_parent(self.root_cell());
        // The build is the flush for every mark it raised — registration
        // echoes, initial-content deliveries and read catch-ups were all
        // recorded by this pass, so only a mark raised after it may
        // schedule more work.
        self.clear_all_marks();
        let _ = self.signals.take_patch_request();
        #[cfg(feature = "accessibility")]
        {
            self.begin_emit_pass();
            tree.emit_accessibility(self, env);
            self.finish_emit_pass();
        }
        self.window_layout.invalidate();
        self.content_minimum = None;
        self.render_tree = Some(tree);
        self.finish_semantic_rebuild_frame();
        self.finish_rebuild();
    }

    /// Apply pending structural changes and re-emit the retained tree's
    /// accessibility tree without laying out or encoding — the semantic
    /// analogue of [`HydrolysisRenderer::flush_window_tree`]. Returns `false`
    /// if no tree is built.
    ///
    /// A `Dynamic` can reconnect (its initial update is gated on the rebuild
    /// generation), so patching is the only structural path here: a rebuild
    /// request would be a programmer error — re-dispatching `body()` is the
    /// one-time build's job.
    // `_env` is consumed only by the accessibility emit path; the signature must
    // match `capture_window_semantics` regardless of feature selection.
    #[allow(clippy::used_underscore_binding)]
    #[cfg(hydrolysis_cherenkov)]
    pub(crate) fn flush_window_semantics(&mut self, _env: &Environment) -> bool {
        let Some(mut tree) = self.render_tree.take() else {
            return false;
        };
        // Producer wakes posted since the last frame mark their owners
        // before the marks below are cleared.
        self.drain_producer_wakes();
        // The marks that brought this frame here are consumed by the flush;
        // clear them up front so a mark raised mid-flush re-arms the next.
        self.clear_all_marks();
        self.begin_outside_read_frame();
        self.app_frame_rate_vote = None;
        self.begin_semantic_emit_frame();
        let patch = tree.patch(self);
        let structural_change = self.take_subview_structural_change() | patch.structural;
        if structural_change {
            self.begin_animation_rebuild();
        }
        self.reset_semantic_scene();
        // The emit-side half of `begin_redraw_frame`: roll the per-frame
        // measurement cache before the walk.
        self.state.measurement.begin_frame();
        #[cfg(feature = "accessibility")]
        {
            self.begin_emit_pass();
            tree.emit_accessibility(self, _env);
            self.finish_emit_pass();
        }
        self.finish_semantic_emit_frame(structural_change);
        self.window_layout.needs_layout |= patch.layout;
        self.content_minimum = None;
        self.render_tree = Some(tree);
        true
    }
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// Refreshes text presentation without walking or laying out the view tree.
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    pub(crate) fn record_text_input_overlay(
        &mut self,
        env: &Environment,
        transform: kurbo::Affine,
    ) {
        let host = self.core.presentation_hosts.text_overlay.clone();
        self.record_host(&host, |renderer| {
            renderer.frame_registries();
            renderer.render_active_text_context_menu_overlay(env, transform);
            renderer.prepare_transient_text_input_overlay(env, transform);
        });
    }

    /// Records every presentation under its host cell — the text context
    /// menu overlay, the open `.context_menu` presentation and the anchored
    /// overlays — after the window content's flush. Each host's record
    /// retires what its previous record registered, so a closed
    /// presentation's occluders and targets leave with it. Every window
    /// pass — the capture paths included — runs it, so no path leaves a
    /// host's registrations from an earlier frame behind.
    fn record_presentation_hosts(
        &mut self,
        env: &Environment,
        transform: kurbo::Affine,
        safe_area: &SafeAreaLayout,
    ) {
        #[cfg(not(any(feature = "text-field", feature = "secure-field")))]
        let _ = env;
        // The overlay-mode text context menu re-encodes with the frame it
        // floats over; drawing it only on the one-time build path would
        // leave it visible for a single frame.
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        self.record_text_input_overlay(env, transform);
        // Same for an open `.context_menu` presentation: its dim backdrop,
        // lifted preview and anchored accessory re-encode per frame and the
        // pass is where dismiss_requests/menu-close is observed.
        #[cfg(feature = "menu")]
        {
            let host = self.core.presentation_hosts.context_menu.clone();
            self.record_host(&host, |renderer| {
                renderer.render_context_menu_presentation(transform, safe_area);
            });
        }
        // Anchored overlays (`.anchored_overlay`) draw above all content:
        // the flush registered each anchor's live bounds, so the placement
        // contract re-runs per frame and the overlay follows moves/resizes.
        let host = self.core.presentation_hosts.anchored.clone();
        self.record_host(&host, |renderer| {
            renderer.render_anchored_overlays(transform, safe_area);
        });
        // The `.dialog` modal layer draws above everything: scrim, card and
        // the modal input scope the card registers through its environment.
        #[cfg(waterui_feature = "dialog")]
        {
            let host = self.core.presentation_hosts.dialog.clone();
            self.record_host(&host, |renderer| {
                renderer.render_dialogs(transform, safe_area);
            });
        }
    }
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// Build the retained tree before its first sized frame, so statically
    /// reachable nodes exist before the first render target is presented.
    #[cfg(test)]
    pub(crate) fn prepare_window_tree(&mut self, content: impl View, env: &Environment) {
        assert!(
            self.render_tree.is_none(),
            "hydrolysis renderer: window tree prepared more than once"
        );
        self.begin_rebuild_frame();
        self.render_depth = 0;
        let tree = RenderNode::build(content, env, self);
        tree.core().cell.set_parent(self.root_cell());
        // Build-time marks are consumed by the build itself.
        self.clear_all_marks();
        let _ = self.signals.take_patch_request();
        self.window_layout.invalidate();
        self.content_minimum = None;
        self.render_tree = Some(tree);
        self.finish_rebuild_frame();
    }

    /// Runs `prepare_for_measure` and `layout` for the window's root tree,
    /// recording every signal read during `layout` into
    /// [`WindowLayoutState::dependencies`] so a skipped layout pass keeps
    /// layout-only signal dependencies subscribed.
    fn run_window_layout(
        &mut self,
        tree: &mut RenderNode<E>,
        env: &Environment,
        safe_area: &safe_area::SafeAreaLayout,
        proposal: ProposalSize,
        size: Size,
    ) {
        #[cfg(feature = "frame-profile")]
        let layout_started_at = Instant::now();
        let _ = tree.take_layout_dirty();
        tree.prepare_for_measure(self);
        let mut dependencies = self
            .core
            .window_layout
            .dependencies
            .take()
            .expect("hydrolysis renderer: retained window tree lost its layout dependency set");
        dependencies.begin_pass();
        self.state.measurement.begin_dependency_pass();
        assert!(
            self.state.layout_dependencies.is_none(),
            "hydrolysis renderer: window layout pass ran while another layout dependency set was active"
        );
        self.state.layout_dependencies = Some(dependencies);
        tree.layout(self, env, Some(safe_area.clone()), proposal, size);
        let mut dependencies =
            self.state.layout_dependencies.take().expect(
                "hydrolysis renderer: window layout pass emptied its layout dependency slot",
            );
        dependencies.finish_pass();
        self.core.window_layout.dependencies = Some(dependencies);
        self.core.window_layout.laid_out = size;
        self.core.window_layout.laid_out_proposal = Some(proposal);
        store_safe_area(
            &mut self.core.window_layout.laid_out_area,
            Some(safe_area.clone()),
        );
        self.core.window_layout.needs_layout = false;
        self.content_minimum = None;
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times
                .set_layout(self.frame_stage_times.layout() + layout_started_at.elapsed());
            self.last_layout_signature = Some(tree.placed_signature(Rect::from_size(size)));
        }
    }

    /// Build the window render tree from `content`, lay it out at `bounds`, and
    /// flush it into the scene — the render-tree analogue of
    /// `HydrolysisRenderer::capture_window_scene`. The built tree is retained in
    /// `render_tree` for subsequent per-frame flushes.
    pub fn capture_window_tree(
        &mut self,
        content: impl View,
        env: &Environment,
        bounds: kurbo::Rect,
        transform: kurbo::Affine,
        hit_transform: kurbo::Affine,
    ) {
        let _flush_span = tracing::debug_span!("hydrolysis_capture_window_tree").entered();
        self.app_frame_rate_vote = None;
        let (content_rect, size, safe_area) = window_root_layout(self, env, bounds, transform);
        let proposal = ProposalSize::new(Some(size.width), Some(size.height));
        // The viewport is recorded here rather than by each caller: every host
        // that builds a window tree — the runner, and a `HydrolysisGpuView`
        // embedding one in someone else's surface — has to agree on where the
        // window lands in device pixels, and forgetting to say so left the
        // embedded host reading a stale one.
        #[cfg(feature = "frame-profile")]
        let update_started_at = Instant::now();
        self.set_window_viewport(bounds, transform);
        let (ctx, hit_delta) = safe_area_context(content_rect, hit_transform);
        // The tree is built once and persists. A later "rebuild" request reuses
        // it — applying pending Dynamic patches, relaying out, and re-flushing —
        // rather than rebuilding (which would re-connect each `Dynamic`, and a
        // `Dynamic` can only connect once). Called within a begin/finish rebuild
        // frame, so scene/layer flushing is handled by the caller.
        if let Some(mut tree) = self.render_tree.take() {
            self.drain_producer_wakes();
            self.clear_all_marks();
            self.begin_outside_read_frame();
            let _ = tree.patch(self);
            #[cfg(feature = "frame-profile")]
            {
                self.frame_stage_times
                    .set_update(self.frame_stage_times.update() + update_started_at.elapsed());
            }
            self.run_window_layout(&mut tree, env, &safe_area, proposal, size);
            #[cfg(feature = "frame-profile")]
            let encode_started_at = Instant::now();
            self.present_window_backdrop(bounds, env);
            self.core.begin_emit_pass();
            // Window-level registrations — payloads emitted with no
            // enclosing node — record under the root's own record.
            let root = self.core.root_core.clone();
            self.record_host(&root, |renderer| {
                tree.flush(renderer, ctx, env, hit_delta);
            });
            self.record_presentation_hosts(env, kurbo::Affine::IDENTITY, &safe_area);
            self.core.finish_emit_pass();
            #[cfg(feature = "frame-profile")]
            {
                self.frame_stage_times
                    .set_encode(self.frame_stage_times.encode() + encode_started_at.elapsed());
            }
            self.render_tree = Some(tree);
            return;
        }
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times
                .set_update(self.frame_stage_times.update() + update_started_at.elapsed());
        }
        self.render_depth = 0;
        let mut node = RenderNode::build(content, env, self);
        node.core().cell.set_parent(self.root_cell());
        // The build is the flush for every mark it raised (dev's rebuild
        // generation dropped the same marks): only a mark raised by the
        // layout/flush passes below may schedule more work.
        self.clear_all_marks();
        let _ = self.signals.take_patch_request();
        self.run_window_layout(&mut node, env, &safe_area, proposal, size);
        #[cfg(feature = "frame-profile")]
        let encode_started_at = Instant::now();
        self.present_window_backdrop(bounds, env);
        self.core.begin_emit_pass();
        let root = self.core.root_core.clone();
        self.record_host(&root, |renderer| {
            node.flush(renderer, ctx, env, hit_delta);
        });
        self.record_presentation_hosts(env, kurbo::Affine::IDENTITY, &safe_area);
        self.core.finish_emit_pass();
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times
                .set_encode(self.frame_stage_times.encode() + encode_started_at.elapsed());
        }
        self.render_tree = Some(node);
    }

    /// Apply pending structural changes, run layout when structure, layout
    /// dependencies or window geometry/safe-area inputs changed, and re-encode
    /// the retained window tree without rebuilding it. Returns `false` if no
    /// tree is built.
    pub fn flush_window_tree(
        &mut self,
        env: &Environment,
        bounds: kurbo::Rect,
        transform: kurbo::Affine,
        hit_transform: kurbo::Affine,
    ) -> bool {
        let Some(mut tree) = self.render_tree.take() else {
            return false;
        };
        let _flush_span = tracing::debug_span!("hydrolysis_flush_window_tree").entered();
        // Track the live window viewport every frame: text-context-menu clamping,
        // effect-rect checks and the direct-to-target test read it.
        #[cfg(feature = "frame-profile")]
        let update_started_at = Instant::now();
        let update_span = tracing::debug_span!("hydrolysis_frame_update").entered();
        self.set_window_viewport(bounds, transform);
        // Producer wakes posted since the last frame mark their owners
        // before the marks below are cleared.
        self.drain_producer_wakes();
        let root_marks = self.root_marks();
        let marked_for_layout =
            root_marks.contains(Dirty::STRUCTURE) || root_marks.contains(Dirty::LAYOUT);
        // The marks that brought this frame here are consumed by the flush;
        // clear them up front so a mark raised mid-flush re-arms the next.
        self.clear_all_marks();
        self.begin_outside_read_frame();
        self.app_frame_rate_vote = None;
        // Roll over this frame's Retain watcher guards exactly like the build path:
        // every re-encode re-reads and re-subscribes reactive visual inputs.
        // Reset frame-bound input registrations. Scroll, list, and table state are
        // owned by their semantic retained nodes.
        self.hit_test.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        #[cfg(feature = "navigation")]
        self.navigation.begin_rebuild_frame();
        // Fold in a structural patch a widget-owned sub-view applied during
        // the previous frame's flush (mid-flush, past that frame's
        // bookkeeping window).
        let subview_structural = self.take_subview_structural_change();
        let patch = tree.patch(self);
        let structural_change = subview_structural | patch.structural;
        if structural_change {
            self.begin_animation_rebuild();
        }
        let tree_dirty = tree.take_layout_dirty();
        drop(update_span);
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times
                .set_update(self.frame_stage_times.update() + update_started_at.elapsed());
        }
        self.reset_scene();
        self.begin_redraw_frame();
        let (content_rect, size, safe_area) = window_root_layout(self, env, bounds, transform);
        let proposal = ProposalSize::new(Some(size.width), Some(size.height));
        let needs_layout = self.core.window_layout.needs_layout
            || marked_for_layout
            || subview_structural
            || patch.layout
            || tree_dirty
            || self
                .core
                .window_layout
                .dependencies
                .as_ref()
                .is_some_and(LayoutDependencies::is_dirty)
            || size != self.core.window_layout.laid_out
            || self.core.window_layout.laid_out_proposal != Some(proposal)
            || self.core.window_layout.laid_out_area.as_deref() != Some(&safe_area);
        if needs_layout {
            let _layout_span = tracing::debug_span!("hydrolysis_frame_layout").entered();
            self.run_window_layout(&mut tree, env, &safe_area, proposal, size);
        }
        let encode_span = tracing::debug_span!("hydrolysis_scene_encode").entered();
        #[cfg(feature = "frame-profile")]
        let encode_started_at = Instant::now();
        let (ctx, hit_delta) = safe_area_context(content_rect, hit_transform);
        self.present_window_backdrop(bounds, env);
        self.core.begin_emit_pass();
        // Window-level registrations — payloads emitted with no
        // enclosing node — record under the root's own record.
        let root = self.core.root_core.clone();
        self.record_host(&root, |renderer| {
            tree.flush(renderer, ctx, env, hit_delta);
        });
        self.record_presentation_hosts(env, kurbo::Affine::IDENTITY, &safe_area);
        self.core.finish_emit_pass();
        drop(encode_span);
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times
                .set_encode(self.frame_stage_times.encode() + encode_started_at.elapsed());
        }
        self.core.frame_registries();
        // The materialized platform-view placements write into their tables
        // once, here at frame end — never mid-materialization.
        self.core.record_platform_views();
        self.core.hit_test.finish_rebuild_frame(
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
            &self.core.text_editing.text_input_targets,
        );
        #[cfg(feature = "navigation")]
        self.core.navigation.finish_rebuild_frame();
        self.core.finish_outside_read_frame();
        if structural_change {
            // The flush re-bound every live animation. Drop slots belonging
            // to subtrees removed by the patch.
            self.core.retire_unbound_animation_slots();
        }
        // Drop focus or drag targets that are no longer emitted, relocate the
        // focus a dropped view released, then publish the refreshed
        // accessibility tree.
        self.validate_focused_text_input_after_flush();
        self.relocate_dropped_focus();
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        self.clear_focused_fields();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
        self.render_tree = Some(tree);
        true
    }

    /// Measures the window content's minimum size, or `None` before the
    /// tree is built.
    ///
    /// This is a whole-tree measure pass at a proposal the frame's own
    /// layout never uses, so it is demand-driven rather than run on every
    /// refresh: only the runner calls it, and only once it knows the answer
    /// will reach a window that acts on it (see `apply_window_size_limits`).
    /// The window contributes no maximum: content that does not stretch on an
    /// axis is laid out inside a larger offer per the layout spec, so an app
    /// pins a maximum only through `Window::max_size`.
    pub(crate) fn measure_content_minimum(&mut self, env: &Environment) -> Option<Size> {
        if let Some(minimum) = self.content_minimum {
            return Some(minimum);
        }
        let tree = self.render_tree.take()?;
        let theme = self.theme();
        // Both axes are probed together, not independently: what a view
        // answers on one axis depends on what the other was offered — text
        // re-wraps at the minimum width and then needs more height than its
        // single-line ideal. `ProposalSize::ZERO` asks for the smallest
        // self-consistent box the content can occupy.
        let min_box = tree
            .measure(&mut self.state, env, &theme, ProposalSize::ZERO)
            .size;
        self.render_tree = Some(tree);
        let minimum = Size::new(
            validated_minimum_axis(min_box.width, "width"),
            validated_minimum_axis(min_box.height, "height"),
        );
        self.content_minimum = Some(minimum);
        Some(minimum)
    }

    /// The retained window tree's root, for render-identity probes.
    #[cfg(test)]
    pub(crate) fn render_tree_root(&self) -> Option<&RenderNode<E>> {
        self.render_tree.as_ref()
    }
}

/// `value` when it is a finite non-negative minimum, else `0.0` after a
/// contract violation: the window then has no minimum on that axis.
fn validated_minimum_axis(value: f32, axis: &str) -> f32 {
    if value.is_finite() && value >= 0.0 {
        return value;
    }
    contract_violation!(
        "hydrolysis window layout reported invalid minimum {axis}: {value}; the window has no minimum {axis}"
    );
    0.0
}

#[cfg(test)]
mod tests {
    use super::validated_minimum_axis;

    #[test]
    fn a_valid_minimum_axis_is_kept() {
        assert!((validated_minimum_axis(120.0, "width") - 120.0).abs() < f32::EPSILON);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "reported invalid minimum height")
    )]
    fn an_invalid_minimum_axis_leaves_the_window_without_a_minimum() {
        assert!(validated_minimum_axis(f32::NAN, "height").abs() < f32::EPSILON);
    }
}
