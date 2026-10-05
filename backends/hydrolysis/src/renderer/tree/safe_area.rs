//! `docs/layout-spec.md` §7.1's safe-area model, carried through layout.
//!
//! [`SafeAreaLayout`] is the typed context the layout pass threads through
//! the retained tree: each node's laid-out frame in window space — before
//! scroll offsets and visual transforms — plus, per edge, the boundary the
//! subtree's laid-out area ends at and the regions already released past
//! it. Touch tests compare frame edges against those boundaries exactly —
//! never a tolerance, never a hit-test transform, never a painted rect —
//! so an offset or animated transform cannot move a frame's answer.
//!
//! [`ScrollSurfaceArea`] is the per-surface bookkeeping the scroll surfaces
//! (`Scroll` nodes, `List`, `Table`) carry: the extension and clearance
//! bounds layout computed once, and the focused-field state the flush
//! drives.

use core::cell::{Cell, RefCell};

use waterui_core::layout::Size;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::safe_area::{EdgeSet, IgnoreSafeArea, SafeAreaRegions};

use crate::renderer::{HydrolysisRenderer, InteractionKey, RenderContext};
use crate::scroll::ScrollHandle;

/// One of the four edges §7.1's regions sit on.
#[derive(Clone, Copy)]
enum Edge {
    Top,
    Leading,
    Bottom,
    Trailing,
}

/// The edges in declaration order — one shared iteration order for the
/// release loop and the extension computation.
const EDGES: [Edge; 4] = [Edge::Top, Edge::Leading, Edge::Bottom, Edge::Trailing];

impl Edge {
    /// Whether `edges` names this edge.
    const fn named_in(self, edges: EdgeSet) -> bool {
        match self {
            Self::Top => edges.top,
            Self::Leading => edges.leading,
            Self::Bottom => edges.bottom,
            Self::Trailing => edges.trailing,
        }
    }

    /// This edge's depth inside `insets`.
    const fn depth_in(self, insets: &EdgeInsets) -> f32 {
        match self {
            Self::Top => insets.top(),
            Self::Leading => insets.leading(),
            Self::Bottom => insets.bottom(),
            Self::Trailing => insets.trailing(),
        }
    }

    /// This edge's coordinate on `rect`: the frame edge the touch test reads.
    const fn frame_edge(self, rect: kurbo::Rect) -> f64 {
        match self {
            Self::Top => rect.y0,
            Self::Leading => rect.x0,
            Self::Bottom => rect.y1,
            Self::Trailing => rect.x1,
        }
    }

    /// Moves `rect`'s frame edge on this edge to `position`.
    const fn set_frame_edge(self, rect: &mut kurbo::Rect, position: f64) {
        match self {
            Self::Top => rect.y0 = position,
            Self::Leading => rect.x0 = position,
            Self::Bottom => rect.y1 = position,
            Self::Trailing => rect.x1 = position,
        }
    }

    /// The boundary position a region of `depth` puts on this edge of
    /// `window` — `depth` inward from the window edge, zero being the edge
    /// itself.
    const fn boundary_at(self, window: kurbo::Rect, depth: f64) -> f64 {
        match self {
            Self::Top => window.y0 + depth,
            Self::Leading => window.x0 + depth,
            Self::Bottom => window.y1 - depth,
            Self::Trailing => window.x1 - depth,
        }
    }

    /// The distance from `position` on this edge back out to the window
    /// edge — the reach a touched-edge extension takes.
    const fn window_gap(self, window: kurbo::Rect, position: f64) -> f64 {
        match self {
            Self::Top => position - window.y0,
            Self::Leading => position - window.x0,
            Self::Bottom => window.y1 - position,
            Self::Trailing => window.x1 - position,
        }
    }
}

/// The regions enclosing `.ignore_safe_area` declarations released on an
/// edge: the subtree boundary sits at the deepest region not named, so an
/// inner declaration can only move it outward — never re-cover a region an
/// outer declaration already released.
#[derive(Clone, Copy, Default)]
struct ReleasedRegions {
    container: bool,
    keyboard: bool,
}

