//! Stroke expansion that splits cubic Béziers at cusps before stroking, and
//! gives zero-length dashes their SVG 2 meaning.
//!
//! [`stroke`] is a drop-in for [`kurbo::stroke`] with
//! [`kurbo::StrokeOpts::default`]. `kurbo` finds a cusp by sampling the
//! closest point of the hodograph to the origin, and the test it applies
//! there fails on a cusp whose parameter is not a dyadic rational and on a
//! cubic that is a cusp to within `f64` precision; the stroke then overshoots
//! the true outline by up to half a width. This crate decides the cusp exactly
//! over the `f64` control points, splits the cubic there with both cusp-side
//! arms exactly zero, and hands the result to `kurbo`, whose join logic then
//! sees the reversal and applies the stroke's own join (no disc is added).
//!
//! The cusp splitting is removable once linebender/kurbo#615 is fixed. The
//! dash handling is the engine's own: `kurbo::dash` leaves a dot for a
//! zero-length dash only where f64 rounding in its `DashIterator::step`
//! leaves a roughly 1e-15 segment behind, while SVG 2 draws both of the
//! dash's caps at the dash's point.

mod cusp;
mod dash;

use kurbo::{BezPath, CubicBez, PathEl, Point, Stroke, StrokeOpts};

/// Expand `path` stroked with `style` into a fill outline, accurate to
/// `tolerance`.
///
/// Dashing runs first: zero-length dashes become cap-shaped dots and every
/// non-zero dash goes through the same cusp-splitting stroke as an undashed
/// path. The dash pattern must be valid: finite and non-negative entries
/// with a positive sum, and a finite offset.
#[must_use]
pub fn stroke(path: impl IntoIterator<Item = PathEl>, style: &Stroke, tolerance: f64) -> BezPath {
    if style.dash_pattern.is_empty() {
        return stroke_undashed(path, style, tolerance);
    }
    let dashed = dash::dash(path, &style.dash_pattern, style.dash_offset);
    let mut undashed = style.clone();
    undashed.dash_pattern.clear();
    undashed.dash_offset = 0.0;
    let mut outline = stroke_undashed(dashed.dashes, &undashed, tolerance);
    for dot in &dashed.dots {
        dot.append_caps(style, tolerance, &mut outline);
    }
    outline
}

/// `kurbo::stroke` over `path` with its cusped cubics split.
fn stroke_undashed(
    path: impl IntoIterator<Item = PathEl>,
    style: &Stroke,
    tolerance: f64,
) -> BezPath {
    kurbo::stroke(split_cusps(path), style, &StrokeOpts::default(), tolerance)
}

/// `path` with every cubic that has an interior cusp split there.
fn split_cusps(path: impl IntoIterator<Item = PathEl>) -> BezPath {
    let mut out = BezPath::new();
    let mut current = Point::ZERO;
    let mut start = Point::ZERO;
    for el in path {
        match el {
            PathEl::MoveTo(p) => {
                start = p;
                current = p;
                out.push(el);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) => {
                current = p;
                out.push(el);
            }
            PathEl::CurveTo(a, b, p) => {
                let cubic = CubicBez::new(current, a, b, p);
                if let Some(t) = cusp::parameter(&cubic) {
                    let (left, right) = split_at_cusp(cubic, t);
                    out.curve_to(left.p1, left.p2, left.p3);
                    out.curve_to(right.p1, right.p2, right.p3);
                } else {
                    out.push(el);
                }
                current = p;
            }
            PathEl::ClosePath => {
                current = start;
                out.push(el);
            }
        }
    }
    out
}

/// Split `c` by de Casteljau at `t`, then set the arms next to the split point
/// exactly to zero so the cusp-side tangent is the curve's true limiting
/// direction.
fn split_at_cusp(c: CubicBez, t: f64) -> (CubicBez, CubicBez) {
    let ab = c.p0.lerp(c.p1, t);
    let bc = c.p1.lerp(c.p2, t);
    let cd = c.p2.lerp(c.p3, t);
    let abc = ab.lerp(bc, t);
    let bcd = bc.lerp(cd, t);
    let at = abc.lerp(bcd, t);
    (
        CubicBez::new(c.p0, ab, at, at),
        CubicBez::new(at, at, cd, c.p3),
    )
}

#[cfg(test)]
mod tests {
    use kurbo::{Join, ParamCurve, ParamCurveNearest, Shape};

    use super::*;

    /// The cell-144 cubic of `stroke-gpu-curves-*`: a cusp to within `f64`
    /// precision, not exactly.
    fn cell_144() -> CubicBez {
        CubicBez::new(
            (1.600_000_000_000_000_8, 310.787_665_198_237_9),
            (5.406_167_400_881_057, 306.981_497_797_356_84),
            (5.406_167_400_881_058, 310.787_665_198_237_9),
            (1.6, 306.981_497_797_356_84),
        )
    }

    fn cubic(p: [(f64, f64); 4]) -> CubicBez {
        CubicBez::new(p[0], p[1], p[2], p[3])
    }

    #[test]
    fn exact_half_cusp_splits_at_one_half() {
        let c = cubic([(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (0.0, -1.0)]);
        assert_eq!(cusp::parameter(&c), Some(0.5));
    }

    #[test]
    fn exact_third_cusp_splits_at_the_rounded_third() {
        let c = cubic([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (-3.0, -3.0)]);
        let t = cusp::parameter(&c).expect("cusp");
        assert_eq!(t, 1.0 / 3.0);
        let (left, right) = split_at_cusp(c, t);
        assert_eq!(left.p2, left.p3);
        assert_eq!(right.p0, right.p1);
        assert!((left.p3 - c.eval(t)).hypot() < 1e-15);
    }

    #[test]
    fn cell_144_near_cusp_splits_at_one_half() {
        let t = cusp::parameter(&cell_144()).expect("near cusp");
        assert!((t - 0.5).abs() < 1e-12, "t = {t}");
    }

    #[test]
    fn hairpin_outside_the_band_is_not_split() {
        let c = cubic([(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1e-9, -1.0)]);
        assert_eq!(cusp::parameter(&c), None);
    }

    #[test]
    fn collinear_cubic_is_not_split() {
        let c = cubic([(0.0, 0.0), (3.0, 3.0), (-1.0, -1.0), (2.0, 2.0)]);
        assert_eq!(cusp::parameter(&c), None);
    }

    #[test]
    fn cell_144_outline_stays_within_half_width() {
        let c = cell_144();
        let mut path = BezPath::new();
        path.move_to(c.p0);
        path.curve_to(c.p1, c.p2, c.p3);
        let (width, tolerance) = (1.2, 0.02);
        let outline = stroke(path, &Stroke::new(width).with_join(Join::Round), tolerance);
        let mut worst = 0.0_f64;
        for seg in outline.segments() {
            for i in 0..=64 {
                let p = seg.eval(f64::from(i) / 64.0);
                worst = worst.max(c.nearest(p, 1e-9).distance_sq.sqrt());
            }
        }
        assert!(
            worst <= width / 2.0 + tolerance,
            "outline reaches {worst} from the centerline"
        );
        assert!(outline.area() != 0.0);
    }
}
