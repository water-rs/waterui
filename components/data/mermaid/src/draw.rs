//! Painting a laid-out diagram into a scene.
//!
//! Only the geometry is painted here: boxes, frames, connectors and their
//! decorations. Every piece of text is a real view placed by [`crate::label`],
//! so the diagram keeps the platform's own text rendering and shows up in the
//! accessibility tree as words rather than as one opaque picture.
//!
//! Everything is drawn at the diagram's natural size, in the diagram's own
//! coordinates. Scaling the geometry to fit a container would break the one
//! guarantee this crate is built on — that the box layout reserved for a label
//! is the box its glyphs land in — because the glyphs would still be drawn at
//! their own size. A diagram that does not fit is the container's problem to
//! solve, by scrolling.

use alloc::vec::Vec;

use waterui_canvas::{DrawingContext, Path};
use waterui_core::layout::Point;

use crate::layout::{DiagramLayout, Edge, EdgeMarker, EdgeStroke};
use crate::shape;
use crate::theme::Palette;

/// Stroke width of a node outline and an ordinary edge.
const STROKE: f32 = 1.0;
/// Stroke width of a `==>` edge.
const THICK_STROKE: f32 = 2.5;
/// Dash pattern of a `-.->` edge.
const DASH: [f32; 2] = [6.0, 4.0];
/// Length of an arrowhead along the edge.
const ARROW_LENGTH: f32 = 10.0;
/// Half-width of an arrowhead across the edge.
const ARROW_HALF_WIDTH: f32 = 4.0;
/// Radius of a circle edge marker.
const MARKER_RADIUS: f32 = 4.0;
/// Half-diagonal of a cross edge marker.
const CROSS_ARM: f32 = 4.0;
/// Largest radius a routed corner is rounded by.
const CORNER_RADIUS: f32 = 8.0;

/// Paints one diagram at its natural size, with its top-left at `origin`.
pub fn diagram(ctx: &mut DrawingContext, layout: &DiagramLayout, palette: &Palette, origin: Point) {
    let at = |point: Point| Point::new(point.x + origin.x, point.y + origin.y);

    for cluster in &layout.clusters {
        let frame = offset_rect(cluster.frame, origin);
        ctx.set_fill_style(palette.cluster);
        ctx.fill_rect(frame);
        ctx.set_stroke_style(palette.border);
        ctx.set_line_width(STROKE);
        ctx.stroke_rect(frame);
    }

    for fragment in &layout.fragments {
        let frame = offset_rect(fragment.frame, origin);
        ctx.set_stroke_style(palette.border);
        ctx.set_line_width(STROKE);
        ctx.stroke_rect(frame);
        for divider in &fragment.dividers {
            let y = divider + origin.y;
            ctx.set_line_dash(DASH.to_vec());
            ctx.stroke_line(Point::new(frame.min_x(), y), Point::new(frame.max_x(), y));
            ctx.set_line_dash(Vec::new());
        }
    }

    for lifeline in &layout.lifelines {
        ctx.set_stroke_style(palette.border);
        ctx.set_line_width(STROKE);
        ctx.set_line_dash(DASH.to_vec());
        ctx.stroke_line(
            at(Point::new(lifeline.x, lifeline.top)),
            at(Point::new(lifeline.x, lifeline.bottom)),
        );
        ctx.set_line_dash(Vec::new());
    }

    for edge in &layout.edges {
        connector(ctx, edge, palette, origin);
    }

    for node in &layout.nodes {
        let outline = shape::outline(node.shape, offset_rect(node.frame, origin));
        if let Some(body) = &outline.body {
            ctx.set_fill_style(palette.node_fill);
            ctx.fill_path(body);
            ctx.set_stroke_style(palette.node_border);
            ctx.set_line_width(STROKE);
            ctx.stroke_path(body);
        }
        for detail in &outline.details {
            ctx.set_stroke_style(palette.node_border);
            ctx.set_line_width(STROKE);
            ctx.stroke_path(detail);
        }
    }
}

fn offset_rect(rect: waterui_core::layout::Rect, origin: Point) -> waterui_core::layout::Rect {
    waterui_core::layout::Rect::new(
        Point::new(rect.x() + origin.x, rect.y() + origin.y),
        *rect.size(),
    )
}

