//! Geometric clipping: coverage inside a layer clip is the exact area of
//! `shape ∩ clip`, never a product of two independent coverages.
//!
//! Vertex and crossing ordinates divide the two operands into bands where
//! their edges cannot change horizontal order. Each band's winding intervals
//! are intersected and emitted as consistently oriented trapezoids. Original
//! contour directions affect the operand's fill rule, never the output
//! direction. The complete closed boundaries can be clipped again, and
//! [`crate::coverage::Coverage`] evaluates their union with `NonZero`.

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

/// A nonhorizontal edge, normalized top to bottom without losing its
/// original winding contribution or which operand it belongs to.
struct Edge {
    segment: Segment,
    direction: i32,
    operand: usize,
}

impl Edge {
    #[expect(clippy::float_cmp, reason = "reuse exact endpoint coordinates")]
    fn x_at(&self, y: f64) -> f64 {
        let (x0, y0, x1, y1) = self.segment;
        if y == y1 {
            x1
        } else {
            (x1 - x0).mul_add((y - y0) / (y1 - y0), x0)
        }
    }
}

/// Interior crossing ordinate. Endpoints are already band boundaries;
/// parallel/collinear edges only change their ordering at those endpoints.
fn crossing_y(a: &Edge, b: &Edge) -> Option<f64> {
    let (ax0, ay0, ax1, ay1) = a.segment;
    let (bx0, by0, bx1, by1) = b.segment;
    if ay0.max(by0) >= ay1.min(by1)
        || ax0.min(ax1).max(bx0.min(bx1)) > ax0.max(ax1).min(bx0.max(bx1))
    {
        return None;
    }
    let (adx, ady) = (ax1 - ax0, ay1 - ay0);
    let (bdx, bdy) = (bx1 - bx0, by1 - by0);
    let determinant = adx.mul_add(bdy, -ady * bdx);
    if determinant == 0.0 {
        return None;
    }
    let (dx, dy) = (bx0 - ax0, by0 - ay0);
    let ta = dx.mul_add(bdy, -dy * bdx) / determinant;
    let tb = dx.mul_add(ady, -dy * adx) / determinant;
    if ta > 0.0 && ta < 1.0 && tb > 0.0 && tb < 1.0 {
        Some(ady.mul_add(ta, ay0))
    } else {
        None
    }
}

/// Directed segments tracing the boundary of `shape ∩ clip`.
///
/// Each operand is classified separately before intersecting its filled
/// intervals. The clip uses `NonZero`; the output always uses `NonZero`,
/// including when `shape_rule` is `EvenOdd`.
///
/// # Panics
///
/// If an internal band interval closes without its opening boundary.
#[must_use]
#[expect(
    clippy::float_cmp,
    reason = "horizontal edges and coincident scanline crossings are geometric equalities"
)]
pub fn intersect_edges(shape: &[Segment], shape_rule: FillRule, clip: &[Segment]) -> Vec<Segment> {
    let mut edges = Vec::new();
    let mut ordinates = Vec::new();
    for (operand, segments) in [shape, clip].into_iter().enumerate() {
        for &(x0, y0, x1, y1) in segments {
            if y0 == y1 {
                continue;
            }
            let (segment, direction) = if y0 < y1 {
                ((x0, y0, x1, y1), 1)
            } else {
                ((x1, y1, x0, y0), -1)
            };
            edges.push(Edge {
                segment,
                direction,
                operand,
            });
            ordinates.extend([y0, y1]);
        }
    }
    // Self-crossings matter too: they can change either operand's filled
    // intervals without crossing an edge of the other operand.
    for (index, edge) in edges.iter().enumerate() {
        for other in &edges[index + 1..] {
            if let Some(y) = crossing_y(edge, other) {
                ordinates.push(y);
            }
        }
    }
    ordinates.sort_by(f64::total_cmp);
    ordinates.dedup();

    let mut out = Vec::new();
    let mut crossings = Vec::new();
    for band in ordinates.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        let middle = top.midpoint(bottom);
        crossings.clear();
        for (index, edge) in edges.iter().enumerate() {
            if edge.segment.1 <= top && edge.segment.3 >= bottom {
                crossings.push((edge.x_at(middle), index));
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = [0; 2];
        let mut left = None;
        let mut index = 0;
        while index < crossings.len() {
            let (x, boundary) = crossings[index];
            let was_inside =
                inside(winding[0], shape_rule) && inside(winding[1], FillRule::NonZero);
            // Apply coincident crossings together, so a shared boundary
            // cannot open a zero-width intersection between operands.
            while index < crossings.len() && crossings[index].0 == x {
                let edge = &edges[crossings[index].1];
                winding[edge.operand] += edge.direction;
                index += 1;
            }
            let is_inside = inside(winding[0], shape_rule) && inside(winding[1], FillRule::NonZero);
            if !was_inside && is_inside {
                left = Some(boundary);
            } else if was_inside && !is_inside {
                let left = &edges[left.take().expect("an interval has a left boundary")];
                let right = &edges[boundary];
                let (lt, lb) = (left.x_at(top), left.x_at(bottom));
                let (rt, rb) = (right.x_at(top), right.x_at(bottom));
                out.extend([
                    (lt, top, rt, top),
                    (rt, top, rb, bottom),
                    (rb, bottom, lb, bottom),
                    (lb, bottom, lt, top),
                ]);
            }
        }
    }
    out
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

    #[test]
    fn an_intersection_keeps_closed_boundaries_for_another_clip() {
        let identity = kurbo::Affine::IDENTITY;
        let shape = rect_edges(kurbo::Rect::new(1.0, 1.0, 13.0, 13.0), identity);
        let mut first = rect_edges(kurbo::Rect::new(0.0, 4.0, 14.0, 10.0), identity);
        for edge in &mut first {
            *edge = (edge.2, edge.3, edge.0, edge.1);
        }
        let second = rect_edges(kurbo::Rect::new(5.0, 0.0, 9.0, 14.0), identity);
        let intersection = intersect_edges(&shape, FillRule::NonZero, &first);
        let intersection = intersect_edges(&intersection, FillRule::NonZero, &second);
        let mut coverage = Coverage::new(16, 16);
        for (x0, y0, x1, y1) in intersection {
            coverage.add_line(x0, y0, x1, y1);
        }
        let pixels = coverage.finish(FillRule::NonZero);
        for y in 0..16 {
            for x in 0..16 {
                let expected = if (5..9).contains(&x) && (4..10).contains(&y) {
                    1.0
                } else {
                    0.0
                };
                assert!(
                    (pixels[y * 16 + x] - expected).abs() < 1e-9,
                    "second clip pixel ({x},{y}): {}",
                    pixels[y * 16 + x]
                );
            }
        }
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
