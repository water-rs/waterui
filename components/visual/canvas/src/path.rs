//! Path builder for HTML5 Canvas-style path construction.
//!
//! This module provides a path builder that uses WaterUI's native types
//! (Point, Size, Rect) while wrapping kurbo's BezPath for rendering.

use core::fmt;
use waterui_core::layout::{Point, Rect, Size};

// Internal imports for rendering (not exposed to users)
use kurbo::{self, Shape};

use super::conversions::{point_to_kurbo, rect_to_kurbo};

/// Path builder for constructing complex shapes.
///
/// This provides an HTML5 Canvas-style API for building paths with
/// `move_to`, `line_to`, bezier curves, arcs, etc.
///
/// # Example
///
/// ```rust
/// # use waterui::prelude::*;
/// # use waterui_canvas::{DrawingContext, Path};
/// # fn draw(ctx: &mut DrawingContext<'_>) {
/// let mut path = Path::new();
/// path.move_to(Point::new(10.0, 10.0));
/// path.line_to(Point::new(100.0, 10.0));
/// path.line_to(Point::new(100.0, 100.0));
/// path.close();
/// ctx.fill_path(&path);
/// # }
/// ```
pub struct Path {
    inner: kurbo::BezPath,
}

impl Path {
    /// Creates a new empty path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: kurbo::BezPath::new(),
        }
    }

    /// Moves the current point to the specified position without drawing.
    ///
    /// This starts a new sub-path at the given point.
    pub fn move_to(&mut self, point: Point) {
        self.inner.move_to(point_to_kurbo(point));
    }

    /// Draws a straight line from the current point to the specified point.
    pub fn line_to(&mut self, point: Point) {
        self.inner.line_to(point_to_kurbo(point));
    }

    /// Draws a quadratic Bezier curve from the current point to `end` using `control_point`.
    ///
    /// # Arguments
    /// * `control_point` - The control point for the curve
    /// * `end` - The end point of the curve
    pub fn quadratic_to(&mut self, control_point: Point, end: Point) {
        self.inner
            .quad_to(point_to_kurbo(control_point), point_to_kurbo(end));
    }

    /// Draws a cubic Bezier curve from the current point to `end`.
    ///
    /// # Arguments
    /// * `control_point1` - The first control point
    /// * `control_point2` - The second control point
    /// * `end` - The end point of the curve
    pub fn bezier_to(&mut self, control_point1: Point, control_point2: Point, end: Point) {
        self.inner.curve_to(
            point_to_kurbo(control_point1),
            point_to_kurbo(control_point2),
            point_to_kurbo(end),
        );
    }

    /// Draws a circular arc.
    ///
    /// # Arguments
    /// * `center` - Center point of the arc
    /// * `radius` - Radius of the arc
    /// * `start_angle` - Starting angle in radians (0 = 3 o'clock)
    /// * `end_angle` - Ending angle in radians
    /// * `anticlockwise` - If true, draws counter-clockwise; otherwise clockwise
    pub fn arc(
        &mut self,
        center: Point,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        anticlockwise: bool,
    ) {
        let center_kurbo = point_to_kurbo(center);
        let radius_f64 = f64::from(radius);

        // Adjust angles based on direction
        let (start, sweep) = if anticlockwise {
            let sweep = f64::from(start_angle - end_angle);
            (f64::from(start_angle), sweep)
        } else {
            let sweep = f64::from(end_angle - start_angle);
            (f64::from(start_angle), sweep)
        };

        // Create arc using kurbo's Arc
        let arc = kurbo::Arc::new(center_kurbo, (radius_f64, radius_f64), start, sweep, 0.0);

        // Convert arc to bezier path segments and append to path
        let bez_path = arc.to_path(0.1);
        for el in bez_path.elements() {
            self.inner.push(*el);
        }
    }

    /// Draws a circular arc tangent to the two lines `current -> point1` and
    /// `point1 -> point2`, joined to the current point by a straight line.
    ///
    /// This is HTML5 Canvas `arcTo()`: `point1` is the corner being rounded and
    /// `point2` only supplies the outgoing direction — the path never reaches
    /// it. When the three points are collinear, or `radius` is zero, or the
    /// corner coincides with a neighbour, the arc degenerates and a straight
    /// line is drawn to `point1`, as the platform does.
    ///
    /// `radius` is clamped so the fillet fits inside both segments; a radius
    /// larger than the corner can accommodate rounds it as much as it can
    /// rather than overshooting into the neighbouring corner.
    ///
    /// Does nothing if the path has no current point.
    ///
    /// # Arguments
    /// * `point1` - The corner to round
    /// * `point2` - The point the outgoing edge heads towards
    /// * `radius` - Radius of the fillet
    pub fn arc_to(&mut self, point1: Point, point2: Point, radius: f32) {
        let Some(current) = self.current_point() else {
            return;
        };

        let corner = point_to_kurbo(point1);
        let p0 = current;
        let p2 = point_to_kurbo(point2);

        // Both vectors point away from the corner: that is the frame a fillet
        // is defined in, and using the travel directions instead is what makes
        // the angle come out as the supplement of the one that is wanted.
        let back = p0 - corner;
        let forward = p2 - corner;
        let (back_len, forward_len) = (back.hypot(), forward.hypot());

        let degenerate = radius <= 0.0 || back_len == 0.0 || forward_len == 0.0;
        if degenerate {
            self.inner.line_to(corner);
            return;
        }

        let back = back / back_len;
        let forward = forward / forward_len;
        let interior = back.dot(forward).clamp(-1.0, 1.0).acos();
        // Collinear in either sense: nothing to round.
        if !interior.is_finite() || interior < 1e-4 || (core::f64::consts::PI - interior) < 1e-4 {
            self.inner.line_to(corner);
            return;
        }

        let half = interior / 2.0;
        // Distance from the corner to each tangent point, clamped so the fillet
        // cannot eat past either neighbour, with the radius reduced to match.
        let tangent = (f64::from(radius) / half.tan())
            .min(back_len)
            .min(forward_len);
        let effective_radius = tangent * half.tan();

        let start = corner + back * tangent;
        let end = corner + forward * tangent;
        let center = corner + (back + forward).normalize() * (effective_radius / half.sin());

        self.inner.line_to(start);

        let start_angle = (start - center).atan2();
        let end_angle = (end - center).atan2();
        let mut sweep = end_angle - start_angle;
        if sweep > core::f64::consts::PI {
            sweep -= core::f64::consts::TAU;
        } else if sweep < -core::f64::consts::PI {
            sweep += core::f64::consts::TAU;
        }

        let arc = kurbo::Arc::new(
            center,
            (effective_radius, effective_radius),
            start_angle,
            sweep,
            0.0,
        );
        // `Arc::to_path` opens with a `MoveTo` to the arc's start. Appending it
        // would lift the pen and start a new contour, breaking the outline into
        // fragments and leaving `close` to close the wrong one.
        for element in arc.to_path(0.1).elements().iter().skip(1) {
            self.inner.push(*element);
        }
    }

    /// The point the pen is currently at, if the path has been started.
    fn current_point(&self) -> Option<kurbo::Point> {
        self.inner
            .elements()
            .last()
            .and_then(|element| match element {
                kurbo::PathEl::MoveTo(point)
                | kurbo::PathEl::LineTo(point)
                | kurbo::PathEl::CurveTo(_, _, point)
                | kurbo::PathEl::QuadTo(_, point) => Some(*point),
                kurbo::PathEl::ClosePath => None,
            })
    }

    /// Draws an elliptical arc.
    ///
    /// # Arguments
    /// * `center` - Center point of the ellipse
    /// * `radii` - Radii of the ellipse (width, height)
    /// * `rotation` - Rotation of the ellipse in radians
    /// * `start_angle` - Starting angle in radians
    /// * `end_angle` - Ending angle in radians
    /// * `anticlockwise` - If true, draws counter-clockwise
    pub fn ellipse(
        &mut self,
        center: Point,
        radii: Size,
        rotation: f32,
        start_angle: f32,
        end_angle: f32,
        anticlockwise: bool,
    ) {
        let center_kurbo = point_to_kurbo(center);
        let radii_tuple = (f64::from(radii.width), f64::from(radii.height));

        let (start, sweep) = if anticlockwise {
            let sweep = f64::from(start_angle - end_angle);
            (f64::from(start_angle), sweep)
        } else {
            let sweep = f64::from(end_angle - start_angle);
            (f64::from(start_angle), sweep)
        };

        let arc = kurbo::Arc::new(center_kurbo, radii_tuple, start, sweep, f64::from(rotation));

        // Convert arc to path and append
        let arc_path = arc.to_path(0.1);
        for el in arc_path.elements() {
            self.inner.push(*el);
        }
    }

    /// Adds a rectangle sub-path.
    ///
    /// This is a convenience method that adds a closed rectangular path.
    pub fn rect(&mut self, rect: Rect) {
        let kurbo_rect = rect_to_kurbo(rect);

        let x = kurbo_rect.x0;
        let y = kurbo_rect.y0;
        let width = kurbo_rect.width();
        let height = kurbo_rect.height();

        self.inner.move_to((x, y));
        self.inner.line_to((x + width, y));
        self.inner.line_to((x + width, y + height));
        self.inner.line_to((x, y + height));
        self.inner.close_path();
    }

    /// Closes the current sub-path by drawing a straight line back to the start.
    pub fn close(&mut self) {
        self.inner.close_path();
    }

    /// Returns a reference to the inner `kurbo::BezPath`.
    ///
    /// This is used internally by the canvas renderer.
    #[must_use]
    pub(crate) const fn inner(&self) -> &kurbo::BezPath {
        &self.inner
    }
}