impl ReleasedRegions {
    /// The regions `regions` adds to this set.
    const fn union(self, regions: SafeAreaRegions) -> Self {
        Self {
            container: self.container | regions.container(),
            keyboard: self.keyboard | regions.keyboard(),
        }
    }
}

/// One edge's safe-area boundary: the window-space position the subtree's
/// laid-out area ends at, and the regions released past it.
#[derive(Clone, Copy)]
struct EdgeBoundary {
    position: f64,
    released: ReleasedRegions,
}

/// Per-edge distances in window logical units, named by the edge they sit
/// on — a fill's paint extension, an `.ignore_safe_area` release, and a
/// scroll surface's content inset all express themselves in this shape.
#[derive(Clone, Copy, Debug, Default)]
pub struct EdgeOffsets {
    pub(crate) top: f64,
    pub(crate) leading: f64,
    pub(crate) bottom: f64,
    pub(crate) trailing: f64,
}

impl EdgeOffsets {
    /// The released pair summed per axis — the amounts an
    /// `.ignore_safe_area` child grows on each axis.
    pub const fn horizontal(&self) -> f64 {
        self.leading + self.trailing
    }

    /// The released pair summed per axis — the amounts an
    /// `.ignore_safe_area` child grows on each axis.
    pub const fn vertical(&self) -> f64 {
        self.top + self.bottom
    }
}

/// The safe-area facts a subtree lays out against (§7.1): the node's own
/// laid-out frame in window space — before scroll offsets and visual
/// transforms — plus, per edge, the boundary the subtree's laid-out area
/// ends at and the regions released past it.
///
/// `None` is passed where there is no context: inside a scroll surface's
/// content (the surface insets and clears its own subtree, so nothing
/// inside touches an edge) and inside the retained sub-views widgets lay
/// out themselves (list rows, table cells, navigation content).
#[derive(Clone)]
pub struct SafeAreaLayout {
    /// This node's laid-out frame in window space.
    frame: kurbo::Rect,
    /// The window rect in the same space.
    window: kurbo::Rect,
    /// The container region's depths at the window edges.
    container: EdgeInsets,
    /// The keyboard region's depths at the window edges.
    keyboard: EdgeInsets,
    top: EdgeBoundary,
    leading: EdgeBoundary,
    bottom: EdgeBoundary,
    trailing: EdgeBoundary,
}

impl SafeAreaLayout {
    /// The window root's context: `frame` is the laid-out content rect —
    /// the window shrunk by the deeper of the two regions on every edge —
    /// and its edges are the boundaries the whole subtree starts from.
    /// Seeding the boundaries from the frame (not from the region depths)
    /// means the f32-measured root frame is what touch tests compare
    /// against: exact, with no rounding window between them.
    pub fn root(
        window: kurbo::Rect,
        container: EdgeInsets,
        keyboard: EdgeInsets,
        frame: kurbo::Rect,
    ) -> Self {
        let boundary = |position: f64| EdgeBoundary {
            position,
            released: ReleasedRegions::default(),
        };
        Self {
            frame,
            window,
            container,
            keyboard,
            top: boundary(frame.y0),
            leading: boundary(frame.x0),
            bottom: boundary(frame.y1),
            trailing: boundary(frame.x1),
        }
    }

    /// This node's laid-out frame in window space.
    pub const fn frame(&self) -> kurbo::Rect {
        self.frame
    }

    /// The context for a child whose laid-out frame is `frame` — the same
    /// boundaries, regions and window.
    pub(crate) fn with_frame(&self, frame: kurbo::Rect) -> Self {
        Self {
            frame,
            ..self.clone()
        }
    }

    const fn boundary(&self, edge: Edge) -> EdgeBoundary {
        match edge {
            Edge::Top => self.top,
            Edge::Leading => self.leading,
            Edge::Bottom => self.bottom,
            Edge::Trailing => self.trailing,
        }
    }

    const fn set_boundary(&mut self, edge: Edge, boundary: EdgeBoundary) {
        match edge {
            Edge::Top => self.top = boundary,
            Edge::Leading => self.leading = boundary,
            Edge::Bottom => self.bottom = boundary,
            Edge::Trailing => self.trailing = boundary,
        }
    }

