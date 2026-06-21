//! Persistent retained render tree (Phase 1 foundation).
//!
//! A [`RenderNode`] is built once from a `View` (and re-built only on a
//! structural `Dynamic`/collection change in later phases). It holds the view's
//! *live* reactive inputs (`Computed`/`Binding`/`impl Signal`), not snapshots,
//! and is re-flushed each frame:
//!
//! - [`RenderNode::layout`] re-measures and re-places the subtree, caching each
//!   container's child frames. It runs on build and only when a geometry-affecting
//!   input changes — the rare, incremental case.
//! - [`RenderNode::flush`] re-encodes the subtree into the renderer's scene using
//!   the cached placements. It runs every frame; for a geometry-static frame
//!   (animation, scroll offset, re-present) it is just re-encode.
//!
//! This is the architecture validated by `tests::perf_full_rebuild`: a
//! geometry-static flush of a 160-row screen is ~tens of microseconds (the layout
//! cache is load-bearing), well under the 120fps budget, whereas re-dispatching
//! the same screen from the `View` tree costs ~15ms.
//!
//! Phase 1 covers leaf color/text and layout containers plus the build walk for
//! those kinds. Later phases add metadata wrapper nodes, `Dynamic`/collection host
//! nodes, scroll, effects, and wire the tree into the frame loop.

use super::*;
use crate::renderer::lazy::{
    LazyStackAxisConfig, lazy_stack_axis_config, place_lazy_stack_item,
    resolve_visible_index_window, sum_cached_or_estimated,
};
use crate::scroll::ScrollHandle;
use core::cell::Cell;
use nami::Computed;
use nami::watcher::BoxWatcherGuard;
use waterui_core::MainThreadBound;
use waterui_core::id::{Id as RawId, SelfId};
use waterui_core::layout::{Rect, Size};
use waterui_core::views::{AnyViews, Views};
use waterui_layout::scroll::{Axis as ScrollAxis, ScrollView};

/// The type-erased item identity used by [`CollectionNode`]'s reconcile.
type CollectionItemId = SelfId<RawId>;

/// A [`SubView`] adapter that measures a child [`RenderNode`] **on demand** at
/// whatever proposal the real [`Layout`] passes — the node analogue of
/// [`HydroSubview`]. A fixed pre-measured size is wrong: layouts re-probe children
/// at refined proposals (e.g. `GridLayout` asks each cell its height at the column
/// width; a `FrameLayout` with no height constraint would otherwise fill the whole
/// proposed height), so the adapter must forward the live proposal. Measuring a
/// node recurses into reactive bodies and borrows the renderer's `HydroState`, so
/// it stays on the main thread (`require_main_thread` is `true`); state is shared
/// across one container level's children through a `RefCell`, mirroring the legacy
/// path's `RefCell::new(state)` discipline.
struct NodeSubView<'a> {
    node: MainThreadBound<&'a RenderNode>,
    state: MainThreadBound<&'a RefCell<&'a mut HydroState>>,
    env: MainThreadBound<Environment>,
    stretch: StretchAxis,
    measure_cache: MainThreadBound<RefCell<Vec<(ProposalSize, ViewDimensions)>>>,
}

impl<'a> NodeSubView<'a> {
    fn new(
        node: &'a RenderNode,
        state: &'a RefCell<&'a mut HydroState>,
        env: &'a Environment,
    ) -> Self {
        Self {
            stretch: node.stretch(),
            node: MainThreadBound::new(node),
            state: MainThreadBound::new(state),
            env: MainThreadBound::new(env.clone()),
            measure_cache: MainThreadBound::new(RefCell::new(Vec::new())),
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
        if self.stretch.stretches_vertical() {
            if let Some(height) = proposal.height {
                dimensions.size.height = dimensions.size.height.max(height);
            }
        } else if let Some(height) = proposal.height {
            dimensions.size.height = dimensions.size.height.min(height);
        }
        dimensions
    }
}

impl SubView for NodeSubView<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
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
        0
    }
    fn require_main_thread(&self) -> bool {
        true
    }
}

/// A retained sub-view a native widget owns and re-renders every flush — the
/// solution for a widget's move-only `AnyView` label sub-views (slider min/max
/// labels, menu label, progress label/value label) which cannot be re-dispatched
/// twice. The source `AnyView` is built into a persistent [`RenderNode`] once
/// (going through the same dispatcher path as everything else, so a reactive label
/// inside it reaches its dedicated `Dynamic`/`Text` node and stays live), then
/// laid out and flushed at the label's rect each frame.
pub(crate) struct RetainedSubview {
    /// The source view, taken on first build (`AnyView` is move-only).
    source: Option<AnyView>,
    /// The built child node, re-laid-out + re-flushed at the label rect each frame.
    node: Option<RenderNode>,
    /// The size the node was last laid out at, so layout re-runs only on a change.
    laid_out: Size,
    /// The default spoken accessibility label extracted from the source view once,
    /// at build time (mirrors `GestureObserverEffect::default_a11y_label`): the
    /// node owns the source after build, so the per-frame a11y path reads this.
    default_a11y_label: Option<String>,
}

impl RetainedSubview {
    pub(crate) fn new(source: AnyView) -> Self {
        Self {
            source: Some(source),
            node: None,
            laid_out: Size::zero(),
            default_a11y_label: None,
        }
    }

    /// Eagerly build the sub-view's node now (the caller has the renderer). Used
    /// at tree-build time so the later measure path — which only has `&mut
    /// HydroState`, not the renderer — can measure the already-built node.
    pub(crate) fn ensure_built(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        if self.node.is_none()
            && let Some(view) = self.source.take()
        {
            // Extract the default a11y label from the source before it is consumed
            // by `build` (the node owns the view afterward).
            #[cfg(feature = "accessibility")]
            {
                self.default_a11y_label = renderer.accessibility_label_from_view(&view, env);
            }
            // Normalize as the container/collection build paths do, so a layout
            // view (stack/spacer/etc.) inside a label lowers to its native form.
            let view = normalize_layout_view(view, env);
            self.node = Some(RenderNode::build(view, env, renderer));
        }
    }

    /// The default spoken a11y label extracted from the source at build time.
    pub(crate) fn default_a11y_label(&self) -> Option<String> {
        self.default_a11y_label.clone()
    }

    /// Transform the still-unbuilt source view (e.g. apply a default foreground
    /// color before build). Panics if the node has already been built — the source
    /// is consumed at first build, so this must run before any flush/measure.
    pub(crate) fn map_source(&mut self, f: impl FnOnce(AnyView) -> AnyView) {
        let source = self.source.take().expect(
            "RetainedSubview::map_source must run before the sub-view is built (source consumed)",
        );
        self.source = Some(f(source));
    }

    /// Measure the sub-view's intrinsic size (building it once if needed), the
    /// node analogue of [`measure_view_intrinsic`] at the unspecified proposal.
    /// For the render path, which has the renderer to build on first use.
    pub(crate) fn measure_intrinsic(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) -> Size {
        self.ensure_built(renderer, env);
        self.measure_built(&mut renderer.state, env)
    }

    /// Measure an already-built sub-view's intrinsic size with only `&mut
    /// HydroState` — the measure-path analogue (no renderer to build on). The node
    /// must already be built (via [`Self::ensure_built`]); an unbuilt one measures
    /// as zero, matching an empty label.
    pub(crate) fn measure_built(&self, state: &mut HydroState, env: &Environment) -> Size {
        let Some(node) = &self.node else {
            return Size::zero();
        };
        node.measure(state, env, ProposalSize::UNSPECIFIED).size
    }

    /// Measure an already-built sub-view at a concrete proposal — the variant for
    /// content-filling sub-views (map/webview) whose composed body wraps text at the
    /// proposed width. Returns the full [`ViewDimensions`]; an unbuilt one measures
    /// as zero.
    pub(crate) fn measure_built_with_proposal(
        &self,
        state: &mut HydroState,
        env: &Environment,
        proposal: ProposalSize,
    ) -> Size {
        let Some(node) = &self.node else {
            return Size::zero();
        };
        node.measure(state, env, proposal).size
    }

    /// Build (once), lay out (only when the rect size changes), and flush the
    /// sub-view at `rect` under `env`. A zero-area rect renders nothing, matching
    /// the dispatch path's empty-rect guard.
    pub(crate) fn flush_in_rect(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        rect: vello::kurbo::Rect,
    ) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        self.ensure_built(renderer, env);
        let Some(node) = &mut self.node else {
            return;
        };
        #[allow(clippy::cast_possible_truncation)]
        let size = Size::new(rect.width() as f32, rect.height() as f32);
        if size != self.laid_out {
            node.layout(renderer, env, size);
            self.laid_out = size;
        }
        let child_ctx = ctx.child(
            vello::kurbo::Affine::translate((rect.x0, rect.y0)),
            vello::kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height()),
        );
        node.flush(renderer, child_ctx, env);
    }

    /// Build (once), lay out at `size` (only when it changes), and flush the
    /// sub-view under a caller-supplied [`RenderContext`] — the variant for a
    /// sub-view drawn under a non-translation transform (the text-field floating
    /// label's animated translate + scale). The caller composes the transform via
    /// [`RenderContext::child`] and passes the local layout `size` the node should
    /// lay out at; a zero-area size renders nothing.
    pub(crate) fn flush_in_ctx(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        size: Size,
    ) {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        self.ensure_built(renderer, env);
        let Some(node) = &mut self.node else {
            return;
        };
        if size != self.laid_out {
            node.layout(renderer, env, size);
            self.laid_out = size;
        }
        node.flush(renderer, ctx, env);
    }

    /// Build (once), lay out at `size`, and flush the sub-view into a fresh,
    /// standalone [`vello::Scene`] in identity (local) coordinates — the retained
    /// analogue of [`HydrolysisRenderer::render_subtree_scene`] for a node that
    /// must survive across flushes (the navigation-stack root). The renderer's
    /// scene is swapped out, the node flushes into the temporary scene, then the
    /// scene is swapped back, so the returned scene can be replayed by the
    /// navigation transition (cross-fade `from`/`to`) without re-dispatch.
    pub(crate) fn render_built_scene(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        size: Size,
    ) -> vello::Scene {
        self.ensure_built(renderer, env);
        let mut scene = vello::Scene::new();
        let Some(node) = &mut self.node else {
            return scene;
        };
        if size != self.laid_out {
            node.layout(renderer, env, size);
            self.laid_out = size;
        }
        let local_ctx = RenderContext::with_transforms(
            vello::kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        core::mem::swap(renderer.scene_mut(), &mut scene);
        node.flush(renderer, local_ctx, env);
        core::mem::swap(renderer.scene_mut(), &mut scene);
        scene
    }
}

/// A cache of retained node sub-views for a *virtualized* collection (a lazy
/// stack, list, or table): only items in the current visible window are built and
/// retained, keyed by a stable identity, so a long collection costs only its
/// visible rows. Items are built lazily as they scroll into view and evicted once
/// they leave the visible set — matching virtualization, where scrolled-away item
/// state is intentionally not preserved (the same lifetime the old dispatch path
/// gave each re-dispatched item). While an item stays visible its node is reused,
/// so its reactive content stays live through the node's own per-frame re-flush.
pub(crate) struct VisibleSubviewCache<K: Eq + core::hash::Hash + Clone> {
    entries: std::collections::HashMap<K, RetainedSubview>,
    /// Keys touched during the in-progress frame; [`Self::end_frame`] evicts the rest.
    touched: std::collections::HashSet<K>,
}

impl<K: Eq + core::hash::Hash + Clone> VisibleSubviewCache<K> {
    pub(crate) fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            touched: std::collections::HashSet::new(),
        }
    }

    /// Begin a frame: forget which keys were visible last frame.
    pub(crate) fn begin_frame(&mut self) {
        self.touched.clear();
    }

    /// Get-or-build the retained sub-view for `key`, marking it visible this frame.
    /// `build` produces the item's source view; it runs only the first time an id
    /// becomes visible (or after it was evicted and scrolled back).
    pub(crate) fn entry(
        &mut self,
        key: K,
        build: impl FnOnce() -> AnyView,
    ) -> &mut RetainedSubview {
        self.touched.insert(key.clone());
        self.entries
            .entry(key)
            .or_insert_with(|| RetainedSubview::new(build()))
    }

    /// Evict every sub-view not touched this frame (items scrolled out of view).
    pub(crate) fn end_frame(&mut self) {
        let touched = &self.touched;
        self.entries.retain(|key, _| touched.contains(key));
    }
}

