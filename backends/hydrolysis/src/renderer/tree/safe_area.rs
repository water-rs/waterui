//! `docs/layout-spec.md` §7.1's safe-area model, carried through layout.
//!
//! [`SafeAreaLayout`] is the typed context the layout pass threads through
//! the retained tree: each node's laid-out frame in window space — before
//! scroll offsets and visual transforms — plus, per edge, the boundary the
//! subtree's laid-out area ends at and the regions already released past
//! it. Touch tests compare frame edges against those boundaries at display
//! resolution — within half a physical pixel at the window's scale factor,
//! never a hit-test transform, never a painted rect — so an offset or
//! animated transform cannot move a frame's answer, and the one-ULP misses
//! the f32 placement arithmetic produces cannot flicker.
//!
//! [`ScrollSurfaceArea`] is the per-surface bookkeeping the scroll surfaces
//! (`Scroll` nodes, `List`, `Table`) carry: the extension and clearance
//! bounds layout computed once, and the focused-field state the flush
//! drives.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

use core::cell::{Cell, RefCell};
use core::ops::Range;

use waterui_core::layout::{Rect, Size};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::safe_area::{EdgeSet, IgnoreSafeArea, SafeAreaRegions};
use waterui_layout::scroll::Axis as ScrollAxis;

/// One of the four edges §7.1's regions sit on.
#[derive(Clone, Copy)]
pub enum Edge {
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

