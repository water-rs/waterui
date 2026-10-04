//! Shapes and their semantic classification.

use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;

use kurbo::{BezPath, Circle, Ellipse, Line, PathEl, Point, Rect, RoundedRect, RoundedRectRadii};
use nami_core::Signal;
use nami_core::watcher::Context;

/// Tolerance, in the shape's own units, for converting curves that have no
/// exact Bézier form (such as arcs) into path elements.
pub const PATH_TOLERANCE: f64 = 1e-3;

/// Something the engine can fill, stroke, clip to or cast a shadow from.
///
/// The trait is open: any type can be a shape. Its [`semantic`](Shape::semantic)
/// classification tells the engine whether a fast path applies. Every kurbo
/// shape is a `Shape`, classified through kurbo's own `as_rect`,
/// `as_rounded_rect`, `as_circle` and `as_line`; `kurbo::Ellipse` is
/// recognized by type; anything else is drawn as its path.
pub trait Shape: 'static {
    /// What the engine may draw with a fast path.
    fn semantic(&self) -> Semantic<'_>;

    /// The shape's recorded data. Shapes that store their path elements move
    /// them instead of copying.
    fn into_data(self) -> ShapeData
    where
        Self: Sized,
    {
        ShapeData::of(&self)
    }
}

/// The engine's fast-path vocabulary: the shapes it draws analytically. Every
/// other shape is a [`Semantic::Path`].
#[derive(Clone, Debug, PartialEq)]
pub enum Semantic<'a> {
    /// An axis-aligned rectangle.
    Rect(Rect),
    /// A rectangle with circular corners.
    RoundedRect(RoundedRect),
    /// A rectangle with continuous (superellipse-blended) corners.
    Continuous(ContinuousRect),
    /// A circle.
    Circle(Circle),
    /// An ellipse, possibly rotated.
    Ellipse(Ellipse),
    /// A line segment.
    Line(Line),
    /// A general path.
    Path(PathRef<'a>),
}

/// A path, borrowed when the shape stores its elements and owned otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct PathRef<'a> {
    /// The path elements.
    pub elements: Cow<'a, [PathEl]>,
    /// How the interior is decided when the path is filled.
    pub rule: FillRule,
}

/// How the interior of a self-intersecting path is decided.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FillRule {
    /// A point is inside when the winding number is non-zero.
    #[default]
    NonZero,
    /// A point is inside when the winding number is odd.
    EvenOdd,
}

impl<T: kurbo::Shape + 'static> Shape for T {
    fn semantic(&self) -> Semantic<'_> {
        if let Some(ellipse) = (self as &dyn Any).downcast_ref::<Ellipse>() {
            return Semantic::Ellipse(*ellipse);
        }
        if let Some(rect) = self.as_rect() {
            return Semantic::Rect(rect);
        }
        if let Some(rounded) = self.as_rounded_rect() {
            return Semantic::RoundedRect(rounded);
        }
        if let Some(circle) = self.as_circle() {
            return Semantic::Circle(circle);
        }
        if let Some(line) = self.as_line() {
            return Semantic::Line(line);
        }
        let elements = self.as_path_slice().map_or_else(
            || Cow::Owned(self.path_elements(PATH_TOLERANCE).collect()),
            Cow::Borrowed,
        );
        Semantic::Path(PathRef {
            elements,
            rule: FillRule::NonZero,
        })
    }

    fn into_data(mut self) -> ShapeData {
        let path = (&mut self as &mut dyn Any)
            .downcast_mut::<BezPath>()
            .map(std::mem::take);
        path.map_or_else(
            || ShapeData::of(&self),
            |path| ShapeData::Path {
                elements: path.into_iter().collect(),
                rule: FillRule::NonZero,
            },
        )
    }
}

/// A rectangle with continuous corners: each corner blends into the straight
/// edges along a superellipse instead of meeting them at a circular arc's
/// tangent point.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContinuousRect {
    /// The rectangle.
    pub rect: Rect,
    /// The corner radii.
    pub radii: RoundedRectRadii,
    /// How far, as a fraction of each radius, the curvature transition extends
    /// into the straight edges. 0 is a circular corner.
    pub smoothing: f64,
}

impl ContinuousRect {
    /// The smoothing of system continuous corners.
    pub const DEFAULT_SMOOTHING: f64 = 0.6;