/// A node in the persistent retained render tree. The render-primitive set is
/// closed by the nature of a self-drawn renderer; the open `HydroDispatcher` maps
/// the open universe of `View` types onto this closed set.
pub(crate) enum RenderNode {
    /// A solid fill of the node's bounds.
    Color(ColorNode),
    /// A styled-text leaf holding its reactive content/alignment.
    Text(Box<TextNode>),
    /// A layout container owning child nodes and their cached frames.
    Container(Box<ContainerNode>),
    /// An animated-opacity layer wrapping a child (layout-transparent).
    Opacity(Box<OpacityNode>),
    /// An animated scale transform wrapping a child (layout-transparent).
    Scale(Box<ScaleNode>),
    /// An animated rotation transform wrapping a child (layout-transparent).
    Rotation(Box<RotationNode>),
    /// An animated offset transform wrapping a child (layout-transparent).
    Offset(Box<OffsetNode>),
    /// Holds a retained guard (e.g. a signal-watcher subscription) alive for its
    /// subtree's lifetime; layout-transparent. Recursing through it (instead of
    /// capturing) lets reactive/effect descendants like `SceneView` reach their
    /// dedicated nodes.
    Retain(Box<RetainNode>),
    /// A scoped-environment wrapper: carries the environment a subtree was built
    /// under so it is also the environment used at `measure`/`layout`/`flush`.
    /// Env scoping (`.font()` / `.foreground()` / locale / theme) is read every
    /// frame by text shaping and accessibility resolution, so it cannot be
    /// flattened away at build time — it must travel with the node. Layout-transparent.
    Env(Box<EnvNode>),
    /// A scroll view: owns its content as a persistent child, lays it out at the
    /// full content size, and applies the scroll offset as a per-frame transform
    /// (viewport clip + translate). No content cache — the child IS the retained
    /// content, so scrolling re-flushes at the new offset without re-dispatch.
    Scroll(Box<ScrollNode>),
    /// A retained, reactive, non-virtualized collection (an `AbsoluteLayout`/
    /// `ZStack` overlay or a transition collection): renders every item, reconciles
    /// membership changes by id (unchanged items keep their node and state, new ids
    /// are built, removed ids are dropped), and relays out — no whole-window rebuild.
    Collection(Box<CollectionNode>),
    /// A viewport-virtualized lazy stack (a `VStack`/`HStack` `LazyContainer`,
    /// typically inside a scroll): builds, measures, and encodes only the items in
    /// the current visible window, so cost is bounded by visible rows regardless of
    /// total count. Re-resolves the window each flush from the enclosing scroll's
    /// pushed viewport, so scrolling reveals new rows without re-dispatch.
    LazyStack(Box<LazyStackNode>),
    /// A self-drawn scene (`Canvas`/SVG/chart): owns its `SceneContent` directly
    /// (no cursor-bound effect slot), so a `Dynamic` swap to a different scene
    /// renders the new content correctly — the real fix for chart Bug 1.
    SceneView(Box<SceneViewNode>),
    /// An embedded `GpuSurface` leaf owning its `EmbeddedGpuSurfaceRuntime`
    /// directly (no cursor-bound slot), composited through an `Rc`-carrying layer.
    GpuSurface(Box<GpuSurfaceNode>),
    /// A `ViewEffect` leaf owning its `ViewEffectRuntime` and its captured child
    /// node directly (no cursor-bound effect slot).
    ViewEffect(Box<ViewEffectNode>),
    /// An `AppliedFilter` wrapper owning its `AppliedFilterRuntime` (textures) and
    /// recursing into its child node (no cursor-bound effect slot).
    AppliedFilter(Box<AppliedFilterNode>),
    /// A reactive `Dynamic` host: holds the live `Dynamic`, rebuilds only its own
    /// child subtree when the content changes (incremental patch + relayout), with
    /// no whole-window rebuild — the structural seam that fixes the chart's
    /// flicker (Bug 2). Layout-transparent around its child.
    Dynamic(Box<DynamicHostNode>),
    /// A transparent metadata wrapper that applies a visual/interaction effect
    /// every flush and *recurses into* its child node (instead of capturing the
    /// subtree once). This is what keeps reactive descendants inside `.clip()` /
    /// `.border()` / `.shadow()` / `.cursor()` / `.draggable()` / drop-destination
    /// / context-menu live: they reach their own dedicated nodes and keep updating.
    /// Layout-transparent (the effect is pure setup + render child).
    Wrapper(Box<WrapperNode>),
    /// A native widget leaf (button, toggle, slider, picker, text field, …) rendered
    /// by its `HydroNativeView` handler **every flush** from a retained,
    /// signal-holding config — never baked. Its `builder` reconstructs the leaf view
    /// (actions retained as `Rc`, label/value signals kept) and the flush re-dispatches
    /// it, so the handler re-reads its live signals and reactive content (a `text!`
    /// label, a bound value) stays live. Re-dispatching a *leaf* is cheap (no body
    /// expansion, no structural rebuild) — this is the new-architecture replacement
    /// for the old capture-once capture/replay path, now removed.
    Widget(Box<WidgetNode>),
}

/// A transparent metadata wrapper node: it carries the effect to re-apply each
/// flush, the environment its subtree was built under (effect colors and a11y
/// read env every frame), and the child node it recurses into.
pub(crate) struct WrapperNode {
    effect: WrapperEffect,
    env: Environment,
    child: RenderNode,
}

/// Re-renders a [`WidgetNode`]'s leaf every flush from its retained config (reads
/// live signals, re-registers interaction targets + a11y at the current bounds).
pub(crate) type WidgetRenderFn = Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>;

/// Measures a [`WidgetNode`]'s leaf for layout (reads the config's sizing signals).
pub(crate) type WidgetMeasureFn =
    Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>;

/// A native widget leaf rendered every flush from a retained config. `builder`
/// reconstructs the leaf `AnyView` (with its actions retained behind `Rc` and its
/// label/value signals kept), so each flush re-dispatches the leaf through its
/// existing handler — re-reading live signals and re-emitting interaction targets
/// and accessibility with current bounds. No bake, no capture-once freeze.
pub(crate) struct WidgetNode {
    render: WidgetRenderFn,
    measure: WidgetMeasureFn,
    stretch: StretchAxis,
    env: Environment,
}

/// The per-flush effect a [`WrapperNode`] re-applies around its child. Each
/// variant defers to the matching `apply_*` helper in `metadata.rs`, so the
/// effect logic is shared byte-for-byte with the dispatch path.
enum WrapperEffect {
    Clip(ClipShape),
    Border(Border),
    Shadow(Shadow),
    Cursor(Cursor),
    Draggable(Draggable),
    DropDestination(DropDestinationHandles),
    ContextMenu(ResolvedContextMenu),
    /// Conditional hit-testing: renders the child, then truncates the interaction
    /// targets it registered when disabled. The bookkeeping counts targets across
    /// the (node-flushed) child render, so reactive descendants stay live.
    Hittable(Hittable),
    /// A hover-enter/move/exit handler re-registered every flush. The handler is
    /// shared so the node can re-register the same `OnEvent` each frame.
    OnEvent(Rc<RefCell<OnEvent>>),
    /// A gesture observer (tap/long-press/drag/…). The two pieces the dispatch
    /// path derives from the (now node-owned) content are resolved at build time
    /// and stored in the effect; see [`GestureObserverEffect`].
    GestureObserver(GestureObserverEffect),
    /// A `.focused(binding)` modifier targeting exactly one text input in the
    /// wrapped subtree. Re-applied every flush through [`apply_focused`]: the
    /// binding is read via `read_signal` (so a focus change schedules a frame) and
    /// the registered text input's `focus_binding`/focus state is set from the
    /// child's just-flushed targets — reactive descendants reach their own nodes.
    Focused(Focused),
    /// An `.on_appear`/`.on_disappear` lifecycle hook, made node-owned (the slot
    /// cursor that bound the old dispatch path is gone — the same fix class as the
    /// `SceneView` Bug 1). Appear fires once at build; disappear fires from this
    /// effect's [`Drop`] when the node leaves the retained tree (a `Dynamic` /
    /// collection reconcile that drops the subtree, or app teardown). Flush is a
    /// passthrough.
    #[allow(
        dead_code,
        reason = "retained only to fire its disappear hook from Drop on structural removal"
    )]
    LifeCycle(LifeCycleEffect),
}

/// The node-owned state of a lifecycle hook (see [`WrapperEffect::LifeCycle`]).
/// An appear hook fires at build time and leaves `disappear` empty; a disappear
/// hook is retained here and fired exactly once when the node is dropped, so
/// structural removal — not a frame-diff slot cursor — drives disappearance.
pub(crate) struct LifeCycleEffect {
    disappear: Option<DeferredLifeCycleHook>,
}

impl Drop for LifeCycleEffect {
    fn drop(&mut self) {
        if let Some(hook) = self.disappear.take() {
            hook.call();
        }
    }
}

/// The build-resolved state of a `.gesture(...)` observer, shared by the dispatch
/// handler and the retained `Wrapper` node. A node has no `content: AnyView` at
/// flush, so the two pieces the dispatch path derives from `content` are resolved
/// at build time and stored here: `default_a11y_label` (the default spoken label,
/// via `accessibility_label_from_view`) and `gesture_group_identity` (via
/// `gesture_group_identity`). The action is shared (`Rc<RefCell<…>>`) so the node
/// can re-register the same action every flush.
pub(crate) struct GestureObserverEffect {
    pub(crate) gesture: Gesture,
    pub(crate) action: Rc<RefCell<BoxedAction<()>>>,
    pub(crate) default_a11y_label: Option<String>,
    pub(crate) gesture_group_identity: usize,
}

pub(crate) struct ColorNode {
    pub(crate) color: vello::peniko::Color,
}

pub(crate) struct TextNode {
    pub(crate) content: Computed<StyledStr>,
    pub(crate) alignment: Computed<HorizontalAlignment>,
}

pub(crate) struct ContainerNode {
    pub(crate) layout: Box<dyn Layout>,
    pub(crate) children: Vec<RenderNode>,
    /// Child frames cached by [`RenderNode::layout`]; reused by
    /// [`RenderNode::flush`] so a geometry-static frame pays only re-encode.
    pub(crate) placed: Vec<Rect>,
}

/// An animated-opacity wrapper: re-samples its alpha each flush and pushes a
/// layer around the child. Layout-transparent (the child measures/places as if
/// the wrapper were absent), matching the SwiftUI/WaterUI transform model.
pub(crate) struct OpacityNode {
    pub(crate) value: Opacity,
    pub(crate) child: RenderNode,
}

pub(crate) struct ScaleNode {
    pub(crate) value: Scale,
    pub(crate) child: RenderNode,
}

pub(crate) struct RotationNode {
    pub(crate) value: Rotation,
    pub(crate) child: RenderNode,
}

pub(crate) struct OffsetNode {
    pub(crate) value: Offset,
    pub(crate) child: RenderNode,
}

pub(crate) struct ScrollNode {
    axis: ScrollAxis,
    child: RenderNode,
    /// Scroll handle bound at layout (offset persists across frames; scroll
    /// events mutate it via the registered scroll target).
    handle: Option<ScrollHandle>,
    /// Full content extent the child is laid out at.
    content_size: Size,
    /// The scroll viewport (the node's own bounds).
    viewport: Size,
    /// Environment captured at build, for scroll-target accessibility.
    env: Environment,
}

pub(crate) struct RetainNode {
    #[allow(
        dead_code,
        reason = "keeps a watcher/subscription guard alive for the subtree"
    )]
    retain: Retain,
    child: RenderNode,
}

pub(crate) struct CollectionNode {
    /// The container layout (e.g. `AbsoluteLayout`, `ZStackLayout`).
    layout: Box<dyn Layout>,
    /// The reactive item collection (`len`/`get_view`/`get_id`, watched).
    views: AnyViews<AnyView>,
    /// Environment captured at build, used to materialize items.
    env: Environment,
    /// Current items in order, keyed by id so a membership change keeps unchanged
    /// items' nodes (and their in-flight state) and only builds/drops the delta.
    items: Vec<(CollectionItemId, RenderNode)>,
    /// Child frames cached by [`RenderNode::layout`], reused by `flush`.
    placed: Vec<Rect>,
    /// Set by the membership watcher; consumed by `patch` to trigger a reconcile.
    dirty: Rc<Cell<bool>>,
    /// Stable allocation whose address is this collection's patch dirty-key.
    #[allow(
        dead_code,
        reason = "pins the dirty-key address for the node's lifetime"
    )]
    dirty_key: Rc<()>,
    /// Membership-change watcher; a change sets `dirty` and schedules a refresh.
    #[allow(dead_code, reason = "keeps the membership watcher subscription alive")]
    guard: BoxWatcherGuard,
}