/// Paints one edge: its routed polyline, then a decoration at each end.
fn connector(ctx: &mut DrawingContext, edge: &Edge, palette: &Palette, origin: Point) {
    let points: Vec<Point> = edge
        .points
        .iter()
        .map(|point| Point::new(point.x + origin.x, point.y + origin.y))
        .collect();
    let Some(path) = edge_path(&points) else {
        return;
    };

    ctx.set_stroke_style(palette.edge);
    ctx.set_line_width(match edge.stroke {
        EdgeStroke::Thick => THICK_STROKE,
        EdgeStroke::Normal | EdgeStroke::Dotted => STROKE,
    });
    if edge.stroke == EdgeStroke::Dotted {
        ctx.set_line_dash(DASH.to_vec());
    }
    ctx.stroke_path(&path);
    ctx.set_line_dash(Vec::new());

    marker(ctx, edge.start_marker, points[0], points[1], palette);
    let last = points.len() - 1;
    marker(
        ctx,
        edge.end_marker,
        points[last],
        points[last - 1],
        palette,
    );
}

/// The stroked path of one edge's routed polyline.
///
/// `None` when there is nothing to stroke: a polyline needs two points before
/// it is a line.
fn edge_path(points: &[Point]) -> Option<Path> {
    let [first, rest @ ..] = points else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }
    let mut path = Path::new();
    path.move_to(*first);
    route(&mut path, points);
    Some(path)
}

/// Appends the routed polyline to `path`, rounding each interior corner.
///
/// `points` starts with the point `path` was already moved to.
fn route(path: &mut Path, points: &[Point]) {
    let [_, interior @ .., last] = points else {
        if let Some(single) = points.get(1) {
            path.line_to(*single);
        }
        return;
    };

    let mut previous = points[0];
    for (index, corner) in interior.iter().enumerate() {
        let next = interior.get(index + 1).copied().unwrap_or(*last);
        let radius = corner_radius(previous, *corner, next);
        path.arc_to(*corner, next, radius);
        previous = *corner;
    }
    path.line_to(*last);
}

/// The radius a corner can be rounded by without eating either segment.
fn corner_radius(previous: Point, current: Point, next: Point) -> f32 {
    let incoming = distance(previous, current);
    let outgoing = distance(current, next);
    (incoming.min(outgoing) / 2.0).min(CORNER_RADIUS)
}

fn distance(from: Point, to: Point) -> f32 {
    (to.x - from.x).hypot(to.y - from.y)
}

/// Paints the decoration at one end of an edge.
///
/// `tip` is the end point; `from` is the neighbouring point, which is what
/// gives the decoration its direction.
fn marker(
    ctx: &mut DrawingContext,
    marker: EdgeMarker,
    tip: Point,
    from: Point,
    palette: &Palette,
) {
    if marker == EdgeMarker::None {
        return;
    }
    let (dx, dy) = (tip.x - from.x, tip.y - from.y);
    let length = dx.hypot(dy);
    if length <= f32::EPSILON {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);

    match marker {
        EdgeMarker::Arrow => {
            let back = Point::new(
                (-ux).mul_add(ARROW_LENGTH, tip.x),
                (-uy).mul_add(ARROW_LENGTH, tip.y),
            );
            let mut head = Path::new();
            head.move_to(tip);
            head.line_to(Point::new(
                (-uy).mul_add(ARROW_HALF_WIDTH, back.x),
                ux.mul_add(ARROW_HALF_WIDTH, back.y),
            ));
            head.line_to(Point::new(
                uy.mul_add(ARROW_HALF_WIDTH, back.x),
                (-ux).mul_add(ARROW_HALF_WIDTH, back.y),
            ));
            head.close();
            ctx.set_fill_style(palette.edge);
            ctx.fill_path(&head);
        }
        EdgeMarker::Circle => {
            ctx.set_stroke_style(palette.edge);
            ctx.set_line_width(STROKE);
            ctx.stroke_circle(tip, MARKER_RADIUS);
        }
        EdgeMarker::Cross => {
            ctx.set_stroke_style(palette.edge);
            ctx.set_line_width(STROKE);
            ctx.stroke_line(
                Point::new(tip.x - CROSS_ARM, tip.y - CROSS_ARM),
                Point::new(tip.x + CROSS_ARM, tip.y + CROSS_ARM),
            );
            ctx.stroke_line(
                Point::new(tip.x - CROSS_ARM, tip.y + CROSS_ARM),
                Point::new(tip.x + CROSS_ARM, tip.y - CROSS_ARM),
            );
        }
        EdgeMarker::None => {}
    }
}

#[cfg(test)]
mod tests {
    use waterui_core::layout::{Point, Rect, Size};

    use waterui_text::FontCollection;

