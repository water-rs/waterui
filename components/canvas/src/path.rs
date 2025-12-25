//! Path builder for HTML5 Canvas-style path construction.
//!
//! This module provides a path builder that uses WaterUI's native types
//! (Point, Size, Rect) while wrapping kurbo's BezPath for rendering.

use waterui_core::layout::{Point, Rect, Size};

// Internal imports for rendering (not exposed to users)
use vello::kurbo::{self, Shape};

use super::conversions::{point_to_kurbo, rect_to_kurbo};

/// Path builder for constructing complex shapes.
///
/// This provides an HTML5 Canvas-style API for building paths with
/// `move_to`, `line_to`, bezier curves, arcs, etc.
///
/// # Example
///
/// ```ignore
/// let mut path = Path::new();
/// path.move_to(Point::new(10.0, 10.0));
/// path.line_to(Point::new(100.0, 10.0));
/// path.line_to(Point::new(100.0, 100.0));
/// path.close();
/// ctx.fill_path(&path);
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

    /// Draws an arc between two points with a given radius.
    ///
    /// This is equivalent to HTML5 Canvas `arcTo()`.
    ///
    /// # Arguments
    /// * `point1` - First control point
    /// * `point2` - Second control point
    /// * `radius` - Radius of the arc
    pub fn arc_to(&mut self, point1: Point, point2: Point, radius: f32) {
        // Get current point
        let current = self.inner.elements().last().and_then(|el| match el {
            kurbo::PathEl::MoveTo(p)
            | kurbo::PathEl::LineTo(p)
            | kurbo::PathEl::CurveTo(_, _, p)
            | kurbo::PathEl::QuadTo(_, p) => Some(*p),
            kurbo::PathEl::ClosePath => None,
        });

        if let Some(current_pt) = current {
            let p0 = current_pt;
            let p1 = point_to_kurbo(point1);
            let p2 = point_to_kurbo(point2);
            let r = f64::from(radius);

            // Calculate tangent arc between two lines
            let v0 = kurbo::Vec2::new(p1.x - p0.x, p1.y - p0.y);
            let v1 = kurbo::Vec2::new(p2.x - p1.x, p2.y - p1.y);

            let len0 = v0.hypot();
            let len1 = v1.hypot();

            if len0 > 0.0 && len1 > 0.0 {
                let v0_norm = v0 / len0;
                let v1_norm = v1 / len1;

                // Angle between vectors
                let cos_angle = v0_norm.dot(v1_norm).clamp(-1.0, 1.0);
                let angle = cos_angle.acos();

                if angle > 0.01 {
                    // Not parallel
                    let tan_half = (angle / 2.0).tan();
                    let dist = r / tan_half;

                    let start_pt = p1 - v0_norm * dist;
                    let end_pt = p1 + v1_norm * dist;

                    // Draw line to arc start
                    self.inner.line_to(start_pt);

                    // Calculate arc center and angles
                    let bisector = (v0_norm + v1_norm).normalize();
                    let center_dist = r / (angle / 2.0).sin();
                    let center = p1 + bisector * center_dist;

                    let start_angle = (start_pt.y - center.y).atan2(start_pt.x - center.x);
                    let end_angle = (end_pt.y - center.y).atan2(end_pt.x - center.x);
                    let mut sweep = end_angle - start_angle;

                    // Normalize sweep to be in correct direction
                    if sweep > core::f64::consts::PI {
                        sweep -= 2.0 * core::f64::consts::PI;
                    } else if sweep < -core::f64::consts::PI {
                        sweep += 2.0 * core::f64::consts::PI;
                    }

                    let arc = kurbo::Arc::new(center, (r, r), start_angle, sweep, 0.0);
                    let arc_path = arc.to_path(0.1);
                    for el in arc_path.elements() {
                        self.inner.push(*el);
                    }
                }
            }
        }
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

impl core::fmt::Debug for Path {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Path")
            .field("elements", &self.inner.elements().len())
            .finish()
    }
}