impl Default for Path {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Path")
            .field("elements", &self.inner.elements().len())
            .finish()
    }
}

#[cfg(test)]
mod arc_to_tests {
    use super::{Path, Point};

    /// Every element after the first `MoveTo` must continue the same contour.
    /// An interior `MoveTo` lifts the pen, which is what turns a rounded outline
    /// into loose fragments and leaves `close` closing the wrong one.
    fn interior_move_tos(path: &Path) -> usize {
        path.inner()
            .elements()
            .iter()
            .skip(1)
            .filter(|element| matches!(element, kurbo::PathEl::MoveTo(_)))
            .count()
    }

    /// How far the path strays from the box its three points span. A fillet
    /// lives inside the corner, so it can never leave that box.
    fn overshoot(path: &Path, min: Point, max: Point) -> f64 {
        path.inner()
            .elements()
            .iter()
            .filter_map(|element| match element {
                kurbo::PathEl::MoveTo(p)
                | kurbo::PathEl::LineTo(p)
                | kurbo::PathEl::CurveTo(_, _, p)
                | kurbo::PathEl::QuadTo(_, p) => Some(*p),
                kurbo::PathEl::ClosePath => None,
            })
            .map(|p| {
                let dx = (f64::from(min.x) - p.x)
                    .max(p.x - f64::from(max.x))
                    .max(0.0);
                let dy = (f64::from(min.y) - p.y)
                    .max(p.y - f64::from(max.y))
                    .max(0.0);
                dx.max(dy)
            })
            .fold(0.0_f64, f64::max)
    }