    /// The edge across the window from this one — the boundary a bar docked
    /// at `self` can never touch: a top bar reaches top, leading and
    /// trailing, never the bottom edge.
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Leading => Self::Trailing,
            Self::Bottom => Self::Top,
            Self::Trailing => Self::Leading,
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
    pub(crate) const fn frame_edge(self, rect: kurbo::Rect) -> f64 {
        match self {
            Self::Top => rect.y0,
            Self::Leading => rect.x0,
            Self::Bottom => rect.y1,
            Self::Trailing => rect.x1,
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

    /// Whether this edge's touch tolerance is read in the window's vertical
    /// axis (top/bottom) or horizontal (leading/trailing).
    const fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    /// The amount `frame` grows on this edge when its frame edge moves to
    /// `position` — the `.ignore_safe_area` release an edge touch grants.
    fn released_amount(self, frame: kurbo::Rect, position: f64) -> f64 {
        match self {
            Self::Top => frame.y0 - position,
            Self::Leading => frame.x0 - position,
            Self::Bottom => position - frame.y1,
            Self::Trailing => position - frame.x1,
        }
    }

    /// `bounds` split into the band `extent` thick whose edge-side edge is
    /// `position` — in `bounds` space — and the remainder clamped inside
    /// `bounds`: the geometry a chrome container's docked bar and its
    /// hosted content share (§7.1). The band may sit past `bounds`' own
    /// edge — a docked bar under a raised keyboard does; the remainder
    /// never does. The band's thickness clamps to the span `position`
    /// leaves to `bounds`' far edge, so a band docked outside `bounds`
    /// keeps `extent` instead of squashing against it.
    pub(crate) fn split_band(
        self,
        bounds: kurbo::Rect,
        position: f64,
        extent: f64,
    ) -> (kurbo::Rect, kurbo::Rect) {
        let span = match self {
            Self::Top => bounds.y1 - position,
            Self::Bottom => position - bounds.y0,
            Self::Leading => bounds.x1 - position,
            Self::Trailing => position - bounds.x0,
        };
        let depth = extent.min(span.max(0.0));
        let inner = match self {
            Self::Top | Self::Leading => position + depth,
            Self::Bottom | Self::Trailing => position - depth,
        };
        let (low, high, across) = if self.is_vertical() {
            (bounds.y0, bounds.y1, (bounds.x0, bounds.x1))
        } else {
            (bounds.x0, bounds.x1, (bounds.y0, bounds.y1))
        };
        let on_axis = |start: f64, end: f64| {
            if self.is_vertical() {
                kurbo::Rect::new(across.0, start, across.1, end)
            } else {
                kurbo::Rect::new(start, across.0, end, across.1)
            }
        };
        match self {
            Self::Top | Self::Leading => (
                on_axis(position, inner),
                on_axis(inner.clamp(low, high), high),
            ),
            Self::Bottom | Self::Trailing => (
                on_axis(inner, position),
                on_axis(low, inner.clamp(low, high)),
            ),
        }
    }

    /// The band edge facing the hosted content — the counterpart of
    /// [`Self::frame_edge`] on the other side of a docked band.
    pub(crate) const fn inner_edge(self, band: kurbo::Rect) -> f64 {
        match self {
            Self::Top => band.y1,
            Self::Leading => band.x1,
            Self::Bottom => band.y0,
            Self::Trailing => band.x0,
        }
    }
}

/// The regions enclosing `.ignore_safe_area` declarations released on an
/// edge: the subtree boundary sits at the deepest region not named, so an
/// inner declaration can only move it outward — never re-cover a region an
/// outer declaration already released.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ReleasedRegions {
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

    /// The deepest region this set leaves covering `edge` under `container`
    /// and `keyboard` inset depths — zero when every region is released,
    /// which puts the boundary at the window edge itself. With the set
    /// empty this is also the window content rect's per-edge inset: the one
    /// place the `max(container, keyboard)` resolution lives.
    pub(super) fn unreleased_depth(
        self,
        edge: Edge,
        container: &EdgeInsets,
        keyboard: &EdgeInsets,
    ) -> f64 {
        let mut depth = 0.0_f64;
        if !self.container {
            depth = depth.max(f64::from(edge.depth_in(container)));
        }
        if !self.keyboard {
            depth = depth.max(f64::from(edge.depth_in(keyboard)));
        }
        depth
    }
}

/// One edge's safe-area boundary.
#[derive(Clone, Copy, PartialEq)]
pub enum EdgeBoundary {
    /// The window-space position the subtree's laid-out area ends at, and
    /// the regions released past it. A frame edge touching this position
    /// can release further and paint or extend through the band beyond it.
    Reachable {
        position: f64,
        released: ReleasedRegions,
    },
    /// Covered by a bar the host docks on this edge — the subtree touches
    /// nothing and releases and extends nowhere through it (§7.1: "its
    /// content touches no edge where a bar sits"). `edge_at` is the hosted
    /// content frame's edge in window space — the only frame edge the dock
    /// applies to, so the boundary reaches a nested bar that lands on the
    /// edge it covers and nothing else (`hosted` keeps it only there,
    /// [`SafeAreaLayout::docked_bar_position`] answers only there). `inner`
    /// is the bar's inner edge in window space: the dock position a bar a
    /// nested chrome container lays out on the same edge lands on, so
    /// nested chrome stacks on the outer bar instead of floating free of
    /// it — or riding onto the keyboard the outer bar ducked under.
    Docked { edge_at: f64, inner: f64 },
    /// No boundary: a hosted subtree's edge that does not touch the host's
    /// boundary is covered — nothing inside can touch, release or extend
    /// past it (§7.1: hosted content inherits, never reseeds).
    Covered,
}

impl EdgeBoundary {
    /// The regions released past the boundary — none while covered, and
    /// none through a bar's band.
    const fn released(&self) -> ReleasedRegions {
        match self {
            Self::Reachable { released, .. } => *released,
            Self::Docked { .. } | Self::Covered => ReleasedRegions {
                container: false,
                keyboard: false,
            },
        }
    }

    /// The window-space position a `Reachable` boundary records — `None`
    /// for edges a bar's band or a host's coverage owns.
    const fn position(&self) -> Option<f64> {
        match self {
            Self::Reachable { position, .. } => Some(*position),
            Self::Docked { .. } | Self::Covered => None,
        }
    }
}

/// Per-edge distances in window logical units, named by the edge they sit
/// on — a fill's paint extension, an `.ignore_safe_area` release, and a
/// scroll surface's content inset all express themselves in this shape.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct EdgeOffsets {
    pub top: f64,
    pub leading: f64,
    pub bottom: f64,
    pub trailing: f64,
}

impl EdgeOffsets {
    /// The leading/trailing pair summed — the amount a released edge pair
    /// or extension grows a frame horizontally.
    pub const fn horizontal(&self) -> f64 {
        self.leading + self.trailing
    }

    /// The top/bottom pair summed — the amount a released edge pair or
    /// extension grows a frame vertically.
    pub const fn vertical(&self) -> f64 {
        self.top + self.bottom
    }

