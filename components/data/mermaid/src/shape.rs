//! Node outlines.
//!
//! Every outline is built from the node's laid-out frame, so the shape that
//! gets drawn is exactly the box Mermaid reserved for it. The proportions that
//! are not derivable from the frame — a stadium's end radius, a cylinder's cap
//! depth, a hexagon's slant — follow Mermaid's own renderer, because the box was
//! sized on that assumption and a different slant would put the label outside
//! its shape.

use alloc::vec;
use alloc::vec::Vec;

use waterui_canvas::Path;
use waterui_core::layout::{Point, Rect, Size};

use crate::layout::NodeShape;

/// Corner radius of a rounded rectangle, as Mermaid draws it.
const CORNER_RADIUS: f32 = 5.0;
/// Inset of a subroutine's inner vertical rules from each end.
const SUBROUTINE_INSET: f32 = 8.0;
/// Horizontal slant of a hexagon and the parallelograms, as a fraction of the
/// frame's height.
const SLANT_RATIO: f32 = 0.5;
/// A cylinder's cap height, as a fraction of the frame's height.
const CYLINDER_CAP_RATIO: f32 = 0.12;
/// The gap between the two rings of a double circle.
const DOUBLE_CIRCLE_GAP: f32 = 5.0;
/// How deep the notch of an asymmetric node cuts in, relative to its height.
const ASYMMETRIC_NOTCH_RATIO: f32 = 0.35;
/// The share of an actor's box its stick figure occupies, leaving the rest for
/// the participant's name written underneath.
const ACTOR_FIGURE_SHARE: f32 = 0.62;
/// Where a quarter-circle's Bezier control points sit along the tangents, as a
/// fraction of the radius: `4/3 * (sqrt(2) - 1)`, the classic approximation,
/// whose worst-case error is under a fifty-thousandth of the radius.
const ARC_KAPPA: f32 = 0.552_284_8;

/// A node's drawable outline.
pub struct Outline {
    /// The shape to fill and then stroke. `None` for a node that has no
    /// outline of its own, such as a bare text node.
    pub body: Option<Path>,
    /// Strokes drawn over the filled body.
    pub details: Vec<Path>,
}

impl Outline {
    /// An outline that is filled and stroked, with nothing drawn over it.
    const fn plain(body: Path) -> Self {
        Self {
            body: Some(body),
            details: Vec::new(),
        }
    }
}

/// A closed outline, held as the segments it is made of before it is lowered
/// into a [`Path`].
///
/// A `Path` is write-only — it takes drawing commands and hands the renderer a
/// `kurbo` path, and nothing can read back what it was given. Keeping the
/// geometry here first is what lets the outlines be checked rather than only
/// looked at: a contour states where it starts, every segment states where it
/// ends, and each one starts where the last one ended by construction, so an
/// outline made of pieces that do not join cannot be expressed.
struct Contour {
    /// Where the outline starts, and where its last segment returns to.
    start: Point,
    /// The segments, each continuing from the end of the one before it.
    segments: Vec<Segment>,
}

/// One segment of a [`Contour`], starting wherever the previous one ended.
#[derive(Clone, Copy)]
enum Segment {
    /// A straight line to this point.
    Line(Point),
    /// A cubic Bezier through two control points to this end point.
    Cubic {
        /// The control point governing the departure from the previous point.
        first: Point,
        /// The control point governing the arrival at `end`.
        second: Point,
        /// Where the curve ends.
        end: Point,
    },
}

impl Segment {
    /// Where this segment ends, which is where the next one starts.
    const fn end(self) -> Point {
        match self {
            Self::Line(end) | Self::Cubic { end, .. } => end,
        }
    }
}

impl Contour {
    /// An outline that starts at `start` and has nothing drawn yet.
    const fn new(start: Point) -> Self {
        Self {
            start,
            segments: Vec::new(),
        }
    }

    /// Where the outline currently ends, which is where the next segment
    /// starts.
    fn end(&self) -> Point {
        self.segments
            .last()
            .map_or(self.start, |segment| segment.end())
    }

    /// Extends the outline with a straight line to `point`.
    fn line_to(&mut self, point: Point) {
        self.segments.push(Segment::Line(point));
    }

