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
        NodeShape::RoundedRectangle => Outline::plain(rounded_rect(frame, CORNER_RADIUS)),
        NodeShape::Stadium => Outline::plain(rounded_rect(frame, frame.height() / 2.0)),
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

/// A rectangle whose corners are rounded by `radius`, clamped so a radius
/// larger than half the shorter side degenerates into a stadium rather than
/// self-intersecting.
fn rounded_rect(frame: Rect, radius: f32) -> Path {
    let radius = radius.min(frame.width() / 2.0).min(frame.height() / 2.0);
    let (x, y) = (frame.x(), frame.y());
    let (right, bottom) = (frame.max_x(), frame.max_y());

    let mut path = Path::new();
    path.move_to(Point::new(x + radius, y));
    path.line_to(Point::new(right - radius, y));
    path.arc_to(Point::new(right, y), Point::new(right, y + radius), radius);
    path.line_to(Point::new(right, bottom - radius));
    path.arc_to(
        Point::new(right, bottom),
        Point::new(right - radius, bottom),
        radius,
    );
    path.line_to(Point::new(x + radius, bottom));
    path.arc_to(
        Point::new(x, bottom),
        Point::new(x, bottom - radius),
        radius,
    );
    path.line_to(Point::new(x, y + radius));
    path.arc_to(Point::new(x, y), Point::new(x + radius, y), radius);
    path.close();
    path
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