    /// Creates a continuous rounded rectangle with the default smoothing.
    #[must_use]
    pub fn new(rect: Rect, radii: impl Into<RoundedRectRadii>) -> Self {
        Self {
            rect,
            radii: radii.into(),
            smoothing: Self::DEFAULT_SMOOTHING,
        }
    }

    /// Returns this shape with a different smoothing.
    #[must_use]
    pub const fn with_smoothing(self, smoothing: f64) -> Self {
        Self { smoothing, ..self }
    }

    /// The shape as a path of Lamé-corner line segments, each corner
    /// subdivided until within `tolerance` of its chord.
    ///
    /// A `smoothing` of 0 gives circular-arc corners, equivalent to
    /// [`RoundedRect::to_path`]; larger values extend the curvature
    /// transition into the straight edges. Each corner is a quarter Lamé
    /// curve `x = r·|cos t|^e`, `y = r·|sin t|^e` with `e = 2/n` and
    /// `n = 2 + 2·smoothing`, recursively bisected in `t` until flat.
    #[expect(
        clippy::too_many_arguments,
        reason = "the emit helper takes the curve constants per call"
    )]
    #[must_use]
    pub fn to_path(&self, tolerance: f64) -> BezPath {
        fn emit(
            centre: Point,
            r: f64,
            c: usize,
            t0: f64,
            t1: f64,
            e: f64,
            tol: f64,
            path: &mut BezPath,
            depth: u32,
        ) {
            const MAX_DEPTH: u32 = 24;
            // Point on corner `c`'s Lamé arc at `t ∈ [0, π/2]`.
            let arc = |t: f64| -> Point {
                let (s, co) = (r * t.sin().powf(e), r * t.cos().powf(e));
                let (dx, dy) = match c {
                    0 => (s, -co),  // TR: from (cx, cy-r) to (cx+r, cy)
                    1 => (co, s),   // BR: from (cx+r, cy) to (cx, cy+r)
                    2 => (-s, co),  // BL: from (cx, cy+r) to (cx-r, cy)
                    _ => (-co, -s), // TL: from (cx-r, cy) to (cx, cy-r)
                };
                Point::new(centre.x + dx, centre.y + dy)
            };
            let (p0, p1) = (arc(t0), arc(t1));
            let flat = (1..4).all(|k| {
                let s = f64::from(k) * 0.25;
                let pm = arc((t1 - t0).mul_add(s, t0));
                let (cx, cy) = (s.mul_add(p1.x - p0.x, p0.x), s.mul_add(p1.y - p0.y, p0.y));
                (pm.x - cx).hypot(pm.y - cy) <= tol
            });
            if flat || depth >= MAX_DEPTH {
                path.line_to(p1);
            } else {
                let tm = (t1 - t0).mul_add(0.5, t0);
                emit(centre, r, c, t0, tm, e, tol, path, depth + 1);
                emit(centre, r, c, tm, t1, e, tol, path, depth + 1);
            }
        }

        let n = 2.0f64.mul_add(self.smoothing.clamp(0.0, 1.0), 2.0);
        let e = 2.0 / n;
        let (rect, radii) = (self.rect, self.radii);
        let half_w = rect.width() / 2.0;
        let half_h = rect.height() / 2.0;
        let r = [
            radii.top_right.clamp(0.0, half_w.min(half_h)),
            radii.bottom_right.clamp(0.0, half_w.min(half_h)),
            radii.bottom_left.clamp(0.0, half_w.min(half_h)),
            radii.top_left.clamp(0.0, half_w.min(half_h)),
        ];
        let Rect { x0, y0, x1, y1 } = rect;
        // Corner centres in order top-right, bottom-right, bottom-left,
        // top-left.
        let corners = [
            (x1 - r[0], y0 + r[0]),
            (x1 - r[1], y1 - r[1]),
            (x0 + r[2], y1 - r[2]),
            (x0 + r[3], y0 + r[3]),
        ];
        let mut path = BezPath::new();
        path.move_to((x0 + r[3], y0));
        for (c, &(cx, cy)) in corners.iter().enumerate() {
            // Straight edge to this corner's arc start.
            let rc = r[c];
            let start = match c {
                0 => Point::new(cx, cy - rc),
                1 => Point::new(cx + rc, cy),
                2 => Point::new(cx, cy + rc),
                _ => Point::new(cx - rc, cy),
            };
            path.line_to(start);
            emit(
                Point::new(cx, cy),
                rc,
                c,
                0.0,
                std::f64::consts::FRAC_PI_2,
                e,
                tolerance,
                &mut path,
                0,
            );
        }
        path.close_path();
        path
    }
}

