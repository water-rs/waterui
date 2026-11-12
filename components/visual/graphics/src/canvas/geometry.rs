//! The geometry a canvas draws: shapes in canvas space.
//!
//! Every shape is in the canvas's own coordinates — logical points, origin at
//! the top-left of its box, y growing downward — and every one lowers to the
//! same path geometry, so a shape used for a fill, a stroke or a clip is the
//! same outline, and [`Path::contains`] answers hit tests over exactly what
//! was drawn.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::f32::consts::TAU;

use nami::impl_constant;
use waterui_core::contract_violation;
use waterui_core::layout::{Point, Rect, Size};

use crate::draw::kurbo::{self, PathEl, Shape as _};
use crate::draw::{FillRule as RecordFillRule, ShapeData};

/// Something a canvas can fill, stroke, shadow or clip to.
///
/// Implemented by the layout [`Rect`] and by the canvas shapes in this
/// module; it is sealed, because every shape must lower to the one path
/// geometry every render target draws.
pub trait Shape: sealed::Lower + Clone + 'static {}

impl<T: sealed::Lower + Clone + 'static> Shape for T {}

pub mod sealed {
    use crate::draw::ShapeData;

    /// Lowers a canvas shape to the recorded shape data.
    pub trait Lower {
        /// The recorded form of this shape.
        fn lower(self) -> ShapeData;
    }
}

/// `size` when both sides are finite and positive. Otherwise a contract
/// violation of `what` (a drawing, a picture): each side that is not finite
/// and positive is one point.
pub fn positive_extent(size: Size, what: &str) -> Size {
    let positive = |side: f32| {
        if side.is_finite() && side > 0.0 {
            side
        } else {
            1.0
        }
    };
    if size.width.is_finite() && size.height.is_finite() && size.width > 0.0 && size.height > 0.0 {
        return size;
    }
    contract_violation!(
        "{what} needs a finite, positive size, got {}x{}; each side that is not finite and positive is 1 point",
        size.width,
        size.height
    );
    Size::new(positive(size.width), positive(size.height))
}

fn point(p: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(p.x), f64::from(p.y))
}

fn rect(r: Rect) -> kurbo::Rect {
    kurbo::Rect::new(
        f64::from(r.x()),
        f64::from(r.y()),
        f64::from(r.max_x()),
        f64::from(r.max_y()),
    )
}

/// The recorded form of a layout rectangle, for operands that take one.
pub fn lower_rect(r: Rect) -> kurbo::Rect {
    rect(r)
}

impl sealed::Lower for Rect {
    fn lower(self) -> ShapeData {
        ShapeData::Rect(rect(self))
    }
}

/// A rectangle with circular corners of one radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedRect {
    /// The rectangle.
    pub rect: Rect,
    /// The corner radius, in points.
    pub radius: f32,
}

impl RoundedRect {
    /// A rectangle with corners of `radius` points.
    #[must_use]
    pub const fn new(rect: Rect, radius: f32) -> Self {
        Self { rect, radius }
    }
}

impl sealed::Lower for RoundedRect {
    fn lower(self) -> ShapeData {
        ShapeData::RoundedRect(kurbo::RoundedRect::from_rect(
            rect(self.rect),
            f64::from(self.radius),
        ))
    }
}

/// A circle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    /// The centre.
    pub center: Point,
    /// The radius, in points.
    pub radius: f32,
}

impl Circle {
    /// A circle of `radius` points around `center`.
    #[must_use]
    pub const fn new(center: Point, radius: f32) -> Self {
        Self { center, radius }
    }

    /// The largest circle inside `rect`, centred in it.
    #[must_use]
    pub fn inscribed(rect: Rect) -> Self {
        Self {
            center: Point::new(rect.mid_x(), rect.mid_y()),
            radius: rect.width().min(rect.height()) / 2.0,
        }
    }
}

impl sealed::Lower for Circle {
    fn lower(self) -> ShapeData {
        ShapeData::Circle(kurbo::Circle::new(
            point(self.center),
            f64::from(self.radius),
        ))
    }
}

/// An axis-aligned ellipse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    /// The rectangle the ellipse is inscribed in.
    pub bounds: Rect,
}

impl Ellipse {
    /// The ellipse inscribed in `bounds`.
    #[must_use]
    pub const fn new(bounds: Rect) -> Self {
        Self { bounds }
    }
}

impl sealed::Lower for Ellipse {
    fn lower(self) -> ShapeData {
        ShapeData::Ellipse(kurbo::Ellipse::from_rect(rect(self.bounds)))
    }
}

/// A straight line segment; it encloses no area, so it is drawn by stroking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    /// Where the line starts.
    pub start: Point,
    /// Where the line ends.
    pub end: Point,
}

impl Line {
    /// The segment from `start` to `end`.
    #[must_use]
    pub const fn new(start: Point, end: Point) -> Self {
        Self { start, end }
    }
}

impl sealed::Lower for Line {
    fn lower(self) -> ShapeData {
        ShapeData::Line(kurbo::Line::new(point(self.start), point(self.end)))
    }
}