    /// The deepest region `released` does not cover on `edge` — zero when
    /// every region is released, putting the boundary at the window edge.
    fn unreleased_depth(&self, edge: Edge, released: ReleasedRegions) -> f64 {
        let mut depth = 0.0_f64;
        if !released.container {
            depth = depth.max(f64::from(edge.depth_in(&self.container)));
        }
        if !released.keyboard {
            depth = depth.max(f64::from(edge.depth_in(&self.keyboard)));
        }
        depth
    }

    /// The context and grown frame for the child of an `.ignore_safe_area`
    /// wrapper carrying `ignore`, plus the released amounts the flush
    /// mirrors.
    ///
    /// On each edge `ignore.edges` names that the wrapper's laid-out frame
    /// touches the subtree boundary on, the boundary moves out to the
    /// deepest region `ignore.regions` does not name — the window edge when
    /// it names both — and the child's frame grows to meet it. An edge the
    /// frame does not touch releases nothing, and because released regions
    /// accumulate, an inner declaration can never shrink a boundary an
    /// outer one already moved (§7.1: nested declarations cannot
    /// double-release).
    #[expect(
        clippy::float_cmp,
        reason = "§7.1's touch is the laid-out frame ending exactly on the boundary"
    )]
    pub(crate) fn release(&self, ignore: IgnoreSafeArea) -> (kurbo::Rect, Self, EdgeOffsets) {
        let mut frame = self.frame;
        let mut next = self.clone();
        let mut released = EdgeOffsets::default();
        for edge in EDGES {
            let boundary = self.boundary(edge);
            if !edge.named_in(ignore.edges) || edge.frame_edge(frame) != boundary.position {
                continue;
            }
            let regions = boundary.released.union(ignore.regions);
            let position = edge.boundary_at(self.window, self.unreleased_depth(edge, regions));
            edge.set_frame_edge(&mut frame, position);
            next.set_boundary(
                edge,
                EdgeBoundary {
                    position,
                    released: regions,
                },
            );
            match edge {
                Edge::Top => released.top = self.frame.y0 - position,
                Edge::Leading => released.leading = self.frame.x0 - position,
                Edge::Bottom => released.bottom = position - self.frame.y1,
                Edge::Trailing => released.trailing = position - self.frame.x1,
            }
        }
        next.frame = frame;
        (frame, next, released)
    }

    /// On each edge, the distance from the frame's edge to the window
    /// edge — non-zero only where the laid-out frame ends exactly on the
    /// subtree's boundary (§7.1's "touches"). A fill's default paint
    /// extension and a scroll surface's extension are this same answer:
    /// both reach the window edge through every region on a touched edge.
    #[expect(
        clippy::float_cmp,
        reason = "§7.1's touch is the laid-out frame ending exactly on the boundary"
    )]
    pub fn touched_edge_offsets(&self) -> EdgeOffsets {
        let mut offsets = EdgeOffsets::default();
        for edge in EDGES {
            let frame_edge = edge.frame_edge(self.frame);
            if frame_edge == self.boundary(edge).position {
                match edge {
                    Edge::Top => offsets.top = edge.window_gap(self.window, frame_edge),
                    Edge::Leading => offsets.leading = edge.window_gap(self.window, frame_edge),
                    Edge::Bottom => offsets.bottom = edge.window_gap(self.window, frame_edge),
                    Edge::Trailing => offsets.trailing = edge.window_gap(self.window, frame_edge),
                }
            }
        }
        offsets
    }

    /// The §7.1 scroll-surface facts this context computes for a surface
    /// sitting at this frame: the touched-edge extension the surface's
    /// viewport, content inset, clip and lazy viewport all grow by, and
    /// the bounds the focused-field clearance clamps against.
    pub fn surface_facts(&self) -> ScrollSurfaceFacts {
        ScrollSurfaceFacts {
            extension: self.touched_edge_offsets(),
            window_frame: self.frame,
            keyboard_top: self.window.y1 - f64::from(self.keyboard.bottom()),
        }
    }
}