    /// Extends the outline with a quarter-circle to `point`, turning around the
    /// sharp corner `pivot` that this edge and the last one would have met at.
    ///
    /// Both ends sit a radius away from `pivot` along their own edge, so the
    /// control points are that same fraction of the way in along the tangents.
    fn corner_to(&mut self, point: Point, pivot: Point) {
        let from = self.end();
        self.segments.push(Segment::Cubic {
            first: toward(from, pivot, ARC_KAPPA),
            second: toward(point, pivot, ARC_KAPPA),
            end: point,
        });
    }

    /// Lowers the contour into one closed sub-path.
    fn to_path(&self) -> Path {
        let mut path = Path::new();
        path.move_to(self.start);
        for segment in &self.segments {
            match *segment {
                Segment::Line(end) => path.line_to(end),
                Segment::Cubic { first, second, end } => path.bezier_to(first, second, end),
            }
        }
        path.close();
        path
    }
}

/// The point `fraction` of the way from `from` to `to`.
fn toward(from: Point, to: Point, fraction: f32) -> Point {
    Point::new(
        (to.x - from.x).mul_add(fraction, from.x),
        (to.y - from.y).mul_add(fraction, from.y),
    )
}

/// Where a shape leaves room for its label.
///
/// Most shapes centre their label in their whole box. An actor does not: its box
/// holds a stick figure with the participant's name written underneath, so
/// centring would write the name across the figure's chest.
#[must_use]
pub fn label_area(shape: NodeShape, frame: Rect) -> Rect {
    if shape == NodeShape::Actor {
        let figure = frame.height() * ACTOR_FIGURE_SHARE;
        Rect::new(
            Point::new(frame.x(), frame.y() + figure),
            Size::new(frame.width(), frame.height() - figure),
        )
    } else {
        frame
    }
}

/// Builds the outline of one node.
///
/// Returns the outline to fill and stroke, plus any inner strokes the shape
/// draws on top of its own fill — a subroutine's two rules, a cylinder's rim,
/// the inner ring of a double circle.
#[must_use]
pub fn outline(shape: NodeShape, frame: Rect) -> Outline {
    match shape {
        NodeShape::Rectangle | NodeShape::Participant | NodeShape::Note => {
            let mut path = Path::new();
            path.rect(frame);
            Outline::plain(path)
        }
        NodeShape::RoundedRectangle => Outline::plain(rounded_rect(frame, CORNER_RADIUS).to_path()),
        NodeShape::Stadium => Outline::plain(stadium(frame).to_path()),
        NodeShape::Subroutine => subroutine(frame),
        NodeShape::Cylinder => cylinder(frame),
        NodeShape::Circle | NodeShape::DoubleCircle => {
            round(frame, shape == NodeShape::DoubleCircle)
        }
        NodeShape::Asymmetric
        | NodeShape::Diamond
        | NodeShape::Hexagon
        | NodeShape::ParallelogramRight
        | NodeShape::ParallelogramLeft
        | NodeShape::Trapezoid
        | NodeShape::TrapezoidInverted => Outline::plain(polygon(&angular(shape, frame))),
        NodeShape::Text => Outline {
            body: None,
            details: Vec::new(),
        },
        NodeShape::Actor => actor(frame),
    }
}

/// `A[[text]]` — a rectangle with a vertical rule inset from each end.
fn subroutine(frame: Rect) -> Outline {
    let mut path = Path::new();
    path.rect(frame);

    let details = [SUBROUTINE_INSET, frame.width() - SUBROUTINE_INSET]
        .into_iter()
        .map(|inset| {
            let mut rule = Path::new();
            rule.move_to(Point::new(frame.x() + inset, frame.y()));
            rule.line_to(Point::new(frame.x() + inset, frame.max_y()));
            rule
        })
        .collect();

    Outline {
        body: Some(path),
        details,
    }
}

/// `A[(text)]` — a database cylinder, drawn as a body with a visible top rim.
fn cylinder(frame: Rect) -> Outline {
    let (x, y) = (frame.x(), frame.y());
    let (right, bottom) = (frame.max_x(), frame.max_y());
    let cap = frame.height() * CYLINDER_CAP_RATIO;

    let mut path = Path::new();
    path.move_to(Point::new(x, y + cap));
    path.line_to(Point::new(x, bottom - cap));
    path.bezier_to(
        Point::new(x, bottom),
        Point::new(right, bottom),
        Point::new(right, bottom - cap),
    );
    path.line_to(Point::new(right, y + cap));
    path.bezier_to(
        Point::new(right, y),
        Point::new(x, y),
        Point::new(x, y + cap),
    );
    path.close();

    // The rim is the far edge of the top cap, drawn over the fill.
    let far_edge = cap.mul_add(2.0, y);
    let mut rim = Path::new();
    rim.move_to(Point::new(x, y + cap));
    rim.bezier_to(
        Point::new(x, far_edge),
        Point::new(right, far_edge),
        Point::new(right, y + cap),
    );

    Outline {
        body: Some(path),
        details: vec![rim],
    }
}