impl Shape for ContinuousRect {
    fn semantic(&self) -> Semantic<'_> {
        Semantic::Continuous(*self)
    }
}

/// A shape filled with the even-odd rule. Only paths can self-intersect, so
/// every other semantic shape is unaffected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvenOdd<S>(pub S);

impl<S: Shape> Shape for EvenOdd<S> {
    fn semantic(&self) -> Semantic<'_> {
        match self.0.semantic() {
            Semantic::Path(path) => Semantic::Path(PathRef {
                rule: FillRule::EvenOdd,
                ..path
            }),
            other => other,
        }
    }

    fn into_data(self) -> ShapeData {
        match self.0.into_data() {
            ShapeData::Path { elements, .. } => ShapeData::Path {
                elements,
                rule: FillRule::EvenOdd,
            },
            other => other,
        }
    }
}

/// A shape in the display list: an owned [`Semantic`].
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ShapeData {
    /// An axis-aligned rectangle.
    Rect(Rect),
    /// A rectangle with circular corners.
    RoundedRect(RoundedRect),
    /// A rectangle with continuous corners.
    Continuous(ContinuousRect),
    /// A circle.
    Circle(Circle),
    /// An ellipse.
    Ellipse(Ellipse),
    /// A line segment.
    Line(Line),
    /// A general path.
    Path {
        /// The path elements.
        elements: Arc<[PathEl]>,
        /// The fill rule.
        rule: FillRule,
    },
}

impl ShapeData {
    /// Records a shape.
    #[must_use]
    pub fn of(shape: &(impl Shape + ?Sized)) -> Self {
        shape.semantic().into()
    }

    /// The smallest rectangle containing the shape's area.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        use kurbo::Shape as _;
        match self {
            Self::Rect(rect) => *rect,
            Self::RoundedRect(rounded) => rounded.bounding_box(),
            Self::Continuous(continuous) => continuous.rect,
            Self::Circle(circle) => circle.bounding_box(),
            Self::Ellipse(ellipse) => ellipse.bounding_box(),
            Self::Line(line) => line.bounding_box(),
            Self::Path { elements, .. } => (&elements[..]).bounding_box(),
        }
    }
}

fn push_point(lanes: &mut Vec<f64>, point: Point) {
    lanes.extend([point.x, point.y]);
}

fn push_el(lanes: &mut Vec<f64>, el: &PathEl) {
    match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => push_point(lanes, *p),
        PathEl::QuadTo(a, b) => {
            push_point(lanes, *a);
            push_point(lanes, *b);
        }
        PathEl::CurveTo(a, b, c) => {
            push_point(lanes, *a);
            push_point(lanes, *b);
            push_point(lanes, *c);
        }
        PathEl::ClosePath => {}
    }
}

/// Whether two paths carry the same verb sequence — the lane layout a
/// `Path` needs to interpolate.
fn same_verbs(from: &[PathEl], to: &[PathEl]) -> bool {
    from.len() == to.len()
        && from
            .iter()
            .zip(to)
            .all(|(a, b)| std::mem::discriminant(a) == std::mem::discriminant(b))
}

impl crate::animation::AnimLanes for ShapeData {
    fn anim_lanes(&self, target: &Self) -> Option<Box<[f64]>> {
        let mut lanes = Vec::new();
        match (self, target) {
            (Self::Rect(from), Self::Rect(_)) => {
                lanes.extend([from.x0, from.y0, from.x1, from.y1]);
            }
            (Self::RoundedRect(from), Self::RoundedRect(_)) => {
                let (rect, radii) = (from.rect(), from.radii());
                lanes.extend([rect.x0, rect.y0, rect.x1, rect.y1]);
                lanes.extend([
                    radii.top_left,
                    radii.top_right,
                    radii.bottom_right,
                    radii.bottom_left,
                ]);
            }
            (Self::Continuous(from), Self::Continuous(_)) => {
                lanes.extend([from.rect.x0, from.rect.y0, from.rect.x1, from.rect.y1]);
                lanes.extend([
                    from.radii.top_left,
                    from.radii.top_right,
                    from.radii.bottom_right,
                    from.radii.bottom_left,
                    from.smoothing,
                ]);
            }
            (Self::Circle(from), Self::Circle(_)) => {
                lanes.extend([from.center.x, from.center.y, from.radius]);
            }
            (Self::Ellipse(from), Self::Ellipse(_)) => {
                let (center, radii) = (from.center(), from.radii());
                lanes.extend([center.x, center.y, radii.x, radii.y]);
            }
            (Self::Line(from), Self::Line(_)) => {
                lanes.extend([from.p0.x, from.p0.y, from.p1.x, from.p1.y]);
            }
            (
                Self::Path { elements, rule },
                Self::Path {
                    elements: to_elements,
                    rule: to_rule,
                },
            ) if rule == to_rule && same_verbs(elements, to_elements) => {
                for el in elements.iter() {
                    push_el(&mut lanes, el);
                }
            }
            _ => return None,
        }
        Some(lanes.into_boxed_slice())
    }

