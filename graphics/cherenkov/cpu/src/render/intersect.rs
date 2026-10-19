//! Geometric clipping: a draw's coverage under a layer clip is the area of
//! `shape ∩ clip`, never a product of two independent coverages.
//!
//! The boundary of `shape ∩ clip` is traced by the parts of the shape's
//! edges that lie inside the clip polygon, together with the parts of the
//! clip's edges that lie inside the shape. Each edge set is split at
//! pairwise intersections and the sub-segments whose midpoint is inside
//! the other polygon (winding test under that polygon's fill rule)
//! survive. Accumulating them under the shape's own rule yields the
//! intersection coverage — the same construction the oracle applies,
//! where the coverage product a coverage mask implies shows as measured
//! error near edges the two boundaries share.
//!
//! Ported from `cherenkov-oracle`'s `clip.rs` and kept line-compatible:
//! the engines may not depend on the independent reference.

use cherenkov::FillRule;

use super::raster::Edge;

/// A directed line segment in device space.
pub type Segment = (f64, f64, f64, f64);

/// `edge` placed at device offset `(ox, oy)`, widened to f64.
#[must_use]
pub fn placed(edge: Edge, ox: f64, oy: f64) -> Segment {
    (
        f64::from(edge.x0) + ox,
        f64::from(edge.y0) + oy,
        f64::from(edge.x1) + ox,
        f64::from(edge.y1) + oy,
    )
}

/// The `segment` endpoints narrowed back to rasterizer precision.
#[expect(
    clippy::cast_possible_truncation,
    reason = "device coordinates stay far below f32's integer bound"
)]
#[must_use]
pub const fn edge(segment: Segment) -> Edge {
    Edge {
        x0: segment.0 as f32,
        y0: segment.1 as f32,
        x1: segment.2 as f32,
        y1: segment.3 as f32,
    }
}

/// Winding number of `edges` at point `(px, py)` (standard crossing rule,
/// following the convention of `font-rs`/PostScript: upward edges crossing
/// to the right of the point count `+1`).
fn winding(edges: &[Segment], px: f64, py: f64) -> i32 {
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
const fn inside(winding: i32, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}

fn is_left(x0: f64, y0: f64, x1: f64, y1: f64, px: f64, py: f64) -> f64 {
    (px - x0).mul_add(-(y1 - y0), (x1 - x0) * (py - y0))
}

/// Axis-aligned bounds of `segment` as `(x0, y0, x1, y1)`.
const fn bounds(seg: Segment) -> (f64, f64, f64, f64) {
    (
        seg.0.min(seg.2),
        seg.1.min(seg.3),
        seg.0.max(seg.2),
        seg.1.max(seg.3),
    )
}

/// Whether two segment bounds can share a point.
fn bounds_overlap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3
}

