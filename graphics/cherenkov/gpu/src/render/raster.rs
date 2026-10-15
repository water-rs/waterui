//! The exact-area coverage rasterizer for glyph outlines.
//!
//! A port of the accumulation rasterizer from font-rs (`raster.rs`): every
//! flattened polygon edge accumulates a signed area into a `(w + 1) * h`
//! buffer, then each row is prefix-summed and clamped to a non-zero-rule
//! coverage. The winding accumulation makes overlapping contours cancel
//! correctly.

/// A `w` × `h` signed-area accumulation buffer.
#[derive(Debug)]
pub struct Raster {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Raster {
    /// A raster of `w` × `h` cells.
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            a: vec![0.0; (w + 1) * h],
        }
    }

    /// Accumulates the signed area of the segment `p0 -> p1`.
    #[expect(
        clippy::similar_names,
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::suboptimal_flops,
        reason = "a direct port of font-rs's scanline area accounting"
    )]
    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if (y0 - y1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, x0, y0, x1, y1) = if y0 < y1 {
            (1.0, x0, y0, x1, y1)
        } else {
            (-1.0, x1, y1, x0, y0)
        };
        let dxdy = (x1 - x0) / (y1 - y0);
        let mut x = if y0 < 0.0 {
            dxdy.mul_add(-y0, x0)
        } else {
            x0
        };
        let y_start = y0.max(0.0) as usize;
        let y_end = self.h.min((y1.ceil() as usize).min(self.h));
        for y in y_start..y_end {
            let row = y * (self.w + 1);
            let bottom = ((y + 1) as f32).min(y1);
            let dy = bottom - (y as f32).max(y0);
            // Evaluate from the segment origin: repeatedly adding to an
            // absolute x accumulates rounding error over tall edges.
            let xnext = dxdy.mul_add(bottom - y0, x0);
            let d = dy * dir;
            let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let xa_floor = xa.floor();
            let xa_i = xa_floor as i32;
            let xb_ceil = xb.ceil();
            let xb_i = xb_ceil as i32;
            if xb_i <= xa_i + 1 {
                // The piece stays within one cell column.
                let xmf = x.midpoint(xnext) - xa_floor;
                let cell = row as isize + xa_i as isize;
                if cell >= 0 && (cell as usize) < self.a.len() {
                    self.a[cell as usize] += d - d * xmf;
                    if (cell as usize) + 1 < self.a.len() {
                        self.a[cell as usize + 1] += d * xmf;
                    }
                }
            } else {
                let s = (xb - xa).recip();
                let xa_f = xa - xa_floor;
                let a0 = 0.5 * s * (1.0 - xa_f) * (1.0 - xa_f);
                let xb_f = xb - xb_ceil + 1.0;
                let am = 0.5 * s * xb_f * xb_f;
                let cell = row as isize + xa_i as isize;
                if cell < 0 {
                    x = xnext;
                    continue;
                }
                let cell = cell as usize;
                self.a[cell] += d * a0;
                if xb_i == xa_i + 2 {
                    self.a[cell + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - xa_f);
                    self.a[cell + 1] += d * (a1 - a0);
                    let mut xi = xa_i + 2;
                    while xi < xb_i - 1 {
                        let c = row as isize + xi as isize;
                        if c >= 0 && (c as usize) < self.a.len() {
                            self.a[c as usize] += d * s;
                        }
                        xi += 1;
                    }
                    let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
                    let last = row as isize + (xb_i - 1) as isize;
                    if last >= 0 && (last as usize) < self.a.len() {
                        self.a[last as usize] += d * (1.0 - a2 - am);
                    }
                    let xb_cell = row as isize + xb_i as isize;
                    if xb_cell >= 0 && (xb_cell as usize) < self.a.len() {
                        self.a[xb_cell as usize] += d * am;
                    }
                    x = xnext;
                    continue;
                }
                let xb_cell = row as isize + xb_i as isize;
                if xb_cell >= 0 && (xb_cell as usize) < self.a.len() {
                    self.a[xb_cell as usize] += d * am;
                }
            }
            x = xnext;
        }
    }

    /// The accumulated coverage per cell, non-zero winding rule.
    pub fn coverage(&self) -> Vec<f32> {
        self.coverage_rule(cherenkov::FillRule::NonZero)
    }

    /// The accumulated coverage per cell under `rule`.
    ///
    /// Non-zero clamps the winding magnitude; even-odd folds it into a
    /// triangle wave of period 2 so odd windings cover and even ones don't.
    pub fn coverage_rule(&self, rule: cherenkov::FillRule) -> Vec<f32> {
        let mut out = vec![0.0; self.w * self.h];
        for y in 0..self.h {
            let row = y * (self.w + 1);
            let mut acc = 0.0;
            for x in 0..self.w {
                acc += self.a[row + x];
                out[y * self.w + x] = match rule {
                    cherenkov::FillRule::NonZero => acc.abs().min(1.0),
                    cherenkov::FillRule::EvenOdd => {
                        let a = acc.abs() % 2.0;
                        1.0 - (a - 1.0).abs()
                    }
                };
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tall_edges_agree_with_independent_pixel_area() {
        let cases = [
            (
                [(900.0_f32, 0.0_f32), (512.0, 900.0), (900.0, 900.0)],
                (598, 699),
            ),
            (
                [
                    (689.4518, 1.803_737_2),
                    (532.2187, 337.7357),
                    (668.394_17, 2.898_188_6),
                ],
                (534, 332),
            ),
        ];
        for (vertices, (px, py)) in cases {
            for reverse in [false, true] {
                let mut raster = Raster::new(901, 901);
                // Translate only the independent f64 oracle to the target pixel.
                // The rasterizer must traverse the full height of each edge.
                let mut reference = cherenkov_oracle::coverage::Coverage::new(1, 1);
                for i in 0..vertices.len() {
                    let (mut a, mut b) = (vertices[i], vertices[(i + 1) % vertices.len()]);
                    if reverse {
                        std::mem::swap(&mut a, &mut b);
                    }
                    raster.draw_line(a.0, a.1, b.0, b.1);
                    reference.add_line(
                        f64::from(a.0) - f64::from(px),
                        f64::from(a.1) - f64::from(py),
                        f64::from(b.0) - f64::from(px),
                        f64::from(b.1) - f64::from(py),
                    );
                }
                let expected = reference.finish(cherenkov_scene::FillRule::NonZero)[0];
                let index = usize::try_from(py * 901 + px).expect("pixel index");
                for rule in [cherenkov::FillRule::NonZero, cherenkov::FillRule::EvenOdd] {
                    let actual = f64::from(raster.coverage_rule(rule)[index]);
                    assert!(
                        (actual - expected).abs() < 0.0002,
                        "{rule:?} reverse={reverse} ({px},{py}): {actual} versus {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_half_pixel_square_has_exact_half_coverage() {
        // A 9x9 square at (0.5, 0.5) - (9.5, 9.5) in a 10x10 raster: every
        // boundary row/column is half covered.
        let mut raster = Raster::new(10, 10);
        let edges = [
            (0.5, 0.5, 9.5, 0.5),
            (9.5, 0.5, 9.5, 9.5),
            (9.5, 9.5, 0.5, 9.5),
            (0.5, 9.5, 0.5, 0.5),
        ];
        for (x0, y0, x1, y1) in edges {
            raster.draw_line(x0, y0, x1, y1);
        }
        let cov = raster.coverage();
        let at = |x: usize, y: usize| cov[y * 10 + x];
        for i in 1..9 {
            assert!((at(i, 0) - 0.5).abs() < 1e-6, "top edge {i}: {}", at(i, 0));
            assert!((at(i, 9) - 0.5).abs() < 1e-6, "bottom edge {i}");
            assert!((at(0, i) - 0.5).abs() < 1e-6, "left edge {i}");
            assert!((at(9, i) - 0.5).abs() < 1e-6, "right edge {i}");
        }
        for y in 1..9 {
            for x in 1..9 {
                assert!((at(x, y) - 1.0).abs() < 1e-6, "interior ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_triangles_total_coverage_matches_its_area() {
        // Right triangle (1,1)-(9,1)-(9,9): area = 32 pixels.
        let mut raster = Raster::new(16, 16);
        raster.draw_line(1.0, 1.0, 9.0, 1.0);
        raster.draw_line(9.0, 1.0, 9.0, 9.0);
        raster.draw_line(9.0, 9.0, 1.0, 1.0);
        let total: f32 = raster.coverage().iter().sum();
        assert!((total - 32.0).abs() < 1e-3, "triangle area: {total}");
    }
}
