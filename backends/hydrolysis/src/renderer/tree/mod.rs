//! Persistent retained render tree — the sole render path.
//!
//! A [`RenderNode`] is built exactly once from the app's `body()` at window
//! startup and retained for the window's lifetime. It holds the view's *live*
//! reactive inputs (`Computed`/`Binding`/`impl Signal`), not snapshots, and is
//! refreshed every frame by [`HydrolysisRenderer::flush_window_tree`] in three
//! steps:
//!
//! - [`RenderNode::patch`] applies pending structural changes first: a
//!   `Dynamic` host rebuilds only its own child subtree and a collection
//!   reconciles membership by id — never a whole-window rebuild.
//! - A geometry refresh runs [`RenderNode::layout`]: it re-reads signals,
//!   re-measures, and re-places the subtree, caching each container's child
//!   frames. A reactive value change that alters a leaf's size therefore reflows
//!   its ancestors with no `body()` rebuild.
//! - [`RenderNode::flush`] records the subtree into the nodes' programs from
//!   the cached placements. Steady-state visual animation frames run only this
//!   step; they do not repeat layout or window-size-limit negotiation.
//!
//! This is the architecture validated by `tests::perf_full_rebuild`: a
//! geometry-static flush of a 160-row screen is ~tens of microseconds (the
//! layout cache is load-bearing), well under the 120fps budget, whereas
//! re-dispatching the same screen from the `View` tree costs ~15ms.

macro_rules! impl_widget_behavior {
    ($state:ty, $render:path, $measure:expr $(, $priority:expr)? $(; prepare: $prepare:ident)? $(; a11y: $a11y:path)? $(; renders_nothing: $renders_nothing:literal)? $(; surface: $surface:ident)?) => {
        impl WidgetBehavior for RefCell<$state> {
            $(fn priority(&self) -> i32 { $priority })?

            $(fn renders_nothing(&self) -> bool { $renders_nothing })?

            fn render(
                self: Rc<Self>,
                renderer: &mut HydrolysisRenderer,
                ctx: RenderContext,
                env: &Environment,
                safe_area: Option<safe_area::SafeAreaLayout>,
            ) {
                let mut widget_ctx = WidgetRenderContext::new(renderer, ctx, safe_area);
                $render(&mut widget_ctx, &self, env);
            }

            fn measure(
                &self,
                state: &mut HydroState,
                proposal: ProposalSize,
                env: &Environment,
                theme: &Rc<dyn crate::engine::WidgetTheme>,
            ) -> ViewDimensions {
                ($measure)(&self.borrow(), proposal, state, env, theme)
            }

            $(fn prepare(&self, renderer: &mut HydrolysisRenderer, env: &Environment) {
                self.borrow_mut().$prepare(renderer, env);
            })?

            $(
                fn update_scroll_surface(&self, facts: Option<safe_area::ScrollSurfaceFacts>) {
                    self.borrow().$surface.facts.set(facts);
                }
            )?

            $(
                #[cfg(feature = "accessibility")]
                fn emit_accessibility(
                    self: Rc<Self>,
                    renderer: &mut SemanticCore,
                    env: &Environment,
                ) {
                    $a11y(renderer, &self, env);
                }
            )?
        }
    };
}

mod build;
mod build_controls;
mod build_views;
mod collection;
mod flush;
#[cfg(hydrolysis_hosted)]
mod hosted;
mod layout;
mod nodes;
pub(super) mod safe_area;
mod subview;
mod window;

pub use collection::*;
pub use nodes::*;
use subview::NodeSubView;

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::renderer::lazy::{
    LazyStackAxisConfig, VirtualExtentIndex, lazy_stack_axis_config, place_lazy_stack_item,
};
use crate::renderer::render::{MemoGate, NodeMeasureEntry};
use crate::scroll::ScrollHandle;
use core::cell::Cell;
use core::ops::Range;
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed};
use std::rc::{Rc, Weak};
use waterui_core::MainThreadBound;
use waterui_core::id::{Id as RawId, SelfId};
use waterui_core::layout::{LayoutPriority, Point, Rect, Size};
use waterui_core::views::{AnyViews, AnyViewsSnapshot, ViewSnapshot, Views};
use waterui_layout::scroll::{Axis as ScrollAxis, ScrollController, ScrollView, ScrollViewParts};

/// The type-erased item identity used by [`CollectionNode`]'s reconcile.
type CollectionItemId = SelfId<RawId>;

