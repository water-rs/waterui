//! Window-space placement for [`AnchoredOverlay`], shared by every backend.
//!
//! An overlay floats above all other content, so only the backend — which
//! sees the anchor's resolved frame and the window's bounds — can place it.
//! [`place_anchored_overlay`] is the single implementation of that contract:
//! every Rust backend calls it, and the FFI re-exports it for the Swift,
//! Kotlin and GTK backends, so no backend re-implements the placement rules.
//!
//! All geometry is in the window's coordinate space: origin at the window's
//! top-left corner, `x` growing right and `y` growing down.

use waterui::metadata::anchored_overlay::{AnchorEdge, AnchorPlacement, Clamp, EdgeAlignment};
use waterui_core::layout::{LayoutDirection, Point, Rect, Size};

/// The physical side of the anchor an overlay is placed against.
///
/// Unlike [`AnchorEdge`], a `PhysicalEdge` names an actual side of the
/// anchor's frame — `Leading` and `Trailing` are already resolved under the
/// layout direction — so it is the type a backend consumes when it positions
/// or animates the overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalEdge {
    /// Above the anchor.
    Top,
    /// Below the anchor.
    Bottom,
    /// To the anchor's left.
    Left,
    /// To the anchor's right.
    Right,
}

impl PhysicalEdge {
    /// The physical edge on the opposite side of the anchor.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// Whether the overlay sits above or below the anchor on this edge.
    const fn is_horizontal(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// The result of [`place_anchored_overlay`]: the overlay's frame in window
/// space and the physical anchor edge it was placed against.
///
/// `edge` is `placement.edge` resolved under the layout direction unless
/// [`flip`](AnchorPlacement::flip) moved the overlay to the opposite edge;
/// backends that draw an arrow or animate from the anchor need it to know
/// which side the overlay actually landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchoredOverlayPlacement {
    /// The overlay's frame in window space.
    pub frame: Rect,
    /// The physical edge of the anchor the overlay was placed against, after
    /// flipping.
    pub edge: PhysicalEdge,
}

/// Places an overlay measured at `overlay` against `anchor` inside `window`,
/// implementing the [`AnchoredOverlay`](waterui::metadata::anchored_overlay::AnchoredOverlay)
/// placement contract:
///
/// 1. The overlay sits against `placement.edge` of `anchor`, `placement.gap`
///    points away.
/// 2. Along that edge, `placement.alignment` aligns it: `Start`/`End` are
///    leading/trailing along the top and bottom edges and top/bottom along
///    the leading and trailing edges. `Center` centers it on the anchor.
///    Leading and trailing follow `direction`.
/// 3. When `placement.flip` is set and the overlay fits on the opposite edge
///    but not between the anchor and the window edge on the preferred side,
///    the opposite edge is used. The usable window edge here is the clamped
///    one: under `Clamp::Window { margin }` the overlay must clear the margin,
///    so a placement that would land inside it counts as not fitting. When it
///    fits on neither side, the preferred edge is kept.
/// 4. `Clamp::Window { margin }` shifts the overlay on both axes so it stays
///    at least `margin` points inside `window`. An overlay larger than the
///    window minus the margins is pinned to the leading/top margin.
///
/// `overlay` is the content's ideal size, already capped by the window size;
/// it is capped again here so a backend that hands the unbounded measurement
/// still gets a frame inside the contract.
#[must_use]
pub fn place_anchored_overlay(
    anchor: Rect,
    window: Rect,
    overlay: Size,
    placement: AnchorPlacement,
    direction: LayoutDirection,
) -> AnchoredOverlayPlacement {
    let overlay = Size::new(
        overlay.width.min(window.width()),
        overlay.height.min(window.height()),
    );

    let edge = resolve_edge(anchor, window, overlay, &placement, direction);
    let mut origin = origin_at(anchor, edge, overlay, &placement, direction);

    if let Clamp::Window { margin } = placement.clamp {
        origin = clamp_origin(origin, window, overlay, margin, direction);
    }

    AnchoredOverlayPlacement {
        frame: Rect::new(origin, overlay),
        edge,
    }
}

/// The physical anchor edge `edge` refers to under `direction`: `Leading` is
/// left and `Trailing` right in a left-to-right layout, swapped in a
/// right-to-left one; `Top` and `Bottom` never move.
const fn physical_edge(edge: AnchorEdge, direction: LayoutDirection) -> PhysicalEdge {
    match edge {
        AnchorEdge::Top => PhysicalEdge::Top,
        AnchorEdge::Bottom => PhysicalEdge::Bottom,
        AnchorEdge::Leading if direction.is_right_to_left() => PhysicalEdge::Right,
        AnchorEdge::Leading => PhysicalEdge::Left,
        AnchorEdge::Trailing if direction.is_right_to_left() => PhysicalEdge::Left,
        AnchorEdge::Trailing => PhysicalEdge::Right,
    }
}

/// Whether `overlay` fits between `anchor` and the window on `edge`, leaving
/// `gap` points against the anchor. The usable window edge is inset by
/// `margin`, so under `Clamp::Window` an overlay that would land within the
/// margin counts as not fitting and is eligible to flip.
fn fits_on(
    anchor: Rect,
    window: Rect,
    overlay: Size,
    edge: PhysicalEdge,
    gap: f32,
    margin: f32,
) -> bool {
    match edge {
        PhysicalEdge::Top => anchor.min_y() - gap - overlay.height >= window.min_y() + margin,
        PhysicalEdge::Bottom => anchor.max_y() + gap + overlay.height <= window.max_y() - margin,
        PhysicalEdge::Left => anchor.min_x() - gap - overlay.width >= window.min_x() + margin,
        PhysicalEdge::Right => anchor.max_x() + gap + overlay.width <= window.max_x() - margin,
    }
}

/// The physical edge the overlay is placed against: `placement.edge` resolved
/// under `direction`, unless `flip` is set and the overlay fits on the
/// opposite side but not the preferred one.
fn resolve_edge(
    anchor: Rect,
    window: Rect,
    overlay: Size,
    placement: &AnchorPlacement,
    direction: LayoutDirection,
) -> PhysicalEdge {
    let preferred = physical_edge(placement.edge, direction);
    let margin = match placement.clamp {
        Clamp::Window { margin } => margin,
        Clamp::Off => 0.0,
    };
    if !placement.flip
        || fits_on(anchor, window, overlay, preferred, placement.gap, margin)
        || !fits_on(
            anchor,
            window,
            overlay,
            preferred.opposite(),
            placement.gap,
            margin,
        )
    {
        preferred
    } else {
        preferred.opposite()
    }
}

/// The overlay's origin against physical `edge` of `anchor` before clamping.
fn origin_at(
    anchor: Rect,
    edge: PhysicalEdge,
    overlay: Size,
    placement: &AnchorPlacement,
    direction: LayoutDirection,
) -> Point {
    let gap = placement.gap;
    if edge.is_horizontal() {
        let y = if edge == PhysicalEdge::Top {
            anchor.min_y() - gap - overlay.height
        } else {
            anchor.max_y() + gap
        };
        // Start/End along a horizontal edge are the leading/trailing ends.
        let x = match placement.alignment {
            EdgeAlignment::Start if direction.is_right_to_left() => anchor.max_x() - overlay.width,
            EdgeAlignment::Start => anchor.min_x(),
            EdgeAlignment::Center => anchor.mid_x() - overlay.width / 2.0,
            EdgeAlignment::End if direction.is_right_to_left() => anchor.min_x(),
            EdgeAlignment::End => anchor.max_x() - overlay.width,
        };
        Point::new(x, y)
    } else {
        let x = if edge == PhysicalEdge::Left {
            anchor.min_x() - gap - overlay.width
        } else {
            anchor.max_x() + gap
        };
        // Start/End along a vertical edge are the top/bottom ends.
        let y = match placement.alignment {
            EdgeAlignment::Start => anchor.min_y(),
            EdgeAlignment::Center => anchor.mid_y() - overlay.height / 2.0,
            EdgeAlignment::End => anchor.max_y() - overlay.height,
        };
        Point::new(x, y)
    }
}

/// Shifts `origin` so the overlay stays at least `margin` points inside
/// `window` on both axes. An axis where the overlay is larger than the window
/// minus the margins pins to the leading (on `x`, in `direction`) or top (on
/// `y`) margin.
fn clamp_origin(
    origin: Point,
    window: Rect,
    overlay: Size,
    margin: f32,
    direction: LayoutDirection,
) -> Point {
    let x = clamp_axis(
        origin.x,
        window.min_x(),
        window.max_x(),
        overlay.width,
        margin,
    );
    let x = if direction.is_right_to_left() && overlay.width > window.width() - margin - margin {
        // Pinning to the leading margin in RTL is the right edge.
        window.max_x() - margin - overlay.width
    } else {
        x
    };
    let y = clamp_axis(
        origin.y,
        window.min_y(),
        window.max_y(),
        overlay.height,
        margin,
    );
    Point::new(x, y)
}

/// `value` moved into `[low + margin, high - margin - extent]`; when `extent`
/// leaves no room, the low (leading/top) margin.
fn clamp_axis(value: f32, low: f32, high: f32, extent: f32, margin: f32) -> f32 {
    let min = low + margin;
    let max = high - margin - extent;
    if max < min {
        min
    } else {
        value.clamp(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Rect = Rect::new(Point::new(0.0, 0.0), Size::new(320.0, 240.0));

    /// An anchor centered in the window, large enough that both sides fit.
    const MIDDLE: Rect = Rect::new(Point::new(110.0, 80.0), Size::new(100.0, 40.0));

    const OVERLAY: Size = Size::new(60.0, 20.0);

    fn place(anchor: Rect, overlay: Size, placement: AnchorPlacement) -> AnchoredOverlayPlacement {
        place_anchored_overlay(
            anchor,
            WINDOW,
            overlay,
            placement,
            LayoutDirection::LeftToRight,
        )
    }

    fn placement(edge: AnchorEdge, alignment: EdgeAlignment) -> AnchorPlacement {
        AnchorPlacement {
            edge,
            alignment,
            ..AnchorPlacement::default()
        }
    }

    #[test]
    fn bottom_center() {
        let placed = place(
            MIDDLE,
            OVERLAY,
            placement(AnchorEdge::Bottom, EdgeAlignment::Center),
        );
        assert_eq!(placed.edge, PhysicalEdge::Bottom);
        // Below the anchor, horizontally centered on it.
        assert_eq!(placed.frame, Rect::new(Point::new(130.0, 120.0), OVERLAY));
    }

    #[test]
    fn every_edge_centered() {
        for (edge, physical, expected) in [
            (AnchorEdge::Top, PhysicalEdge::Top, (130.0, 60.0)),
            (AnchorEdge::Bottom, PhysicalEdge::Bottom, (130.0, 120.0)),
            (AnchorEdge::Leading, PhysicalEdge::Left, (50.0, 90.0)),
            (AnchorEdge::Trailing, PhysicalEdge::Right, (210.0, 90.0)),
        ] {
            let placed = place(MIDDLE, OVERLAY, placement(edge, EdgeAlignment::Center));
            assert_eq!(placed.edge, physical);
            assert_eq!(
                placed.frame,
                Rect::new(Point::new(expected.0, expected.1), OVERLAY),
                "edge {edge:?}"
            );
        }
    }

    #[test]
    fn start_and_end_along_horizontal_edges() {
        // Start = leading, End = trailing on top/bottom edges (LTR).
        for edge in [AnchorEdge::Top, AnchorEdge::Bottom] {
            let start = place(MIDDLE, OVERLAY, placement(edge, EdgeAlignment::Start));
            assert_eq!(start.frame.x(), MIDDLE.min_x());
            let end = place(MIDDLE, OVERLAY, placement(edge, EdgeAlignment::End));
            assert_eq!(end.frame.max_x(), MIDDLE.max_x());
        }
    }

    #[test]
    fn start_and_end_along_vertical_edges() {
        // Start = top, End = bottom on leading/trailing edges.
        for edge in [AnchorEdge::Leading, AnchorEdge::Trailing] {
            let start = place(MIDDLE, OVERLAY, placement(edge, EdgeAlignment::Start));
            assert_eq!(start.frame.y(), MIDDLE.min_y());
            let end = place(MIDDLE, OVERLAY, placement(edge, EdgeAlignment::End));
            assert_eq!(end.frame.max_y(), MIDDLE.max_y());
        }
    }

    #[test]
    fn gap_moves_overlay_away() {
        let placed = place(
            MIDDLE,
            OVERLAY,
            AnchorPlacement {
                gap: 8.0,
                ..placement(AnchorEdge::Bottom, EdgeAlignment::Center)
            },
        );
        assert_eq!(placed.frame.y(), MIDDLE.max_y() + 8.0);
    }

    #[test]
    fn start_end_follow_leading_in_rtl() {
        let rtl = |anchor, alignment| {
            place_anchored_overlay(
                anchor,
                WINDOW,
                OVERLAY,
                placement(AnchorEdge::Bottom, alignment),
                LayoutDirection::RightToLeft,
            )
        };
        // Start = leading = the right side in RTL.
        assert_eq!(
            rtl(MIDDLE, EdgeAlignment::Start).frame.max_x(),
            MIDDLE.max_x()
        );
        assert_eq!(rtl(MIDDLE, EdgeAlignment::End).frame.x(), MIDDLE.min_x());
    }

    #[test]
    fn leading_trailing_swap_in_rtl() {
        let rtl = |edge| {
            place_anchored_overlay(
                MIDDLE,
                WINDOW,
                OVERLAY,
                placement(edge, EdgeAlignment::Center),
                LayoutDirection::RightToLeft,
            )
        };
        // Leading in RTL is the physical right edge — on the frame and in
        // the returned edge.
        let leading = rtl(AnchorEdge::Leading);
        assert_eq!(leading.edge, PhysicalEdge::Right);
        assert_eq!(leading.frame.x(), MIDDLE.max_x());
        let trailing = rtl(AnchorEdge::Trailing);
        assert_eq!(trailing.edge, PhysicalEdge::Left);
        assert_eq!(trailing.frame.max_x(), MIDDLE.min_x());
    }

    #[test]
    fn leading_trailing_report_physical_edges_ltr() {
        for (edge, physical) in [
            (AnchorEdge::Leading, PhysicalEdge::Left),
            (AnchorEdge::Trailing, PhysicalEdge::Right),
        ] {
            let placed = place(MIDDLE, OVERLAY, placement(edge, EdgeAlignment::Center));
            assert_eq!(placed.edge, physical);
        }
    }

    #[test]
    fn flips_when_preferred_side_overflows() {
        // Anchor near the top: a top-edge overlay cannot fit above it, so it
        // flips to the bottom.
        let top_anchor = Rect::new(Point::new(110.0, 10.0), Size::new(100.0, 40.0));
        let placed = place(
            top_anchor,
            OVERLAY,
            placement(AnchorEdge::Top, EdgeAlignment::Center),
        );
        assert_eq!(placed.edge, PhysicalEdge::Bottom);
        assert_eq!(placed.frame.y(), top_anchor.max_y());
    }

    #[test]
    fn flips_leading_to_trailing() {
        // Anchor against the leading window edge: a leading overlay flips.
        let anchor = Rect::new(Point::new(10.0, 100.0), Size::new(40.0, 40.0));
        let placed = place(
            anchor,
            OVERLAY,
            placement(AnchorEdge::Leading, EdgeAlignment::Center),
        );
        assert_eq!(placed.edge, PhysicalEdge::Right);
        assert_eq!(placed.frame.x(), anchor.max_x());
    }

    #[test]
    fn keeps_preferred_edge_when_neither_side_fits() {
        // A tall anchor leaves no room above or below; the preferred edge stays.
        let tall_anchor = Rect::new(Point::new(110.0, 10.0), Size::new(100.0, 220.0));
        let placed = place(
            tall_anchor,
            OVERLAY,
            placement(AnchorEdge::Top, EdgeAlignment::Center),
        );
        assert_eq!(placed.edge, PhysicalEdge::Top);
        // Placement went above the anchor (y = -10); the default window clamp
        // then shifted it back inside at the top margin.
        assert_eq!(placed.frame.y(), WINDOW.min_y());
    }

    #[test]
    fn no_flip_without_flag() {
        let top_anchor = Rect::new(Point::new(110.0, 10.0), Size::new(100.0, 40.0));
        let placed = place(
            top_anchor,
            OVERLAY,
            AnchorPlacement {
                flip: false,
                ..placement(AnchorEdge::Top, EdgeAlignment::Center)
            },
        );
        assert_eq!(placed.edge, PhysicalEdge::Top);
    }

    #[test]
    fn flip_measures_against_the_clamp_margin() {
        // One point of room below the anchor: without the margin the overlay
        // fits on Bottom, but `Clamp::Window { margin: 2.0 }` shrinks the
        // usable edge, so it does not fit and flips to Top.
        let anchor = Rect::new(Point::new(110.0, 179.0), Size::new(100.0, 40.0));
        let placed = place(
            anchor,
            OVERLAY,
            AnchorPlacement {
                clamp: Clamp::Window { margin: 2.0 },
                ..placement(AnchorEdge::Bottom, EdgeAlignment::Center)
            },
        );
        assert_eq!(placed.edge, PhysicalEdge::Top);
        assert_eq!(placed.frame.max_y(), anchor.min_y());
    }

    #[test]
    fn clamps_at_every_window_edge() {
        let margin = 4.0;
        let clamped = |anchor| {
            place(
                anchor,
                OVERLAY,
                AnchorPlacement {
                    flip: false,
                    clamp: Clamp::Window { margin },
                    ..placement(AnchorEdge::Trailing, EdgeAlignment::Center)
                },
            )
        };
        // Trailing overflow pushes the overlay back inside the window.
        let at_trailing = Rect::new(Point::new(280.0, 100.0), Size::new(40.0, 40.0));
        let placed = clamped(at_trailing);
        assert_eq!(placed.frame.max_x(), WINDOW.max_x() - margin);

        let clamped_bottom = |anchor| {
            place(
                anchor,
                OVERLAY,
                AnchorPlacement {
                    flip: false,
                    clamp: Clamp::Window { margin },
                    ..placement(AnchorEdge::Bottom, EdgeAlignment::End)
                },
            )
        };
        // End-aligned at the trailing edge clamps onto the window's x margin.
        let placed = clamped_bottom(at_trailing);
        assert_eq!(placed.frame.max_x(), WINDOW.max_x() - margin);
        // Bottom-aligned below the window's bottom edge clamps back inside.
        let at_bottom = Rect::new(Point::new(110.0, 228.0), Size::new(40.0, 40.0));
        let placed = clamped_bottom(at_bottom);
        assert_eq!(placed.frame.max_y(), WINDOW.max_y() - margin);
        // Start-aligned past the leading edge clamps to the x margin.
        let at_leading = Rect::new(Point::new(-50.0, 100.0), Size::new(40.0, 40.0));
        let placed = place(
            at_leading,
            OVERLAY,
            AnchorPlacement {
                clamp: Clamp::Window { margin },
                flip: false,
                ..placement(AnchorEdge::Bottom, EdgeAlignment::Start)
            },
        );
        assert_eq!(placed.frame.x(), WINDOW.min_x() + margin);
        // Above the top edge clamps to the y margin.
        let at_top = Rect::new(Point::new(110.0, -10.0), Size::new(40.0, 40.0));
        let placed = place(
            at_top,
            OVERLAY,
            AnchorPlacement {
                clamp: Clamp::Window { margin },
                flip: false,
                ..placement(AnchorEdge::Top, EdgeAlignment::Center)
            },
        );
        assert_eq!(placed.frame.y(), WINDOW.min_y() + margin);
    }

    #[test]
    fn oversized_overlay_pins_to_margins() {
        let huge = Size::new(400.0, 300.0);
        let placed = place(
            MIDDLE,
            huge,
            AnchorPlacement {
                clamp: Clamp::Window { margin: 4.0 },
                ..placement(AnchorEdge::Bottom, EdgeAlignment::Center)
            },
        );
        // Larger than the window minus margins: pinned to leading/top margin.
        assert_eq!(placed.frame.x(), 4.0);
        assert_eq!(placed.frame.y(), 4.0);
        // The measured size itself is capped at the window.
        assert_eq!(placed.frame.size(), &Size::new(320.0, 240.0));
    }

    #[test]
    fn clamp_off_leaves_overflow() {
        let at_bottom = Rect::new(Point::new(110.0, 228.0), Size::new(40.0, 40.0));
        let placed = place(
            at_bottom,
            OVERLAY,
            AnchorPlacement {
                flip: false,
                clamp: Clamp::Off,
                ..placement(AnchorEdge::Bottom, EdgeAlignment::Center)
            },
        );
        // No clamp: the frame is where the edge/alignment math put it.
        assert_eq!(placed.frame.y(), at_bottom.max_y());
    }
}
