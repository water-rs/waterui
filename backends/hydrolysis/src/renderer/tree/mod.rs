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
//! - [`RenderNode::flush`] re-encodes the subtree into the renderer's scene from
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
            ) {
                let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
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
                fn update_scroll_surface(&self, facts: Option<ScrollSurfaceFacts>) {
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
mod layout;
mod nodes;
mod safe_area;
mod subview;
mod window;

pub use collection::*;
pub use nodes::*;
pub use safe_area::*;
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
use std::rc::Rc;
use waterui_core::MainThreadBound;
use waterui_core::id::{Id as RawId, SelfId};
use waterui_core::layout::{LayoutPriority, Point, Rect, Size};
use waterui_core::views::{AnyViews, AnyViewsSnapshot, Views};
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
    /// The render identity of this visual node itself — never looked through
    /// to a child's. Even a layout-transparent transform/opacity wrapper owns a
    /// distinct [`RenderId`] (separate visual ownership for engine mounts):
    /// accessibility ancestry and render ancestry differ on purpose.
    ///
    /// A node keeps its id across signal-driven updates — the id is allocated
    /// once in the node's constructor and only a structural replacement (a
    /// `Dynamic` rebuild, a collection reconcile removal or rebuild of an
    /// unchanged id) replaces the node and its id.
    #[allow(dead_code)]
    pub(crate) fn render_id(&self) -> RenderId {
        match self {
            Self::Color(node) => node.render_id,
            Self::Text(node) => node.render_id,
            Self::Container(node) => node.render_id,
            Self::Opacity(node) => node.render_id,
            Self::Scale(node) => node.render_id,
            Self::Rotation(node) => node.render_id,
            Self::Offset(node) => node.render_id,
            Self::Retain(node) => node.render_id,
            Self::Env(node) => node.render_id,
            Self::Scroll(node) => node.render_id,
            Self::Collection(node) => node.render_id,
            Self::LazyStack(node) => node.render_id,
            Self::SceneView(node) => node.render_id,
            Self::GpuContent(node) => node.render_id,
            Self::ExternalFrame(node) => node.render_id,
            Self::Filtered(node) => node.render_id,
            Self::Dynamic(node) => node.render_id,
            Self::Wrapper(node) => node.render_id,
            Self::Widget(node) => node.render_id,
        }
    }

    /// This node's mount key in the given presentation placement — ordinary
    /// content mounts under [`PresentationId::ORDINARY`]; a hosted preview,
    /// accessory or popup instance mounts under its own `PresentationId` so
    /// the two can never collide.
    #[allow(dead_code)]
    pub(crate) fn render_key(&self, presentation: PresentationId) -> RenderKey {
        RenderKey {
            render: self.render_id(),
            presentation,
        }
    }

    /// Pre-order collection of every visual node's [`RenderId`] in the subtree,
    /// in tree order. Test-only: the acceptance probes compare the whole
    /// sequence across an update (unchanged nodes keep their position and id).
    #[cfg(test)]
    pub(crate) fn collect_render_ids(&self, out: &mut Vec<RenderId>) {
        out.push(self.render_id());
        match self {
            Self::Color(_)
            | Self::Text(_)
            | Self::Widget(_)
            | Self::SceneView(_)
            | Self::GpuContent(_)
            | Self::ExternalFrame(_) => {}
            Self::Container(node) => {
                for child in &node.children {
                    child.collect_render_ids(out);
                }
            }
            Self::Collection(node) => {
                for entry in &node.entries {
                    entry.node.collect_render_ids(out);
                }
            }
            Self::LazyStack(node) => {
                // The visible-window cache is id-keyed (unordered); collect then
                // sort so the sequence is comparable across frames.
                let mut ids = Vec::new();
                for subview in node.item_cache.borrow().values() {
                    if let Some(child) = subview.node() {
                        child.collect_render_ids(&mut ids);
                    }
                }
                ids.sort();
                out.extend(ids);
            }
            Self::Opacity(node) => node.child.collect_render_ids(out),
            Self::Scale(node) => node.child.collect_render_ids(out),
            Self::Rotation(node) => node.child.collect_render_ids(out),
            Self::Offset(node) => node.child.collect_render_ids(out),
            Self::Retain(node) => node.child.collect_render_ids(out),
            Self::Env(node) => node.child.collect_render_ids(out),
            Self::Scroll(node) => node.child.collect_render_ids(out),
            Self::Filtered(node) => node.child.collect_render_ids(out),
            Self::Dynamic(node) => node.child.borrow().collect_render_ids(out),
            Self::Wrapper(node) => node.child.collect_render_ids(out),
        }
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
            Self::Color(_) => None,
        }
    }
}
