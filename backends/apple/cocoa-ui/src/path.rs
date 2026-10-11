//! `CGPath` construction in point space.
//!
//! [`MutablePath`] wraps `CGMutablePath` for building paths segment by
//! segment; [`shape_path`] resolves a structured shape — a kind plus a unit
//! space command list — to a `CGPath` in a given rect, and [`border_path`]
//! strokes a rect edge by edge, inset by half the stroke width with rounded
//! corners.
//!
//! # Safety
//!
//! The `unsafe` here calls Core Graphics's path functions on retained paths
//! the caller keeps alive, with `transform` arguments pointing at stack
//! values that outlive each call — the documented contract of every
//! `CGPathAdd*` function.

use objc2_core_foundation::CFRetained;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGAffineTransformMakeTranslation, CGAffineTransformScale, CGMutablePath, CGPath,
};

use crate::geometry::{Point, Rect};

/// One drawing operation in unit space — coordinates `0.0…1.0` relative to
/// the rect the path resolves against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Move to `(x, y)` without drawing.
    MoveTo {
        /// Normalized x.
        x: f64,
        /// Normalized y.
        y: f64,
    },
    /// A straight line to `(x, y)`.
    LineTo {
        /// Normalized x.
        x: f64,
        /// Normalized y.
        y: f64,
    },
    /// A quadratic Bézier to `(x, y)` through `control`.
    QuadTo {
        /// Control point x.
        cx: f64,
        /// Control point y.
        cy: f64,
        /// End x.
        x: f64,
        /// End y.
        y: f64,
    },
    /// A cubic Bézier to `(x, y)` through `c1` and `c2`.
    CubicTo {
        /// First control point x.
        c1x: f64,
        /// First control point y.
        c1y: f64,
        /// Second control point x.
        c2x: f64,
        /// Second control point y.
        c2y: f64,
        /// End x.
        x: f64,
        /// End y.
        y: f64,
    },
    /// An elliptical arc of `start..start+sweep` radians centered at
    /// `(cx, cy)` with radii `(rx, ry)`; `sweep < 0` draws clockwise.
    Arc {
        /// Center x.
        cx: f64,
        /// Center y.
        cy: f64,
        /// Radius along x.
        rx: f64,
        /// Radius along y.
        ry: f64,
        /// Start angle in radians.
        start: f64,
        /// Sweep angle in radians.
        sweep: f64,
    },
    /// Close the current subpath.
    Close,
}

/// What a shape is, independent of the commands that approximate it.
///
/// The normalized radii of [`ShapeKind::RoundedRect`] and
/// [`ShapeKind::UnevenRoundedRect`] are fractions of the shorter side; the
/// `Fixed*` variants carry absolute points. Both clamp to half the shorter
/// side at resolve time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeKind {
    /// A rectangle with sharp corners.
    Rect,
    /// A circle centered in the rect, diameter the shorter side.
    Circle,
    /// An ellipse filling the rect.
    Ellipse,
    /// A rectangle with a uniform normalized corner radius.
    RoundedRect {
        /// Corner radius as a fraction of the shorter side (`0.0…0.5`).
        corner_radius: f64,
    },
    /// A rectangle with per-corner normalized radii.
    UnevenRoundedRect {
        /// Top-left corner.
        top_left: f64,
        /// Top-right corner.
        top_right: f64,
        /// Bottom-right corner.
        bottom_right: f64,
        /// Bottom-left corner.
        bottom_left: f64,
    },
    /// A pill — corner radius half the shorter side.
    Capsule,
    /// A rectangle with a uniform corner radius in points.
    FixedRoundedRect {
        /// Corner radius in points.
        corner_radius: f64,
    },
    /// A rectangle with per-corner radii in points.
    FixedUnevenRoundedRect {
        /// Top-left corner.
        top_left: f64,
        /// Top-right corner.
        top_right: f64,
        /// Bottom-right corner.
        bottom_right: f64,
        /// Bottom-left corner.
        bottom_left: f64,
    },
    /// Only the command list describes the shape.
    CustomPath,
}

/// Which of a rect's four edges a border stroke covers.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeMask {
    /// The top edge.
    pub top: bool,
    /// The leading edge — left in left-to-right layout.
    pub leading: bool,
    /// The bottom edge.
    pub bottom: bool,
    /// The trailing edge — right in left-to-right layout.
    pub trailing: bool,
}

