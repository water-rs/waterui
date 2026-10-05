//! The window-level entry points: build-or-patch the retained window tree
//! ([`HydrolysisRenderer::capture_window_tree`]) and the per-frame pass
//! ([`HydrolysisRenderer::flush_window_tree`]), plus [`RenderNode::patch`].

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::platform::{
    AccumulatedSafeAreaIgnores, InsideScrollSurface, WindowKeyboardArea, WindowSafeArea,
};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::safe_area::IgnoreSafeArea;

/// The window's container-region insets in logical units, read through the
/// renderer so the frame subscribes to them — a host write on the
/// [`WindowSafeArea`] binding re-lays out through the ordinary input path with
/// no explicit refresh request. An environment that carries none reports zero
/// insets, which is also what the whole window pipeline saw before.
pub(super) fn window_container_insets(
    renderer: &mut HydrolysisRenderer,
    env: &Environment,
) -> EdgeInsets {
    env.get::<WindowSafeArea>()
        .map(|area| renderer.read_signal(&area.0))
        .unwrap_or_default()
}

/// The window's keyboard-region insets in logical units — zero inside a
/// scroll surface's subtree, where the surface's content inset and
/// focused-field scrolling own the keyboard behaviour instead.
pub(super) fn window_keyboard_insets(
    renderer: &mut HydrolysisRenderer,
    env: &Environment,
) -> EdgeInsets {
    if env.get::<InsideScrollSurface>().is_some() {
        return EdgeInsets::default();
    }
    env.get::<WindowKeyboardArea>()
        .map(|area| renderer.read_signal(&area.0))
        .unwrap_or_default()
}

/// The insets a child of `env` avoids: the deeper of the two regions on each
/// edge — the rule §7.1's "layout avoids" applies whether the bands stack or
/// overlap.
pub(super) fn window_safe_area_insets(
    renderer: &mut HydrolysisRenderer,
    env: &Environment,
) -> EdgeInsets {
    let container = window_container_insets(renderer, env);
    let keyboard = window_keyboard_insets(renderer, env);
    EdgeInsets::new(
        container.top().max(keyboard.top()),
        container.bottom().max(keyboard.bottom()),
        container.leading().max(keyboard.leading()),
        container.trailing().max(keyboard.trailing()),
    )
}

/// Edge indices in mask/depth order: top, leading, bottom, trailing — the
/// `EdgeSet` order `AccumulatedSafeAreaIgnores` packs.
const EDGE_TOP: usize = 0;
const EDGE_LEADING: usize = 1;
const EDGE_BOTTOM: usize = 2;
const EDGE_TRAILING: usize = 3;
/// The mask bit marking that an `IgnoreSafeArea` declaration exists: the
/// accumulated mask of an `ignore_safe_area(NONE)` differs from no
/// declaration — it replaces a fill's default extension with none.
const IGNORER_MARK: u16 = 0x100;

/// The `AccumulatedSafeAreaIgnores` mask `ignore` declares: its regions'
/// edges in the shared `(region, edge)` order, plus the declaration mark.
pub(super) fn declared_ignore_mask(ignore: IgnoreSafeArea) -> u16 {
    let mut mask = IGNORER_MARK;
    for (region, base) in [
        (ignore.regions.container, 0u16),
        (ignore.regions.keyboard, 4u16),
    ] {
        if region {
            mask |= u16::from(ignore.edges.top) << base
                | u16::from(ignore.edges.leading) << (base + 1)
                | u16::from(ignore.edges.bottom) << (base + 2)
                | u16::from(ignore.edges.trailing) << (base + 3);
        }
    }
    mask
}