    fn with_lanes(&self, lanes: &[f64]) -> Self {
        match self {
            Self::Rect(_) => Self::Rect(Rect::new(lanes[0], lanes[1], lanes[2], lanes[3])),
            Self::RoundedRect(_) => Self::RoundedRect(RoundedRect::from_rect(
                Rect::new(lanes[0], lanes[1], lanes[2], lanes[3]),
                RoundedRectRadii::new(lanes[4], lanes[5], lanes[6], lanes[7]),
            )),
            Self::Continuous(_) => Self::Continuous(ContinuousRect {
                rect: Rect::new(lanes[0], lanes[1], lanes[2], lanes[3]),
                radii: RoundedRectRadii::new(lanes[4], lanes[5], lanes[6], lanes[7]),
                smoothing: lanes[8],
            }),
            Self::Circle(_) => Self::Circle(Circle::new(Point::new(lanes[0], lanes[1]), lanes[2])),
            Self::Ellipse(ellipse) => Self::Ellipse(Ellipse::new(
                Point::new(lanes[0], lanes[1]),
                (lanes[2], lanes[3]),
                ellipse.rotation(),
            )),
            Self::Line(_) => Self::Line(Line::new(
                Point::new(lanes[0], lanes[1]),
                Point::new(lanes[2], lanes[3]),
            )),
            Self::Path { elements, rule } => {
                let mut lanes = lanes.iter().copied();
                let mut point = || Point::new(lanes.next().unwrap(), lanes.next().unwrap());
                let elements = elements
                    .iter()
                    .map(|el| match el {
                        PathEl::MoveTo(_) => PathEl::MoveTo(point()),
                        PathEl::LineTo(_) => PathEl::LineTo(point()),
                        PathEl::QuadTo(..) => PathEl::QuadTo(point(), point()),
                        PathEl::CurveTo(..) => PathEl::CurveTo(point(), point(), point()),
                        PathEl::ClosePath => PathEl::ClosePath,
                    })
                    .collect();
                Self::Path {
                    elements,
                    rule: *rule,
                }
            }
        }
    }
}

impl Shape for ShapeData {
    fn semantic(&self) -> Semantic<'_> {
        match self {
            Self::Rect(rect) => Semantic::Rect(*rect),
            Self::RoundedRect(rounded) => Semantic::RoundedRect(*rounded),
            Self::Continuous(continuous) => Semantic::Continuous(*continuous),
            Self::Circle(circle) => Semantic::Circle(*circle),
            Self::Ellipse(ellipse) => Semantic::Ellipse(*ellipse),
            Self::Line(line) => Semantic::Line(*line),
            Self::Path { elements, rule } => Semantic::Path(PathRef {
                elements: Cow::Borrowed(elements),
                rule: *rule,
            }),
        }
    }

    fn into_data(self) -> ShapeData {
        self
    }
}

impl From<Semantic<'_>> for ShapeData {
    fn from(semantic: Semantic<'_>) -> Self {
        match semantic {
            Semantic::Rect(rect) => Self::Rect(rect),
            Semantic::RoundedRect(rounded) => Self::RoundedRect(rounded),
            Semantic::Continuous(continuous) => Self::Continuous(continuous),
            Semantic::Circle(circle) => Self::Circle(circle),
            Semantic::Ellipse(ellipse) => Self::Ellipse(ellipse),
            Semantic::Line(line) => Self::Line(line),
            Semantic::Path(path) => Self::Path {
                elements: path.elements.into_owned().into(),
                rule: path.rule,
            },
        }
    }
}

nami_core::impl_constant!(ContinuousRect);
nami_core::impl_constant!(ShapeData);