impl EdgeMask {
    /// Every edge.
    pub const ALL: Self = Self {
        top: true,
        leading: true,
        bottom: true,
        trailing: true,
    };

    /// Whether all four edges are covered — when they are, a layer's
    /// `borderWidth` draws the same stroke and no path is needed.
    #[must_use]
    pub const fn is_all(self) -> bool {
        self.top && self.leading && self.bottom && self.trailing
    }
}

/// A `CGMutablePath` the caller mutates through methods.
#[derive(Debug)]
pub struct MutablePath(CFRetained<CGMutablePath>);

impl Default for MutablePath {
    fn default() -> Self {
        Self::new()
    }
}

impl MutablePath {
    /// An empty mutable path.
    #[must_use]
    pub fn new() -> Self {
        Self(CGMutablePath::new())
    }

    /// The path as an immutable `CGPath` — for installing on a layer.
    #[must_use]
    pub fn path(&self) -> &CGPath {
        &self.0
    }

    /// The same path retained as its immutable superclass.
    #[must_use]
    pub fn immutable(&self) -> CFRetained<CGPath> {
        CFRetained::from(&*self.0)
    }

    /// Start a subpath at `to`.
    pub fn move_to(&self, to: Point) {
        // SAFETY: `transform` is null, `path` live; see module safety note.
        unsafe { CGMutablePath::move_to_point(Some(&self.0), core::ptr::null(), to.x, to.y) }
    }

    /// A straight segment to `to`.
    pub fn line_to(&self, to: Point) {
        // SAFETY: see `move_to`.
        unsafe { CGMutablePath::add_line_to_point(Some(&self.0), core::ptr::null(), to.x, to.y) }
    }

    /// A quadratic curve to `to` through `control`.
    pub fn quad_to(&self, control: Point, to: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_quad_curve_to_point(
                Some(&self.0),
                core::ptr::null(),
                control.x,
                control.y,
                to.x,
                to.y,
            );
        }
    }

    /// A cubic curve to `to` through `c1` and `c2`.
    pub fn cubic_to(&self, c1: Point, c2: Point, to: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_curve_to_point(
                Some(&self.0),
                core::ptr::null(),
                c1.x,
                c1.y,
                c2.x,
                c2.y,
                to.x,
                to.y,
            );
        }
    }

    /// An arc centered at `center` — `start..end` radians, `clockwise`
    /// picking the winding direction.
    pub fn arc(&self, center: Point, radius: f64, start: f64, end: f64, clockwise: bool) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_arc(
                Some(&self.0),
                core::ptr::null(),
                center.x,
                center.y,
                radius,
                start,
                end,
                clockwise,
            );
        }
    }

    /// An arc of `radius` tangent to the line to `t1` and the line `t1..t2`.
    pub fn arc_to_tangent(&self, t1: Point, t2: Point, radius: f64) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_arc_to_point(
                Some(&self.0),
                core::ptr::null(),
                t1.x,
                t1.y,
                t2.x,
                t2.y,
                radius,
            );
        }
    }

    /// An ellipse inscribed in `rect`.
    pub fn ellipse_in(&self, rect: Rect) {
        // SAFETY: see `move_to`.
        unsafe { CGMutablePath::add_ellipse_in_rect(Some(&self.0), core::ptr::null(), rect.into()) }
    }

    /// `rect` as a closed subpath.
    pub fn rect(&self, rect: Rect) {
        // SAFETY: see `move_to`.
        unsafe { CGMutablePath::add_rect(Some(&self.0), core::ptr::null(), rect.into()) }
    }

    /// `rect` with rounded corners of `radius` in both axes.
    pub fn rounded_rect(&self, rect: Rect, radius: f64) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_rounded_rect(
                Some(&self.0),
                core::ptr::null(),
                rect.into(),
                radius,
                radius,
            );
        }
    }

    /// Close the current subpath.
    pub fn close(&self) {
        CGMutablePath::close_subpath(Some(&self.0));
    }
}