/// The `t` parameters along `(x0,y0)-(x1,y1)` where it crosses `clip` edges
/// (paired with their bounds: disjoint bounds cannot meet).
fn intersection_params(seg: Segment, clip_edges: &[Segment], clip_bounds: &Bounds) -> Vec<f64> {
    let (x0, y0, x1, y1) = seg;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let own = bounds(seg);
    let mut ts = Vec::new();
    for (i, &(cx0, cy0, cx1, cy1)) in clip_edges.iter().enumerate() {
        if !bounds_overlap(own, clip_bounds.get(i)) {
            continue;
        }
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

/// Per-edge bounds of a boundary, computed once per call.
#[derive(Default)]
struct Bounds(Vec<(f64, f64, f64, f64)>);

impl Bounds {
    fn of(edges: &[Segment]) -> Self {
        Self(edges.iter().map(|&s| bounds(s)).collect())
    }

    fn get(&self, i: usize) -> (f64, f64, f64, f64) {
        self.0[i]
    }
}

/// Sub-segments of `subject` lying inside `clip_edges` under `rule`.
fn clipped(
    subject: &[Segment],
    clip_edges: &[Segment],
    clip_bounds: &Bounds,
    rule: FillRule,
) -> Vec<Segment> {
    let mut out = Vec::new();
    for &seg in subject {
        let (x0, y0, x1, y1) = seg;
        let dx = x1 - x0;
        let dy = y1 - y0;
        if dx == 0.0 && dy == 0.0 {
            continue;
        }
        let mut ts = vec![0.0];
        ts.extend(intersection_params(seg, clip_edges, clip_bounds));
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
/// shape" for clip-edge portions. `clip` is always `NonZero`: a layer clip
/// is a solid region.
#[must_use]
pub fn intersect_edges(shape: &[Segment], shape_rule: FillRule, clip: &[Segment]) -> Vec<Segment> {
    let shape_bounds = Bounds::of(shape);
    let clip_bounds = Bounds::of(clip);
    let mut out = clipped(shape, clip, &clip_bounds, FillRule::NonZero);
    // Where a clip edge runs along a shape edge in the same direction, the
    // shape pass already keeps that boundary (its midpoint lies on the
    // clip); keeping the clip's portion too would trace it twice. The two
    // passes split at different parameters, so under a rotation their
    // endpoints differ in the last bits and only geometry can tell them
    // apart. Opposite directions are kept: they cancel, as the boundary of
    // shapes that only touch must.
    out.extend(
        clipped(clip, shape, &shape_bounds, shape_rule)
            .into_iter()
            .filter(|&segment| !runs_along(shape, segment)),
    );
    out
}

/// Whether `box` (axis-aligned bounds `(x0, y0, x1, y1)`) touches the
/// boundary band of `clip` — some clip edge's box intersecting it. A draw
/// whose box it does not touch is entirely inside or outside the clip.
#[must_use]
pub fn boundary_touches(clip: &[Segment], box_: (f64, f64, f64, f64)) -> bool {
    clip.iter()
        .map(|&s| bounds(s))
        .any(|b| bounds_overlap(b, box_))
}

/// Whether the point `(px, py)` is inside `clip` (`NonZero`, including its
/// boundary).
#[must_use]
pub fn point_inside(clip: &[Segment], px: f64, py: f64) -> bool {
    inside(winding(clip, px, py), FillRule::NonZero) || on_edge(clip, px, py)
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
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use cherenkov::kurbo;

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
    /// edges; the intersection traces each shared edge once, so the
    /// boundary keeps the band's shape and nothing leaks outside it. This
    /// is the corpus `filter-overscroll-stretch-rotated-p3` geometry of
    /// water-rs/waterui#2933.
    #[test]
    fn a_shape_sharing_a_rotated_clips_edges_keeps_its_own_boundary() {
        let transform = kurbo::Affine::translate((40.0, 4.0))
            * kurbo::Affine::rotate(0.4)
            * kurbo::Affine::scale(0.9);
        let clip = rect_edges(kurbo::Rect::new(24.0, 16.0, 120.0, 128.0), transform);
        let band = rect_edges(kurbo::Rect::new(24.0, 40.0, 120.0, 44.0), transform);
        let segs = intersect_edges(&band, FillRule::NonZero, &clip);
        // The band's whole boundary survives: it lies inside the clip,
        // and every clip-edge piece inside the band runs along a band
        // edge in the same direction.
        assert_eq!(segs.len(), band.len());
    }

    /// A shape reaching past the clip keeps only the inside pieces plus
    /// the clip boundary running through it.
    #[test]
    fn an_overrunning_shape_takes_the_clip_edge() {
        let clip = rect_edges(
            kurbo::Rect::new(0.0, 0.0, 10.0, 10.0),
            kurbo::Affine::IDENTITY,
        );
        let shape = rect_edges(
            kurbo::Rect::new(5.0, -5.0, 15.0, 5.0),
            kurbo::Affine::IDENTITY,
        );
        let segs = intersect_edges(&shape, FillRule::NonZero, &clip);
        // The intersection is the 5×10 rect [5,10]×[0,5]: four boundary
        // segments, none reaching outside the clip.
        assert_eq!(segs.len(), 4);
        for &s in &segs {
            for &(x, y) in &[(s.0, s.1), (s.2, s.3)] {
                assert!((-1e-9..=10.0 + 1e-9).contains(&x) && (-1e-9..=10.0 + 1e-9).contains(&y));
            }
        }
    }
}
