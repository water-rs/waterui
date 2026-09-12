//! Resolving a custom [`PathCommand`] list against the size it is drawn at.
//!
//! [`crate::shape_geometry`] answers for every shape that has a
//! [`ShapeKind`](waterui_shape::ShapeKind); a custom path is the one kind it
//! cannot describe, and this module is where its unit-space commands become
//! geometry in points. Both the fill and the clip of a custom-path shape go
//! through here, so the two agree exactly — including on the arcs, which are
//! converted once, to cubic Béziers, rather than traced differently by each.
//!
//! This module is platform independent on purpose, like `shape_geometry`: it
//! is plain geometry, so it compiles and is tested on every host even though
//! the rest of the backend is Linux only.

use kurbo::{Arc, BezPath, PathEl, Point, Shape};
use waterui_shape::PathCommand;

/// Tolerance for approximating an arc with cubic Béziers, in points: well
/// below one device pixel at any plausible scale factor, so the approximation
/// is invisible.
const ARC_TOLERANCE: f64 = 0.05;

/// Resolves unit-space `commands` against a `width` by `height` rect.
///
/// An arc joins the path the way every path builder does: a line from the
/// current point to the arc's start when there is one, and a move there when
/// the arc opens the path — a [`Circle`](waterui_shape::Circle) is a single arc
/// with no `MoveTo` in front of it.
#[must_use]
pub fn bez_path(commands: &[PathCommand], width: f64, height: f64) -> BezPath {
    let point = |x: f32, y: f32| Point::new(f64::from(x) * width, f64::from(y) * height);
    let mut path = BezPath::new();
    for command in commands {
        match *command {
            PathCommand::MoveTo { x, y } => path.move_to(point(x, y)),
            PathCommand::LineTo { x, y } => path.line_to(point(x, y)),
            PathCommand::QuadTo { cx, cy, x, y } => path.quad_to(point(cx, cy), point(x, y)),
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => path.curve_to(point(c1x, c1y), point(c2x, c2y), point(x, y)),
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                let radii = (f64::from(rx) * width, f64::from(ry) * height);
                let arc = Arc::new(
                    point(cx, cy),
                    radii,
                    f64::from(start),
                    f64::from(sweep),
                    0.0,
                );
                append_arc(&mut path, &arc);
            }
            PathCommand::Close => path.close_path(),
        }
    }
    path
}

fn append_arc(path: &mut BezPath, arc: &Arc) {
    // `Shape::path_elements`, not `Arc::append_iter`: only the former leads with
    // a `MoveTo` to the arc's start, and the latter's first element is the first
    // cubic, which a `MoveTo` match would silently discard.
    let mut segments = arc.path_elements(ARC_TOLERANCE);
    // A path already under construction wants a line to the start instead of
    // the move, and an empty one wants the move.
    if let Some(PathEl::MoveTo(entry)) = segments.next() {
        if path.elements().is_empty() {
            path.move_to(entry);
        } else {
            path.line_to(entry);
        }
    }
    path.extend(segments);
}

#[cfg(test)]
mod tests {
    use core::f32::consts::{FRAC_PI_2, PI, TAU};

    use kurbo::{PathEl, Point, Shape};
    use waterui_shape::PathCommand;

    use super::bez_path;

    fn close_to(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-3
    }

    #[test]
    fn commands_scale_per_axis() {
        let path = bez_path(
            &[
                PathCommand::MoveTo { x: 0.0, y: 0.0 },
                PathCommand::LineTo { x: 1.0, y: 0.5 },
                PathCommand::Close,
            ],
            200.0,
            50.0,
        );
        assert_eq!(
            path.elements(),
            [
                PathEl::MoveTo(Point::new(0.0, 0.0)),
                PathEl::LineTo(Point::new(200.0, 25.0)),
                PathEl::ClosePath,
            ]
        );
    }

    #[test]
    fn an_arc_opening_the_path_moves_to_its_start() {
        let path = bez_path(
            &[PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: 0.0,
                sweep: TAU,
            }],
            100.0,
            100.0,
        );
        assert!(matches!(
            path.elements().first(),
            Some(PathEl::MoveTo(entry)) if close_to(*entry, Point::new(100.0, 50.0))
        ));
        assert!(
            path.elements()
                .iter()
                .skip(1)
                .all(|element| matches!(element, PathEl::CurveTo(..))),
            "a full circle is cubic segments and nothing else after the move"
        );
        // The area of a circle of radius 50, which a polyline approximation of
        // the arc would visibly undershoot.
        let expected = core::f64::consts::PI * 50.0 * 50.0;
        assert!((path.area().abs() - expected).abs() / expected < 1e-3);
    }

    #[test]
    fn an_arc_continuing_the_path_lines_to_its_start() {
        // The capsule outline: a move, two half-turn arcs, a close.
        let path = bez_path(
            &[
                PathCommand::MoveTo { x: 0.5, y: 0.0 },
                PathCommand::Arc {
                    cx: 0.5,
                    cy: 0.5,
                    rx: 0.5,
                    ry: 0.5,
                    start: -FRAC_PI_2,
                    sweep: PI,
                },
                PathCommand::Arc {
                    cx: 0.5,
                    cy: 0.5,
                    rx: 0.5,
                    ry: 0.5,
                    start: FRAC_PI_2,
                    sweep: PI,
                },
                PathCommand::Close,
            ],
            100.0,
            100.0,
        );
        let mut moves = 0;
        let mut lines = 0;
        for element in path.elements() {
            match element {
                PathEl::MoveTo(_) => moves += 1,
                PathEl::LineTo(_) => lines += 1,
                _ => {}
            }
        }
        assert_eq!(
            moves, 1,
            "the arcs continue the path rather than restarting it"
        );
        assert_eq!(
            lines, 2,
            "each arc joins from the current point with a line"
        );
    }
}
