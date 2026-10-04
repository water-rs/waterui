//! Flattening of `kurbo` paths into polylines of `(f64, f64)` points, and
//! expansion of strokes into outlines with `kurbo::stroke`.

use cherenkov_scene::{Shape, StrokeStyle};
use kurbo::{Affine, BezPath, PathEl, Point};

use crate::coverage::FLATTEN_TOLERANCE;

/// Per-half tolerance inside the shape→polyline pipeline: the curve
/// approximation and the cubic flattening each get half of
/// [`FLATTEN_TOLERANCE`] so the combined deviation stays below it.
pub const SEGMENT_TOLERANCE: f64 = FLATTEN_TOLERANCE * 0.5;

/// The largest singular value of `t`'s linear part — the worst-case factor
/// by which a user-space distance error can grow under the transform.
///
/// `σmax²` is the larger eigenvalue of `MᵀM` for `M = [[a, c], [b, d]]`.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "a/b/c/d are the conventional affine matrix coefficient names"
)]
pub fn sigma_max(t: Affine) -> f64 {
    let [a, b, c, d, _, _] = t.as_coeffs();
    // Characteristic polynomial of MᵀM: σmax² = (p + √(p² − 4det²)) / 2
    // where p = tr(MᵀM) and det² = det(M)² = det(MᵀM).
    let p = a.mul_add(a, b * b) + c.mul_add(c, d * d);
    let det = a.mul_add(d, -(b * c));
    let disc = p.mul_add(p, (-4.0 * det) * det).sqrt();
    p.midpoint(disc).sqrt()
}

/// A shape's boundary polylines in *device* space.
///
/// The shape's curves are first approximated at `SEGMENT_TOLERANCE / σmax`
/// in user space (so the approximation error stays within half the device
/// tolerance), then the transform is applied and the path flattened to
/// `SEGMENT_TOLERANCE` *device* px — tolerances therefore hold under
/// scaling.
#[must_use]
pub fn shape_polylines(shape: &Shape, tf: Affine) -> Vec<Polyline> {
    let sm = sigma_max(tf).max(1e-12);
    let tol_u = SEGMENT_TOLERANCE / sm;
    flatten_at(&(tf * shape.to_path_at(tol_u)), SEGMENT_TOLERANCE)
}

/// A stroked shape's outline polylines in *device* space (same tolerance
/// split as [`shape_polylines`]).
#[must_use]
pub fn stroke_polylines_device(shape: &Shape, style: &StrokeStyle, tf: Affine) -> Vec<Polyline> {
    let sm = sigma_max(tf).max(1e-12);
    let tol_u = SEGMENT_TOLERANCE / sm;
    let stroke: kurbo::Stroke = style.into();
    let outline = kurbo::stroke(
        shape.to_path_at(tol_u),
        &stroke,
        &kurbo::StrokeOpts::default(),
        tol_u,
    );
    flatten_at(&(tf * outline), SEGMENT_TOLERANCE)
}

/// A flattened subpath: a point list plus whether it closes back to its
/// first point.
#[derive(Clone, Debug)]
pub struct Polyline {
    /// Vertices in order.
    pub points: Vec<(f64, f64)>,
    /// Whether the subpath is closed (implicit final edge).
    pub closed: bool,
}

/// Flatten `path` into polylines at `tolerance` px.
#[must_use]
pub fn flatten_at(path: &BezPath, tolerance: f64) -> Vec<Polyline> {
    let mut out: Vec<Polyline> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    kurbo::flatten(path.clone(), tolerance, |el| match el {
        PathEl::MoveTo(p) => {
            if !current.is_empty() {
                out.push(Polyline {
                    points: std::mem::take(&mut current),
                    closed: false,
                });
            }
            current.push((p.x, p.y));
        }
        PathEl::LineTo(p) => {
            current.push((p.x, p.y));
        }
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
            // `kurbo::flatten` never emits curves.
            unreachable!("flatten emits only lines")
        }
        PathEl::ClosePath => {
            if !current.is_empty() {
                out.push(Polyline {
                    points: std::mem::take(&mut current),
                    closed: true,
                });
            }
        }
    });
    if !current.is_empty() {
        out.push(Polyline {
            points: current,
            closed: false,
        });
    }
    out
}

/// Flatten `path` into polylines at [`FLATTEN_TOLERANCE`].
#[must_use]
pub fn flatten(path: &BezPath) -> Vec<Polyline> {
    flatten_at(path, FLATTEN_TOLERANCE)
}

/// Expand a stroked path into its outline polylines.
#[must_use]
pub fn stroke_polylines(path: &BezPath, style: &StrokeStyle) -> Vec<Polyline> {
    let stroke: kurbo::Stroke = style.into();
    let opts = kurbo::StrokeOpts::default();
    let outline = kurbo::stroke(path.clone(), &stroke, &opts, FLATTEN_TOLERANCE);
    flatten(&outline)
}

/// Transform every point of `polylines` by `affine`.
#[must_use]
pub fn transformed(polylines: &[Polyline], affine: Affine) -> Vec<Polyline> {
    polylines
        .iter()
        .map(|pl| Polyline {
            points: pl
                .points
                .iter()
                .map(|&(x, y)| {
                    let p = affine * Point::new(x, y);
                    (p.x, p.y)
                })
                .collect(),
            closed: pl.closed,
        })
        .collect()
}

/// All directed edges of `polylines` as `(x0, y0, x1, y1)` segments.
///
/// Fill semantics close every subpath implicitly (the PostScript
/// convention, also used by the engine rasterizers): the last vertex
/// connects back to the first whether or not the path carried an explicit
/// `ClosePath`. This matters for curve-approximated paths whose final
/// point lands a rounding ulp short of the start — without the implicit
/// edge the boundary is an open contour and winding-based tests misjudge
/// the scanlines that fall in the gap.
#[must_use]
pub fn edges(polylines: &[Polyline]) -> Vec<(f64, f64, f64, f64)> {
    let mut out = Vec::new();
    for pl in polylines {
        for w in pl.points.windows(2) {
            out.push((w[0].0, w[0].1, w[1].0, w[1].1));
        }
        if pl.points.len() > 1 {
            let p0 = pl.points[pl.points.len() - 1];
            let p1 = pl.points[0];
            out.push((p0.0, p0.1, p1.0, p1.1));
        }
    }
    out
}