/// The mask `env`'s accumulated ignores resolve to for a node flushed under
/// it. `default` applies only when no declaration ever accumulated — the
/// fill/scroll default that extends through every region; a scroll
/// surface's reset (an accumulation carrying no declaration mark) restores
/// the same state for its subtree.
pub(super) fn accumulated_ignore_mask(env: &Environment, default: u16) -> u16 {
    env.get::<AccumulatedSafeAreaIgnores>()
        .map_or(default, |accumulated| {
            if accumulated.0 & IGNORER_MARK == 0 {
                default
            } else {
                accumulated.0 & 0xff
            }
        })
}

/// The depth the `edge` of a view with `mask` may reach through, given both
/// regions' depths at its frame: the shallowest region the mask does not
/// ignore — an extension never paints inside a zone it does not ignore —
/// the avoided depth itself when no region on the edge is ignored (no
/// extension), and zero when every region is ignored (the window edge).
const fn edge_target(mask: u16, edge: usize, container: f64, keyboard: f64) -> f64 {
    let marked = mask & (1 << edge) != 0 || mask & (1 << (edge + 4)) != 0;
    if !marked {
        return if container > keyboard {
            container
        } else {
            keyboard
        };
    }
    let unmarked_container = mask & (1 << edge) == 0;
    let unmarked_keyboard = mask & (1 << (edge + 4)) == 0;
    match (unmarked_container, unmarked_keyboard) {
        (true, true) => unreachable!(),
        (true, false) => container,
        (false, true) => keyboard,
        (false, false) => 0.0,
    }
}

/// The inset each edge of an `.ignore_safe_area` view releases: how much of
/// the avoided band the declared regions and edges give back — zero on
/// unflagged edges, the avoided depth minus the reachable target on flagged
/// ones. `container`/`keyboard` are the two regions' depths at the frame.
pub(super) fn released_insets(
    mask: u16,
    container: &EdgeInsets,
    keyboard: &EdgeInsets,
) -> [f64; 4] {
    let edge = |index: usize, c: f32, k: f32| {
        let avoided = f64::from(c.max(k));
        (avoided - edge_target(mask, index, f64::from(c), f64::from(k))).max(0.0)
    };
    [
        edge(EDGE_TOP, container.top(), keyboard.top()),
        edge(EDGE_LEADING, container.leading(), keyboard.leading()),
        edge(EDGE_BOTTOM, container.bottom(), keyboard.bottom()),
        edge(EDGE_TRAILING, container.trailing(), keyboard.trailing()),
    ]
}

/// A window-space extension a leaf or scroll surface may take on each edge:
/// where its frame sits on the avoided boundary, the edge reaches through
/// the regions `mask` ignores and stops at the nearest unignored one.
///
/// Returns the extension's `[top, leading, bottom, trailing]` amounts in
/// window logical units — also the content inset a scroll surface applies.
/// `frame` is the node's bounds in window space, `window` the window rect,
/// and the insets the two regions' depths at the window edge.
pub(super) fn band_extension(
    frame: kurbo::Rect,
    window: kurbo::Rect,
    mask: u16,
    container: &EdgeInsets,
    keyboard: &EdgeInsets,
) -> [f64; 4] {
    const TOUCH: f64 = 0.5;
    let mut out = [0.0; 4];
    let edges = [
        (
            EDGE_TOP,
            f64::from(container.top()),
            f64::from(keyboard.top()),
            frame.y0 - window.y0,
        ),
        (
            EDGE_LEADING,
            f64::from(container.leading()),
            f64::from(keyboard.leading()),
            frame.x0 - window.x0,
        ),
        (
            EDGE_BOTTOM,
            f64::from(container.bottom()),
            f64::from(keyboard.bottom()),
            window.y1 - frame.y1,
        ),
        (
            EDGE_TRAILING,
            f64::from(container.trailing()),
            f64::from(keyboard.trailing()),
            window.x1 - frame.x1,
        ),
    ];
    for (index, c, k, gap) in edges {
        let avoided = c.max(k);
        // The edge extends only where the frame already sits on the avoided
        // boundary — a band the frame never touches releases nothing.
        if (gap - avoided).abs() > TOUCH {
            continue;
        }
        out[index] = (avoided - edge_target(mask, index, c, k)).max(0.0);
    }
    out
}