/// Which points a path's outline encloses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FillRule {
    /// A point is inside when the outline winds around it a non-zero number
    /// of times.
    #[default]
    NonZero,
    /// A point is inside when the outline crosses an odd number of times
    /// between it and infinity.
    EvenOdd,
}

/// A general outline: lines, quadratic and cubic Béziers and elliptical arcs,
/// in one or more subpaths.
///
/// Built with [`Path::builder`]. A finished path is immutable and cheap to
/// clone — clones share the outline.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    elements: Arc<[PathEl]>,
    rule: FillRule,
}

impl Path {
    /// Starts an empty path.
    #[must_use]
    pub fn builder() -> PathBuilder {
        PathBuilder {
            path: kurbo::BezPath::new(),
            rule: FillRule::NonZero,
        }
    }

    /// The path's fill rule.
    #[must_use]
    pub const fn fill_rule(&self) -> FillRule {
        self.rule
    }

    /// Whether `point` lies inside the area the path encloses under its fill
    /// rule: the hit test over exactly the outline a fill of this path draws.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        let winding = self.elements.as_ref().winding(self::point(point));
        match self.rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }

    /// The smallest rectangle containing the outline.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let bounds = self.elements.as_ref().bounding_box();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "canvas geometry is authored in f32 points; bounds round-trip through f64"
        )]
        Rect::new(
            Point::new(bounds.x0 as f32, bounds.y0 as f32),
            waterui_core::layout::Size::new(bounds.width() as f32, bounds.height() as f32),
        )
    }
}

impl sealed::Lower for Path {
    fn lower(self) -> ShapeData {
        ShapeData::Path {
            elements: self.elements,
            rule: match self.rule {
                FillRule::NonZero => RecordFillRule::NonZero,
                FillRule::EvenOdd => RecordFillRule::EvenOdd,
            },
        }
    }
}

/// Builds a [`Path`] one segment at a time.
#[derive(Clone, Debug)]
pub struct PathBuilder {
    path: kurbo::BezPath,
    rule: FillRule,
}

impl PathBuilder {
    /// Starts a new subpath at `to`.
    #[must_use]
    pub fn move_to(mut self, to: Point) -> Self {
        self.path.move_to(point(to));
        self
    }

    /// A straight segment to `to`.
    #[must_use]
    pub fn line_to(mut self, to: Point) -> Self {
        self.path.line_to(point(to));
        self
    }

    /// A quadratic Bézier through `control` to `to`.
    #[must_use]
    pub fn quad_to(mut self, control: Point, to: Point) -> Self {
        self.path.quad_to(point(control), point(to));
        self
    }

    /// A cubic Bézier through `control1` and `control2` to `to`.
    #[must_use]
    pub fn cubic_to(mut self, control1: Point, control2: Point, to: Point) -> Self {
        self.path
            .curve_to(point(control1), point(control2), point(to));
        self
    }

    /// An elliptical arc around `center` with radii `radii` (x, y), starting
    /// at angle `start` and sweeping `sweep` radians clockwise in canvas
    /// space. The arc joins the current point with a straight segment, or
    /// starts a subpath when there is none.
    ///
    /// A `sweep` of more than one full turn either way is a contract
    /// violation: the arc sweeps one full turn in that direction (none when
    /// `sweep` is `NaN`).
    #[must_use]
    pub fn arc(mut self, center: Point, radii: (f32, f32), start: f32, sweep: f32) -> Self {
        let sweep = if sweep.abs() <= TAU {
            sweep
        } else {
            contract_violation!("an arc sweeps at most one full turn, got {sweep} radians");
            if sweep.is_nan() {
                0.0
            } else {
                sweep.clamp(-TAU, TAU)
            }
        };
        let arc = kurbo::Arc::new(
            point(center),
            (f64::from(radii.0), f64::from(radii.1)),
            f64::from(start),
            f64::from(sweep),
            0.0,
        );
        let begin = arc.path_elements(crate::draw::PATH_TOLERANCE).next();
        let mut elements = arc.path_elements(crate::draw::PATH_TOLERANCE);
        // The arc's own first element is the move to its start; keep the
        // subpath going with a line instead when one is open.
        if let (Some(PathEl::MoveTo(start)), false) = (begin, self.path.elements().is_empty()) {
            let _ = elements.next();
            self.path.line_to(start);
        }
        for element in elements {
            self.path.push(element);
        }
        self
    }

    /// Closes the current subpath with a straight segment to its start.
    #[must_use]
    pub fn close(mut self) -> Self {
        self.path.close_path();
        self
    }

    /// Sets which points the finished path encloses.
    #[must_use]
    pub const fn fill_rule(mut self, rule: FillRule) -> Self {
        self.rule = rule;
        self
    }

    /// Finishes the path.
    #[must_use]
    pub fn build(self) -> Path {
        let elements: Vec<PathEl> = self.path.into_elements();
        Path {
            elements: elements.into(),
            rule: self.rule,
        }
    }
}

impl_constant!(RoundedRect, Circle, Ellipse, Line, Path, FillRule);