    use super::{CORNER_RADIUS, edge_path};
    use crate::engine::render;
    use crate::layout::DiagramLayout;
    use crate::shape;

    /// A flowchart whose edges turn: `TD` stacks the nodes, and `D --> A` runs
    /// back up past them, so every edge is a routed polyline with corners in it
    /// rather than one straight segment.
    const TURNING: &str = "\
flowchart TD
    A[Start] --> B{Ready?}
    B -->|yes| C([Go])
    B -->|no| D[(Wait)]
    D --> A
";

    /// How far a piece of geometry may sit outside where it belongs before the
    /// diagram is wrong rather than rounded.
    const SLACK: f32 = 0.5;

    fn diagram() -> DiagramLayout {
        render(TURNING, FontCollection::system()).expect("the flowchart lays out")
    }

    /// The box every point of `points` lies in.
    fn hull(points: &[Point]) -> Rect {
        let (mut min, mut max) = (points[0], points[0]);
        for point in points {
            min = Point::new(min.x.min(point.x), min.y.min(point.y));
            max = Point::new(max.x.max(point.x), max.y.max(point.y));
        }
        Rect::new(min, Size::new(max.x - min.x, max.y - min.y))
    }

    /// How far `inner` sticks out of `outer`, in the worst direction.
    fn escape(inner: Rect, outer: Rect) -> f32 {
        (outer.x() - inner.x())
            .max(inner.max_x() - outer.max_x())
            .max(outer.y() - inner.y())
            .max(inner.max_y() - outer.max_y())
            .max(0.0)
    }

    fn canvas(diagram: &DiagramLayout) -> Rect {
        Rect::new(Point::zero(), diagram.size)
    }

    /// Nothing this crate paints for a node may leave the diagram's own bounds:
    /// the scene is exactly `diagram.size` and anything outside it is clipped
    /// away by the container.
    #[core::prelude::v1::test]
    fn every_node_outline_stays_on_the_canvas() {
        let diagram = diagram();
        let canvas = canvas(&diagram);
        for node in &diagram.nodes {
            let outline = shape::outline(node.shape, node.frame);
            for path in outline.body.iter().chain(&outline.details) {
                let bounds = path.bounding_box().expect("a node outline draws something");
                assert!(
                    escape(bounds, canvas) <= SLACK,
                    "node `{}` draws {bounds:?}, outside the {canvas:?} canvas",
                    node.id
                );
            }
        }
    }

    /// The routed path of an edge is its polyline with the corners rounded, so
    /// it can never travel further than the polyline itself does — a fillet
    /// lives inside the corner it rounds.
    ///
    /// This is what a broken corner-rounding primitive breaks: the tangent
    /// distance comes out inverted, the arc's centre lands on the wrong side,
    /// and the path shoots off along the last segment's direction far past the
    /// node it was heading for. `D --> A` here left the diagram entirely.
    #[core::prelude::v1::test]
    fn every_edge_path_follows_its_own_polyline() {
        let diagram = diagram();
        let canvas = canvas(&diagram);
        for edge in &diagram.edges {
            let path = edge_path(&edge.points).expect("a routed edge draws something");
            let bounds = path.bounding_box().expect("a routed edge draws something");
            let corridor = hull(&edge.points);
            assert!(
                escape(bounds, corridor) <= CORNER_RADIUS,
                "edge `{}` draws {bounds:?}, off the {corridor:?} its points span",
                edge.id
            );
            assert!(
                escape(bounds, canvas) <= SLACK,
                "edge `{}` draws {bounds:?}, outside the {canvas:?} canvas",
                edge.id
            );
        }
    }

    /// An edge starts and ends on a node. Mermaid trims the polyline to the
    /// node's border, so each end lands on the box of the node it belongs to —
    /// a diamond's slanted face is inside its box, which is why this is stated
    /// as the box rather than its outline.
    #[core::prelude::v1::test]
    fn every_edge_ends_on_a_node() {
        let diagram = diagram();
        for edge in &diagram.edges {
            for (which, end) in [("start", edge.points.first()), ("end", edge.points.last())] {
                let end = *end.expect("a routed edge has endpoints");
                let touched = diagram
                    .nodes
                    .iter()
                    .find(|node| escape(Rect::new(end, Size::new(0.0, 0.0)), node.frame) <= SLACK);
                assert!(
                    touched.is_some(),
                    "the {which} of edge `{}` is at {end:?}, on no node",
                    edge.id
                );
            }
        }
    }
}