    /// `self` with `edge` zeroed — the mask a chrome surface applies so a
    /// bar only ever extends through the edges its docking can touch, even
    /// when a clamped frame lands its inner edge on the opposite boundary.
    pub const fn cleared(mut self, edge: Edge) -> Self {
        self.set(edge, 0.0);
        self
    }

    /// Writes `value` onto `edge` — the per-edge assignment the release
    /// loop and the extension computation share.
    const fn set(&mut self, edge: Edge, value: f64) {
        match edge {
            Edge::Top => self.top = value,
            Edge::Leading => self.leading = value,
            Edge::Bottom => self.bottom = value,
            Edge::Trailing => self.trailing = value,
        }
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
/// out themselves (list rows and table cells).
#[derive(Clone, PartialEq)]
pub struct SafeAreaLayout {
    /// This node's laid-out frame in window space.
    frame: kurbo::Rect,
    /// The window rect in the same space.
    window: kurbo::Rect,
    /// The container region's depths at the window edges.
    container: EdgeInsets,
    /// The keyboard region's depths at the window edges.
    keyboard: EdgeInsets,
    /// The window's horizontal scale factor — physical pixels per logical
    /// unit: the x-axis touch tolerance derives from it.
    x_scale: f64,
    /// The window's vertical scale factor — the y-axis touch tolerance
    /// derives from it.
    y_scale: f64,
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
    /// against. `x_scale`/`y_scale` are the window transform's axis scale
    /// factors, the physical-pixel units the touch tolerance derives from.
    pub fn root(
        window: kurbo::Rect,
        container: EdgeInsets,
        keyboard: EdgeInsets,
        frame: kurbo::Rect,
        x_scale: f64,
        y_scale: f64,
    ) -> Self {
        let boundary = |position: f64| EdgeBoundary::Reachable {
            position,
            released: ReleasedRegions::default(),
        };
        Self {
            frame,
            window,
            container,
            keyboard,
            x_scale,
            y_scale,
            top: boundary(frame.y0),
            leading: boundary(frame.x0),
            bottom: boundary(frame.y1),
            trailing: boundary(frame.x1),
        }
    }

    /// The context for a child whose laid-out frame is `frame` — the same
    /// boundaries, regions and window.
    pub fn with_frame(&self, frame: kurbo::Rect) -> Self {
        Self {
            frame,
            ..self.clone()
        }
    }

    /// The context for a child laid out at `placement` — a frame in this
    /// node's local (f32) coordinates — the one placement→window-frame
    /// mapping the container and collection layout loops share.
    pub fn child(&self, placement: Rect) -> Self {
        self.with_frame(kurbo::Rect::new(
            self.frame.x0 + f64::from(placement.x()),
            self.frame.y0 + f64::from(placement.y()),
            self.frame.x0 + f64::from(placement.max_x()),
            self.frame.y0 + f64::from(placement.max_y()),
        ))
    }

    /// The window-space frame of hosted content placed at `rect` inside a
    /// container laid out at `bounds` — the f64 counterpart of [`Self::child`]
    /// for retained sub-views: `rect`'s offset inside `bounds` applied to
    /// this context's recorded layout frame, so touches and extensions
    /// derive from the laid-out position before scroll offsets and visual
    /// transforms (§7.1). The render transform is never consulted — it is
    /// in device pixels and carries visual transforms.
    pub fn hosted_frame(&self, bounds: kurbo::Rect, rect: kurbo::Rect) -> kurbo::Rect {
        kurbo::Rect::from_origin_size(
            self.frame.origin() + (rect.origin() - bounds.origin()),
            rect.size(),
        )
    }

    /// The context for retained *content* a chrome container places at
    /// `frame` — `NavigationView`/`Tabs` content, a split pane's columns.
    /// The subtree inherits this context's window, boundaries and released
    /// regions: on each edge whose frame edge touches this context's
    /// boundary (§7.1's touch, at display resolution) the boundary stays
    /// reachable with its released regions, so a scroll surface inside
    /// still extends and clears and `.ignore_safe_area` inside still
    /// releases; an edge whose frame edge lands on a recorded dock edge
    /// keeps its `Docked` boundary — the host's bar edge follows this
    /// content's edge wherever the deeper layout places it, so a bar a
    /// nested chrome container docks there stacks on the outer bar; every
    /// other edge is `Covered` — nothing inside can touch, release or
    /// extend past it, so chrome content never reseeds a band the host
    /// did not leave on the boundary.
    pub fn hosted(&self, frame: kurbo::Rect) -> Self {
        let mut next = self.with_frame(frame);
        for edge in EDGES {
            let covered = match next.boundary(edge) {
                EdgeBoundary::Docked { .. } => !next.touches_dock(edge),
                _ => !next.touches(edge),
            };
            if covered {
                next.set_boundary(edge, EdgeBoundary::Covered);
            }
        }
        next
    }

    /// §7.1's chrome split on `edge`, produced in one derivation so no
    /// caller can pair a bar with a context the split did not derive: the
    /// band `extent` thick the docked bar occupies, the context the bar's
    /// subtree lays out against, the remainder of `bounds` the hosted
    /// content keeps, and the content's context — all in `bounds` space
    /// (the contexts record window space).
    ///
    /// The band and the bar's context follow [`Self::chrome_splits`]'s
    /// per-bar rule. The content's context is [`Self::hosted`] on the
    /// remainder's frame with the bar's edge `Docked` on the band's inner
    /// edge — nothing inside touches, releases or extends through it, and
    /// a bar a nested chrome container docks on the same edge stacks on
    /// this bar's inner edge.
    #[must_use]
    pub fn chrome_split(&self, bounds: kurbo::Rect, edge: Edge, extent: f64) -> ChromeSplit {
        let (band, content, bar_area, inner) = self.bar_split(bounds, edge, extent);
        let content_area = if extent > 0.0 {
            self.chrome_content_area(self.hosted_frame(bounds, content), [(edge, inner)])
        } else {
            self.hosted(self.hosted_frame(bounds, content))
        };
        ChromeSplit {
            bar: ChromeBar {
                bounds,
                edge,
                inner: (extent > 0.0).then_some(inner),
                band,
                rest: content,
                bar_area,
            },
            content,
            content_area,
        }
    }

    /// §7.1's chrome split over every edge in `bars`, produced in one
    /// derivation so a chrome container drawing bars on several edges
    /// pairs every bar with the context the split derived and never
    /// re-derives or allocates per bar: each bar's [`ChromeBar`], the one
    /// content rect `bounds` leaves once every band is carved, and the
    /// one content context — [`Self::hosted`] on the content's frame with
    /// each bar's edge `Docked` on its band's inner edge.
    #[must_use]
    pub fn chrome_splits<const N: usize>(
        &self,
        bounds: kurbo::Rect,
        bars: [(Edge, f64); N],
    ) -> ChromeSplits<N> {
        let splits = bars.map(|(edge, extent)| {
            let (band, rest, bar_area, inner) = self.bar_split(bounds, edge, extent);
            ChromeBar {
                bounds,
                edge,
                inner: (extent > 0.0).then_some(inner),
                band,
                rest,
                bar_area,
            }
        });
        let content = splits
            .iter()
            .fold(bounds, |content, bar| content.intersect(bar.rest));
        let content_area = self.chrome_content_area(
            self.hosted_frame(bounds, content),
            splits.iter().filter_map(ChromeBar::dock),
        );
        ChromeSplits {
            bars: splits,
            content,
            content_area,
        }
    }

    /// The per-edge derivation both chrome splits build on: the band
    /// `extent` thick, the remainder of `bounds` outside it, the context
    /// the bar's subtree lays out against, and the band's inner edge in
    /// window space — in `bounds` space for the rects (the context
    /// records window space).
    ///
    /// On an edge whose boundary this context's frame touches, the band's
    /// outer edge lands on the *container* region's boundary: a bar
    /// docked to its edge is laid out clear of the container region only,
    /// so the keyboard region covers it instead of lifting it — under a
    /// raised keyboard the band sits wholly past `bounds`' own edge. On
    /// an edge the frame does not touch, the band keeps `bounds`' edge;
    /// on a `Docked` edge whose dock the frame's edge lands on, the band
    /// stacks on the outer bar's inner edge. The bar's context is this
    /// context with the keyboard region released on `edge`, so the bar's
    /// fills and extensions still reach the window edge through both
    /// regions.
    fn bar_split(
        &self,
        bounds: kurbo::Rect,
        edge: Edge,
        extent: f64,
    ) -> (kurbo::Rect, kurbo::Rect, Self, f64) {
        let bar_area = self.releasing_keyboard(edge);
        let position = self
            .docked_bar_position(&bar_area, bounds, edge)
            .unwrap_or_else(|| edge.frame_edge(bounds));
        let (band, rest) = edge.split_band(bounds, position, extent);
        let inner = edge.inner_edge(self.hosted_frame(bounds, band));
        (band, rest, bar_area, inner)
    }

    /// The context for hosted content laid out at `frame` with a bar
    /// docked on each of `docks`' edges — [`Self::hosted`] plus a `Docked`
    /// boundary carrying the bar's inner edge in window space and
    /// recording `frame`'s edge as the dock edge, so the content touches
    /// no edge a bar sits on and a nested bar whose own frame edge lands
    /// on it stacks on it (§7.1).
    #[must_use]
    pub fn chrome_content_area(
        &self,
        frame: kurbo::Rect,
        docks: impl IntoIterator<Item = (Edge, f64)>,
    ) -> Self {
        let mut area = self.hosted(frame);
        for (edge, inner) in docks {
            area.set_boundary(
                edge,
                EdgeBoundary::Docked {
                    edge_at: edge.frame_edge(frame),
                    inner,
                },
            );
        }
        area
    }

    /// This context with the keyboard region released on `edge` — the
    /// context a chrome container hands its bar's subtree: §7.1 lays a
    /// bar docked to its edge out clear of the container region only, so
    /// the keyboard covers it instead of lifting it — while the bar's
    /// fills and extensions still reach the window edge through both
    /// regions. When releasing the keyboard would leave the boundary
    /// where it stands — the keyboard is down, already released, or the
    /// edge carries no keyboard depth — the context keeps its recorded
    /// boundary rather than rebuilding it from f64 inset depths (the
    /// module seeds boundaries from f32 frames on purpose).
    #[must_use]
    pub fn releasing_keyboard(&self, edge: Edge) -> Self {
        let mut next = self.clone();
        let EdgeBoundary::Reachable { released, .. } = self.boundary(edge) else {
            return next;
        };
        let released_with_keyboard = released.union(SafeAreaRegions::KEYBOARD);
        // Releasing the keyboard changes nothing when the keyboard is
        // already released or has no depth on this edge — keyboard down,
        // or a bottom-only keyboard on the top edge: keep the recorded
        // boundary rather than rebuilding the same position from f64 inset
        // depths (the module seeds boundaries from f32 frames on purpose).
        if released_with_keyboard == released || f64::from(edge.depth_in(&self.keyboard)) <= 0.0 {
            return next;
        }
        let unreleased = self.unreleased_depth(edge, released_with_keyboard);
        let position = edge.boundary_at(self.window, unreleased);
        next.set_boundary(
            edge,
            EdgeBoundary::Reachable {
                position,
                released: released_with_keyboard,
            },
        );
        next
    }

    /// The bounds-space position a bar docked to `edge` reaches: the
    /// recorded dock — the outer bar's inner edge — when the frame's edge
    /// lands on the dock edge, or, on an edge the frame touches, the
    /// boundary the keyboard-released context `bar_area` records (§7.1's
    /// docked bar clears the container region only). Mapped into `bounds`
    /// space through [`Self::hosted_frame`]'s offset. `None` on an edge
    /// this context's frame does not touch and does not end at a dock on:
    /// the band keeps `bounds`' edge.
    fn docked_bar_position(&self, bar_area: &Self, bounds: kurbo::Rect, edge: Edge) -> Option<f64> {
        let position = match self.boundary(edge) {
            EdgeBoundary::Docked { inner, .. } if self.touches_dock(edge) => Some(inner),
            EdgeBoundary::Reachable { .. } if self.touches(edge) => {
                bar_area.boundary(edge).position()
            }
            EdgeBoundary::Docked { .. }
            | EdgeBoundary::Reachable { .. }
            | EdgeBoundary::Covered => None,
        }?;
        Some(match edge {
            Edge::Top | Edge::Bottom => position - self.frame.y0 + bounds.y0,
            Edge::Leading | Edge::Trailing => position - self.frame.x0 + bounds.x0,
        })
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
        released.unreleased_depth(edge, &self.container, &self.keyboard)
    }

    /// The edge's touch tolerance in window logical units: half a physical
    /// pixel at the window's scale factor on the axis the edge moves along
    /// — the resolution the display can actually show, which is also what
    /// absorbs the one-ULP gaps f32 placement arithmetic produces.
    fn touch_tolerance(&self, edge: Edge) -> f64 {
        0.5 / if edge.is_vertical() {
            self.y_scale
        } else {
            self.x_scale
        }
    }

    /// §7.1's "touches": `frame`'s edge lands on the edge's boundary at
    /// display resolution — within half a physical pixel. Frames come
    /// through f32 layout arithmetic (stack cursors, negotiated offers,
    /// released sizes), so an edge a sub-pixel away from the boundary is
    /// the same touch to the eye; comparing exactly would flip the answer
    /// frame to frame while a keyboard animation resizes the window.
    fn touches(&self, edge: Edge) -> bool {
        let EdgeBoundary::Reachable { position, .. } = self.boundary(edge) else {
            return false;
        };
        (edge.frame_edge(self.frame) - position).abs() < self.touch_tolerance(edge)
    }

    /// Whether the frame's `edge` lands on this edge's recorded dock edge —
    /// the same half-physical-pixel test [`Self::touches`] runs, against
    /// the hosted content edge the dock was built for. A `Docked` boundary
    /// answers to frames that end where the dock was established and stays
    /// inert under any other frame (`hosted` covers it there).
    fn touches_dock(&self, edge: Edge) -> bool {
        let EdgeBoundary::Docked { edge_at, .. } = self.boundary(edge) else {
            return false;
        };
        (edge.frame_edge(self.frame) - edge_at).abs() < self.touch_tolerance(edge)
    }

    /// The context and laid-out size for the child of an
    /// `.ignore_safe_area` wrapper carrying `ignore` (the wrapper's frame
    /// grown by the release), plus the released amounts the flush mirrors.
    ///
    /// On each edge `ignore.edges` names that the wrapper's laid-out frame
    /// touches the subtree boundary on, the boundary moves out to the
    /// deepest region `ignore.regions` does not name — the window edge when
    /// it names both — and the child's frame grows to meet it. An edge the
    /// frame does not touch releases nothing, and because released regions
    /// accumulate, an inner declaration can never shrink a boundary an
    /// outer one already moved (§7.1: nested declarations cannot
    /// double-release).
    ///
    /// Like [`Self::root`], the moved boundary and the grown frame are
    /// seeded from the child's f32-measured `size`: leading edges anchor at
    /// the released boundary positions and the trailing edges come back
    /// from `size`, so a descendant's f32-derived frame edge lands on the
    /// moved boundary the same way the root's own frame lands on its own.
    pub fn release(&self, ignore: IgnoreSafeArea, size: Size) -> (Self, EdgeOffsets, Size) {
        let mut positions: [Option<(f64, ReleasedRegions)>; 4] = [None; 4];
        let mut released = EdgeOffsets::default();
        for (index, edge) in EDGES.iter().enumerate() {
            let boundary = self.boundary(*edge);
            if !edge.named_in(ignore.edges) || !self.touches(*edge) {
                continue;
            }
            let regions = boundary.released().union(ignore.regions);
            let position = edge.boundary_at(self.window, self.unreleased_depth(*edge, regions));
            // A frame edge inside the tolerance but past the boundary puts
            // the amount under zero — a release never moves a frame inward.
            released.set(*edge, edge.released_amount(self.frame, position).max(0.0));
            positions[index] = Some((position, regions));
        }
        let child_size = released_size(size, released);
        // The leading edges anchor at their released positions (or keep the
        // frame's); the trailing edges are seeded back from the f32 child
        // size, exactly as `root` seeds them — a released bottom edge lands
        // on the same f32-derived position a descendant computes for it.
        let mut frame = self.frame;
        if let Some((position, _)) = positions[0] {
            frame.y0 = position;
        }
        if let Some((position, _)) = positions[1] {
            frame.x0 = position;
        }
        frame.x1 = frame.x0 + f64::from(child_size.width);
        frame.y1 = frame.y0 + f64::from(child_size.height);
        let mut next = self.clone();
        next.frame = frame;
        for (index, edge) in EDGES.iter().enumerate() {
            if let Some((_, regions)) = positions[index] {
                // The moved boundary is the grown frame's edge — seeded
                // from the f32 size, like `root`, not the f64 region depth.
                next.set_boundary(
                    *edge,
                    EdgeBoundary::Reachable {
                        position: edge.frame_edge(frame),
                        released: regions,
                    },
                );
            }
        }
        (next, released, child_size)
    }

    /// On each edge, the distance from the frame's edge to the window
    /// edge — non-zero only where the laid-out frame ends on the subtree's
    /// boundary (§7.1's "touches", resolved at display resolution). A
    /// fill's default paint extension and a scroll surface's extension are
    /// this same answer: both reach the window edge through every region
    /// on a touched edge.
    pub fn touched_edge_offsets(&self) -> EdgeOffsets {
        let mut offsets = EdgeOffsets::default();
        for edge in EDGES {
            if self.touches(edge) {
                let frame_edge = edge.frame_edge(self.frame);
                offsets.set(edge, edge.window_gap(self.window, frame_edge));
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
#[derive(Clone, Copy, Default)]
pub struct ScrollSurfaceFacts {
    /// Per-edge distance from the surface's laid-out frame to the window
    /// edge — non-zero only on edges the frame touched the boundary on.
    /// The clip, the wheel target, the published lazy viewport and the
    /// scroll metrics' viewport/content all extend by it.
    pub extension: EdgeOffsets,
    /// The surface's laid-out frame in window space — the clearance
    /// bound's lower term and the taller-field clamp.
    pub window_frame: kurbo::Rect,
    /// The keyboard region's top edge in window space — the clearance
    /// bound's upper term.
    pub keyboard_top: f64,
}

/// The answers [`SafeAreaLayout::chrome_split`] derives together for one
/// edge of one `bounds` (§7.1): the bar's [`ChromeBar`], the hosted
/// content's rect, and the content's context. A chrome container drawing
/// bars on several edges runs [`SafeAreaLayout::chrome_splits`] instead —
/// one call returning each bar's share and the single composed content
/// rect and context.
pub struct ChromeSplit {
    /// The bar's share of the split — its band, the context its subtree
    /// lays out against, and the dock it establishes.
    pub bar: ChromeBar,
    /// The remainder of `bounds` the hosted content lays out in — always
    /// inside `bounds`, clear of both regions. Equal to `bar.rest`: a
    /// single-edge split's content rect is the rest outside its one band.
    pub content: kurbo::Rect,
    /// The hosted content's context: [`SafeAreaLayout::hosted`] on
    /// `content`'s frame plus a `Docked` boundary on the bar's edge
    /// carrying the band's inner edge — nothing inside the content
    /// touches, releases or extends through the edge the bar sits on.
    pub content_area: SafeAreaLayout,
}

/// One bar's share of [`ChromeSplits`]: its band, the context its
/// subtree lays out against, and the dock it hands the composed content
/// context — the same per-bar answer [`ChromeSplit`] carries for the
/// single-edge split.
pub struct ChromeBar {
    /// The `bounds` the split ran on — the space `band`, `rest` and
    /// [`Self::bar_area_for`]'s `rect` live in.
    bounds: kurbo::Rect,
    /// The edge this bar's split ran on.
    edge: Edge,
    /// The band's inner edge in window space — the dock position a nested
    /// bar on `edge` lands on. `Some` only when a bar exists (`extent > 0`).
    inner: Option<f64>,
    /// The band the bar occupies, in `bounds` space — it may sit wholly
    /// past `bounds`' own edge once the keyboard covers it.
    pub band: kurbo::Rect,
    /// The remainder of `bounds` outside this bar's band alone —
    /// intersected with every other bar's into [`ChromeSplits::content`].
    pub rest: kurbo::Rect,
    /// The context for views placed inside the band — the keyboard
    /// region released on the bar's edge so the bar's fills and
    /// extensions still reach the window edge. Its frame is the chrome
    /// container's own; re-frame it per placed rect with
    /// [`Self::bar_area_for`].
    pub bar_area: SafeAreaLayout,
}

impl ChromeBar {
    /// The `(edge, inner-edge-in-window-space)` pair a composed
    /// [`SafeAreaLayout::chrome_content_area`] call takes — the dock this
    /// bar establishes, `None` when the split ran for no bar.
    pub fn dock(&self) -> Option<(Edge, f64)> {
        self.inner.map(|inner| (self.edge, inner))
    }

    /// The context for a view placed at `rect` inside the band — the
    /// bar's context re-framed the way [`SafeAreaLayout::hosted_frame`]
    /// maps `rect` inside `bounds`.
    pub fn bar_area_for(&self, rect: kurbo::Rect) -> SafeAreaLayout {
        self.bar_area
            .with_frame(self.bar_area.hosted_frame(self.bounds, rect))
    }
}

/// The answers [`SafeAreaLayout::chrome_splits`] derives together for a
/// chrome container's bars (§7.1): each bar's [`ChromeBar`], the one
/// content rect `bounds` leaves once every band is carved, and the one
/// content context — [`SafeAreaLayout::hosted`] on the content's frame
/// plus every bar's `Docked` boundary.
pub struct ChromeSplits<const N: usize> {
    /// Each bar's share of the split, in the order `chrome_splits` took
    /// the edges.
    pub bars: [ChromeBar; N],
    /// The remainder of `bounds` outside every band — always inside
    /// `bounds`, clear of both regions.
    pub content: kurbo::Rect,
    /// The hosted content's context: `Docked` on every edge a bar sits
    /// on, carrying that band's inner edge, so nothing inside touches,
    /// releases or extends through those edges and a nested bar whose
    /// frame lands on a dock edge stacks on the outer bar.
    pub content_area: SafeAreaLayout,
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
    pub facts: Cell<Option<ScrollSurfaceFacts>>,
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
    /// The touched-edge extension layout recorded for this surface — zero
    /// where the surface has no safe-area context. The one place the
    /// `facts → extension` read lives, so `ScrollNode`, `List` and `Table`
    /// share it.
    pub fn extension(&self) -> EdgeOffsets {
        self.facts
            .get()
            .map_or_else(EdgeOffsets::default, |facts| facts.extension)
    }

    /// The span of content the viewport shows on `axis`, in content
    /// coordinates: the offset pushed inward by the leading extension
    /// through the already extension-grown viewport the metrics report.
    /// `List`'s visible row windows and `Table`'s column windows read
    /// this.
    pub fn visible_span(
        &self,
        metrics: &crate::scroll::ScrollMetrics,
        axis: ScrollAxis,
    ) -> Range<f64> {
        let extension = self.extension();
        let (offset, viewport, inset) = match axis {
            ScrollAxis::Vertical => (metrics.offset_y, metrics.viewport_height, extension.top),
            ScrollAxis::Horizontal => (metrics.offset_x, metrics.viewport_width, extension.leading),
            other => panic!("hydrolysis scroll surface: unsupported span axis {other:?}"),
        };
        (offset - inset)..(offset - inset + viewport)
    }

    /// The viewport and content extents the scroll handle binds: both
    /// grown by the touched-edge extension, so the scrollable range keeps
    /// the resting edges on the avoided boundary while scrolling paints
    /// through the band (§7.1). The one place the growth lives —
    /// `ScrollNode`, `List` and `Table` all rebind through it.
    pub fn extended(
        &self,
        viewport: kurbo::Size,
        content: kurbo::Size,
    ) -> (kurbo::Size, kurbo::Size) {
        let extension = self.extension();
        (
            kurbo::Size::new(
                viewport.width + extension.horizontal(),
                viewport.height + extension.vertical(),
            ),
            kurbo::Size::new(
                content.width + extension.horizontal(),
                content.height + extension.vertical(),
            ),
        )
    }

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
        // Only an outermost surface holds facts (a surface's content lays
        // out without a safe-area context), so the field a surface finds
        // here is always one it alone covers — nested surfaces clear the
        // same field once through the outer surface.
        let field = renderer
            .text_editing
            .focused_index()
            .filter(|index| *index >= targets_start)
            .map(|index| renderer.text_editing.text_input_targets[index].clone())
            .map(|target| ClearedField {
                key: target.interaction_key,
                rect: target.frame,
                offset_y,
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
    /// it back. This assumes the field did not move inside the content
    /// between frames — a relayout that moves it re-captures through
    /// `end_flush` before the next keyboard-moving pass reads it.
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