/// What a scroll surface's layout pass caches for its flush: §7.1's
/// surface facts measured in window space — the distances hold in the
/// surface's local space too, because layout space carries no visual
/// transforms.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScrollSurfaceFacts {
    /// Per-edge distance from the surface's laid-out frame to the window
    /// edge — non-zero only on edges the frame touched the boundary on.
    /// The clip, the wheel target, the published lazy viewport and the
    /// scroll metrics' viewport/content all extend by it.
    pub(crate) extension: EdgeOffsets,
    /// The surface's laid-out frame in window space — the clearance
    /// bound's lower term and the taller-field clamp.
    pub(crate) window_frame: kurbo::Rect,
    /// The keyboard region's top edge in window space — the clearance
    /// bound's upper term.
    pub(crate) keyboard_top: f64,
}

/// `rect` grown by `offsets` on each edge, in the same space.
pub fn grow_rect(rect: kurbo::Rect, offsets: EdgeOffsets) -> kurbo::Rect {
    kurbo::Rect::new(
        rect.x0 - offsets.leading,
        rect.y0 - offsets.top,
        rect.x1 + offsets.trailing,
        rect.y1 + offsets.bottom,
    )
}

/// The [`RenderContext`] a released `.ignore_safe_area` subtree flushes
/// under: bounds grown by the released amounts, the transform carrying the
/// leading/top overhang so the child's local origin lands where the grown
/// frame's does — the flush-side mirror of [`SafeAreaLayout::release`].
pub fn released_ctx(ctx: RenderContext, released: EdgeOffsets) -> RenderContext {
    ctx.child(
        kurbo::Affine::translate((-released.leading, -released.top)),
        kurbo::Rect::new(
            0.0,
            0.0,
            ctx.bounds.width() + released.horizontal(),
            ctx.bounds.height() + released.vertical(),
        ),
    )
}

/// The [`RenderContext`] a fill leaf flushes under when layout marked it a
/// background fill: bounds grown by the paint extension on every edge its
/// laid-out frame touched — the transforms unchanged, so nothing else
/// moves (§7.1: extension is a paint fact, not a layout fact).
pub fn fill_paint_ctx(ctx: RenderContext, extension: EdgeOffsets) -> RenderContext {
    RenderContext::with_transforms(
        grow_rect(ctx.bounds, extension),
        ctx.transform,
        ctx.hit_transform,
    )
}

/// The §7.1 bookkeeping one scroll surface carries: the facts layout
/// computed once, and the focused-field clearance state the flush drives.
///
/// `None` facts mean the surface has no safe-area context — content inside
/// another surface, or a widget laid out by the semantic pipeline — and
/// the surface extends and clears nothing.
#[derive(Default)]
pub struct ScrollSurfaceArea {
    /// What [`SafeAreaLayout::surface_facts`] recorded at layout; `None`
    /// where the surface owns no safe-area behaviour.
    pub(crate) facts: Cell<Option<ScrollSurfaceFacts>>,
    /// The field the clearance last handled: its window-space rect and the
    /// scroll offset it was captured at — the early pass's starting point.
    cleared: RefCell<Option<ClearedField>>,
    /// The keyboard top the last early pass saw — a change re-runs the
    /// clearance before the content paints.
    keyboard_top: Cell<Option<f64>>,
    /// Whether the keyboard top moved in this flush's early pass — selects
    /// the direct (not eased) scroll for a focus landing now.
    keyboard_moved: Cell<bool>,
}

/// One cleared field's identity and last observed geometry.
#[derive(Clone)]
struct ClearedField {
    /// The field's stable target identity — a different key means the
    /// focus moved, which re-runs the clearance.
    key: InteractionKey,
    /// The field's window-space rect when `offset_y` was the surface's
    /// scroll offset.
    rect: kurbo::Rect,
    /// The offset `rect` was captured at.
    offset_y: f64,
}