/// A node in the persistent retained render tree. The render-primitive set is
/// closed by the nature of a self-drawn renderer; the open `HydroDispatcher` maps
/// the open universe of `View` types onto this closed set.
pub enum RenderNode {
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
    /// rather than through a frame-ordered effect slot, so a `Dynamic` swap to a
    /// different scene renders the new content instead of the previous scene's.
    SceneView(Box<SceneViewNode>),
    /// An embedded `GpuContentView` leaf owning its `GpuContentRuntime`
    /// directly (no cursor-bound slot), composited through an `Rc`-carrying layer.
    GpuContent(Box<GpuContentNode>),
    /// An `ExternalFrameView` leaf owning its `ExternalFrameRuntime`,
    /// composited through a keyed engine layer that presents the frames its
    /// source publishes.
    ExternalFrame(Box<ExternalFrameNode>),
    /// A `FilteredView` wrapper owning its `FilteredRuntime` (the engine
    /// `Filter`) and recursing into its child node — the child mounts inside
    /// the filtered group so the filter covers the whole subtree.
    Filtered(Box<FilteredNode>),
    /// A reactive `Dynamic` host: holds the live `Dynamic` and rebuilds only its
    /// own child subtree when the content changes (incremental patch + relayout).
    /// This is the structural seam that keeps a content swap from resetting the
    /// scene and re-dispatching the whole window, which is visible as a flicker.
    /// Layout-transparent around its child.
    Dynamic(Box<DynamicHostNode>),
    /// A transparent metadata wrapper that applies a visual/interaction effect
    /// every flush and *recurses into* its child node (instead of capturing the
    /// subtree once). This is what keeps reactive descendants inside `.clip()` /
    /// `.border()` / `.shadow()` / `.cursor()` / `.draggable()` / drop-destination
    /// / context-menu live: they reach their own dedicated nodes and keep updating.
    /// Layout-transparent (the effect is pure setup + render child).
    Wrapper(Box<WrapperNode>),
    /// The fill occupying a [`BackgroundLayout`]'s background slot (§7.1):
    /// wraps the slot's paint fill — a `Color` or the gradient leaf, inside
    /// any layout-transparent wrappers — and carries the paint extension
    /// layout records and flush applies. Layout-transparent: it is a paint
    /// marker, not a frame.
    Fill(Box<FillNode>),
    /// A native widget leaf (button, toggle, slider, picker, text field, …) rendered
    /// by its `HydroNativeView` handler **every flush** from a retained,
    /// signal-holding config — never baked. Its `builder` reconstructs the leaf view
    /// (actions retained as `Rc`, label/value signals kept) and the flush re-dispatches
    /// it, so the handler re-reads its live signals and reactive content (a `text!`
    /// label, a bound value) stays live. Re-dispatching a *leaf* is cheap: no body
    /// expansion and no structural rebuild, so the leaf never has to be captured
    /// and replayed to stay affordable.
    Widget(WidgetNode),
}

impl RenderNode {
    /// The node's mount core: its cell (marks, parent link), placement
    /// mirror and signal-guard stores. Every node owns one for its whole
    /// lifetime — layer-owning or pass-through alike — so a mark or a
    /// structural edit always has a live owner to attach to.
    pub(crate) fn core(&self) -> &NodeCore {
        match self {
            Self::Color(node) => &node.core,
            Self::Text(node) => &node.core,
            Self::Container(node) => &node.core,
            Self::Opacity(node) => &node.core,
            Self::Scale(node) => &node.core,
            Self::Rotation(node) => &node.core,
            Self::Offset(node) => &node.core,
            Self::Retain(node) => &node.core,
            Self::Env(node) => &node.core,
            Self::Scroll(node) => &node.core,
            Self::Collection(node) => &node.core,
            Self::LazyStack(node) => &node.core,
            Self::SceneView(node) => &node.core,
            Self::GpuContent(node) => &node.core,
            Self::ExternalFrame(node) => &node.core,
            Self::Filtered(node) => &node.core,
            Self::Dynamic(node) => &node.core,
            Self::Wrapper(node) => &node.core,
            Self::Fill(node) => &node.core,
            Self::Widget(node) => &node.core,
        }
    }