    #[test]
    fn a_rounded_corner_stays_one_contour() {
        let mut path = Path::new();
        path.move_to(Point::new(0.0, 0.0));
        path.arc_to(Point::new(100.0, 0.0), Point::new(100.0, 100.0), 20.0);
        assert_eq!(interior_move_tos(&path), 0);
    }

    /// The bug this replaced put the arc centre 90 degrees off and computed the
    /// tangent distance from the supplement of the interior angle, so the path
    /// wandered outside the corner it was meant to round.
    #[test]
    fn a_right_angle_fillet_stays_inside_its_corner() {
        let mut path = Path::new();
        path.move_to(Point::new(0.0, 0.0));
        path.arc_to(Point::new(100.0, 0.0), Point::new(100.0, 100.0), 20.0);
        assert!(
            overshoot(&path, Point::new(0.0, 0.0), Point::new(100.0, 100.0)) < 0.5,
            "the fillet left the box its own points span: {path:?}"
        );
    }

    /// A nearly straight turn rounds by almost nothing. Computing the angle the
    /// wrong way round inverted this: the shallower the turn, the further the
    /// path shot away from it.
    #[test]
    fn a_shallow_turn_rounds_by_a_little_not_a_lot() {
        let mut path = Path::new();
        path.move_to(Point::new(0.0, 0.0));
        path.arc_to(Point::new(100.0, 0.0), Point::new(200.0, 10.0), 8.0);
        assert!(
            overshoot(&path, Point::new(0.0, -1.0), Point::new(200.0, 11.0)) < 0.5,
            "a 6-degree turn threw the path off the polyline it follows: {path:?}"
        );
    }

    #[test]
    fn collinear_points_draw_a_straight_line_to_the_corner() {
        let mut path = Path::new();
        path.move_to(Point::new(0.0, 0.0));
        path.arc_to(Point::new(50.0, 0.0), Point::new(100.0, 0.0), 10.0);
        let last = path.inner().elements().last().copied();
        assert!(
            matches!(last, Some(kurbo::PathEl::LineTo(p)) if (p.x - 50.0).abs() < 1e-6),
            "collinear input should degenerate to a line to the corner, got {last:?}"
        );
    }

    #[test]
    fn a_zero_radius_draws_a_straight_line_to_the_corner() {
        let mut path = Path::new();
        path.move_to(Point::new(0.0, 0.0));
        path.arc_to(Point::new(50.0, 0.0), Point::new(50.0, 50.0), 0.0);
        let last = path.inner().elements().last().copied();
        assert!(matches!(last, Some(kurbo::PathEl::LineTo(_))), "{last:?}");
    }

    /// A radius larger than either segment is clamped rather than allowed to
    /// overshoot into the neighbouring corner.
    #[test]
    fn an_oversized_radius_is_clamped_to_the_segments() {
        let mut path = Path::new();
        path.move_to(Point::new(0.0, 0.0));
        path.arc_to(Point::new(10.0, 0.0), Point::new(10.0, 10.0), 1000.0);
        assert!(
            overshoot(&path, Point::new(0.0, 0.0), Point::new(10.0, 10.0)) < 0.5,
            "an oversized radius escaped its corner: {path:?}"
        );
    }

    #[test]
    fn arc_to_without_a_current_point_does_nothing() {
        let mut path = Path::new();
        path.arc_to(Point::new(10.0, 0.0), Point::new(10.0, 10.0), 5.0);
        assert_eq!(path.inner().elements().len(), 0);
    }
}
