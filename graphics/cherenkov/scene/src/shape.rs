use kurbo::{BezPath, Circle, Ellipse, Line, PathEl, Point, Rect, RoundedRect, Shape as _};
use serde::{Deserialize, Serialize};

/// A rounded rectangle with "continuous" (superellipse) corners.
///
/// The corner is a Lamé curve: `|x|^n + |y|^n = r^n` where the exponent
/// `n = 2 + 2 * smoothing`. `smoothing = 0.0` is a circular arc (identical to
/// a [`RoundedRect`] corner); `smoothing = 1.0` gives `n = 4`, the classic
/// squircle. The corner radius `r` is uniform for all four corners.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContinuousRect {
    /// The rectangle bounds.
    pub rect: Rect,
    /// The corner radius in pixels (all corners).
    pub corner_radius: f64,
    /// Corner smoothing, `0.0` (circular) through `1.0` (squircle, `n = 4`).
    pub smoothing: f64,
}

impl ContinuousRect {
    /// Create a continuous rounded rect.
    #[must_use]
    pub const fn new(rect: Rect, corner_radius: f64, smoothing: f64) -> Self {
        Self {
            rect,
            corner_radius,
            smoothing,
        }
    }

    /// The superellipse exponent for `smoothing`.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // `mul_add`/`clamp` are not const
    pub fn exponent(smoothing: f64) -> f64 {
        2.0f64.mul_add(smoothing.clamp(0.0, 1.0), 2.0)
    }

    /// Expand to a [`BezPath`] of line segments at the default tolerance
    /// (`0.25` px, matching [`Shape::to_path`]).
    #[must_use]
    pub fn to_path(&self) -> BezPath {
        self.to_path_at(0.25)
    }

    /// Expand to a [`BezPath`] of line segments within `tolerance` px of the
    /// true superellipse boundary.
    ///
    /// Each corner is a quarter Lamé curve `x = r·|cos t|^e`, `y = r·|sin t|^e`
    /// with `e = 2/n`, recursively bisected in `t` until every sampled point
    /// of the curve lies within `tolerance` of its chord. The recursion is
    /// depth-capped; a degenerate `corner_radius <= 0` produces a plain rect.
    #[must_use]
    #[allow(clippy::many_single_char_names)] // x/y/r/e/n geometry names
    pub fn to_path_at(&self, tolerance: f64) -> BezPath {
        // Emit the chord [t0, t1] of corner `c`, bisecting until the sampled
        // deviation from the chord is within `tolerance`.
        fn emit(
            c: usize,
            t0: f64,
            t1: f64,
            arc: &dyn Fn(usize, f64) -> Point,
            tol: f64,
            path: &mut BezPath,
            depth: u32,
        ) {
            const MAX_DEPTH: u32 = 24;
            let (p0, p1) = (arc(c, t0), arc(c, t1));
            // Probe the quarter and three-quarter points too: a Lamé arc is
            // steepest near its ends, so one midpoint check can miss it.
            let flat = (1..4).all(|k| {
                let s = f64::from(k) * 0.25;
                let pm = arc(c, (t1 - t0).mul_add(s, t0));
                let (cx, cy) = (s.mul_add(p1.x - p0.x, p0.x), s.mul_add(p1.y - p0.y, p0.y));
                libm::hypot(pm.x - cx, pm.y - cy) <= tol
            });
            if flat || depth >= MAX_DEPTH {
                path.line_to(p1);
            } else {
                let tm = (t1 - t0).mul_add(0.5, t0);
                emit(c, t0, tm, arc, tol, path, depth + 1);
                emit(c, tm, t1, arc, tol, path, depth + 1);
            }
        }

        let n = Self::exponent(self.smoothing);
        let e = 2.0 / n;
        let r = self
            .corner_radius
            .min(self.rect.width() / 2.0)
            .min(self.rect.height() / 2.0)
            .max(0.0);
        let Rect { x0, y0, x1, y1 } = self.rect;
        // Corner centres in order top-right, bottom-right, bottom-left, top-left.
        let corners = [
            (x1 - r, y0 + r),
            (x1 - r, y1 - r),
            (x0 + r, y1 - r),
            (x0 + r, y0 + r),
        ];
        // Point on corner `c`'s Lamé arc at parameter `t ∈ [0, π/2]`.
        let arc = |c: usize, t: f64| -> Point {
            let (cx, cy) = corners[c];
            let (s, co) = (
                r * libm::pow(libm::sin(t), e),
                r * libm::pow(libm::cos(t), e),
            );
            let (dx, dy) = match c {
                0 => (s, -co),  // TR: from (cx, cy-r) to (cx+r, cy)
                1 => (co, s),   // BR: from (cx+r, cy) to (cx, cy+r)
                2 => (-s, co),  // BL: from (cx, cy+r) to (cx-r, cy)
                _ => (-co, -s), // TL: from (cx-r, cy) to (cx, cy-r)
            };
            Point::new(cx + dx, cy + dy)
        };
        let mut path = BezPath::new();
        path.move_to((x0 + r, y0));
        for c in 0..corners.len() {
            path.line_to(arc(c, 0.0));
            emit(
                c,
                0.0,
                std::f64::consts::FRAC_PI_2,
                &arc,
                tolerance,
                &mut path,
                0,
            );
        }
        path.close_path();
        path
    }
}