/// A `CGPath` for `kind` resolved against `rect`.
///
/// The kind carries what unit-space commands cannot: a normalized corner
/// radius resolves against the rect's shorter side so a rounded corner
/// stays circular in points. Only [`ShapeKind::CustomPath`] falls back to
/// `commands`.
#[must_use]
pub fn shape_path(kind: ShapeKind, commands: &[Command], rect: Rect) -> CFRetained<CGPath> {
    let shorter = rect.size.width.min(rect.size.height);
    match kind {
        // SAFETY: `transform` is null; see the module safety note.
        ShapeKind::Rect => unsafe { CGPath::with_rect(rect.into(), core::ptr::null()) },
        // SAFETY: see `ShapeKind::Rect`.
        ShapeKind::Circle => unsafe {
            CGPath::with_ellipse_in_rect(
                Rect::new(
                    (rect.origin.x + rect.size.width / 2.0) - shorter / 2.0,
                    (rect.origin.y + rect.size.height / 2.0) - shorter / 2.0,
                    shorter,
                    shorter,
                )
                .into(),
                core::ptr::null(),
            )
        },
        // SAFETY: see `ShapeKind::Rect`.
        ShapeKind::Ellipse => unsafe {
            CGPath::with_ellipse_in_rect(rect.into(), core::ptr::null())
        },
        ShapeKind::RoundedRect { corner_radius } => {
            let radius = (corner_radius * shorter).min(shorter / 2.0);
            // SAFETY: see `ShapeKind::Rect`.
            unsafe { CGPath::with_rounded_rect(rect.into(), radius, radius, core::ptr::null()) }
        }
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } => uneven_rounded_rect(
            [top_left, top_right, bottom_right, bottom_left],
            shorter,
            rect,
        ),
        ShapeKind::Capsule => {
            let radius = shorter / 2.0;
            // SAFETY: see `ShapeKind::Rect`.
            unsafe { CGPath::with_rounded_rect(rect.into(), radius, radius, core::ptr::null()) }
        }
        ShapeKind::CustomPath => commands_path(commands, rect),
        ShapeKind::FixedRoundedRect { corner_radius } => {
            let radius = corner_radius.min(shorter / 2.0);
            // SAFETY: see `ShapeKind::Rect`.
            unsafe { CGPath::with_rounded_rect(rect.into(), radius, radius, core::ptr::null()) }
        }
        ShapeKind::FixedUnevenRoundedRect {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } => uneven_rounded_rect([top_left, top_right, bottom_right, bottom_left], 1.0, rect),
    }
}

/// The `[tl, tr, br, bl]` radii scaled by `scale` and clamped to half the
/// shorter side, walked counterclockwise with tangent arcs.
fn uneven_rounded_rect(radii: [f64; 4], scale: f64, rect: Rect) -> CFRetained<CGPath> {
    let [x, y, w, h] = [
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    ];
    let limit = w.min(h) / 2.0;
    let tl = (radii[0] * scale).min(limit);
    let tr = (radii[1] * scale).min(limit);
    let br = (radii[2] * scale).min(limit);
    let bl = (radii[3] * scale).min(limit);

    let path = MutablePath::new();
    path.move_to(Point::new(x + tl, y));
    path.arc_to_tangent(Point::new(x + w, y), Point::new(x + w, y + h), tr);
    path.arc_to_tangent(Point::new(x + w, y + h), Point::new(x, y + h), br);
    path.arc_to_tangent(Point::new(x, y + h), Point::new(x, y), bl);
    path.arc_to_tangent(Point::new(x, y), Point::new(x + w, y), tl);
    path.close();
    path.immutable()
}

/// Unit-space `commands` drawn into `rect` — x scales with width, y with
/// height.
#[must_use]
pub fn commands_path(commands: &[Command], rect: Rect) -> CFRetained<CGPath> {
    let path = MutablePath::new();
    let (w, h) = (rect.size.width, rect.size.height);
    let denormalize = |x: f64, y: f64| Point::new(x * w, y * h);

    for command in commands {
        match *command {
            Command::MoveTo { x, y } => path.move_to(denormalize(x, y)),
            Command::LineTo { x, y } => path.line_to(denormalize(x, y)),
            Command::QuadTo { cx, cy, x, y } => {
                path.quad_to(denormalize(cx, cy), denormalize(x, y));
            }
            Command::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => path.cubic_to(
                denormalize(c1x, c1y),
                denormalize(c2x, c2y),
                denormalize(x, y),
            ),
            Command::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => add_arc(&path, cx * w, cy * h, rx * w, ry * h, start, sweep),
            Command::Close => path.close(),
        }
    }
    path.immutable()
}

