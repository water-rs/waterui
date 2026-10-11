//! Geometric clipping: coverage inside a layer clip is the exact area of
//! `shape ∩ clip`, never a product of two independent coverages.
//!
//! The boundary of `shape ∩ clip` is traced by the parts of the shape's
//! edges that lie inside the clip polygon, together with the parts of the
//! clip's edges that lie inside the shape. This module splits each edge set
//! at pairwise intersections, keeps the sub-segments whose midpoint is inside
//! the other polygon (winding test under that polygon's fill rule), and
//! returns the surviving directed segments. Accumulating them with the
//! `NonZero` rule in [`crate::coverage::Coverage`] yields the exact
//! intersection coverage — engines that merely multiply coverage images show
//! the difference as measured error near intersecting partial edges.

use cherenkov_scene::FillRule;

/// An undirected-or-directed line segment.
pub type Segment = (f64, f64, f64, f64);

/// Winding number of `edges` at point `(px, py)` (standard crossing rule,
/// following the convention of `font-rs`/PostScript: upward edges crossing to
/// the right of the point count `+1`).
#[must_use]
pub fn winding(edges: &[Segment], px: f64, py: f64) -> i32 {
    let mut w = 0;
    for &(x0, y0, x1, y1) in edges {
        if y0 <= py {
            if y1 > py && is_left(x0, y0, x1, y1, px, py) > 0.0 {
                w += 1;
            }
        } else if y1 <= py && is_left(x0, y0, x1, y1, px, py) < 0.0 {
            w -= 1;
        }
    }
    w
}

/// Whether `winding` satisfies `rule`.
#[must_use]
pub const fn inside(winding: i32, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}

pub(crate) fn is_left(x0: f64, y0: f64, x1: f64, y1: f64, px: f64, py: f64) -> f64 {
    (px - x0).mul_add(-(y1 - y0), (x1 - x0) * (py - y0))
}

/// The `t` parameters along `(x0,y0)-(x1,y1)` where it crosses `clip` edges.
fn intersection_params(seg: Segment, clip_edges: &[Segment]) -> Vec<f64> {
    let (x0, y0, x1, y1) = seg;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let mut ts = Vec::new();
    for &(cx0, cy0, cx1, cy1) in clip_edges {
        let cdx = cx1 - cx0;
        let cdy = cy1 - cy0;
        let denom = dy.mul_add(-cdx, dx * cdy);
        if denom.abs() < 1e-12 {
            // Parallel or collinear: no clean crossing, but a collinear
            // overlap still needs splits at the clip edge's endpoints so
            // the boundary-on-edge portions can be kept individually.
            let len2 = dy.mul_add(dy, dx * dx);
            let len = len2.sqrt();
            if len > 1e-12 {
                for &(ex, ey) in &[(cx0, cy0), (cx1, cy1)] {
                    let perp = (ey - y0).mul_add(-dx, (ex - x0) * dy);
                    if perp.abs() <= 1e-9 * len.max(1.0) {
                        let t = (ey - y0).mul_add(dy, (ex - x0) * dx) / len2;
                        if t > 1e-9 && t < 1.0 - 1e-9 {
                            ts.push(t);
                        }
                    }
                }
            }
            continue;
        }
        let t = (cy0 - y0).mul_add(-cdx, (cx0 - x0) * cdy) / denom;
        let u = (cy0 - y0).mul_add(-dx, (cx0 - x0) * dy) / denom;
        if t > 1e-9 && t < 1.0 - 1e-9 && (-1e-9..=1.0 + 1e-9).contains(&u) {
            ts.push(t);
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    ts
}

/// Sub-segments of `subject` lying inside `clip_edges` under `rule`.
fn clipped(subject: &[Segment], clip_edges: &[Segment], rule: FillRule) -> Vec<Segment> {
    let mut out = Vec::new();
    for &seg in subject {
        let (x0, y0, x1, y1) = seg;
        let dx = x1 - x0;
        let dy = y1 - y0;
        if dx == 0.0 && dy == 0.0 {
            continue;
        }
        let mut ts = vec![0.0];
        ts.extend(intersection_params(seg, clip_edges));
        ts.push(1.0);
        for w in ts.windows(2) {
            let (ta, tb) = (w[0], w[1]);
            if tb - ta < 1e-12 {
                continue;
            }
            let tm = ta.midpoint(tb);
            let (mx, my) = (dx.mul_add(tm, x0), dy.mul_add(tm, y0));
            // A midpoint exactly on a boundary edge belongs to the
            // intersection boundary: the winding convention is ambiguous
            // there, so count it as inside.
            if inside(winding(clip_edges, mx, my), rule) || on_edge(clip_edges, mx, my) {
                out.push((
                    dx.mul_add(ta, x0),
                    dy.mul_add(ta, y0),
                    dx.mul_add(tb, x0),
                    dy.mul_add(tb, y0),
                ));
            }
        }
    }
    out
}

/// Directed segments tracing the boundary of `shape ∩ clip`.
///
/// `shape` edges keep their direction and `shape_rule` decides "inside the
/// shape" for clip-edge portions. `clip` is always `NonZero`: a layer clip is a
/// solid region.
#[must_use]
pub fn intersect_edges(shape: &[Segment], shape_rule: FillRule, clip: &[Segment]) -> Vec<Segment> {
    let mut out = clipped(shape, clip, FillRule::NonZero);
    out.extend(clipped(clip, shape, shape_rule));
    // Where shape and clip edges coincide, both passes can emit the same
    // directed segment; keep each once so coverage is not doubled.
    out.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
    });
    out.dedup();
    out
}

/// Distance from `(px, py)` to segment `(x0,y0)-(x1,y1)`.
fn point_seg_dist(px: f64, py: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len2 = dy.mul_add(dy, dx * dx);
    if len2 < 1e-24 {
        return (px - x0).hypot(py - y0);
    }
    let t = ((py - y0).mul_add(dy, (px - x0) * dx) / len2).clamp(0.0, 1.0);
    (px - dx.mul_add(t, x0)).hypot(py - dy.mul_add(t, y0))
}

/// Whether `(px, py)` lies on (within a hair of) any edge in `edges`.
fn on_edge(edges: &[Segment], px: f64, py: f64) -> bool {
    edges
        .iter()
        .any(|&(x0, y0, x1, y1)| point_seg_dist(px, py, x0, y0, x1, y1) < 1e-9)
}