    /// Visits this node's direct structural children in paint order: the
    /// subview nodes it owns inside the render tree (not widget-owned
    /// `RetainedSubview`s, which attach to their owner's cell at
    /// `ensure_built`). `f` receives `(attach_parent, child)` — the core the
    /// child's cell parents to: `self`'s own core for ordinary children, a
    /// [`CollectionEntry`]'s core for an entry's node.
    fn for_each_child(&self, f: &mut impl FnMut(&NodeCore, &Self)) {
        let own = self.core();
        match self {
            Self::Color(_)
            | Self::Text(_)
            | Self::Widget(_)
            | Self::SceneView(_)
            | Self::GpuContent(_)
            | Self::ExternalFrame(_) => {}
            Self::Container(node) => {
                for child in &node.children {
                    f(own, child);
                }
            }
            Self::Collection(node) => {
                for entry in &node.entries {
                    f(&entry.core, &entry.node);
                }
            }
            Self::LazyStack(node) => {
                // Visit materialized items in collection index order — the
                // cache's map order is arbitrary and would scramble both
                // attach order and the test-time `collect_cells` sequence.
                let snapshot = node.snapshot.borrow().clone();
                let cache = node.item_cache.borrow();
                for index in snapshot.range() {
                    let Some(id) = snapshot.get_id(index) else {
                        continue;
                    };
                    if let Some(child) = cache.get(&id).and_then(|subview| subview.node()) {
                        f(own, child);
                    }
                }
            }
            Self::Opacity(node) => f(own, &node.child),
            Self::Scale(node) => f(own, &node.child),
            Self::Rotation(node) => f(own, &node.child),
            Self::Offset(node) => f(own, &node.child),
            Self::Retain(node) => f(own, &node.child),
            Self::Env(node) => f(own, &node.child),
            Self::Scroll(node) => f(own, &node.child),
            Self::Filtered(node) => f(own, &node.child),
            Self::Dynamic(node) => {
                let child = node.child.borrow();
                f(own, &child);
            }
            Self::Wrapper(node) => f(own, &node.child),
            Self::Fill(node) => f(own, &node.child),
        }
    }

    /// Re-roots the cell parent links of a freshly built subtree: every
    /// descendant cell points at its structural attach parent's cell.
    /// Called on a `RenderNode::build` product at the moment it lands in
    /// the tree; the node's own parent is set by its caller.
    pub(crate) fn attach_subtree(&self) {
        self.for_each_child(&mut |parent, child| {
            child.core().cell.set_parent(&parent.cell);
            child.attach_subtree();
        });
    }

    /// Unmounts the subtree's engine layers (§F): drops every
    /// `NodeLayers` below this node and sets `PAINT|COMMIT` on each cell,
    /// keeping the nodes and their retained state. Decision 3 runs it for
    /// content leaving the screen, and the engine-window replacement runs
    /// it on the whole tree before the remount. The cell walk below
    /// reaches widget-attached `RetainedSubview` roots too — navigation
    /// pages, lazy items and overlay content — so nothing keeps a stale
    /// mount.
    pub(crate) fn unmount(&self) {
        self.core().cell.unmount_subtree();
    }

    /// Pre-order collection of every node's cell in the subtree, in tree
    /// order (a collection entry's own cell precedes its node). Holding the
    /// `Rc`s keeps retired addresses un-reused across a comparison.
    /// Test-only: the acceptance probes compare the whole sequence across
    /// an update (unchanged nodes keep their position and cell).
    #[cfg(test)]
    pub(crate) fn collect_cells(&self, out: &mut Vec<Rc<NodeCell>>) {
        out.push(Rc::clone(&self.core().cell));
        self.for_each_child(&mut |parent, child| {
            if !Rc::ptr_eq(&parent.cell, &self.core().cell) {
                out.push(Rc::clone(&parent.cell));
            }
            child.collect_cells(out);
        });
    }

    /// The retained identity marking where this node's view begins, looked
    /// through the transparent single-child wrappers (env scopes, animation
    /// layers, retained guards, dynamic hosts) to the first node that carries
    /// one. Input ancestry reads it to tell a gesture registered inside the
    /// view from one attached to the view itself; leaves with no children and
    /// no identity have no descendants to tell apart, so they report none.
    pub(crate) fn accessibility_identity(&self) -> Option<Rc<()>> {
        match self {
            Self::Wrapper(node) => Some(node.accessibility_identity.clone()),
            Self::Widget(node) => Some(node.accessibility_identity.clone()),
            Self::Text(node) => Some(node.accessibility_identity.clone()),
            Self::Container(node) => Some(node.accessibility_identity.clone()),
            Self::Scroll(node) => Some(node.accessibility_identity.clone()),
            Self::SceneView(node) => Some(node.accessibility_identity.clone()),
            Self::GpuContent(node) => Some(node.accessibility_identity.clone()),
            Self::ExternalFrame(node) => Some(node.accessibility_identity.clone()),
            Self::Collection(node) => Some(node.accessibility_identity.clone()),
            Self::LazyStack(node) => Some(node.accessibility_identity.clone()),
            Self::Retain(node) => node.child.accessibility_identity(),
            Self::Env(node) => node.child.accessibility_identity(),
            Self::Opacity(node) => node.child.accessibility_identity(),
            Self::Scale(node) => node.child.accessibility_identity(),
            Self::Rotation(node) => node.child.accessibility_identity(),
            Self::Offset(node) => node.child.accessibility_identity(),
            Self::Filtered(node) => node.child.accessibility_identity(),
            Self::Dynamic(node) => node.child.borrow().accessibility_identity(),
            Self::Fill(node) => node.child.accessibility_identity(),
            Self::Color(_) => None,
        }
    }
}