/// One elliptical arc command: `CGPathAddArc` on a unit circle under a
/// scale-and-translate transform, an ellipse when the sweep covers a full
/// turn.
fn add_arc(path: &MutablePath, cx: f64, cy: f64, rx: f64, ry: f64, start: f64, sweep: f64) {
    const FULL_TURN_EPSILON: f64 = 0.0001;
    if sweep.abs() >= core::f64::consts::TAU - FULL_TURN_EPSILON {
        path.ellipse_in(Rect::new(cx - rx, cy - ry, rx * 2.0, ry * 2.0));
        return;
    }

    let transform = CGAffineTransformScale(CGAffineTransformMakeTranslation(cx, cy), rx, ry);
    // SAFETY: `transform` points at a stack value that outlives the call.
    unsafe {
        CGMutablePath::add_arc(
            Some(&path.0),
            &raw const transform,
            0.0,
            0.0,
            1.0,
            start,
            start + sweep,
            sweep < 0.0,
        );
    }
}

/// A stroke path tracing `edges` of `rect`, inset by `width / 2` and rounded
/// by `corner_radius` shrunk by the same half-width (capped at half the
/// inset's shorter side).
///
/// This is the path a `CAShapeLayer` strokes when a border covers fewer
/// than all four edges; when [`EdgeMask::is_all`] holds a layer's
/// `borderWidth`/`cornerRadius` draws the same thing.
#[must_use]
pub fn border_path(
    rect: Rect,
    width: f64,
    corner_radius: f64,
    edges: EdgeMask,
) -> CFRetained<CGPath> {
    let path = MutablePath::new();
    let half = width / 2.0;
    let rect = Rect::new(
        rect.origin.x + half,
        rect.origin.y + half,
        rect.size.width - width,
        rect.size.height - width,
    );
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return path.immutable();
    }

    let (min_x, min_y) = (rect.origin.x, rect.origin.y);
    let (max_x, max_y) = (min_x + rect.size.width, min_y + rect.size.height);
    let radius = (corner_radius - half)
        .max(0.0)
        .min(rect.size.width.min(rect.size.height) / 2.0);

    if edges.top {
        path.move_to(Point::new(min_x + radius, min_y));
        path.line_to(Point::new(max_x - radius, min_y));
        if radius > 0.0 {
            path.arc(
                Point::new(max_x - radius, min_y + radius),
                radius,
                -core::f64::consts::FRAC_PI_2,
                0.0,
                false,
            );
        }
    }
    if edges.trailing {
        path.move_to(Point::new(max_x, min_y + radius));
        path.line_to(Point::new(max_x, max_y - radius));
        if radius > 0.0 {
            path.arc(
                Point::new(max_x - radius, max_y - radius),
                radius,
                0.0,
                core::f64::consts::FRAC_PI_2,
                false,
            );
        }
    }
    if edges.bottom {
        path.move_to(Point::new(max_x - radius, max_y));
        path.line_to(Point::new(min_x + radius, max_y));
        if radius > 0.0 {
            path.arc(
                Point::new(min_x + radius, max_y - radius),
                radius,
                core::f64::consts::FRAC_PI_2,
                core::f64::consts::PI,
                false,
            );
        }
    }
    if edges.leading {
        path.move_to(Point::new(min_x, max_y - radius));
        path.line_to(Point::new(min_x, min_y + radius));
        if radius > 0.0 {
            path.arc(
                Point::new(min_x + radius, min_y + radius),
                radius,
                core::f64::consts::PI,
                core::f64::consts::PI * 1.5,
                false,
            );
        }
    }
    path.immutable()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds_of(path: &CGPath) -> Rect {
        CGPath::bounding_box(Some(path)).into()
    }

    #[test]
    fn rect_shape_fills_its_rect() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        let path = shape_path(ShapeKind::Rect, &[], rect);
        assert_eq!(bounds_of(&path), rect);
    }

    #[test]
    fn circle_is_the_centered_inscribed_square() {
        let rect = Rect::new(0.0, 0.0, 200.0, 80.0);
        let path = shape_path(ShapeKind::Circle, &[], rect);
        let bounds = bounds_of(&path);
        assert!((bounds.size.width - 80.0).abs() < 1.0);
        assert!((bounds.size.height - 80.0).abs() < 1.0);
        assert!((bounds.origin.x - 60.0).abs() < 1.0);
        assert!(bounds.origin.y.abs() < 1.0);
    }

    #[test]
    fn uneven_rounded_rect_covers_the_rect() {
        let rect = Rect::new(0.0, 0.0, 120.0, 60.0);
        let path = shape_path(
            ShapeKind::UnevenRoundedRect {
                top_left: 0.1,
                top_right: 0.3,
                bottom_right: 0.0,
                bottom_left: 0.5,
            },
            &[],
            rect,
        );
        assert!(!CGPath::is_empty(Some(&*path)));
        let bounds = bounds_of(&path);
        assert!(bounds.origin.x.abs() < 0.001);
        assert!(bounds.origin.y.abs() < 0.001);
        assert!((bounds.size.width - 120.0).abs() < 0.001);
        assert!((bounds.size.height - 60.0).abs() < 0.001);
    }

    #[test]
    fn border_path_is_inset_by_half_the_width() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let path = border_path(rect, 4.0, 8.0, EdgeMask::ALL);
        let bounds = bounds_of(&path);
        assert!((bounds.origin.x - 2.0).abs() < 0.5);
        assert!((bounds.origin.y - 2.0).abs() < 0.5);
        assert!((bounds.size.width - 96.0).abs() < 1.0);
        assert!((bounds.size.height - 96.0).abs() < 1.0);
    }

    #[test]
    fn border_path_with_one_edge_stays_on_that_edge() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let path = border_path(
            rect,
            4.0,
            0.0,
            EdgeMask {
                top: true,
                leading: false,
                bottom: false,
                trailing: false,
            },
        );
        let bounds = bounds_of(&path);
        assert!((bounds.origin.y - 2.0).abs() < 0.001);
        assert!(bounds.size.height.abs() < 0.001);
        assert!((bounds.size.width - 96.0).abs() < 0.001);
    }

    #[test]
    fn command_path_scales_unit_space_to_the_rect() {
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        let path = commands_path(
            &[
                Command::MoveTo { x: 0.0, y: 0.0 },
                Command::LineTo { x: 1.0, y: 1.0 },
            ],
            rect,
        );
        let bounds = bounds_of(&path);
        assert!((bounds.size.width - 200.0).abs() < 0.001);
        assert!((bounds.size.height - 100.0).abs() < 0.001);
    }
}