impl ScrollSurfaceArea {
    /// Runs before the surface's content flushes: the keyboard-moving
    /// branch of §7.1's clearance. While the keyboard inset changes, the
    /// offset follows each frame directly — the host's own animation is
    /// the pacing, never an eased chase — using the previous frame's
    /// stored field rect, so the content paints already clear. Returns the
    /// text-input target count before the subtree's registrations to hand
    /// to [`Self::end_flush`].
    pub fn begin_flush(&self, renderer: &HydrolysisRenderer, handle: &ScrollHandle) -> usize {
        let targets_start = renderer.text_editing.text_input_targets.len();
        let Some(facts) = self.facts.get() else {
            return targets_start;
        };
        let moved = self.keyboard_top.replace(Some(facts.keyboard_top)) != Some(facts.keyboard_top);
        self.keyboard_moved.set(moved);
        if moved && let Some(field) = self.field_window_rect(handle.metrics().offset_y) {
            Self::scroll_field_clear(renderer, handle, facts, field, false);
        }
        targets_start
    }

    /// Runs after the surface's content flushes: the focus-change branch —
    /// a field this subtree's own registrations reported gaining focus
    /// scrolls the minimum distance to `min(keyboard top, surface bottom)`,
    /// eased while the keyboard is settled and directly while it moves.
    /// The cleared field is refreshed for the next frame's early pass, and
    /// dropped when nothing in this subtree holds focus — so a user scroll
    /// afterwards is never fought.
    pub fn end_flush(
        &self,
        renderer: &HydrolysisRenderer,
        handle: &ScrollHandle,
        targets_start: usize,
    ) {
        let Some(facts) = self.facts.get() else {
            self.cleared.replace(None);
            return;
        };
        let offset_y = handle.metrics().offset_y;
        let field = renderer
            .text_editing
            .focused_index()
            .filter(|index| *index >= targets_start)
            .map(|index| {
                let target = renderer.text_editing.text_input_targets[index].clone();
                ClearedField {
                    key: target.interaction_key,
                    rect: target.frame,
                    offset_y,
                }
            });
        let newly_focused = field.as_ref().is_some_and(|field| {
            self.cleared
                .borrow()
                .as_ref()
                .is_none_or(|previous| previous.key != field.key)
        });
        if newly_focused && let Some(field) = &field {
            Self::scroll_field_clear(
                renderer,
                handle,
                facts,
                field.rect,
                !self.keyboard_moved.get(),
            );
        }
        *self.cleared.borrow_mut() = field;
    }

    /// The stored field's window-space rect at `offset_y`: it was captured
    /// at its own offset and the field rides the content, so an offset
    /// change since — a user scroll or a previous clearance — translates
    /// it back.
    fn field_window_rect(&self, offset_y: f64) -> Option<kurbo::Rect> {
        self.cleared
            .borrow()
            .as_ref()
            .map(|field| field.rect + kurbo::Vec2::new(0.0, field.offset_y - offset_y))
    }

    /// §7.1's minimum scroll: the field's bottom reaches
    /// `min(keyboard top, surface bottom)`; a field taller than the
    /// surface clamps the distance so its top stays inside the surface's
    /// frame.
    fn scroll_field_clear(
        renderer: &HydrolysisRenderer,
        handle: &ScrollHandle,
        facts: ScrollSurfaceFacts,
        field: kurbo::Rect,
        animated: bool,
    ) {
        let bound = facts.keyboard_top.min(facts.window_frame.y1);
        let mut distance = (field.y1 - bound).max(0.0);
        if field.height() > facts.window_frame.height() {
            distance = distance.min(field.y0 - facts.window_frame.y0);
        }
        if distance == 0.0 {
            return;
        }
        let metrics = handle.metrics();
        let target_y = (metrics.offset_y + distance).clamp(0.0, metrics.max_y);
        let scrolled = if animated {
            handle.scroll_to_animated(metrics.offset_x, target_y)
        } else {
            handle.scroll_to(metrics.offset_x, target_y)
        };
        // The offset lands outside the reactive graph — schedule the frame
        // that applies it (a glide also arms the pump through the scroll
        // target the surface registers every flush).
        if scrolled {
            renderer.request_refresh();
        }
    }
}

/// `size` grown by the released amounts on each axis — the laid-out size
/// an `.ignore_safe_area` child is placed at.
pub fn released_size(size: Size, released: EdgeOffsets) -> Size {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "released depths are window insets — within display scale"
    )]
    Size::new(
        size.width + (released.horizontal() as f32),
        size.height + (released.vertical() as f32),
    )
}