pub(crate) struct LazyStackNode {
    /// Stack axis + spacing + cross-axis alignment.
    axis: LazyStackAxisConfig,
    /// The reactive item collection (`len`/`get_view`, watched for membership).
    views: AnyViews<AnyView>,
    /// Environment captured at build, used to materialize and style items.
    env: Environment,
    /// Per-index main-axis extent cache, persisting across flushes so a steady
    /// scroll only measures items entering the visible window.
    item_extents: RefCell<Vec<Option<f64>>>,
    /// Retained node sub-views for the items currently in the visible window, keyed
    /// by stable id so a steady scroll reuses each visible item's node (keeping its
    /// reactive content live) and only builds items entering the window.
    item_cache: RefCell<VisibleSubviewCache<CollectionItemId>>,
    /// Estimated extent for not-yet-measured items, seeded from the first measure;
    /// used to size the scroll content without measuring the whole collection.
    estimate: Cell<f64>,
    /// Stable allocation whose address is this collection's patch dirty-key,
    /// owned so the key cannot be reused by another allocation while it lives.
    #[allow(
        dead_code,
        reason = "pins the dirty-key address for the node's lifetime"
    )]
    dirty_key: Rc<()>,
    /// Membership-change watcher: a change schedules a window refresh, which
    /// re-resolves the visible window (the collection `len`/items are re-read).
    #[allow(dead_code, reason = "keeps the membership watcher subscription alive")]
    guard: BoxWatcherGuard,
}

impl CollectionNode {
    /// Build the `NodeSubView` proxies for this collection's items, sharing one
    /// state cell across the level (mirrors [`ContainerNode`] measurement).
    fn measure(&self, state: &mut HydroState, proposal: ProposalSize) -> ViewDimensions {
        let cell = RefCell::new(state);
        let subs: Vec<NodeSubView> = self
            .items
            .iter()
            .map(|(_, node)| NodeSubView::new(node, &cell, &self.env))
            .collect();
        let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
        ViewDimensions::new(self.layout.size_that_fits(proposal, &refs))
    }

    fn layout(&mut self, renderer: &mut HydrolysisRenderer, size: Size) {
        let env = self.env.clone();
        let rects = {
            let cell = RefCell::new(&mut renderer.state);
            let subs: Vec<NodeSubView> = self
                .items
                .iter()
                .map(|(_, node)| NodeSubView::new(node, &cell, &env))
                .collect();
            let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
            self.layout.place(Rect::from_size(size), &refs)
        };
        for ((_, child), rect) in self.items.iter_mut().zip(rects.iter()) {
            child.layout(renderer, &env, *rect.size());
        }
        self.placed = rects;
    }

    fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        for ((_, child), rect) in self.items.iter().zip(self.placed.iter()) {
            let child_ctx = ctx.child(
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y()))),
                vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(rect.width()),
                    f64::from(rect.height()),
                ),
            );
            child.flush(renderer, child_ctx, &self.env);
        }
    }

    /// Apply a membership change: keep each surviving id's node (and its in-flight
    /// state), build newly-present ids, and drop departed ones — in the new order.
    fn reconcile(&mut self, renderer: &mut HydrolysisRenderer) {
        let env = self.env.clone();
        let len = self.views.len().get();
        let mut surviving: std::collections::BTreeMap<CollectionItemId, RenderNode> =
            self.items.drain(..).collect();
        let mut next = Vec::with_capacity(len);
        for index in 0..len {
            let id = self
                .views
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"));
            let node = surviving.remove(&id).unwrap_or_else(|| {
                let view = self
                    .views
                    .get_view(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                RenderNode::build(normalize_layout_view(view, &env), &env, renderer)
            });
            next.push((id, node));
        }
        self.items = next;
    }
}

impl LazyStackNode {
    fn spacing(&self) -> f64 {
        match self.axis {
            LazyStackAxisConfig::Vertical { spacing, .. }
            | LazyStackAxisConfig::Horizontal { spacing, .. } => spacing,
        }
    }

    /// Measures item `index` under the given cross-axis extent, returning its
    /// measured size and stretch axis (both needed to place it).
    fn measure_item(
        &self,
        state: &mut HydroState,
        index: usize,
        cross: f64,
    ) -> (Size, StretchAxis) {
        let view = self
            .views
            .get_view(index)
            .unwrap_or_else(|| panic!("hydrolysis LazyStack failed to materialize item {index}"));
        let view = normalize_layout_view(view, &self.env);
        let bound = RefCell::new(&mut *state);
        let subview = HydroSubview::from_view(&view, &bound, &self.env);
        #[allow(clippy::cast_possible_truncation)]
        let proposal = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => ProposalSize::new(Some(cross as f32), None),
            LazyStackAxisConfig::Horizontal { .. } => ProposalSize::new(None, Some(cross as f32)),
        };
        (subview.measure(proposal).size, subview.stretch_axis())
    }

    fn main_extent(&self, size: Size) -> f64 {
        match self.axis {
            LazyStackAxisConfig::Vertical { .. } => f64::from(size.height),
            LazyStackAxisConfig::Horizontal { .. } => f64::from(size.width),
        }
    }

    /// Seeds the estimated item extent from item 0 if not yet known.
    fn ensure_estimate(&self, state: &mut HydroState, cross: f64) {
        if self.estimate.get() > 0.0 {
            return;
        }
        let (size, _) = self.measure_item(state, 0, cross);
        let extent = self.main_extent(size);
        self.estimate.set(extent.max(1.0));
        self.item_extents.borrow_mut()[0] = Some(extent);
    }

    /// Total content extent along the main axis: cached extents where known,
    /// estimated elsewhere, plus inter-item spacing.
    fn content_main_extent(&self, count: usize) -> f64 {
        let extents = self.item_extents.borrow();
        let sum = sum_cached_or_estimated(&extents, self.estimate.get());
        sum + self.spacing() * count.saturating_sub(1) as f64
    }

    #[allow(clippy::cast_possible_truncation)]
    fn measure(&self, state: &mut HydroState, proposal: ProposalSize) -> ViewDimensions {
        let count = self.views.len().get();
        self.item_extents.borrow_mut().resize(count, None);
        if count == 0 {
            return ViewDimensions::new(Size::zero());
        }
        let cross = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => proposal.width.unwrap_or(0.0),
            LazyStackAxisConfig::Horizontal { .. } => proposal.height.unwrap_or(0.0),
        };
        self.ensure_estimate(state, f64::from(cross));
        let main = self.content_main_extent(count) as f32;
        let size = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => Size::new(cross, main),
            LazyStackAxisConfig::Horizontal { .. } => Size::new(main, cross),
        };
        ViewDimensions::new(size)
    }

    /// Resolves the visible window from the enclosing scroll's pushed viewport and
    /// re-dispatches only those items at their placed rects. Bounded by visible rows.
    fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext, _env: &Environment) {
        let count = self.views.len().get();
        self.item_extents.borrow_mut().resize(count, None);
        if count == 0 {
            return;
        }
        let cross = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => ctx.bounds.width(),
            LazyStackAxisConfig::Horizontal { .. } => ctx.bounds.height(),
        };
        self.ensure_estimate(&mut renderer.state, cross);
        self.item_cache.borrow_mut().begin_frame();
        let visible = renderer
            .lazy
            .lazy_viewport_stack
            .last()
            .copied()
            .unwrap_or(ctx.bounds);
        let (visible_start, visible_end) = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => (visible.y0, visible.y1),
            LazyStackAxisConfig::Horizontal { .. } => (visible.x0, visible.x1),
        };
        let spacing = self.spacing();
        let window = resolve_visible_index_window(count, visible_start, visible_end, |index| {
            let cached = self.item_extents.borrow()[index];
            let extent = cached.unwrap_or_else(|| {
                let (size, _) = self.measure_item(&mut renderer.state, index, cross);
                let extent = self.main_extent(size);
                self.item_extents.borrow_mut()[index] = Some(extent);
                extent
            });
            if index + 1 < count {
                extent + spacing
            } else {
                extent
            }
        });
        let mut cursor = window.leading_offset;
        for index in window.start..window.end {
            let (size, stretch) = self.measure_item(&mut renderer.state, index, cross);
            let child_rect = place_lazy_stack_item(self.axis, stretch, size, ctx.bounds, cursor);
            let extent = match self.axis {
                LazyStackAxisConfig::Vertical { .. } => child_rect.height(),
                LazyStackAxisConfig::Horizontal { .. } => child_rect.width(),
            };
            self.item_extents.borrow_mut()[index] = Some(extent);
            let id = self
                .views
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
            {
                let env = &self.env;
                let views = &self.views;
                let mut cache = self.item_cache.borrow_mut();
                let subview = cache.entry(id, || {
                    let view = views.get_view(index).unwrap_or_else(|| {
                        panic!("hydrolysis LazyStack failed to materialize item {index}")
                    });
                    normalize_layout_view(view, env)
                });
                subview.flush_in_rect(renderer, ctx, env, child_rect);
            }
            cursor += extent;
            if index + 1 < count {
                cursor += spacing;
            }
        }
        self.item_cache.borrow_mut().end_frame();
    }
}

pub(crate) struct EnvNode {
    /// The scoped environment this subtree was built under, used to override the
    /// inherited environment at every measure/layout/flush.
    env: Environment,
    child: RenderNode,
}

pub(crate) struct SceneViewNode {
    /// The owned scene content, re-drawn each flush (it reads its own reactive
    /// inputs in `build_scene`). `RefCell` because `build_scene` needs `&mut` but
    /// `flush` takes `&self`.
    content: RefCell<Box<dyn waterui_graphics::SceneContent>>,
}

/// An embedded `GpuSurface` leaf that OWNS its `EmbeddedGpuSurfaceRuntime`
/// (textures, setup state, redraw handle) — the node analogue of
/// [`SceneViewNode`], for a `Native<GpuSurface>` reached through the retained
/// tree. Identity is structural: a reactive swap builds a fresh node with a
/// fresh runtime, and a per-frame re-flush re-binds the *same* runtime via an
/// `Rc`-carrying compositor layer, so there is no cursor-ordered slot to desync.
/// The runtime is shared (`Rc<RefCell<…>>`) with the renderer's node-surface
/// registry so its off-thread redraw handle is polled even on frames that do not
/// re-flush the tree.
pub(crate) struct GpuSurfaceNode {
    runtime: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
}

/// A `ViewEffect` leaf that OWNS its `ViewEffectRuntime` (the effect renderer +
/// setup state) and builds its captured child as a persistent [`RenderNode`], so
/// reactive descendants inside the effect's content reach their own dedicated
/// nodes and stay live. Each flush renders the child node into an input texture,
/// runs the effect into an output texture, and draws the output image — mirroring
/// the dispatch path's `render_view_effect` exactly, but with no cursor-bound
/// effect slot.
pub(crate) struct ViewEffectNode {
    runtime: RefCell<ViewEffectRuntime>,
    /// The effect's content, built once as a persistent node (recursed into, not
    /// baked), re-rendered into the input texture each flush.
    child: RefCell<RenderNode>,
    /// The size `child` was last laid out at, so layout re-runs only on a change.
    laid_out: Cell<Size>,
    env: Environment,
}

/// An `AppliedFilter` metadata wrapper that OWNS its `AppliedFilterRuntime`
/// (input/output textures, setup state, output image) and builds its wrapped
/// child as a persistent [`RenderNode`]. Layout-transparent: it measures, lays
/// out, and patches the child exactly as the child would on its own. Each flush
/// renders the child into the runtime's input texture, runs the filter into the
/// output texture, and draws the resulting image — reusing the runtime's
/// texture-reuse logic verbatim, with no cursor-bound effect slot.
pub(crate) struct AppliedFilterNode {
    runtime: Rc<RefCell<AppliedFilterRuntime>>,
    child: RenderNode,
    env: Environment,
}

impl GpuSurfaceNode {
    /// Push a GPU-surface compositor layer that carries the node-owned runtime by
    /// `Rc` (no cursor-ordered slot). Mirrors the dispatch path's
    /// [`HydrolysisRenderer::render_gpu_surface`] exactly, but with an `Owned`
    /// layer source so a per-frame re-flush re-binds the same runtime.
    pub(crate) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        renderer.push_gpu_surface_layer(
            GpuSurfaceSource::Owned(Rc::clone(&self.runtime)),
            ctx.transform,
            ctx.bounds,
        );
    }
}

