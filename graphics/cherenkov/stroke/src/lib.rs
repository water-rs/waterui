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
//! arms a quarter of the tolerance long along the curve's limiting tangent,
//! and hands the result to `kurbo`, whose join logic then sees the reversal
//! and applies the stroke's own join (no disc is added).
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
///
/// # Panics
///
/// Panics, naming the offending element, if the outline holds a non-finite
/// point: a finite input and a valid style never produce one, so it marks a
/// defect in the input or in the expansion.
#[must_use]
pub fn stroke(path: impl IntoIterator<Item = PathEl>, style: &Stroke, tolerance: f64) -> BezPath {
    let outline = stroke_expanded(path, style, tolerance);
    for (ix, el) in outline.elements().iter().enumerate() {
        let points: &[Point] = match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => core::slice::from_ref(p),
            PathEl::QuadTo(a, b) => &[*a, *b],
            PathEl::CurveTo(a, b, c) => &[*a, *b, *c],
            PathEl::ClosePath => &[],
        };
        assert!(
            points.iter().all(|p| p.x.is_finite() && p.y.is_finite()),
            "cherenkov_stroke::stroke produced a non-finite point in outline element {ix}: {el:?}"
        );
    }
    outline
}

fn stroke_expanded(
    path: impl IntoIterator<Item = PathEl>,
    style: &Stroke,
    tolerance: f64,
) -> BezPath {
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
    kurbo::stroke(
        split_cusps(path, tolerance),
        style,
        &StrokeOpts::default(),
        tolerance,
    )
}

/// `path` with every cubic that has an interior cusp split there for a stroke
/// accurate to `tolerance`, produced lazily so stroking a path allocates
/// nothing beyond `kurbo`'s own output.
fn split_cusps(
    path: impl IntoIterator<Item = PathEl>,
    tolerance: f64,
) -> impl Iterator<Item = PathEl> {
    let mut path = path.into_iter();
    let mut current = Point::ZERO;
    let mut start = Point::ZERO;
    let mut right_half = None;
    core::iter::from_fn(move || {
        if let Some(el) = right_half.take() {
            return Some(el);
        }
        let el = path.next()?;
        Some(match el {
            PathEl::MoveTo(p) => {
                start = p;
                current = p;
                el
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) => {
                current = p;
                el
            }
            PathEl::CurveTo(a, b, p) => {
                let cubic = CubicBez::new(current, a, b, p);
                current = p;
                cusp::parameter(&cubic).map_or(el, |t| {
                    let (left, right) = split_at_cusp(cubic, t, tolerance);
                    right_half = Some(PathEl::CurveTo(right.p1, right.p2, right.p3));
                    PathEl::CurveTo(left.p1, left.p2, left.p3)
                })
            }
            PathEl::ClosePath => {
                current = start;
                el
            }
        })
    })
}