/// `A((text))` and `A(((text)))`.
fn round(frame: Rect, doubled: bool) -> Outline {
    let radius = frame.width().min(frame.height()) / 2.0;
    let mut path = Path::new();
    circle(&mut path, frame.center(), radius);

    let details = if doubled {
        let mut inner = Path::new();
        circle(&mut inner, frame.center(), radius - DOUBLE_CIRCLE_GAP);
        vec![inner]
    } else {
        Vec::new()
    };

    Outline {
        body: Some(path),
        details,
    }
}

/// The corners of the straight-edged shapes, each a closed polygon over the
/// frame.
fn angular(shape: NodeShape, frame: Rect) -> Vec<Point> {
    let (x, y) = (frame.x(), frame.y());
    let (right, bottom) = (frame.max_x(), frame.max_y());
    let slant = frame.height() * SLANT_RATIO;
    let (mid_x, mid_y) = (frame.mid_x(), frame.mid_y());

    match shape {
        NodeShape::Asymmetric => {
            let notch = frame.height() * ASYMMETRIC_NOTCH_RATIO;
            vec![
                Point::new(x, y),
                Point::new(right, y),
                Point::new(right, bottom),
                Point::new(x, bottom),
                Point::new(x + notch, mid_y),
            ]
        }
        NodeShape::Diamond => vec![
            Point::new(mid_x, y),
            Point::new(right, mid_y),
            Point::new(mid_x, bottom),
            Point::new(x, mid_y),
        ],
        NodeShape::Hexagon => vec![
            Point::new(x + slant, y),
            Point::new(right - slant, y),
            Point::new(right, mid_y),
            Point::new(right - slant, bottom),
            Point::new(x + slant, bottom),
            Point::new(x, mid_y),
        ],
        NodeShape::ParallelogramRight => vec![
            Point::new(x + slant, y),
            Point::new(right, y),
            Point::new(right - slant, bottom),
            Point::new(x, bottom),
        ],
        NodeShape::ParallelogramLeft => vec![
            Point::new(x, y),
            Point::new(right - slant, y),
            Point::new(right, bottom),
            Point::new(x + slant, bottom),
        ],
        NodeShape::Trapezoid => vec![
            Point::new(x + slant, y),
            Point::new(right - slant, y),
            Point::new(right, bottom),
            Point::new(x, bottom),
        ],
        NodeShape::TrapezoidInverted => vec![
            Point::new(x, y),
            Point::new(right, y),
            Point::new(right - slant, bottom),
            Point::new(x + slant, bottom),
        ],
        other => unreachable!("{other:?} is not a straight-edged shape"),
    }
}

/// A closed path through `points`.
fn polygon(points: &[Point]) -> Path {
    let mut path = Path::new();
    let Some((first, rest)) = points.split_first() else {
        return path;
    };
    path.move_to(*first);
    for point in rest {
        path.line_to(*point);
    }
    path.close();
    path
}

/// A full circle of `radius` centred on `centre`.
fn circle(path: &mut Path, centre: Point, radius: f32) {
    path.ellipse(
        centre,
        Size::new(radius, radius),
        0.0,
        0.0,
        core::f32::consts::TAU,
        false,
    );
}

/// `A([text])` — a rectangle whose left and right ends are semicircles of
/// radius half the height.
///
/// It is [`rounded_rect`] at the largest radius that does not self-intersect,
/// which is what makes each end meet itself in the middle of its short side.
fn stadium(frame: Rect) -> Contour {
    rounded_rect(frame, frame.height() / 2.0)
}