/// The bounds/transform `ctx` reports after extending `[top, leading,
/// bottom, trailing]` window-space amounts: the transform carries the
/// leading/top overhang so descendants place from the shifted origin.
pub(super) fn extended_ctx(ctx: RenderContext, extension: [f64; 4]) -> RenderContext {
    let [top, leading, bottom, trailing] = extension;
    ctx.child(
        kurbo::Affine::translate((-leading, -top)),
        kurbo::Rect::new(
            0.0,
            0.0,
            ctx.bounds.width() + leading + trailing,
            ctx.bounds.height() + top + bottom,
        ),
    )
}

/// The local-space rect `ctx` grows to once each of its window-space edges
/// reaches through the regions `mask` ignores — the band extension mapped
/// back through `hit_transform`, so any affine lands the paint exactly.
/// `None` when no edge touches an avoided band or the transform degenerates.
pub(super) fn extended_local_rect(
    ctx: RenderContext,
    window: kurbo::Rect,
    mask: u16,
    container: &EdgeInsets,
    keyboard: &EdgeInsets,
) -> Option<kurbo::Rect> {
    let frame = transformed_rect(ctx.hit_transform, ctx.bounds);
    let extension = band_extension(frame, window, mask, container, keyboard);
    if !extension.iter().any(|depth| *depth > 0.0) {
        return None;
    }
    let [top, leading, bottom, trailing] = extension;
    let win_rect = kurbo::Rect::new(
        frame.x0 - leading,
        frame.y0 - top,
        frame.x1 + trailing,
        frame.y1 + bottom,
    );
    if ctx.hit_transform.determinant().abs() < 1e-9 {
        return None;
    }
    Some(transformed_rect(ctx.hit_transform.inverse(), win_rect))
}

/// `bounds` shrunk by the safe-area insets — the rect the window's root
/// content lays out inside. Clamps per axis so insets larger than the window
/// collapse to the origin rather than inverting the rect.
fn window_content_rect(bounds: kurbo::Rect, insets: &EdgeInsets) -> kurbo::Rect {
    let x0 = bounds.x0 + f64::from(insets.leading());
    let y0 = bounds.y0 + f64::from(insets.top());
    let x1 = (bounds.x1 - f64::from(insets.trailing())).max(x0);
    let y1 = (bounds.y1 - f64::from(insets.bottom())).max(y0);
    kurbo::Rect::new(x0, y0, x1, y1)
}

/// The [`RenderContext`] the window tree flushes under: `bounds` inset to the
/// safe area, with the content rect's origin folded into the transforms so
/// placement, drawing and hit-testing all agree the window begins at (0, 0).
fn safe_area_context(
    content: kurbo::Rect,
    transform: kurbo::Affine,
    hit_transform: kurbo::Affine,
) -> RenderContext {
    let shift = kurbo::Affine::translate((content.x0, content.y0));
    RenderContext::with_transforms(
        kurbo::Rect::new(0.0, 0.0, content.width(), content.height()),
        transform * shift,
        hit_transform * shift,
    )
}

