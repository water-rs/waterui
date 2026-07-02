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
//! - [`RenderNode::layout`] then runs a FULL re-layout: it re-reads signals,
//!   re-measures, and re-places the subtree, caching each container's child
//!   frames. Full layout every frame is cheap by construction — the only heavy
//!   work, text shaping, is memoized in the persistent content-keyed text
//!   cache, so a reactive value change that alters a leaf's size reflows its
//!   ancestors with no `body()` rebuild.
//! - [`RenderNode::flush`] re-encodes the subtree into the renderer's scene
//!   from the cached placements, re-reading the live signals so reactive
//!   content stays current without touching the tree's structure.
//!
//! This is the architecture validated by `tests::perf_full_rebuild`: a
//! geometry-static flush of a 160-row screen is ~tens of microseconds (the
//! layout cache is load-bearing), well under the 120fps budget, whereas
//! re-dispatching the same screen from the `View` tree costs ~15ms.

mod build;
mod build_controls;
mod build_views;
mod collection;
mod flush;
mod layout;
mod nodes;
mod subview;
mod window;

pub(crate) use collection::*;
pub(crate) use nodes::*;
use subview::*;

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