impl<S: Clone + 'static> Signal for EvenOdd<S> {
    type Output = Self;
    type Guard = ();

    fn snapshot(&self) -> Self::Output {
        self.clone()
    }

    fn watch(&self, _watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {}
}

#[cfg(test)]
mod tests {
    use kurbo::{Arc, BezPath, Point, Shape as _, Vec2};

    use super::*;

    #[test]
    fn kurbo_shapes_keep_their_semantics() {
        let rect = Rect::new(0., 0., 10., 20.);
        assert_eq!(rect.semantic(), Semantic::Rect(rect));

        let rounded = RoundedRect::from_rect(rect, 4.);
        assert_eq!(rounded.semantic(), Semantic::RoundedRect(rounded));

        let circle = Circle::new((5., 5.), 3.);
        assert_eq!(circle.semantic(), Semantic::Circle(circle));

        let ellipse = Ellipse::new((5., 5.), (3., 2.), 0.3);
        assert_eq!(ellipse.semantic(), Semantic::Ellipse(ellipse));

        let line = Line::new((0., 0.), (1., 1.));
        assert_eq!(line.semantic(), Semantic::Line(line));

        let continuous = ContinuousRect::new(rect, 6.);
        assert_eq!(continuous.semantic(), Semantic::Continuous(continuous));
    }

    #[test]
    fn stored_paths_are_borrowed_and_other_curves_are_flattened_to_elements() {
        let mut path = BezPath::new();
        path.move_to((0., 0.));
        path.line_to((10., 0.));
        path.line_to((0., 10.));
        path.close_path();
        let Semantic::Path(borrowed) = path.semantic() else {
            panic!("a BezPath is a general path");
        };
        assert!(matches!(borrowed.elements, Cow::Borrowed(_)));

        let arc = Arc::new(Point::ZERO, Vec2::new(4., 4.), 0., 1., 0.);
        let Semantic::Path(owned) = arc.semantic() else {
            panic!("an arc is a general path");
        };
        assert!(matches!(owned.elements, Cow::Owned(_)));
    }

    #[test]
    fn even_odd_applies_only_to_paths() {
        let rect = Rect::new(0., 0., 1., 1.);
        assert_eq!(EvenOdd(rect).semantic(), Semantic::Rect(rect));

        let mut path = BezPath::new();
        path.move_to((0., 0.));
        path.line_to((1., 0.));
        path.close_path();
        let even_odd = EvenOdd(path);
        let Semantic::Path(even_odd) = even_odd.semantic() else {
            panic!("still a path");
        };
        assert_eq!(even_odd.rule, FillRule::EvenOdd);
    }

    #[test]
    fn continuous_rect_to_path_bounds() {
        let rect = Rect::new(1., 2., 21., 22.);

        let circular = ContinuousRect::new(rect, 4.).with_smoothing(0.);
        let path = circular.to_path(1e-3);
        assert_eq!(path.bounding_box(), rect);
        assert!(matches!(path.elements().last(), Some(PathEl::ClosePath)));

        let smoothed = ContinuousRect::new(rect, 8.).with_smoothing(1.);
        let path = smoothed.to_path(1e-3);
        assert!(path.bounding_box().contains_rect(rect) || rect.contains_rect(path.bounding_box()));
        let bbox = path.bounding_box();
        assert!(
            bbox.x0 >= rect.x0 && bbox.y0 >= rect.y0 && bbox.x1 <= rect.x1 && bbox.y1 <= rect.y1
        );
    }

    #[test]
    fn into_data_moves_path_elements() {
        let mut path = BezPath::new();
        path.move_to((0., 0.));
        path.line_to((10., 0.));
        path.line_to((0., 10.));
        path.close_path();
        let elements: Vec<PathEl> = path.iter().collect();
        let ShapeData::Path {
            elements: moved,
            rule,
        } = path.into_data()
        else {
            panic!("a BezPath records as a path");
        };
        assert_eq!(moved.as_ref(), elements.as_slice());
        assert_eq!(rule, FillRule::NonZero);

        let mut path = BezPath::new();
        path.move_to((0., 0.));
        path.line_to((1., 0.));
        path.close_path();
        let ShapeData::Path { rule, .. } = EvenOdd(path).into_data() else {
            panic!("still a path");
        };
        assert_eq!(rule, FillRule::EvenOdd);

        let rect = Rect::new(0., 0., 1., 1.);
        assert_eq!(rect.into_data(), ShapeData::Rect(rect));
    }
}