/// A mutable Core Graphics path being assembled.
#[derive(Debug)]
pub struct PathBuilder {
    path: CFRetained<CGMutablePath>,
}

impl PathBuilder {
    /// An empty path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            path: CGMutablePath::new(),
        }
    }

    /// Starts a new subpath at `point`.
    pub fn move_to(&mut self, point: Point) {
        // SAFETY: `path` is a live mutable path; the transform is null.
        unsafe {
            CGMutablePath::move_to_point(Some(&self.path), std::ptr::null(), point.x, point.y);
        }
    }

    /// Extends the subpath with a straight line to `point`.
    pub fn line_to(&mut self, point: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_line_to_point(Some(&self.path), std::ptr::null(), point.x, point.y);
        }
    }

    /// A quadratic curve to `end` through `control`.
    pub fn quad_to(&mut self, control: Point, end: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_quad_curve_to_point(
                Some(&self.path),
                std::ptr::null(),
                control.x,
                control.y,
                end.x,
                end.y,
            );
        }
    }

    /// A cubic curve to `end` through `c1` and `c2`.
    pub fn cubic_to(&mut self, c1: Point, c2: Point, end: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_curve_to_point(
                Some(&self.path),
                std::ptr::null(),
                c1.x,
                c1.y,
                c2.x,
                c2.y,
                end.x,
                end.y,
            );
        }
    }

    /// The full rect `rect`.
    pub fn rect(&mut self, rect: Rect) {
        let rect = CGRect::new(
            CGPoint::new(rect.origin.x, rect.origin.y),
            CGSize::new(rect.size.width, rect.size.height),
        );
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_rect(Some(&self.path), std::ptr::null(), rect);
        }
    }

    /// The ellipse inscribed in `rect`.
    pub fn ellipse_in_rect(&mut self, rect: Rect) {
        let rect = CGRect::new(
            CGPoint::new(rect.origin.x, rect.origin.y),
            CGSize::new(rect.size.width, rect.size.height),
        );
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_ellipse_in_rect(Some(&self.path), std::ptr::null(), rect);
        }
    }

    /// A rect whose corners are quarter-ellipses of `radius`.
    pub fn rounded_rect(&mut self, rect: Rect, radius: f64) {
        let rect = CGRect::new(
            CGPoint::new(rect.origin.x, rect.origin.y),
            CGSize::new(rect.size.width, rect.size.height),
        );
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_rounded_rect(
                Some(&self.path),
                std::ptr::null(),
                rect,
                radius,
                radius,
            );
        }
    }

    /// A rect with per-corner radii, drawn as tangent arcs — the corners are
    /// clamped to at most half the shorter side by the caller's radii.
    ///
    /// - `radii`: `(top_left, top_right, bottom_right, bottom_left)`.
    pub fn uneven_rounded_rect(&mut self, rect: Rect, radii: (f64, f64, f64, f64)) {
        let (tl, tr, br, bl) = radii;
        let (min_x, min_y) = (rect.origin.x, rect.origin.y);
        let (max_x, max_y) = (min_x + rect.size.width, min_y + rect.size.height);
        self.move_to(Point::new(min_x + tl, min_y));
        self.arc_to_point(Point::new(max_x, min_y), Point::new(max_x, max_y), tr);
        self.arc_to_point(Point::new(max_x, max_y), Point::new(min_x, max_y), br);
        self.arc_to_point(Point::new(min_x, max_y), Point::new(min_x, min_y), bl);
        self.arc_to_point(Point::new(min_x, min_y), Point::new(max_x, min_y), tl);
        self.close();
    }

    /// An arc from the current point: the tangent-line arc of `radius`
    /// through `tangent_end` heading toward `tangent_end_2`.
    pub fn arc_to_point(&mut self, tangent_end: Point, tangent_end_2: Point, radius: f64) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_arc_to_point(
                Some(&self.path),
                std::ptr::null(),
                tangent_end.x,
                tangent_end.y,
                tangent_end_2.x,
                tangent_end_2.y,
                radius,
            );
        }
    }

    /// An elliptical arc from `center` with radii `(rx, ry)` from `start`
    /// through `sweep` radians; `sweep` near a full turn degenerates to the
    /// whole ellipse.
    pub fn arc(&mut self, center: Point, rx: f64, ry: f64, start: f64, sweep: f64) {
        const NEARLY_FULL: f64 = std::f64::consts::TAU - 0.0001;
        if sweep.abs() >= NEARLY_FULL {
            self.ellipse_in_rect(Rect::new(center.x - rx, center.y - ry, rx * 2.0, ry * 2.0));
            return;
        }
        // The arc commands are circular; the ellipse is a translate+scale of
        // the unit circle about `center`.
        let transform = CGAffineTransformMakeTranslation(center.x, center.y);
        let transform = CGAffineTransformScale(transform, rx, ry);
        // SAFETY: `path` is live; `transform` points to a valid affine.
        unsafe {
            CGMutablePath::add_arc(
                Some(&self.path),
                &raw const transform,
                0.0,
                0.0,
                1.0,
                start,
                start + sweep,
                sweep < 0.0,
            );
        }
    }

    /// Closes the current subpath.
    pub fn close(&mut self) {
        CGMutablePath::close_subpath(Some(&self.path));
    }

    /// The finished immutable path.
    #[must_use]
    /// # Panics
    ///
    /// When copying the live path fails — impossible for a valid path.
    pub fn build(&self) -> CFRetained<CGPath> {
        // SAFETY: copies a live path.
        let copy =
            CGMutablePath::new_copy(Some(&self.path)).expect("copying a valid path cannot fail");
        // SAFETY: an immutable `CGPath` and its mutable copy are the same
        // CoreGraphics object after this point — nothing mutates it.
        unsafe { CFRetained::from_raw(CFRetained::into_raw(copy).cast::<CGPath>()) }
    }
}

impl Default for PathBuilder {
    fn default() -> Self {
        Self::new()
    }
}