impl RenderNode {
    /// Apply pending reactive `Dynamic` content changes by rebuilding only the
    /// affected child subtree — no whole-window re-dispatch. Returns whether
    /// anything changed; the caller relays the whole (retained, cheap) tree out
    /// when so, which lets a size-changing swap reflow its ancestors without
    /// resetting the scene and re-dispatching, which is visible as a flash.
    /// Walks the whole tree.
    pub(crate) fn patch(&mut self, renderer: &mut SemanticCore) -> bool {
        // No environment is threaded through: a rebuild uses the node's own captured
        // environment (`Dynamic`/`Collection`/`Env` carry it), so the walk only needs
        // the renderer.
        match self {
            Self::Dynamic(node) => {
                if node.apply_pending(renderer) {
                    true
                } else {
                    node.child.borrow_mut().patch(renderer)
                }
            }
            Self::Container(container) => {
                let mut changed = false;
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
                let mut changed = membership_changed | node.advance_transitions(renderer);
                for entry in &mut node.entries {
                    changed |= entry.node.patch(renderer);
                }
                changed
            }
            Self::Scroll(node) => {
                // A patched subtree may measure differently: drop the cached
                // `0`-probe floor so the next window-minimum pass re-measures
                // it on a frame that already does structural work.
                let changed = node.child.patch(renderer);
                if changed {
                    node.non_scrolling_minimum.set(None);
                }
                changed
            }
            // A filtered view wraps a child render node whose reactive
            // descendants must keep patching, so the walk recurses into it
            // (the filter itself owns its runtime, with no structural patch).
            Self::Filtered(node) => node.child.patch(renderer),
            Self::Color(_)
            | Self::Text(_)
            | Self::SceneView(_)
            // GPU content owns its runtime and produces every frame; like a
            // self-drawn scene it has no structural patch.
            | Self::GpuContent(_)
            | Self::ExternalFrame(_)
            // A widget leaf re-dispatches from its live config every flush, so it
            // needs no structural patch.
            | Self::Widget(_) => false,
            // A lazy stack keeps only visible item subtrees. Patch those retained
            // items before parent layout so a Dynamic row-height change updates the
            // scroll extent in the same refresh instead of one frame later.
            Self::LazyStack(node) => node.patch_visible(renderer),
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
            Self::Dynamic(node) => {
                out.insert(node.source.identity());
                node.child.borrow().collect_dynamic_identities_into(out);
            }
            Self::Container(container) => {
                for child in &container.children {
                    child.collect_dynamic_identities_into(out);
                }
            }
            Self::Opacity(node) => node.child.collect_dynamic_identities_into(out),
            Self::Scale(node) => node.child.collect_dynamic_identities_into(out),
            Self::Rotation(node) => node.child.collect_dynamic_identities_into(out),
            Self::Offset(node) => node.child.collect_dynamic_identities_into(out),
            Self::Retain(node) => node.child.collect_dynamic_identities_into(out),
            Self::Env(node) => node.child.collect_dynamic_identities_into(out),
            Self::Wrapper(node) => node.child.collect_dynamic_identities_into(out),
            Self::Collection(node) => {
                for entry in &node.entries {
                    entry.node.collect_dynamic_identities_into(out);
                }
            }
            Self::Scroll(node) => node.child.collect_dynamic_identities_into(out),
            Self::Filtered(node) => node.child.collect_dynamic_identities_into(out),
            Self::Color(_)
            | Self::Text(_)
            | Self::SceneView(_)
            | Self::GpuContent(_)
            | Self::ExternalFrame(_)
            | Self::Widget(_) => {}
            Self::LazyStack(node) => node
                .item_cache
                .borrow()
                .collect_dynamic_identities_into(out),
        }
    }

    /// Consume the subtree's layout-invalidated marks: `true` when any
    /// `FixedContainer` in the subtree had a `Layout::watch_invalidation`
    /// subscription fire since the last consume, meaning `place` must re-run
    /// even where an outer `RetainedSubview` reports an unchanged rect and
    /// proposal. Walks the same child-bearing variants as [`Self::patch`]. Every
    /// visited mark is cleared, so a stale mark cannot force a second relayout.
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
            Self::Filtered(node) => node.child.take_layout_dirty(),
            Self::Collection(node) => node
                .entries
                .iter_mut()
                .fold(false, |dirty, entry| entry.node.take_layout_dirty() | dirty),
            Self::LazyStack(node) => node.item_cache.borrow_mut().take_layout_dirty(),
            Self::Color(_)
            | Self::Text(_)
            | Self::SceneView(_)
            | Self::GpuContent(_)
            | Self::ExternalFrame(_)
            | Self::Widget(_) => false,
        }
    }
}