impl ViewEffectNode {
    /// Render the captured child node into an input texture, run the effect into
    /// an output texture, and draw the output image — the node analogue of the
    /// dispatch path's [`HydrolysisRenderer::render_view_effect`], with the
    /// runtime and child owned by this node (no cursor-bound effect slot).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(crate) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        let mut runtime = self.runtime.borrow_mut();
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };

        let input_width = (ctx.bounds.width().max(1.0).round()) as u32;
        let input_height = (ctx.bounds.height().max(1.0).round()) as u32;
        let output_size = runtime.effect.output_size();
        let (output_width, output_height) = output_size.compute(input_width, input_height);
        assert!(
            !(output_width == 0 || output_height == 0),
            "hydrolysis ViewEffect requires non-zero output dimensions"
        );

        // Render the persistent child node into a fresh, standalone scene in local
        // coordinates (reactive descendants reach their own dedicated nodes and
        // stay live), then into the input texture.
        let subtree = renderer.render_child_node_scene(&self.child.borrow(), ctx, &self.env);

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_view_effect_input"),
            size: wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .vello_renderer
            .render_to_texture(
                &device,
                &queue,
                &subtree,
                &input_view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::TRANSPARENT,
                    width: input_width,
                    height: input_height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("hydrolysis ViewEffect failed to capture child scene");

        let setup_context = ViewEffectContext {
            device: &device,
            queue: &queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        if !runtime.setup_complete {
            pollster::block_on(runtime.effect.setup(&setup_context));
            runtime.setup_complete = true;
        }

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_view_effect_output"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let input = ViewEffectInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: input_width,
            height: input_height,
        };
        let output = ViewEffectOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        runtime.effect.render(&input, &output);
        let needs_redraw = runtime.effect.needs_redraw();
        drop(runtime);
        if needs_redraw {
            renderer.request_next_frame_rebuild();
        }

        let image = renderer.vello_renderer.register_texture(output_texture);
        renderer.compositor.active_filter_images.push(image.clone());
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(output_width),
                ctx.bounds.height() / f64::from(output_height),
            );
        renderer.scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }
}

impl AppliedFilterNode {
    /// Render the wrapped child node into the runtime's input texture, run the
    /// filter into the output texture, and draw the resulting image — the node
    /// analogue of the dispatch path's
    /// [`HydrolysisRenderer::render_applied_filter_metadata`], reusing the
    /// runtime's texture-reuse logic verbatim, with no cursor-bound effect slot.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(crate) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };

        let width = (ctx.bounds.width().max(1.0).round()) as u32;
        let height = (ctx.bounds.height().max(1.0).round()) as u32;
        let should_capture_input = {
            let runtime = self.runtime.borrow();
            !renderer.reuse_applied_filter_inputs || !runtime.has_input_texture(width, height)
        };
        let input_view = {
            let mut runtime = self.runtime.borrow_mut();
            let (_, view) = runtime.input_texture(&device, width, height);
            view.clone()
        };
        if should_capture_input {
            let capture_started_at = Instant::now();
            // Render the persistent child node into a fresh, standalone scene in
            // local coordinates (reactive descendants stay live), then into the
            // runtime's reused input texture.
            let subtree_scene = renderer.render_child_node_scene(&self.child, ctx, &self.env);
            renderer
                .vello_renderer
                .render_to_texture(
                    &device,
                    &queue,
                    &subtree_scene,
                    &input_view,
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::TRANSPARENT,
                        width,
                        height,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("hydrolysis AppliedFilter: failed to render subtree");
            renderer.frame_applied_filter_capture += capture_started_at.elapsed();
        }

        let effect_started_at = Instant::now();
        let (image, needs_redraw) = self.runtime.borrow_mut().render_output(
            &device,
            &queue,
            &mut renderer.vello_renderer,
            width,
            height,
        );
        renderer.frame_applied_filter_effect += effect_started_at.elapsed();
        renderer.frame_applied_filter_count = renderer
            .frame_applied_filter_count
            .checked_add(1)
            .expect("hydrolysis applied filter counter overflow");
        if needs_redraw {
            renderer.request_redraw();
        }

        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(image.width),
                ctx.bounds.height() / f64::from(image.height),
            );
        let scene = renderer.scene_mut();
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }
}

pub(crate) struct DynamicHostNode {
    /// The source `Dynamic`, kept alive so its identity cannot be reused while
    /// this node lives (the ABA guard from the legacy `DynamicNode`). Also read by
    /// [`RenderNode::collect_dynamic_identities`] to keep the measure-path dynamic
    /// dimension cache pruned to the identities still live in the retained tree.
    source: waterui_core::dynamic::Dynamic,
    /// Latest content delivered by the `Dynamic`, awaiting a patch.
    pending: Rc<RefCell<Option<AnyView>>>,
    /// Environment captured at build, used to rebuild the child on a change.
    env: Environment,
    /// The current expansion of the `Dynamic`'s content.
    child: RenderNode,
}

impl TextNode {
    /// Emit this text leaf's accessibility node, mirroring
    /// [`Native<TextConfig>::accessibility`] so the render-tree path produces the
    /// same a11y tree the dispatch path did. Called from `flush` with the node's
    /// scoped environment (label/role resolution reads env).
    #[cfg(feature = "accessibility")]
    fn emit_accessibility(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        styled: &StyledStr,
        env: &Environment,
    ) {
        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            return;
        }
        let plain = styled.to_plain().to_string();
        let default_label = (!plain.is_empty()).then_some(plain);
        let Some(label) = renderer.resolve_accessibility_label(env, default_label) else {
            return;
        };
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Label),
        );
        node.set_label(label);
        let _ = renderer.register_accessibility_node(
            node,
            transformed_rect(ctx.hit_transform, ctx.bounds),
            env,
            None,
        );
    }

    #[cfg(not(feature = "accessibility"))]
    #[allow(
        clippy::unused_self,
        reason = "parity with the accessibility-enabled signature"
    )]
    fn emit_accessibility(
        &self,
        _renderer: &mut HydrolysisRenderer,
        _ctx: RenderContext,
        _styled: &StyledStr,
        _env: &Environment,
    ) {
    }
}

/// Emit an `Image`-role accessibility node for a self-drawn graphics leaf
/// (`Canvas`/`SceneView`, `GpuSurface`, shapes/gradients) at its bounds, reading
/// the role/label scoped into `env` by any `.a11y_role()` / `.a11y_label()`
/// wrappers. These leaves draw their own pixels, so the node tree is the only place
/// their semantic node can be emitted — mirroring `TextNode::emit_accessibility`
/// for the text leaf. Suppressed when the subtree is accessibility-hidden.
#[cfg(feature = "accessibility")]
fn emit_graphics_image_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    env: &Environment,
) {
    if env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden)
    {
        return;
    }
    let mut node =
        AccessibilityNode::new(renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image));
    if let Some(label) = renderer.resolve_accessibility_label(env, None) {
        node.set_label(label);
    }
    let _ = renderer.register_accessibility_node(
        node,
        transformed_rect(ctx.hit_transform, ctx.bounds),
        env,
        None,
    );
}

#[cfg(not(feature = "accessibility"))]
fn emit_graphics_image_accessibility(
    _renderer: &mut HydrolysisRenderer,
    _ctx: RenderContext,
    _env: &Environment,
) {
}

impl RenderNode {
    /// The stretch axis this node exposes when it is a layout child.
    fn stretch(&self) -> StretchAxis {
        match self {
            RenderNode::Color(_) => StretchAxis::Both,
            RenderNode::Text(_) => StretchAxis::None,
            RenderNode::Container(container) => container.layout.stretch_axis(),
            RenderNode::Opacity(node) => node.child.stretch(),
            RenderNode::Scale(node) => node.child.stretch(),
            RenderNode::Rotation(node) => node.child.stretch(),
            RenderNode::Offset(node) => node.child.stretch(),
            RenderNode::Retain(node) => node.child.stretch(),
            RenderNode::Env(node) => node.child.stretch(),
            RenderNode::Dynamic(node) => node.child.stretch(),
            RenderNode::SceneView(_) => StretchAxis::Both,
            // A GpuSurface fills its proposal (`GpuView::stretch_axis` default);
            // a ViewEffect is a `StretchAxis::None` raw view; an AppliedFilter is
            // a layout-transparent wrapper delegating to its child.
            RenderNode::GpuSurface(_) => StretchAxis::Both,
            RenderNode::ViewEffect(_) => StretchAxis::None,
            RenderNode::AppliedFilter(node) => node.child.stretch(),
            RenderNode::Scroll(_) => StretchAxis::Both,
            RenderNode::LazyStack(_) => StretchAxis::Both,
            RenderNode::Collection(node) => node.layout.stretch_axis(),
            RenderNode::Wrapper(node) => node.child.stretch(),
            RenderNode::Widget(node) => node.stretch,
        }
    }