/// A rectangle whose corners are rounded by `radius`, clamped so a radius
/// larger than half the shorter side degenerates into a stadium rather than
/// self-intersecting. A stadium is this shape with `radius` at half the
/// height, so the two ends meet in the middle and become semicircles.
///
/// The corners are cubic Beziers appended to the one running contour rather
/// than `Path::arc_to` tangent arcs. `arc_to` would be the obvious spelling and
/// it is the wrong one twice over: it starts a fresh sub-path at each arc, so
/// the outline arrives at the renderer in five disjoint pieces that no `close`
/// can join, and it places the tangent arc's centre outside the corner it is
/// turning, so each arc lands away from the edges it was meant to join.
fn rounded_rect(frame: Rect, radius: f32) -> Contour {
    let radius = radius.min(frame.width() / 2.0).min(frame.height() / 2.0);
    let (x, y) = (frame.x(), frame.y());
    let (right, bottom) = (frame.max_x(), frame.max_y());

    // The outline runs clockwise from the top edge, and every corner is
    // entered and left a radius away from the frame's own corner.
    let mut contour = Contour::new(Point::new(x + radius, y));
    contour.line_to(Point::new(right - radius, y));
    contour.corner_to(Point::new(right, y + radius), Point::new(right, y));
    contour.line_to(Point::new(right, bottom - radius));
    contour.corner_to(
        Point::new(right - radius, bottom),
        Point::new(right, bottom),
    );
    contour.line_to(Point::new(x + radius, bottom));
    contour.corner_to(Point::new(x, bottom - radius), Point::new(x, bottom));
    contour.line_to(Point::new(x, y + radius));
    contour.corner_to(Point::new(x + radius, y), Point::new(x, y));
    contour
}

/// The stick figure a sequence diagram draws for an `actor`.
///
/// The figure takes the upper part of the box; [`label_area`] hands the rest to
/// the participant's name, which is how Mermaid draws it.
fn actor(frame: Rect) -> Outline {
    let (x, y) = (frame.x(), frame.y());
    let (w, h) = (frame.width(), frame.height() * ACTOR_FIGURE_SHARE);
    let head_radius = (w.min(h) * 0.18).max(1.0);
    let centre_x = frame.mid_x();
    let shoulders = head_radius.mul_add(2.0, y);
    let hips = h.mul_add(0.62, y);
    let arms = (hips - shoulders).mul_add(0.3, shoulders);

    let mut head = Path::new();
    circle(
        &mut head,
        Point::new(centre_x, y + head_radius),
        head_radius,
    );

    let mut body = Path::new();
    body.move_to(Point::new(centre_x, shoulders));
    body.line_to(Point::new(centre_x, hips));
    body.move_to(Point::new(w.mul_add(0.2, x), arms));
    body.line_to(Point::new(w.mul_add(0.8, x), arms));
    body.move_to(Point::new(centre_x, hips));
    body.line_to(Point::new(w.mul_add(0.25, x), y + h));
    body.move_to(Point::new(centre_x, hips));
    body.line_to(Point::new(w.mul_add(0.75, x), y + h));

    Outline {
        body: Some(head),
        details: vec![body],
    }
}

#[cfg(test)]
mod tests {
    use super::{ARC_KAPPA, Contour, Point, Rect, Segment, Size, stadium};

    /// How far apart two points may be and still be the same point. The
    /// geometry is `f32` arithmetic over the frame, so the property under test
    /// is where the outline runs, never an exact bit pattern.
    const TOLERANCE: f32 = 1e-4;

    /// A stadium's frame as flowchart layout hands one over: wider than it is
    /// tall, so its two semicircular ends are joined by a straight run.
    const WIDE: Rect = Rect::new(Point::new(7.0, 274.0), Size::new(42.0, 34.0));

    /// The same shape for a label short enough that the box is narrower than
    /// its own two ends put together, which is what `C([Go])` produces.
    const NARROW: Rect = Rect::new(Point::new(7.0, 274.0), Size::new(24.0, 34.0));

    fn near(left: Point, right: Point) -> bool {
        (left.x - right.x).abs() <= TOLERANCE && (left.y - right.y).abs() <= TOLERANCE
    }

    /// Every point the outline is built from: where it starts, where each
    /// segment ends, and the control points that steer the curves.
    ///
    /// A cubic never leaves the hull of its own four points, so these bound the
    /// drawn outline from the outside while every segment end is a point the
    /// outline actually passes through.
    fn control_net(contour: &Contour) -> Vec<Point> {
        let mut points = vec![contour.start];
        for segment in &contour.segments {
            match *segment {
                Segment::Line(end) => points.push(end),
                Segment::Cubic { first, second, end } => points.extend([first, second, end]),
            }
        }
        points
    }

    fn bounds(contour: &Contour) -> Rect {
        let points = control_net(contour);
        let (mut min, mut max) = (points[0], points[0]);
        for point in &points[1..] {
            min = Point::new(min.x.min(point.x), min.y.min(point.y));
            max = Point::new(max.x.max(point.x), max.y.max(point.y));
        }
        Rect::new(min, Size::new(max.x - min.x, max.y - min.y))
    }