impl SemanticCore {
    /// The emit-pass equivalent of the `SemanticCore` half of
    /// `HydrolysisRenderer::reset_scene`: clears every pure-emission registry
    /// the accessibility walk re-pushes — input targets, gesture targets,
    /// text-input targets and the accessibility builder's node/action
    /// state — so each walk re-registers exactly what is live. The rendered
    /// pump runs this after the structural patch, so state the patch orphaned
    /// drops in the renderer's teardown order rather than ahead of it.
    fn reset_semantic_scene(&mut self) {
        self.hit_test.reset_scene();
        self.gesture_engine.clear_targets();
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
    fn begin_semantic_emit_frame(&mut self) {
        self.lifecycle.begin_rebuild_frame();
        self.hit_test.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
    }

    /// The emit-pass equivalent of the `SemanticCore` half of
    /// `HydrolysisRenderer::begin_rebuild_frame` — the build path's frame
    /// opens, including the animation and accessibility registries the pump
    /// path leaves to `reset_semantic_scene`.
    /// `signals.begin_rebuild` stays with the caller — only a build enters one.
    fn begin_semantic_rebuild_frame(&mut self) {
        self.state.measurement.begin_frame();
        self.lifecycle.begin_rebuild_frame();
        self.hit_test.begin_rebuild_frame();
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.animation_controller.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
    }

    /// The emit-pass equivalent of the non-scene half of
    /// `HydrolysisRenderer::finish_rebuild_frame` — the build path: Retain
    /// watcher rollover, the measurement-cache and animation-slot prunes,
    /// focus validation, and the accessibility tree's publication.
    /// `signals.finish_rebuild` stays with the caller — only a build entered
    /// one.
    fn finish_semantic_rebuild_frame(&mut self, live_dynamics: &FxHashSet<usize>) {
        self.lifecycle.finish_rebuild_frame();
        self.prune_dynamic_measurements(live_dynamics);
        self.validate_focused_text_input_after_flush();
        self.animation_controller
            .finish_rebuild_frame_with_inactive_slot_retention(false);
        self.hit_test
            .finish_rebuild_frame(&self.text_editing.text_input_targets);
        self.relocate_dropped_focus();
        self.navigation.finish_rebuild_frame();
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
    fn finish_semantic_emit_frame(&mut self, tree: &RenderNode, structural_change: bool) {
        self.hit_test
            .finish_rebuild_frame(&self.text_editing.text_input_targets);
        self.navigation.finish_rebuild_frame();
        if structural_change {
            // The emit re-bound every live animation slot; drop the slots and
            // cached Dynamic measurements belonging to subtrees the patch
            // removed, before the frame's retained subscriptions release.
            self.animation_controller
                .finish_rebuild_frame_with_inactive_slot_retention(false);
            self.prune_dynamic_measurements(&tree.collect_dynamic_identities());
        }
        self.lifecycle.finish_rebuild_frame();
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
    pub(crate) fn capture_window_semantics(&mut self, content: AnyView, env: &Environment) {
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
        self.signals.begin_rebuild();
        self.begin_semantic_rebuild_frame();
        self.render_depth = 0;
        let tree = RenderNode::build(content, env, self);
        let live_dynamics = tree.collect_dynamic_identities();
        #[cfg(feature = "accessibility")]
        tree.emit_accessibility(self, env);
        self.render_tree = Some(tree);
        self.finish_semantic_rebuild_frame(&live_dynamics);
        self.signals.finish_rebuild();
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
    pub(crate) fn flush_window_semantics(&mut self, _env: &Environment) -> bool {
        let Some(mut tree) = self.render_tree.take() else {
            return false;
        };
        self.begin_semantic_emit_frame();
        let structural_change = self.take_subview_structural_change() | tree.patch(self);
        if structural_change {
            self.animation_controller.begin_rebuild_frame();
        }
        self.reset_semantic_scene();
        // The emit-side half of `begin_redraw_frame`: roll the per-frame
        // measurement cache before the walk.
        self.state.measurement.begin_frame();
        #[cfg(feature = "accessibility")]
        tree.emit_accessibility(self, _env);
        self.finish_semantic_emit_frame(&tree, structural_change);
        self.render_tree = Some(tree);
        true
    }
}

impl HydrolysisRenderer {
    /// Build the retained tree before its first sized frame, so statically
    /// reachable nodes exist before the first render target is presented.
    #[cfg(test)]
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
    /// `HydrolysisRenderer::capture_window_scene`. The built tree is retained in
    /// `render_tree` for subsequent per-frame flushes.
    pub fn capture_window_tree(
        &mut self,
        content: AnyView,
        env: &Environment,
        bounds: kurbo::Rect,
        transform: kurbo::Affine,
        hit_transform: kurbo::Affine,
    ) {
        let _flush_span = tracing::debug_span!("hydrolysis_capture_window_tree").entered();
        let insets = window_safe_area_insets(self, env);
        let content_rect = window_content_rect(bounds, &insets);
        let size = Size::new(
            crate::num_cast::f64_as_f32(content_rect.width()),
            crate::num_cast::f64_as_f32(content_rect.height()),
        );
        let proposal = ProposalSize::new(Some(size.width), Some(size.height));
        // The viewport is recorded here rather than by each caller: every host
        // that builds a window tree — the runner, and a `HydrolysisGpuView`
        // embedding one in someone else's surface — has to agree on where the
        // window lands in device pixels, and forgetting to say so left the
        // embedded host reading a stale one.
        #[cfg(feature = "frame-profile")]
        let update_started_at = Instant::now();
        self.set_window_viewport(bounds, transform);
        let ctx = safe_area_context(content_rect, transform, hit_transform);
        // The tree is built once and persists. A later "rebuild" request reuses
        // it — applying pending Dynamic patches, relaying out, and re-flushing —
        // rather than rebuilding (which would re-connect each `Dynamic`, and a
        // `Dynamic` can only connect once). Called within a begin/finish rebuild
        // frame, so scene/layer flushing is handled by the caller.
        if let Some(mut tree) = self.render_tree.take() {
            tree.patch(self);
            #[cfg(feature = "frame-profile")]
            {
                self.frame_stage_times.update += update_started_at.elapsed();
            }
            #[cfg(feature = "frame-profile")]
            let layout_started_at = Instant::now();
            tree.prepare_for_measure(self);
            tree.layout(self, env, proposal, size);
            #[cfg(feature = "frame-profile")]
            {
                self.frame_stage_times.layout += layout_started_at.elapsed();
                self.last_layout_signature = Some(tree.placed_signature(Rect::from_size(size)));
            }
            #[cfg(feature = "frame-profile")]
            let encode_started_at = Instant::now();
            tree.flush(self, ctx, env);
            self.render_anchored_overlays(transform);
            #[cfg(feature = "frame-profile")]
            {
                self.frame_stage_times.encode += encode_started_at.elapsed();
            }
            self.render_tree = Some(tree);
            return;
        }
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times.update += update_started_at.elapsed();
        }
        self.render_depth = 0;
        let mut node = RenderNode::build(content, env, self);
        #[cfg(feature = "frame-profile")]
        let layout_started_at = Instant::now();
        node.prepare_for_measure(self);
        node.layout(self, env, proposal, size);
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times.layout += layout_started_at.elapsed();
            self.last_layout_signature = Some(node.placed_signature(Rect::from_size(size)));
        }
        #[cfg(feature = "frame-profile")]
        let encode_started_at = Instant::now();
        node.flush(self, ctx, env);
        self.render_anchored_overlays(transform);
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times.encode += encode_started_at.elapsed();
        }
        self.render_tree = Some(node);
    }

    /// Apply pending structural changes, run layout, and re-encode the retained
    /// window tree without rebuilding it. Returns `false` if no tree is built.
    /// This is the one per-frame pass: every awake frame patches, lays out, and
    /// re-encodes, so the presented scene can never go stale against layout.
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
        drop(update_span);
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times.update += update_started_at.elapsed();
        }
        let layout_span = tracing::debug_span!("hydrolysis_frame_layout").entered();
        #[cfg(feature = "frame-profile")]
        let layout_started_at = Instant::now();
        self.reset_scene();
        self.begin_redraw_frame();
        // Layout runs every frame: geometry can never go stale against the
        // scene encoded right after it.
        let insets = window_safe_area_insets(self, env);
        let content_rect = window_content_rect(bounds, &insets);
        let size = Size::new(
            crate::num_cast::f64_as_f32(content_rect.width()),
            crate::num_cast::f64_as_f32(content_rect.height()),
        );
        let proposal = ProposalSize::new(Some(size.width), Some(size.height));
        tree.prepare_for_measure(self);
        tree.layout(self, env, proposal, size);
        drop(layout_span);
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times.layout += layout_started_at.elapsed();
            self.last_layout_signature = Some(tree.placed_signature(Rect::from_size(size)));
        }
        let encode_span = tracing::debug_span!("hydrolysis_scene_encode").entered();
        #[cfg(feature = "frame-profile")]
        let encode_started_at = Instant::now();
        let ctx = safe_area_context(content_rect, transform, hit_transform);
        tree.flush(self, ctx, env);
        // The overlay-mode text context menu re-encodes with the frame it floats
        // over; drawing it only on the one-time build path would leave it visible
        // for a single frame.
        self.render_active_text_context_menu_overlay(env, transform);
        // Same for an open `.context_menu` presentation: its dim backdrop,
        // lifted preview and anchored accessory re-encode per frame and the
        // pass is where dismiss_requests/menu-close is observed.
        self.render_context_menu_presentation(transform);
        // Anchored overlays (`.anchored_overlay`) draw above all content: the
        // flush registered each anchor's live bounds, so the placement
        // contract re-runs per frame and the overlay follows moves/resizes.
        self.render_anchored_overlays(transform);
        self.flush_scene_layer();
        drop(encode_span);
        #[cfg(feature = "frame-profile")]
        {
            self.frame_stage_times.encode += encode_started_at.elapsed();
        }
        self.core
            .hit_test
            .finish_rebuild_frame(&self.core.text_editing.text_input_targets);
        self.core.navigation.finish_rebuild_frame();
        if structural_change {
            // The flush re-bound every live animation. Drop slots and cached
            // Dynamic measurements belonging to subtrees removed by the patch.
            self.core
                .animation_controller
                .finish_rebuild_frame_with_inactive_slot_retention(false);
            self.prune_dynamic_measurements(&tree.collect_dynamic_identities());
        }
        self.lifecycle.finish_rebuild_frame();
        // Drop focus or drag targets that are no longer emitted, relocate the
        // focus a dropped view released, then publish the refreshed
        // accessibility tree.
        self.validate_focused_text_input_after_flush();
        self.relocate_dropped_focus();
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
        Some(Size::new(
            validated_minimum_axis(min_box.width, "width"),
            validated_minimum_axis(min_box.height, "height"),
        ))
    }

    /// The retained window tree's root, for render-identity probes.
    #[cfg(test)]
    pub(crate) fn render_tree_root(&self) -> Option<&RenderNode> {
        self.render_tree.as_ref()
    }
}

fn validated_minimum_axis(value: f32, axis: &str) -> f32 {
    assert!(
        value.is_finite() && value >= 0.0,
        "hydrolysis window layout reported invalid minimum {axis}: {value}"
    );
    value
}
