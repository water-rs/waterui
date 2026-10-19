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
    // Where a clip edge runs along a shape edge in the same direction, the
    // shape pass already keeps that boundary (its midpoint lies on the
    // clip); keeping the clip's portion too would trace it twice. The two
    // passes split at different parameters, so under a rotation their
    // endpoints differ in the last bits and only geometry can tell them
    // apart. Opposite directions are kept: they cancel, as the boundary of
    // shapes that only touch must.
    out.extend(
        clipped(clip, shape, shape_rule)
            .into_iter()
            .filter(|&segment| !runs_along(shape, segment)),
    );
    out
}

/// Whether `segment`'s midpoint lies on an edge of `edges` that points the
/// same way.
fn runs_along(edges: &[Segment], segment: Segment) -> bool {
    let (x0, y0, x1, y1) = segment;
    let (mx, my) = (x0.midpoint(x1), y0.midpoint(y1));
    edges.iter().any(|&(ex0, ey0, ex1, ey1)| {
        (x1 - x0).mul_add(ex1 - ex0, (y1 - y0) * (ey1 - ey0)) > 0.0
            && point_seg_dist(mx, my, ex0, ey0, ex1, ey1) < 1e-9
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::Coverage;

    /// `rect`'s boundary carried through `transform`, as directed edges.
    fn rect_edges(rect: kurbo::Rect, transform: kurbo::Affine) -> Vec<Segment> {
        let corners = [
            (rect.x0, rect.y0),
            (rect.x1, rect.y0),
            (rect.x1, rect.y1),
            (rect.x0, rect.y1),
        ]
        .map(|corner| transform * kurbo::Point::from(corner));
        (0..4)
            .map(|index| {
                let (from, to) = (corners[index], corners[(index + 1) % 4]);
                (from.x, from.y, to.x, to.y)
            })
            .collect()
    }

    /// A band spanning a rotated clip's full width shares two of its
    /// edges; the intersection traces each shared edge once, so its
    /// coverage is exactly the band's area and nothing streaks outside it.
    #[test]
    fn a_shape_sharing_a_rotated_clips_edges_covers_its_own_area() {
        let transform = kurbo::Affine::translate((40.0, 4.0))
            * kurbo::Affine::rotate(0.4)
            * kurbo::Affine::scale(0.9);
        let clip = rect_edges(kurbo::Rect::new(24.0, 16.0, 120.0, 128.0), transform);
        let band = rect_edges(kurbo::Rect::new(24.0, 40.0, 120.0, 44.0), transform);
        let mut coverage = Coverage::new(160, 160);
        for (x0, y0, x1, y1) in intersect_edges(&band, FillRule::NonZero, &clip) {
            coverage.add_line(x0, y0, x1, y1);
        }
        let area: f64 = coverage.finish(FillRule::NonZero).iter().sum();
        let expected = 96.0 * 4.0 * 0.81;
        assert!(
            (area - expected).abs() < 1e-6,
            "covered area {area}, the band's is {expected}"
        );
    }
}