/// A drawable shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    /// An axis-aligned rectangle.
    Rect(Rect),
    /// A rectangle with per-corner radii.
    RoundedRect(RoundedRect),
    /// A rectangle with continuous (superellipse) corners.
    Continuous(ContinuousRect),
    /// A circle.
    Circle(Circle),
    /// An axis-aligned ellipse.
    Ellipse(Ellipse),
    /// A straight line segment (for stroking).
    Line(Line),
    /// An arbitrary path of kurbo `BezPath` elements.
    ///
    /// A struct variant so the path's sequence serialization is wrapped in a
    /// named field (internally tagged newtypes cannot wrap sequences).
    Path {
        /// The path elements.
        path: BezPath,
    },
}

impl Shape {
    /// Convert to a [`BezPath`] at the default curve-approximation
    /// tolerance (`0.25` px, like `kurbo`'s own `to_path`).
    #[must_use]
    pub fn to_path(&self) -> BezPath {
        self.to_path_at(0.25)
    }

    /// Convert to a [`BezPath`] whose curve segments approximate the true
    /// boundary within `tolerance` px in the shape's own coordinate space.
    ///
    /// `Rect`, `RoundedRect`, `Circle` and `Ellipse` defer to `kurbo`'s
    /// arc fitting at `tolerance`; [`ContinuousRect`] subdivides its Lamé
    /// corners adaptively.
    #[must_use]
    pub fn to_path_at(&self, tolerance: f64) -> BezPath {
        match self {
            Self::Rect(r) => r.to_path(tolerance),
            Self::RoundedRect(r) => r.to_path(tolerance),
            Self::Continuous(c) => c.to_path_at(tolerance),
            Self::Circle(c) => c.to_path(tolerance),
            Self::Ellipse(e) => e.to_path(tolerance),
            Self::Line(l) => {
                let mut p = BezPath::new();
                p.push(PathEl::MoveTo(l.p0));
                p.push(PathEl::LineTo(l.p1));
                p
            }
            Self::Path { path } => path.clone(),
        }
    }

    /// A bounding box of the shape.
    #[must_use]
    pub fn bounding_box(&self) -> Rect {
        self.to_path().bounding_box()
    }

    /// A simple rect-construction helper.
    #[must_use]
    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self::Rect(Rect::new(x, y, x + w, y + h))
    }

    /// A rounded-rect construction helper with a uniform radius.
    #[must_use]
    pub fn rounded_rect(x: f64, y: f64, w: f64, h: f64, radius: f64) -> Self {
        Self::RoundedRect(RoundedRect::new(x, y, x + w, y + h, radius))
    }

    /// A circle construction helper.
    #[must_use]
    pub fn circle(cx: f64, cy: f64, r: f64) -> Self {
        Self::Circle(Circle::new(Point::new(cx, cy), r))
    }
}