    /// Measure this node under a proposal (recursive). Text shaping runs through
    /// the renderer's [`HydroState`] on the main thread.
    fn measure(
        &self,
        state: &mut HydroState,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        match self {
            RenderNode::Color(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            RenderNode::Text(text) => HydrolysisRenderer::measure_text_dimensions(
                state,
                text.content.get(),
                text.alignment.get(),
                env,
                proposal.width,
                None,
            ),
            RenderNode::Container(container) => {
                let cell = RefCell::new(state);
                let subs: Vec<NodeSubView> = container
                    .children
                    .iter()
                    .map(|child| NodeSubView::new(child, &cell, env))
                    .collect();
                let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
                ViewDimensions::new(container.layout.size_that_fits(proposal, &refs))
            }
            // Transform/opacity wrappers are layout-transparent.
            RenderNode::Opacity(node) => node.child.measure(state, env, proposal),
            RenderNode::Scale(node) => node.child.measure(state, env, proposal),
            RenderNode::Rotation(node) => node.child.measure(state, env, proposal),
            RenderNode::Offset(node) => node.child.measure(state, env, proposal),
            RenderNode::Retain(node) => node.child.measure(state, env, proposal),
            RenderNode::Env(node) => node.child.measure(state, &node.env, proposal),
            RenderNode::Dynamic(node) => node.child.measure(state, env, proposal),
            RenderNode::SceneView(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            // A GpuSurface fills its proposal, like a self-drawn scene.
            RenderNode::GpuSurface(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            // A ViewEffect and an AppliedFilter are sized by their content: the
            // effect captures the child into a texture at the child's bounds.
            RenderNode::ViewEffect(node) => node.child.borrow().measure(state, &node.env, proposal),
            RenderNode::AppliedFilter(node) => node.child.measure(state, &node.env, proposal),
            RenderNode::Scroll(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            RenderNode::LazyStack(node) => node.measure(state, proposal),
            RenderNode::Collection(node) => node.measure(state, proposal),
            // Layout-transparent: the wrapper measures its child under the node's
            // scoped environment (effect colors/a11y read env every frame).
            RenderNode::Wrapper(node) => node.child.measure(state, &node.env, proposal),
            RenderNode::Widget(node) => (node.measure)(state, proposal, &node.env),
        }
    }

    /// Re-measure and re-place this subtree, caching each container's child
    /// frames. Run on build and whenever a geometry-affecting input changes.
    pub(crate) fn layout(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        size: Size,
    ) {
        // No proposal parameter: at layout time a node is always placed at a concrete
        // `size`, so a layout-transparent wrapper lays its child out at exactly that
        // size (`ProposalSize::new(Some(size.w), Some(size.h))`).
        match self {
            RenderNode::Container(container) => {
                let rects = {
                    let cell = RefCell::new(&mut renderer.state);
                    let subs: Vec<NodeSubView> = container
                        .children
                        .iter()
                        .map(|child| NodeSubView::new(child, &cell, env))
                        .collect();
                    let refs: Vec<&dyn SubView> =
                        subs.iter().map(|sub| sub as &dyn SubView).collect();
                    container.layout.place(Rect::from_size(size), &refs)
                };
                for (child, rect) in container.children.iter_mut().zip(rects.iter()) {
                    child.layout(renderer, env, *rect.size());
                }
                container.placed = rects;
            }
            // Transform/opacity wrappers are layout-transparent: the child lays out
            // at the same concrete size as the wrapper.
            RenderNode::Opacity(node) => node.child.layout(renderer, env, size),
            RenderNode::Scale(node) => node.child.layout(renderer, env, size),
            RenderNode::Rotation(node) => node.child.layout(renderer, env, size),
            RenderNode::Offset(node) => node.child.layout(renderer, env, size),
            RenderNode::Retain(node) => node.child.layout(renderer, env, size),
            RenderNode::Env(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, size);
            }
            // Layout-transparent: the child lays out at the same concrete size,
            // under the wrapper's scoped environment.
            RenderNode::Wrapper(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, size);
            }
            RenderNode::Dynamic(node) => node.child.layout(renderer, env, size),
            RenderNode::Scroll(node) => {
                let child_proposal = match node.axis {
                    ScrollAxis::Horizontal => ProposalSize::new(None, Some(size.height)),
                    ScrollAxis::Vertical => ProposalSize::new(Some(size.width), None),
                    ScrollAxis::All => ProposalSize::UNSPECIFIED,
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                };
                let intrinsic = node
                    .child
                    .measure(&mut renderer.state, env, child_proposal)
                    .size;
                let content_size = match node.axis {
                    ScrollAxis::Horizontal => {
                        Size::new(intrinsic.width.max(size.width), size.height)
                    }
                    ScrollAxis::Vertical => {
                        Size::new(size.width, intrinsic.height.max(size.height))
                    }
                    ScrollAxis::All => Size::new(
                        intrinsic.width.max(size.width),
                        intrinsic.height.max(size.height),
                    ),
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                };
                node.child.layout(renderer, env, content_size);
                node.handle = Some(renderer.bind_render_scroll_handle(
                    node.axis,
                    f64::from(size.width),
                    f64::from(size.height),
                    f64::from(content_size.width),
                    f64::from(content_size.height),
                ));
                node.content_size = content_size;
                node.viewport = size;
            }
            RenderNode::Collection(node) => node.layout(renderer, size),
            // A ViewEffect captures its child into a texture sized to the child's
            // bounds, so the child lays out at the concrete size (re-laid-out only
            // on a change). An AppliedFilter is layout-transparent: its child lays
            // out at the same concrete size under the node's scoped environment.
            RenderNode::ViewEffect(node) => {
                if size != node.laid_out.get() {
                    let node_env = node.env.clone();
                    node.child.borrow_mut().layout(renderer, &node_env, size);
                    node.laid_out.set(size);
                }
            }
            RenderNode::AppliedFilter(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, size);
            }
            // A lazy stack places its items lazily at flush (offset-dependent); a
            // widget leaf or GpuSurface renders itself at flush from `ctx.bounds`.
            // Nothing to pre-lay-out for any of these.
            RenderNode::Color(_)
            | RenderNode::Text(_)
            | RenderNode::SceneView(_)
            | RenderNode::GpuSurface(_)
            | RenderNode::LazyStack(_)
            | RenderNode::Widget(_) => {}
        }
    }

    /// Re-encode this subtree into the renderer's scene using the cached
    /// placements. Runs every frame.
    pub(crate) fn flush(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
    ) {
        match self {
            RenderNode::Color(color) => {
                renderer.scene_mut().fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    color.color,
                    None,
                    &ctx.bounds,
                );
            }
            RenderNode::Text(text) => {
                // Read the content/alignment signals through `read_signal` so a change
                // re-subscribes this frame and schedules a window refresh — the same
                // cheap pump every other reactive leaf uses. (A bare reactive `Text`
                // has no surrounding widget to watch it, so the node must do so itself.)
                let styled = renderer.read_signal(&text.content);
                let alignment = renderer.read_signal(&text.alignment);
                text.emit_accessibility(renderer, ctx, &styled, env);
                let (state, scene) = renderer.state_and_scene_mut();
                HydrolysisRenderer::render_styled_text(state, scene, ctx, styled, alignment, env);
            }
            RenderNode::Container(container) => {
                for (child, rect) in container.children.iter().zip(container.placed.iter()) {
                    let child_ctx = ctx.child(
                        vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y()))),
                        vello::kurbo::Rect::new(
                            0.0,
                            0.0,
                            f64::from(rect.width()),
                            f64::from(rect.height()),
                        ),
                    );
                    child.flush(renderer, child_ctx, env);
                }
            }
            RenderNode::Opacity(node) => {
                let alpha = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.value,
                    OPACITY_ANIMATION_KEY,
                );
                renderer.push_layer_rect(alpha, ctx.transform, ctx.bounds);
                node.child.flush(renderer, ctx, env);
                renderer.pop_layer();
            }
            RenderNode::Scale(node) => {
                let center = anchor_point(ctx.bounds, node.value.anchor);
                let scale_x = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.x,
                    SCALE_X_ANIMATION_KEY,
                );
                let scale_y = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.y,
                    SCALE_Y_ANIMATION_KEY,
                );
                let transform = vello::kurbo::Affine::translate((center.x, center.y))
                    * vello::kurbo::Affine::scale_non_uniform(
                        f64::from(scale_x),
                        f64::from(scale_y),
                    )
                    * vello::kurbo::Affine::translate((-center.x, -center.y));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            RenderNode::Rotation(node) => {
                let center = anchor_point(ctx.bounds, node.value.anchor);
                let radians = f64::from(renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.angle,
                    ROTATION_ANIMATION_KEY,
                ))
                .to_radians();
                let transform = vello::kurbo::Affine::translate((center.x, center.y))
                    * vello::kurbo::Affine::rotate(radians)
                    * vello::kurbo::Affine::translate((-center.x, -center.y));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            RenderNode::Offset(node) => {
                let offset_x = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.x,
                    OFFSET_X_ANIMATION_KEY,
                );
                let offset_y = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.y,
                    OFFSET_Y_ANIMATION_KEY,
                );
                let transform =
                    vello::kurbo::Affine::translate((f64::from(offset_x), f64::from(offset_y)));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            RenderNode::Dynamic(node) => node.child.flush(renderer, ctx, env),
            RenderNode::Retain(node) => node.child.flush(renderer, ctx, env),
            RenderNode::Env(node) => node.child.flush(renderer, ctx, &node.env),
            RenderNode::Wrapper(node) => {
                // Each effect re-applies through the shared `apply_*` helper, with
                // a closure that flushes the child node under the wrapper's scoped
                // environment — so reactive descendants reach their own nodes and
                // keep updating, instead of being frozen by a one-shot capture.
                let child_env = &node.env;
                match &node.effect {
                    WrapperEffect::Clip(value) => {
                        HydrolysisRenderer::apply_clip_shape(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Border(value) => {
                        HydrolysisRenderer::apply_border(renderer, ctx, child_env, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Shadow(value) => {
                        HydrolysisRenderer::apply_shadow(renderer, ctx, child_env, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Cursor(value) => {
                        HydrolysisRenderer::apply_cursor(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Draggable(value) => {
                        HydrolysisRenderer::apply_draggable(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::DropDestination(handles) => {
                        HydrolysisRenderer::apply_drop_destination(
                            renderer,
                            ctx,
                            child_env,
                            handles,
                            |r| {
                                node.child.flush(r, ctx, child_env);
                            },
                        );
                    }
                    WrapperEffect::ContextMenu(value) => {
                        HydrolysisRenderer::apply_context_menu(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Hittable(value) => {
                        HydrolysisRenderer::apply_hittable(renderer, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::OnEvent(handler) => {
                        HydrolysisRenderer::apply_on_event(
                            renderer,
                            ctx,
                            child_env,
                            Rc::clone(handler),
                            |r| {
                                node.child.flush(r, ctx, child_env);
                            },
                        );
                    }
                    WrapperEffect::GestureObserver(effect) => {
                        HydrolysisRenderer::apply_gesture_observer(
                            renderer,
                            ctx,
                            child_env,
                            effect,
                            |r| {
                                node.child.flush(r, ctx, child_env);
                            },
                        );
                    }
                    WrapperEffect::Focused(value) => {
                        HydrolysisRenderer::apply_focused(renderer, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::LifeCycle(_) => {
                        // Appear already fired at build; disappear fires from the
                        // effect's `Drop`. Flush is a plain passthrough.
                        node.child.flush(renderer, ctx, child_env);
                    }
                }
            }
            RenderNode::SceneView(node) => {
                emit_graphics_image_accessibility(renderer, ctx, env);
                let mut scene = vello::Scene::new();
                // Scope `scene2d` so its `&mut scene` borrow ends before `&scene` is
                // appended below.
                let needs_next = {
                    let mut scene2d = VelloScene2D::new(&mut scene);
                    #[allow(clippy::cast_possible_truncation)]
                    node.content.borrow_mut().build_scene(
                        &mut scene2d,
                        ctx.bounds.width() as f32,
                        ctx.bounds.height() as f32,
                    )
                };
                renderer.scene_mut().append(
                    &scene,
                    Some(
                        ctx.transform
                            * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0)),
                    ),
                );
                if needs_next {
                    renderer.request_next_frame_rebuild();
                }
            }
            RenderNode::GpuSurface(node) => {
                emit_graphics_image_accessibility(renderer, ctx, env);
                node.flush(renderer, ctx);
            }
            RenderNode::ViewEffect(node) => node.flush(renderer, ctx),
            RenderNode::AppliedFilter(node) => node.flush(renderer, ctx),
            RenderNode::Scroll(node) => {
                let Some(handle) = node.handle.clone() else {
                    return;
                };
                let metrics = handle.metrics();
                let viewport_rect = vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(node.viewport.width),
                    f64::from(node.viewport.height),
                );
                renderer.push_layer_rect(1.0, ctx.transform, viewport_rect);
                let scroll_offset =
                    vello::kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
                let content_bounds = vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(node.content_size.width),
                    f64::from(node.content_size.height),
                );
                let content_ctx = RenderContext::with_transforms(
                    content_bounds,
                    ctx.transform * scroll_offset,
                    ctx.hit_transform * scroll_offset,
                );
                // Publish the visible window (in content coordinates) so a
                // virtualized `LazyStack` child only builds the rows on screen.
                let lazy_viewport = vello::kurbo::Rect::new(
                    metrics.offset_x,
                    metrics.offset_y,
                    metrics.offset_x + f64::from(node.viewport.width),
                    metrics.offset_y + f64::from(node.viewport.height),
                );
                renderer.push_lazy_viewport(lazy_viewport);
                node.child.flush(renderer, content_ctx, env);
                renderer.pop_lazy_viewport("hydrolysis render tree ScrollNode");
                renderer.pop_layer();
                let target_handle = handle.clone();
                renderer.register_scroll_target(
                    transformed_rect(ctx.hit_transform, viewport_rect),
                    move |dx, dy, is_line_delta| {
                        target_handle.apply_scroll_delta(dx, dy, is_line_delta)
                    },
                );
                crate::widgets::scroll::register_scroll_accessibility_node(
                    renderer,
                    &node.env,
                    transformed_rect(ctx.hit_transform, viewport_rect),
                    &handle,
                    metrics,
                    node.axis,
                );
                let scroll_ctx =
                    RenderContext::with_transforms(viewport_rect, ctx.transform, ctx.hit_transform);
                let mut widget_ctx = WidgetRenderContext::new(renderer, scroll_ctx);
                crate::widgets::draw_scroll_indicators(
                    &mut widget_ctx,
                    &node.env,
                    viewport_rect,
                    metrics,
                    node.axis,
                );
            }
            RenderNode::LazyStack(node) => node.flush(renderer, ctx, env),
            RenderNode::Collection(node) => node.flush(renderer, ctx),
            RenderNode::Widget(node) => {
                // Re-render the leaf widget from its retained config so its handler
                // re-reads live signals and re-emits interaction targets + a11y at the
                // current bounds. A leaf render starts a fresh recursion depth.
                renderer.render_depth = 0;
                (node.render)(renderer, ctx, &node.env);
            }
        }
    }

    /// Build a node from a view, capturing live reactive inputs. Native leaves
    /// and layout containers map to concrete nodes; composite views expand via
    /// `body()` once and recurse.
    pub(crate) fn build(
        view: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let view = match view.downcast::<Native<ResolvedColor>>() {
            Ok(color) => {
                return RenderNode::Color(ColorNode {
                    color: resolved_color_to_peniko((*color).into_inner()),
                });
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<TextConfig>>() {
            Ok(text) => {
                let config = (*text).into_inner();
                return RenderNode::Text(Box::new(TextNode {
                    content: config.content,
                    alignment: config.paragraph_alignment,
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<FixedContainer>>() {
            Ok(container) => {
                let (layout, children) = (*container).into_inner().into_inner();
                let children = children
                    .into_iter()
                    .map(|child| {
                        RenderNode::build(normalize_layout_view(child, env), env, renderer)
                    })
                    .collect();
                return RenderNode::Container(Box::new(ContainerNode {
                    layout,
                    children,
                    placed: Vec::new(),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<LazyContainer>>() {
            Ok(container) => {
                let (layout, children) = (*container).into_inner().into_inner();
                // A viewport-virtualizable stack layout (and not opting into a
                // membership transition, which must retain every item) becomes a
                // virtualized LazyStack: only visible rows are built/measured.
                let wants_transition = env
                    .get::<waterui_layout::collection_transition::CollectionTransition>()
                    .is_some();
                if let Some(axis) =
                    lazy_stack_axis_config(layout.as_ref()).filter(|_| !wants_transition)
                {
                    return RenderNode::build_lazy_stack(axis, children, env, renderer);
                }
                // A non-virtualizable layout (AbsoluteLayout/ZStack overlay) or a
                // transition collection: a retained reactive collection that
                // reconciles membership by id (recursing into each item, so inner
                // SceneView/Dynamic reach their dedicated nodes).
                return RenderNode::build_collection(layout, children, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Opacity>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Opacity(Box::new(OpacityNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Scale>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Scale(Box::new(ScaleNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Rotation>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Rotation(Box::new(RotationNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Offset>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Offset(Box::new(OffsetNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Environment>>() {
            Ok(meta) => {
                let (content, scoped_env) =
                    flatten_environment_metadata_owned(AnyView::new(*meta), env);
                // Carry the scoped environment in the node (not flattened away), so
                // it is also the env used at flush/measure/layout — text shaping and
                // a11y read env every frame.
                let child = RenderNode::build(content, &scoped_env, renderer);
                return RenderNode::Env(Box::new(EnvNode {
                    env: scoped_env,
                    child,
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Retain>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Retain(Box::new(RetainNode {
                    retain: value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        // Accessibility metadata are environment-scoping wrappers (the dispatch
        // handlers just `env.insert(value)` then render the content), so the tree
        // models them as an `Env` node carrying the extended environment — the Text
        // node and a11y emission read these from env every flush, and the wrapped
        // (possibly reactive) content stays live instead of freezing in `Captured`.
        let view = match view.downcast::<IgnorableMetadata<AccessibilityLabel>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityRole>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityHidden>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityChildren>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityState>>() {
            Ok(meta) => {
                let IgnorableMetadata { content, value } = *meta;
                let mut scoped = env.clone();
                if value.is_hidden() {
                    scoped.insert(AccessibilityHidden::new(true));
                }
                scoped.insert(value);
                let child = RenderNode::build(content, &scoped, renderer);
                return RenderNode::Env(Box::new(EnvNode { env: scoped, child }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityStateSignal>>() {
            Ok(meta) => {
                let IgnorableMetadata { content, value } = *meta;
                let state = value.state().get();
                let mut scoped = env.clone();
                if state.is_hidden() {
                    scoped.insert(AccessibilityHidden::new(true));
                }
                scoped.insert(state);
                let child = RenderNode::build(content, &scoped, renderer);
                return RenderNode::Env(Box::new(EnvNode { env: scoped, child }));
            }
            Err(view) => view,
        };
        // Passthrough metadata: the dispatch handlers discard the value and just
        // render the content (no-ops in Hydrolysis), so the tree unwraps them to
        // the content directly — fully transparent, keeping reactive descendants live.
        let view = match view.downcast::<Metadata<Secure>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<StandardDynamicRange>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<HighDynamicRange>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<IgnoreSafeArea>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<ContextMenu>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Background>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<MaterialBackground>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        // Transparent metadata wrappers: each applies its visual/interaction
        // effect every flush and recurses into the child node, so reactive
        // descendants reach their dedicated nodes (instead of freezing inside a
        // one-shot `Captured`). The effect is shared with the dispatch path via
        // the `apply_*` helpers in `metadata.rs`.
        let view = match view.downcast::<Metadata<ClipShape>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Clip(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Border>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Border(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Shadow>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Shadow(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Cursor>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Cursor(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Draggable>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Draggable(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<DropDestination>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::DropDestination(DropDestinationHandles::from_destination(value)),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<ResolvedContextMenu>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::ContextMenu(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Hittable>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Hittable(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<OnEvent>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::OnEvent(Rc::new(RefCell::new(value))),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<GestureObserver>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                let GestureObserver {
                    gesture, action, ..
                } = value;
                // A node has no `content` at flush, so resolve the two
                // content-derived pieces now (the default a11y label string and
                // the gesture group identity) and store them in the effect.
                #[cfg(feature = "accessibility")]
                let default_a11y_label = renderer.accessibility_label_from_view(&content, env);
                #[cfg(not(feature = "accessibility"))]
                let default_a11y_label = None;
                let effect = GestureObserverEffect {
                    gesture,
                    action: Rc::new(RefCell::new(action)),
                    default_a11y_label,
                    gesture_group_identity: gesture_group_identity(&content),
                };
                return RenderNode::build_wrapper(
                    WrapperEffect::GestureObserver(effect),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Focused>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Focused(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<LifeCycleHook>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_lifecycle(value, content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ScrollView>>() {
            Ok(scroll) => {
                let (axis, content) = (*scroll).into_inner().into_inner();
                let content = normalize_layout_view(content, env);
                return RenderNode::Scroll(Box::new(ScrollNode {
                    axis,
                    child: RenderNode::build(content, env, renderer),
                    handle: None,
                    content_size: Size::zero(),
                    viewport: Size::zero(),
                    env: env.clone(),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SceneView>>() {
            Ok(scene_view) => {
                return RenderNode::build_scene_view_node(*scene_view, renderer);
            }
            Err(view) => view,
        };
        // GPU/effect leaves and wrappers: each owns its effect runtime directly
        // (textures, setup state, redraw handle), so a reactive swap renders the
        // new content and a per-frame re-flush re-binds the *same* runtime — no
        // cursor-ordered effect slot to desync (the chart Bug 1 fix, generalized).
        let view = match view.downcast::<Native<GpuSurface>>() {
            Ok(surface) => {
                return RenderNode::build_gpu_surface((*surface).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ViewEffectErased>>() {
            Ok(effect) => {
                return RenderNode::build_view_effect((*effect).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<AppliedFilter>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_applied_filter(value, content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Dynamic>>() {
            Ok(dynamic) => {
                return RenderNode::build_dynamic_host((*dynamic).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        // A native widget leaf: build a `Widget` node that re-renders it every flush
        // from a retained, signal-holding config (action retained behind `Rc`), so
        // its handler re-reads live signals — reactive labels/values stay live
        // instead of freezing in a one-shot `Captured` bake.
        let view = match view.downcast::<Native<ButtonConfig>>() {
            Ok(button) => {
                return RenderNode::build_button((*button).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedMenu>>() {
            Ok(menu) => return RenderNode::build_menu((*menu).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ToggleConfig>>() {
            Ok(toggle) => return RenderNode::build_toggle((*toggle).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SliderConfig>>() {
            Ok(slider) => return RenderNode::build_slider((*slider).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<StepperConfig>>() {
            Ok(stepper) => {
                return RenderNode::build_stepper((*stepper).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ProgressConfig>>() {
            Ok(progress) => {
                return RenderNode::build_progress((*progress).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<DatePickerConfig>>() {
            Ok(date_picker) => {
                return RenderNode::build_date_picker((*date_picker).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ColorPickerConfig>>() {
            Ok(color_picker) => {
                return RenderNode::build_color_picker((*color_picker).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<PickerConfig>>() {
            Ok(picker) => {
                return RenderNode::build_picker((*picker).into_inner(), env);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedTextFieldConfig>>() {
            Ok(text_field) => {
                return RenderNode::build_text_field((*text_field).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SecureFieldConfig>>() {
            Ok(secure_field) => {
                return RenderNode::build_secure_field((*secure_field).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<BadgeConfig>>() {
            Ok(badge) => return RenderNode::build_badge((*badge).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ListConfig>>() {
            Ok(list) => return RenderNode::build_list((*list).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<TableConfig>>() {
            Ok(table) => return RenderNode::build_table((*table).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SystemIcon>>() {
            Ok(icon) => return RenderNode::build_icon((*icon).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedGradient>>() {
            Ok(gradient) => return RenderNode::build_gradient((*gradient).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedShape>>() {
            Ok(shape) => return RenderNode::build_shape((*shape).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedMorphShape>>() {
            Ok(shape) => return RenderNode::build_morph_shape((*shape).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<MapConfig>>() {
            Ok(map) => return RenderNode::build_map((*map).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<WebView>() {
            Ok(webview) => return RenderNode::build_webview(*webview, env, renderer),
            Err(view) => view,
        };
        // Navigation containers (navigation view / split / stack / tabs): each is a
        // persistent `Widget` node re-rendered every flush from a retained config —
        // the navigation controller, transition slot, and tab-bar state stay live
        // and reactive route/tab selection keeps driving updates, instead of
        // freezing in a one-shot `Captured` bake.
        let view = match view.downcast::<Native<NavigationView>>() {
            Ok(navigation) => {
                return RenderNode::build_navigation_view(
                    (*navigation).into_inner(),
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<NavigationSplitLayout>>() {
            Ok(split) => {
                return RenderNode::build_navigation_split((*split).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<NavigationStack<(), ()>>>() {
            Ok(stack) => {
                return RenderNode::build_navigation_stack((*stack).into_inner(), env);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Tabs>>() {
            Ok(tabs) => return RenderNode::build_tabs((*tabs).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Spacer>>() {
            Ok(spacer) => return RenderNode::build_spacer((*spacer).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<()>>() {
            // `Native<()>` carries no data — drop the wrapper and build the empty leaf.
            Ok(_) => return RenderNode::build_empty(env),
            Err(view) => view,
        };
        // `Divider` and `Str` are registered renderers (not `Native<…>` leaves), so
        // they are downcast as their value type directly and built into persistent
        // `Widget` nodes that re-render from a retained cell each flush.
        let view = match view.downcast::<Divider>() {
            Ok(divider) => return RenderNode::build_divider(*divider, env),
            Err(view) => view,
        };
        let view = match view.downcast::<Str>() {
            Ok(text) => return RenderNode::build_str(*text, env),
            Err(view) => view,
        };
        // Every native leaf and metadata wrapper now has a dedicated `RenderNode`
        // build arm above. Anything reaching here is a composite, expanded via
        // `body()` once. A native leaf with no build arm panics here in `body()` —
        // the acceptable fast-fail for a missing arm.
        RenderNode::build(AnyView::new(view.body(env)), env, renderer)
    }

    /// Build a `Widget` node from its per-flush render + measure closures and its
    /// stretch axis. The closures retain the widget's signal-holding config.
    fn build_widget(
        render: WidgetRenderFn,
        measure: WidgetMeasureFn,
        stretch: StretchAxis,
        env: &Environment,
    ) -> RenderNode {
        RenderNode::Widget(Box::new(WidgetNode {
            render,
            measure,
            stretch,
            env: env.clone(),
        }))
    }

    /// Build a persistent button node: retain the config behind an `Rc<RefCell<…>>`
    /// (its `Label` carries the live content signal; its action is invoked through the
    /// shared cell), and re-render it every flush so a reactive label stays live.
    fn build_button(
        config: ButtonConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::button::ButtonRenderState;
        let mut state = ButtonRenderState::from_config(config);
        // Pre-build the general label sub-view (the measure path has no renderer).
        state.prebuild_label(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    crate::widgets::controls::button::render_button_node(
                        &mut widget_ctx,
                        &state,
                        env,
                    );
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    crate::widgets::controls::button::measure_button_node(
                        &state.borrow(),
                        proposal,
                        hydro,
                        env,
                    )
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, StretchAxis::None, env)
    }

    /// Build a persistent toggle node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the control + accessibility, and its
    /// `toggle` binding is read through `resolve_toggle_progress` which watches it.
    fn build_toggle(
        config: ToggleConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::toggle::{
            ToggleRenderState, measure_toggle_node, render_toggle_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ToggleRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_toggle_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_toggle_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent slider node: its value-end labels are move-only
    /// `AnyView`s, so they are pre-built into [`RetainedSubview`]s (the measure
    /// path has only `&mut HydroState`, no renderer to build on); the `value`
    /// binding is read through `read_signal` so a change schedules a frame.
    fn build_slider(
        config: SliderConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::slider::{
            SliderRenderState, measure_slider_node, render_slider_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SliderRenderState::from_config(config);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_slider_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_slider_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent stepper node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the buttons + accessibility, and its
    /// value/step signals are read through `read_signal` so a change schedules a frame.
    fn build_stepper(
        config: StepperConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::stepper::{
            StepperRenderState, measure_stepper_node, render_stepper_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = StepperRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_stepper_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_stepper_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent progress node: its label/value labels are move-only
    /// `AnyView`s pre-built into [`RetainedSubview`]s; the `value` is read through
    /// `read_signal` so a change schedules a frame. Stretch is style-dependent
    /// (Linear → Horizontal, Circular → None), read from the config.
    fn build_progress(
        config: ProgressConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::progress::{
            ProgressRenderState, measure_progress_node, render_progress_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ProgressRenderState::from_config(config);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_progress_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_progress_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent menu node: its trigger label is a move-only `AnyView`
    /// pre-built into a [`RetainedSubview`]; its `accessibility_label` and `items`
    /// signals are read through `read_signal` so a change schedules a frame.
    fn build_menu(
        menu: ResolvedMenu,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::button::{
            MenuRenderState, measure_menu_node, render_menu_node,
        };
        let stretch = <ResolvedMenu as waterui_core::NativeView>::stretch_axis(&menu);
        let mut state = MenuRenderState::from_resolved(menu);
        state.prebuild_label(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_menu_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_menu_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent date-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the field + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    fn build_date_picker(
        config: DatePickerConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::date_picker::{
            DatePickerRenderState, measure_date_picker_node, render_date_picker_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = DatePickerRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_date_picker_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_date_picker_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent color-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the swatch + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    fn build_color_picker(
        config: ColorPickerConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::color_picker::{
            ColorPickerRenderState, measure_color_picker_node, render_color_picker_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ColorPickerRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_color_picker_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_color_picker_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent picker node: retain the config (its `items`/`selection`
    /// signals are read through `read_signal` each frame so a membership or selection
    /// change schedules a frame). Stretch is content-sized (read from the config).
    fn build_picker(config: PickerConfig, env: &Environment) -> RenderNode {
        use crate::widgets::controls::picker::{measure_picker_node, render_picker_node};
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let config = Rc::new(RefCell::new(config));
        // Node-owned menu open/closed state: it persists across frames as a field of
        // this node's render closure, not in a renderer-global, flush-order-indexed
        // slot pool. The renderer keeps only an Rc-pruned registry of these handles so
        // an outside click can dismiss every open menu; a dropped picker's handle falls
        // out of the registry by strong count.
        let menu_open = Rc::new(Cell::new(false));
        let render = {
            let config = Rc::clone(&config);
            let menu_open = Rc::clone(&menu_open);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_picker_node(&mut widget_ctx, &config, &menu_open, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let config = Rc::clone(&config);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_picker_node(&config.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent text-field node: its floating label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on) and re-flushed under the animated label transform each frame; the
    /// clonable config's prompt/value/selection_menu are read each frame, with the
    /// value `Binding<StyledStr>` read through `read_signal` so typing or a binding
    /// change schedules a frame. The node re-runs the same text-input target
    /// registration each flush, so cursor/focus/IME state is preserved. Stretch is
    /// horizontal (read from the config).
    fn build_text_field(
        config: ResolvedTextFieldConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::text_field::{
            TextFieldRenderState, measure_text_field_node, render_text_field_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = TextFieldRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_text_field_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_text_field_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent secure-field node: its floating label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on) and re-flushed under the animated label transform each frame; the
    /// clonable config's `Binding<Secure>` value is read through `read_signal` each
    /// frame so typing or a binding change schedules a frame and the masked display
    /// updates. The node re-runs the same text-input target registration each flush,
    /// preserving cursor/focus/IME state. Stretch is horizontal (read from the config).
    fn build_secure_field(
        config: SecureFieldConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::text_field::{
            SecureFieldRenderState, measure_secure_field_node, render_secure_field_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SecureFieldRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_secure_field_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_secure_field_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent badge node: its wrapped content is a move-only `AnyView`
    /// pre-built into a [`RetainedSubview`]; the `value` is read through
    /// `read_signal` so a change schedules a frame. Badge sizes to its content and
    /// never stretches (`StretchAxis::None`, read from the config).
    fn build_badge(
        config: BadgeConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::layout::badge::{
            BadgeRenderState, measure_badge_node, render_badge_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = BadgeRenderState::from_config(config);
        state.prebuild_content(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_badge_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_badge_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent list node: retain the config (its `contents` collection,
    /// `editing` signal, and `on_delete`/`on_move` handlers stay owned by the cell).
    /// Each flush re-resolves the visible row window from the enclosing scroll's
    /// pushed viewport and re-dispatches only those rows, so reactive row content and
    /// a changing collection length stay live; the `LazyListController` row-extent
    /// cache (keyed by body-order cursor slot) persists across frames. Stretches to
    /// fill the proposal (`StretchAxis::Both`, read from the config).
    fn build_list(config: ListConfig, env: &Environment) -> RenderNode {
        use crate::widgets::layout::list::{ListRenderState, measure_list_node, render_list_node};
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = Rc::new(RefCell::new(ListRenderState::from_config(config)));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_list_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_list_node(&state.borrow().config, proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent table node: retain the config (its `columns` signal and
    /// per-column reactive `rows` stay owned by the cell). Each flush re-reads the
    /// columns, re-resolves the visible row/column windows from the enclosing
    /// scroll's pushed viewport, and re-dispatches only those header/data cells, so
    /// reactive cell content and a changing column/row set stay live; the
    /// `LazyTableController` column-width/row-count cache (keyed by body-order cursor
    /// slot) persists across frames. Stretch is read from the config.
    fn build_table(config: TableConfig, env: &Environment) -> RenderNode {
        use crate::widgets::layout::table::{
            TableRenderState, measure_table_node, render_table_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = Rc::new(RefCell::new(TableRenderState::from_config(config)));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_table_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_table_node(&state.borrow().config, proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent navigation-view node: its bar `title`/`leading`/`trailing`
    /// and screen `content` are move-only `AnyView`s pre-built into
    /// [`RetainedSubview`]s; the bar's `color`/`hidden` signals are read through
    /// `read_signal` each frame so an appearance change schedules a frame and the
    /// bar re-renders. Stretch is `Both` (read from the config).
    fn build_navigation_view(
        navigation: NavigationView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::nav::navigation::{
            NavigationViewRenderState, measure_navigation_view_node, render_navigation_view_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&navigation);
        let mut state = NavigationViewRenderState::from_view(navigation);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_navigation_view_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        // The navigation view's measure reads its un-retained config; it is
        // intrinsic-sized (or fills a concrete proposal), independent of the
        // retained sub-views, so the measure closure holds the original config.
        let measure_config = Rc::clone(&state);
        let measure = Box::new(
            move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                measure_navigation_view_node(&measure_config.borrow(), proposal, hydro, env)
            },
        )
            as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>;
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent navigation-split node: the layout's sidebar/detail/
    /// placeholder are `Rc`-backed builders rebuilt fresh and re-dispatched each
    /// frame, and the `selection` binding is read through `read_signal` so a
    /// selection change schedules a frame and the detail re-resolves. Stretch is
    /// `Both` (read from the config).
    fn build_navigation_split(
        split: NavigationSplitLayout,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::nav::navigation::{
            NavigationSplitRenderState, measure_navigation_split_node, render_navigation_split_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&split);
        let mut state = NavigationSplitRenderState::from_layout(split);
        // Pre-build the sidebar + placeholder sub-views (the measure path has no renderer).
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_navigation_split_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_navigation_split_node(&state.borrow().split, proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent navigation-stack node: the move-only stack root is held in
    /// a [`RetainedSubview`] (re-rendered into a fresh scene each frame for the
    /// transition cross-fade), and pushed destinations are rebuilt from their
    /// `Rc`-backed builders each flush; the navigation controller and transition
    /// slot persist across frames so push/pop transitions keep animating. Stretch is
    /// `Both` (read from the config).
    fn build_navigation_stack(stack: NavigationStack<(), ()>, env: &Environment) -> RenderNode {
        use crate::widgets::nav::navigation::{
            NavigationStackRenderState, measure_navigation_stack_node, render_navigation_stack_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&stack);
        let state = Rc::new(RefCell::new(NavigationStackRenderState::from_stack(stack)));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_navigation_stack_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_navigation_stack_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent tabs node: the tab labels are move-only `AnyView`s
    /// pre-built into [`RetainedSubview`]s, the selected tab's content is rebuilt
    /// from its `Rc`-backed builder each flush, and the `selection` binding is read
    /// through `read_signal` so a tab change schedules a frame and the active
    /// content/indicator updates. Stretch is `Both` (read from the config).
    fn build_tabs(tabs: Tabs, env: &Environment, renderer: &mut HydrolysisRenderer) -> RenderNode {
        use crate::widgets::nav::tabs::{TabsRenderState, measure_tabs_node, render_tabs_node};
        let stretch = waterui_core::NativeView::stretch_axis(&tabs);
        let mut state = TabsRenderState::from_tabs(tabs);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_tabs_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_tabs_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent system-icon node: the placeholder marker is a move-only
    /// `AnyView` pre-built into a [`RetainedSubview`] (Hydrolysis has no OS icon
    /// catalog, so it draws a neutral missing-glyph marker). Static, content-sized
    /// (`StretchAxis::None`, read from the config).
    fn build_icon(
        icon: SystemIcon,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::visual::icon::{IconRenderState, measure_icon_node, render_icon_node};
        let stretch = waterui_core::NativeView::stretch_axis(&icon);
        let mut state = IconRenderState::from_icon(&icon, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_icon_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_icon_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent gradient node: retain the fully-resolved gradient payload
    /// behind an `Rc<RefCell<…>>` and re-fill it every flush at the current bounds.
    /// The payload carries no signal, so nothing is watched; the gradient stretches
    /// to fill the proposal (`StretchAxis::Both`, read from the payload).
    fn build_gradient(gradient: ResolvedGradient, env: &Environment) -> RenderNode {
        use crate::renderer::{measure_gradient_node, render_gradient_node};
        let stretch = waterui_core::NativeView::stretch_axis(&gradient);
        let gradient = Rc::new(RefCell::new(gradient));
        let render = {
            let gradient = Rc::clone(&gradient);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_gradient_node(&mut widget_ctx, &gradient, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let gradient = Rc::clone(&gradient);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_gradient_node(&gradient.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent shape node: retain the fully-resolved shape payload and
    /// re-fill its bounds-aware path every flush. The payload carries no signal, so
    /// nothing is watched; the shape stretches to fill the proposal
    /// (`StretchAxis::Both`, read from the payload).
    fn build_shape(shape: ResolvedShape, env: &Environment) -> RenderNode {
        use crate::renderer::{measure_shape_node, render_shape_node};
        let stretch = waterui_core::NativeView::stretch_axis(&shape);
        let shape = Rc::new(RefCell::new(shape));
        let render = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_shape_node(&mut widget_ctx, &shape, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_shape_node(&shape.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent morph-shape node: retain the resolved morph payload and
    /// re-fill its interpolated path every flush. The morph progress is resolved
    /// through the animation controller each frame — an explicit `progress` signal
    /// is watched via `read_signal`/`resolve_animated_scalar_with_discriminator`, and
    /// a time-based animation drives continuous frames via `sample_morph_progress` —
    /// so the morph stays live. Stretches to fill the proposal (`StretchAxis::Both`).
    fn build_morph_shape(shape: ResolvedMorphShape, env: &Environment) -> RenderNode {
        use crate::renderer::{measure_morph_shape_node, render_morph_shape_node};
        let stretch = waterui_core::NativeView::stretch_axis(&shape);
        let shape = Rc::new(RefCell::new(shape));
        let render = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_morph_shape_node(&mut widget_ctx, &shape, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_morph_shape_node(&shape.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent map node: the map is a Rust-side composer whose content
    /// (`vstack` of a gradient surface + reactive region/annotation `Text`s) is
    /// built once into a [`RetainedSubview`] and re-flushed at the node's bounds each
    /// frame. Region/annotation reactivity is carried by the inner `Text` nodes
    /// (`config.region.map(...)`), which become live `Dynamic`/`Text` nodes inside the
    /// sub-view, so a region or annotation change updates without rebuilding the node.
    /// A11y is render-driven (the inner content's own dispatch emits it). Stretches to
    /// fill the proposal (`StretchAxis::Both`, read from the config).
    fn build_map(
        config: MapConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::visual::map::{MapRenderState, measure_map_node, render_map_node};
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = MapRenderState::from_config(config, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_map_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_map_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent webview node: the webview is a Rust-side composer whose
    /// content (`vstack` of a gradient surface + reactive status/navigation `Text`s)
    /// is built once into a [`RetainedSubview`] and re-flushed each frame. The
    /// `event`/`can_go_back`/`can_go_forward` reactivity is carried by the inner
    /// `Text` nodes, which become live `Dynamic`/`Text` nodes, so a navigation or load
    /// event updates without rebuilding the node. A11y is render-driven (the inner
    /// content's own dispatch emits it). Stretches to fill the proposal.
    fn build_webview(
        webview: WebView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::platform::webview::{
            WebViewRenderState, measure_webview_node, render_webview_node,
        };
        let stretch = waterui_core::View::stretch_axis(&webview);
        let mut state = WebViewRenderState::from_view(&webview, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_webview_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_webview_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent spacer node: a no-op render with zero intrinsic; it
    /// expands during placement, not from its intrinsic size. Stretch is its
    /// main-axis fill (`StretchAxis::MainAxis`, read from the config).
    fn build_spacer(spacer: Spacer, env: &Environment) -> RenderNode {
        use crate::widgets::layout::spacer::{measure_spacer_node, render_spacer_node};
        let stretch = waterui_core::NativeView::stretch_axis(&spacer);
        let spacer = Rc::new(RefCell::new(spacer));
        let render = {
            let spacer = Rc::clone(&spacer);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_spacer_node(&mut widget_ctx, &spacer, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let spacer = Rc::clone(&spacer);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_spacer_node(&spacer.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent empty (`()`) node: a no-op render with zero intrinsic and
    /// no accessibility. Never stretches (`StretchAxis::None`, read from the unit
    /// view).
    fn build_empty(env: &Environment) -> RenderNode {
        use crate::widgets::layout::spacer::{measure_empty_node, render_empty_node};
        let stretch = waterui_core::NativeView::stretch_axis(&());
        let empty = Rc::new(RefCell::new(()));
        let render = {
            let empty = Rc::clone(&empty);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_empty_node(&mut widget_ctx, &empty, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let empty = Rc::clone(&empty);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_empty_node(&empty.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent divider node: a static separator line drawn from theme
    /// metrics, oriented by the enclosing stack axis (read from env every flush).
    /// Stretches on its cross axis (`StretchAxis::CrossAxis`, matching the dispatch
    /// path's [`effective_stretch_axis`] for `Divider`).
    fn build_divider(divider: Divider, env: &Environment) -> RenderNode {
        use crate::widgets::divider::{measure_divider_node, render_divider_node};
        let divider = Rc::new(RefCell::new(divider));
        let render = {
            let divider = Rc::clone(&divider);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_divider_node(&mut widget_ctx, &divider, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let divider = Rc::clone(&divider);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_divider_node(&divider.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, StretchAxis::CrossAxis, env)
    }

    /// Build a persistent string node: an immutable `Str` rendered as plain styled
    /// text each flush (no signal — its content never changes for a given node).
    /// Never stretches (`StretchAxis::None`, like text).
    fn build_str(text: Str, env: &Environment) -> RenderNode {
        use crate::renderer::views::{measure_str_node, render_str_node};
        let text = Rc::new(RefCell::new(text));
        let render = {
            let text = Rc::clone(&text);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_str_node(&mut widget_ctx, &text, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let text = Rc::clone(&text);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_str_node(&text.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, StretchAxis::None, env)
    }

    /// Build a transparent wrapper node: capture the per-flush effect and recurse
    /// into the child so reactive descendants reach their own dedicated nodes.
    fn build_wrapper(
        effect: WrapperEffect,
        content: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        RenderNode::Wrapper(Box::new(WrapperNode {
            effect,
            env: env.clone(),
            child: RenderNode::build(content, env, renderer),
        }))
    }

    /// Build a node-owned lifecycle hook wrapper. An appear hook fires its callback
    /// once now (before the child subtree is built, matching the old dispatch
    /// order); a disappear hook is retained in the effect and fired from its `Drop`
    /// when the node leaves the retained tree. No frame-diff slot cursor — structural
    /// removal alone drives disappearance.
    fn build_lifecycle(
        hook: LifeCycleHook,
        content: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let effect = match hook.lifecycle() {
            LifeCycle::Appear => {
                hook.handle(env);
                LifeCycleEffect { disappear: None }
            }
            LifeCycle::Disappear => LifeCycleEffect {
                disappear: Some(DeferredLifeCycleHook::new(hook, env.clone())),
            },
            _ => panic!("hydrolysis lifecycle variant is not supported"),
        };
        RenderNode::build_wrapper(WrapperEffect::LifeCycle(effect), content, env, renderer)
    }

    /// Build an environment-scoping `Env` node from an [`IgnorableMetadata`] whose
    /// only effect is `env.insert(value)` (the accessibility metadata wrappers): the
    /// extended environment travels with the node so it is read at every flush, and
    /// the wrapped content recurses so reactive descendants stay live.
    fn build_env_scoped<T: MetadataKey + 'static>(
        meta: IgnorableMetadata<T>,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let IgnorableMetadata { content, value } = meta;
        let mut scoped = env.clone();
        scoped.insert(value);
        let child = RenderNode::build(content, &scoped, renderer);
        RenderNode::Env(Box::new(EnvNode { env: scoped, child }))
    }

    /// Build a retained reactive collection (non-virtualized): materialize every
    /// current item keyed by id and subscribe to membership changes (a change marks
    /// it dirty and schedules a refresh, which reconciles by id).
    fn build_collection(
        layout: Box<dyn Layout>,
        views: AnyViews<AnyView>,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let dirty = Rc::new(Cell::new(false));
        let dirty_key = Rc::new(());
        let key = Rc::as_ptr(&dirty_key) as usize;
        let signals = renderer.signals.clone();
        let guard = views.watch(.., {
            let dirty = Rc::clone(&dirty);
            move |_changed| {
                dirty.set(true);
                signals.mark_collection_dirty(key, 0);
            }
        });
        let len = views.len().get();
        let items = (0..len)
            .map(|index| {
                let id = views
                    .get_id(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"));
                let view = views
                    .get_view(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                (
                    id,
                    RenderNode::build(normalize_layout_view(view, env), env, renderer),
                )
            })
            .collect();
        RenderNode::Collection(Box::new(CollectionNode {
            layout,
            views,
            env: env.clone(),
            items,
            placed: Vec::new(),
            dirty,
            dirty_key,
            guard,
        }))
    }

    /// Build a viewport-virtualized lazy stack: store the collection and subscribe
    /// to membership changes (a change schedules a refresh so the visible window
    /// re-resolves). Items are materialized lazily at flush, not here.
    fn build_lazy_stack(
        axis: LazyStackAxisConfig,
        views: AnyViews<AnyView>,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let dirty_key = Rc::new(());
        let key = Rc::as_ptr(&dirty_key) as usize;
        let signals = renderer.signals.clone();
        let guard = views.watch(.., move |_changed| {
            // Membership changed: request a fine-grained refresh; the flush re-reads
            // the collection length/items and re-resolves the visible window.
            signals.mark_collection_dirty(key, 0);
        });
        RenderNode::LazyStack(Box::new(LazyStackNode {
            axis,
            views,
            env: env.clone(),
            item_extents: RefCell::new(Vec::new()),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            estimate: Cell::new(0.0),
            dirty_key,
            guard,
        }))
    }

    /// Build a self-drawn scene node owning its `SceneContent` (no effect slot).
    fn build_scene_view_node(
        scene_view: Native<SceneView>,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let mut content = scene_view.into_inner().into_content();
        let signals = renderer.signals.clone();
        content.set_invalidator(Some(Rc::new(move || {
            signals.request_next_frame_rebuild();
        })));
        RenderNode::SceneView(Box::new(SceneViewNode {
            content: RefCell::new(content),
        }))
    }

    /// Build an embedded `GpuSurface` node owning its
    /// [`EmbeddedGpuSurfaceRuntime`] directly. The runtime is shared with the
    /// renderer's node-surface registry so its off-thread redraw handle is polled
    /// even on frames that do not re-flush the tree.
    fn build_gpu_surface(
        surface: GpuSurface,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let runtime = Rc::new(RefCell::new(EmbeddedGpuSurfaceRuntime::new(surface, env)));
        renderer.register_node_gpu_surface(Rc::clone(&runtime));
        RenderNode::GpuSurface(Box::new(GpuSurfaceNode { runtime }))
    }

    /// Build a `ViewEffect` node owning its [`ViewEffectRuntime`] and building its
    /// captured content as a persistent child [`RenderNode`] (recursed into, not
    /// baked), so reactive descendants inside the effect stay live.
    fn build_view_effect(
        mut effect: ViewEffectErased,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let content = effect.take_content();
        let child = RenderNode::build(normalize_layout_view(content, env), env, renderer);
        RenderNode::ViewEffect(Box::new(ViewEffectNode {
            runtime: RefCell::new(ViewEffectRuntime::new(effect)),
            child: RefCell::new(child),
            laid_out: Cell::new(Size::zero()),
            env: env.clone(),
        }))
    }

    /// Build an `AppliedFilter` node owning its [`AppliedFilterRuntime`]
    /// (input/output textures) and building its wrapped content as a persistent
    /// child [`RenderNode`]. The runtime is registered with the renderer so
    /// animated filters refresh on redraw-only frames.
    fn build_applied_filter(
        filter: AppliedFilter,
        content: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let runtime = Rc::new(RefCell::new(AppliedFilterRuntime::new(filter)));
        renderer.register_node_applied_filter(Rc::clone(&runtime));
        let child = RenderNode::build(normalize_layout_view(content, env), env, renderer);
        RenderNode::AppliedFilter(Box::new(AppliedFilterNode {
            runtime,
            child,
            env: env.clone(),
        }))
    }

    /// Build a reactive `Dynamic` host: connect to receive content updates, build
    /// the initial child, and wrap it so later content changes patch in isolation.
    fn build_dynamic_host(
        dynamic: waterui_core::dynamic::Dynamic,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let identity = dynamic.identity();
        let pending: Rc<RefCell<Option<AnyView>>> = Rc::new(RefCell::new(None));
        let source = dynamic.clone();
        let signals = renderer.signals.clone();
        dynamic.connect_with_pending_view(Rc::clone(&pending), {
            let pending = Rc::clone(&pending);
            move |update| {
                let is_initial = update
                    .metadata()
                    .try_get::<DynamicInitialContent>()
                    .is_some();
                *pending.borrow_mut() = Some(update.into_value());
                // A real content change schedules a fine-grained patch; the
                // render tree rebuilds only this node's child on the next frame.
                if !is_initial {
                    signals.mark_dynamic_dirty(identity, 0);
                }
            }
        });
        let initial = pending.borrow_mut().take();
        let child = match initial {
            Some(content) => RenderNode::build(content, env, renderer),
            None => RenderNode::build(AnyView::new(()), env, renderer),
        };
        RenderNode::Dynamic(Box::new(DynamicHostNode {
            source,
            pending,
            env: env.clone(),
            child,
        }))
    }

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
                // Reconcile membership first (keeps surviving items' nodes), then
                // always patch every item so surviving items' nested reactive
                // content (e.g. an active-indicator `.background(Computed)`) updates.
                let membership_changed = node.dirty.replace(false);
                if membership_changed {
                    node.reconcile(renderer);
                }
                let mut changed = membership_changed;
                for (_, child) in &mut node.items {
                    changed |= child.patch(renderer);
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
                for (_, child) in &node.items {
                    child.collect_dynamic_identities_into(out);
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
    /// Flush an already-laid-out child [`RenderNode`] into a fresh, standalone
    /// [`vello::Scene`] in local (identity) coordinates — the node analogue of
    /// [`HydrolysisRenderer::render_subtree_scene`], for a GPU/effect node that
    /// captures its child into a texture each flush. The renderer's scene is
    /// swapped out, the child flushes into the temporary scene, then it is swapped
    /// back, so the returned scene can be rendered to the effect's input texture.
    pub(crate) fn render_child_node_scene(
        &mut self,
        child: &RenderNode,
        ctx: RenderContext,
        env: &Environment,
    ) -> vello::Scene {
        let mut scene = vello::Scene::new();
        let local_ctx = ctx.with_identity_transforms(vello::kurbo::Rect::new(
            0.0,
            0.0,
            ctx.bounds.width(),
            ctx.bounds.height(),
        ));
        core::mem::swap(&mut self.scene, &mut scene);
        child.flush(self, local_ctx, env);
        core::mem::swap(&mut self.scene, &mut scene);
        scene
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
        // Roll over this frame's Retain watcher guards exactly like the rebuild path:
        // a full re-flush re-reads (re-subscribes) every reactive input, so last
        // frame's guards must move current -> previous (dropped next frame) instead of
        // accumulating. ONLY the lifecycle (Retain) bracket runs here — never the
        // animation/scroll controller brackets, whose in-flight cross-frame state the
        // refresh path must preserve. `patch` may build new `Dynamic` subtrees that
        // subscribe, so this precedes it.
        self.lifecycle.begin_rebuild_frame();
        // Reset the flush-bound interaction cursors (press/hover slot indices,
        // hit-test order). A full re-flush re-binds every widget's interaction slot in
        // the same walk order, so resetting + rebinding + truncating here keeps the
        // cursor mapping stable across refresh frames (slot state persists by index,
        // with bounds-matching guarding against reuse) — without this, a refresh would
        // hand every widget a fresh slot and drop its hover/press state. The
        // animation/scroll/nav controllers are deliberately NOT reset (cross-frame).
        self.hit_test.begin_rebuild_frame();
        // Apply any pending reactive Dynamic content changes incrementally
        // (rebuild only the affected child subtrees), then run FULL layout every
        // frame. Layout is cheap by construction: the only heavy work — text
        // shaping — is memoized in the persistent, content-keyed text cache, and
        // containers just recompute `place()` arithmetic over cached child
        // measurements. Always relaying out lets a reactive value change that
        // alters a leaf's size (text content, a widget's intrinsic size) reflow
        // its ancestors through this same cheap pump, with no `body()` rebuild and
        // no reset-and-redispatch flash. `begin_redraw_frame` (above) cleared the
        // per-frame `stable_ptr` view-dimension cache so this measure pass is sound.
        let _ = tree.patch(self);
        self.reset_scene();
        self.begin_redraw_frame();
        let size = Size::new(bounds.width() as f32, bounds.height() as f32);
        tree.layout(self, env, size);
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        tree.flush(self, ctx, env);
        self.flush_vello_scene_layer();
        self.hit_test.finish_rebuild_frame();
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
}