    fn passes_through(contour: &Contour, point: Point) -> bool {
        core::iter::once(contour.start)
            .chain(contour.segments.iter().map(|segment| segment.end()))
            .any(|end| near(end, point))
    }

    /// The outline has to come back to where it started, or it is not an
    /// outline at all — it is a run of strokes with the shape open at one end.
    #[test]
    fn a_stadium_closes() {
        for frame in [WIDE, NARROW] {
            let contour = stadium(frame);
            assert!(
                near(contour.end(), contour.start),
                "the outline of {frame:?} ends at {:?} instead of its start {:?}",
                contour.end(),
                contour.start
            );
        }
    }

    /// A stadium fills the box layout reserved for it: it touches all four
    /// sides of its frame and never strays outside it. That is what says the
    /// ends turn the right way — an outline whose arcs are centred outside the
    /// corners they turn bulges past every side of the frame instead.
    #[test]
    fn a_stadium_fills_its_frame_without_leaving_it() {
        for frame in [WIDE, NARROW] {
            let drawn = bounds(&stadium(frame));
            assert!(
                near(drawn.origin(), frame.origin())
                    && near(
                        Point::new(drawn.max_x(), drawn.max_y()),
                        Point::new(frame.max_x(), frame.max_y())
                    ),
                "the outline of {frame:?} spans {drawn:?}"
            );
        }
    }

    /// Every straight edge stops a radius short of the corner and the end that
    /// follows picks it up exactly there, so those eight meeting points are
    /// where the outline has to pass. A stadium's radius is half its height,
    /// so a box wide enough for both ends spends what is left over on the
    /// straight top and bottom and closes each end into a semicircle bulging
    /// to the middle of its short side; a narrower box keeps its ends whole
    /// and loses the straight run instead.
    #[test]
    fn a_stadiums_edges_meet_its_ends_a_radius_from_each_corner() {
        for frame in [WIDE, NARROW] {
            let contour = stadium(frame);
            let radius = (frame.height() / 2.0).min(frame.width() / 2.0);
            for expected in [
                Point::new(frame.x() + radius, frame.y()),
                Point::new(frame.max_x() - radius, frame.y()),
                Point::new(frame.max_x(), frame.y() + radius),
                Point::new(frame.max_x(), frame.max_y() - radius),
                Point::new(frame.max_x() - radius, frame.max_y()),
                Point::new(frame.x() + radius, frame.max_y()),
                Point::new(frame.x(), frame.max_y() - radius),
                Point::new(frame.x(), frame.y() + radius),
            ] {
                assert!(
                    passes_through(&contour, expected),
                    "the outline of {frame:?} misses {expected:?}"
                );
            }
        }

        let wide = stadium(WIDE);
        assert!(
            passes_through(&wide, Point::new(WIDE.max_x(), WIDE.mid_y()))
                && passes_through(&wide, Point::new(WIDE.x(), WIDE.mid_y())),
            "a box wider than its own ends should bulge to the middle of each short side"
        );
    }

    /// A quarter-circle drawn as a cubic passes closest to the true arc at its
    /// midpoint, where the approximation is at its worst. If the control points
    /// ever drift from the classic fraction of the way along each tangent, the
    /// ends stop being round and this is where it shows.
    #[test]
    fn a_stadiums_ends_are_round_between_their_endpoints() {
        let frame = WIDE;
        let contour = stadium(frame);
        let radius = frame.height() / 2.0;
        let centre = Point::new(frame.max_x() - radius, frame.mid_y());

        let Some(&Segment::Cubic { first, second, end }) = contour.segments.get(1) else {
            panic!("the segment after the top edge is the first end's upper quarter");
        };
        let start = Point::new(frame.max_x() - radius, frame.y());
        let midpoint = Point::new(
            (start.x + 3.0f32.mul_add(first.x + second.x, end.x)) / 8.0,
            (start.y + 3.0f32.mul_add(first.y + second.y, end.y)) / 8.0,
        );
        let error = (midpoint.x - centre.x).hypot(midpoint.y - centre.y) - radius;
        assert!(
            error.abs() < radius * 1e-3,
            "the end curves {error} away from a circle of radius {radius}, \
             which means ARC_KAPPA ({ARC_KAPPA}) is not the quarter-circle fraction"
        );
    }
}