/// Split `c` by de Casteljau at `t`, then give each half a cusp-side arm of
/// `k = min(tolerance / 4, half its other arm)` along the curve's limiting
/// direction at the cusp.
///
/// The exact split leaves both cusp-side arms zero, a singular endpoint that
/// kurbo 0.13.1's `offset_cubic` turns into non-finite points on the inner
/// side of some halves. With the arm at length `k` the end tangent keeps
/// exactly the limiting direction (`ab − at` on the left, `cd − at` on the
/// right, the direction kurbo's tangent fallback takes for a zero arm), so
/// the reversal still reaches kurbo's join with the stroke's own join. `k` is
/// kurbo's own interior-cusp nudge (`DIM_TUNE = 0.25` of the tolerance in
/// `regularize_cusp`), so the moved curve stays within the tolerance budget.
fn split_at_cusp(c: CubicBez, t: f64, tolerance: f64) -> (CubicBez, CubicBez) {
    let ab = c.p0.lerp(c.p1, t);
    let bc = c.p1.lerp(c.p2, t);
    let cd = c.p2.lerp(c.p3, t);
    let abc = ab.lerp(bc, t);
    let bcd = bc.lerp(cd, t);
    let at = abc.lerp(bcd, t);
    let cusp_arm = |toward: Point| {
        let arm = toward - at;
        at + arm * (0.25 * tolerance / arm.hypot()).min(0.5)
    };
    (
        CubicBez::new(c.p0, ab, cusp_arm(ab), at),
        CubicBez::new(at, cusp_arm(cd), cd, c.p3),
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
    #[should_panic(expected = "non-finite point in outline element")]
    fn a_non_finite_outline_point_panics() {
        let path = [
            PathEl::MoveTo(Point::new(0.0, 0.0)),
            PathEl::LineTo(Point::new(f64::NAN, 1.0)),
        ];
        let _ = stroke(path, &Stroke::new(1.0), 0.01);
    }

    #[test]
    fn exact_third_cusp_splits_at_the_rounded_third() {
        let c = cubic([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (-3.0, -3.0)]);
        let t = cusp::parameter(&c).expect("cusp");
        assert_eq!(t, 1.0 / 3.0);
        let tolerance = 0.02;
        let (left, right) = split_at_cusp(c, t, tolerance);
        for (arm, limit) in [
            (left.p2 - left.p3, left.p1 - left.p3),
            (right.p1 - right.p0, right.p2 - right.p0),
        ] {
            assert!((arm.hypot() - tolerance / 4.0).abs() < 1e-15);
            assert!(arm.cross(limit).abs() < 1e-15 && arm.dot(limit) > 0.0);
        }
        assert!((left.p3 - c.eval(t)).hypot() < 1e-15);
        assert_eq!(left.p3, right.p0);
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

    /// Stroke `c` round-joined at width 1.2, as `stroke-gpu-curves-*` does,
    /// check that every outline point is finite, and return how far the
    /// outline reaches from the centerline.
    fn finite_outline_reach(c: CubicBez, tolerance: f64) -> f64 {
        let mut path = BezPath::new();
        path.move_to(c.p0);
        path.curve_to(c.p1, c.p2, c.p3);
        let outline = stroke(path, &Stroke::new(1.2).with_join(Join::Round), tolerance);
        assert!(outline.area() != 0.0);
        let mut worst = 0.0_f64;
        for seg in outline.segments() {
            for i in 0..=64 {
                let p = seg.eval(f64::from(i) / 64.0);
                assert!(p.is_finite(), "non-finite outline point at {tolerance}");
                worst = worst.max(c.nearest(p, 1e-9).distance_sq.sqrt());
            }
        }
        worst
    }

    /// The cell-102 element-3 cubic of `stroke-gpu-curves-*`: a collinear
    /// reversal whose `α`, `β`, `γ` are rounding noise of the baked rotation
    /// with one sign. Split as a cusp, its outline reached 1.18 from the
    /// centerline.
    #[test]
    fn cell_102_collinear_noise_is_not_split() {
        let c = cubic([
            (206.559_557_697_619_65, 203.091_063_748_859_23),
            (213.912_319_365_047, 198.178_105_472_972_36),
            (203.802_272_072_334_4, 204.933_423_102_316_8),
            (211.155_033_739_761_73, 200.020_464_826_429_93),
        ]);
        assert_eq!(cusp::parameter(&c), None);
        let reach = finite_outline_reach(c, 0.02);
        assert!(reach <= 0.6 + 0.02, "outline reaches {reach}");
    }

    #[test]
    fn cell_144_outline_stays_within_half_width() {
        let reach = finite_outline_reach(cell_144(), 0.02);
        assert!(reach <= 0.6 + 0.02, "outline reaches {reach}");
    }

    /// The cell-90 element-11 cubic of `stroke-gpu-curves-*`, an exact cusp at
    /// `t = 1/2`. With both cusp-side arms exactly zero, kurbo 0.13.1's
    /// `offset_cubic` gave the left half's inner offset non-finite points at
    /// both tolerances. The reach is checked only at the coarse tolerance:
    /// kurbo 0.13.1 builds round joins and caps at a fixed unit-arc tolerance
    /// of 1e-3, about 1.6e-4 at this half width, whatever tolerance is asked.
    #[test]
    fn cell_90_split_outline_is_finite_and_within_half_width() {
        let c = cubic([
            (324.682_665_422_288_3, 165.285_787_276_862_38),
            (322.213_180_276_986_8, 168.981_632_976_137_43),
            (325.295_845_699_275_1, 168.368_452_699_150_65),
            (321.6, 165.898_967_553_849_16),
        ]);
        assert_eq!(cusp::parameter(&c), Some(0.5));
        finite_outline_reach(c, 5e-5);
        let reach = finite_outline_reach(c, 0.02);
        assert!(reach <= 0.6 + 0.02, "outline reaches {reach}");
    }
}
