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
        // Every row boundary is evaluated from the segment origin:
        // repeatedly adding to an absolute x accumulates rounding error
        // over tall edges.
        let line = OriginLine::new((x1 - x0) / (y1 - y0), x0);
        let mut x = if y0 < 0.0 { line.at(-y0) } else { x0 };
        let y_start = y0.max(0.0) as usize;
        let y_end = self.h.min((y1.ceil() as usize).min(self.h));
        for y in y_start..y_end {
            let row = y * (self.w + 1);
            let bottom = ((y + 1) as f32).min(y1);
            let dy = bottom - (y as f32).max(y0);
            let xnext = line.at(bottom - y0);
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

/// `x(t) = origin + slope * t`, rounded once to `f32` — the value
/// `slope.mul_add(t, origin)` computes.
///
/// Where the target guarantees a fused multiply-add instruction, that
/// instruction evaluates it. Elsewhere `f32::mul_add` lowers to a libm
/// call per row, so the fused result is formed exactly in `f64` instead:
/// the product of two `f32` significands (24 + 24 bits) is exact in `f64`,
/// and the rounded `f64` sum rounds to the fused `f32` result unless it
/// sits exactly on an `f32` rounding midpoint without being exact, which
/// the sum's exact residual resolves.
#[derive(Clone, Copy)]
struct OriginLine {
    #[cfg(any(target_arch = "aarch64", target_feature = "fma"))]
    slope: f32,
    #[cfg(any(target_arch = "aarch64", target_feature = "fma"))]
    origin: f32,
    #[cfg(not(any(target_arch = "aarch64", target_feature = "fma")))]
    slope: f64,
    #[cfg(not(any(target_arch = "aarch64", target_feature = "fma")))]
    origin: f64,
}

#[cfg(any(target_arch = "aarch64", target_feature = "fma"))]
impl OriginLine {
    const fn new(slope: f32, origin: f32) -> Self {
        Self { slope, origin }
    }

    #[inline]
    fn at(self, t: f32) -> f32 {
        self.slope.mul_add(t, self.origin)
    }
}

#[cfg(not(any(target_arch = "aarch64", target_feature = "fma")))]
impl OriginLine {
    fn new(slope: f32, origin: f32) -> Self {
        Self {
            slope: f64::from(slope),
            origin: f64::from(origin),
        }
    }

    #[inline]
    fn at(self, t: f32) -> f32 {
        fused_via_f64(self.slope, self.origin, t)
    }
}

/// `slope * t + origin` rounded once to `f32`, for `slope` and `origin`
/// widened from `f32`.
#[cfg(any(test, not(any(target_arch = "aarch64", target_feature = "fma"))))]
#[inline]
#[expect(
    clippy::cast_possible_truncation,
    clippy::suboptimal_flops,
    reason = "the exact f64 product and its rounded sum are the evaluation"
)]
fn fused_via_f64(slope: f64, origin: f64, t: f32) -> f32 {
    let product = slope * f64::from(t);
    let sum = product + origin;
    // Rounding a normal f32 drops the low 29 of f64's 52 fraction bits;
    // they read exactly 1 followed by zeros only at a rounding midpoint.
    // Subnormal f32 results drop more bits and take the exact path.
    if sum.to_bits() & 0x1fff_ffff != 0x1000_0000 && sum.abs() >= f64::from(f32::MIN_POSITIVE) {
        return sum as f32;
    }
    fused_at_midpoint(product, origin, sum)
}

/// The once-rounded `f32` of `product + origin` from its rounded `f64` sum
/// `sum`, for a `sum` that may sit on an `f32` rounding midpoint.
#[cfg(any(test, not(any(target_arch = "aarch64", target_feature = "fma"))))]
#[cold]
#[expect(
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    reason = "exact float comparisons decide the rounding"
)]
fn fused_at_midpoint(product: f64, origin: f64, sum: f64) -> f32 {
    let rounded = sum as f32;
    if f64::from(rounded) == sum || !sum.is_finite() {
        return rounded;
    }
    // The other f32 neighbour of `sum`; their midpoint is exact in f64.
    let other = if f64::from(rounded) < sum {
        rounded.next_up()
    } else {
        rounded.next_down()
    };
    if sum != 0.5 * (f64::from(rounded) + f64::from(other)) {
        return rounded;
    }
    // Exact residual of the f64 sum (TwoSum): the exact value lies on the
    // residual's side of the midpoint, or on it when the residual is zero.
    let b = sum - product;
    let residual = (product - (sum - b)) + (origin - b);
    let (low, high) = if rounded < other {
        (rounded, other)
    } else {
        (other, rounded)
    };
    if residual > 0.0 {
        high
    } else if residual < 0.0 {
        low
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fused_via_f64` against `f32::mul_add` (a hardware or libm fused
    /// multiply-add): on f32 midpoints the rounded f64 sum hides the
    /// residual, on subnormal results, and on random row evaluations.
    #[test]
    fn f64_evaluation_is_the_fused_multiply_add() {
        let up = 1.0 + f32::EPSILON;
        let down = 1.0 - f32::EPSILON;
        let mut cases = vec![
            // 2^24 + 3 is an f32 midpoint; the exact value is 2^-46 below it.
            (up, down, 16_777_218.0_f32),
            (up, down, -16_777_218.0),
            (-up, down, 16_777_218.0),
            (up, up, 16_777_218.0),
            (down, down, 16_777_216.0),
            (f32::MIN_POSITIVE, 0.75, f32::MIN_POSITIVE * 0.25),
            (f32::MIN_POSITIVE, -0.5, f32::MIN_POSITIVE),
            (1.5, 2.0, -3.0),
            (-0.0, 1.0, 0.0),
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..200_000 {
            let bits = next();
            // Slopes and origins of every magnitude a raster edge can carry,
            // including the ones that put sums on midpoints.
            let slope = f32::from_bits((bits as u32 & 0x807f_ffff) | ((112 + (bits >> 32) as u32 % 30) << 23));
            let origin = f32::from_bits((next() as u32 & 0x807f_ffff) | ((120 + (bits >> 40) as u32 % 20) << 23));
            let t = f32::from_bits((next() as u32 & 0x007f_ffff) | ((120 + (bits >> 48) as u32 % 20) << 23));
            cases.push((slope, t, origin));
            // A product of half an ulp of `origin` minus m²·2⁻⁴⁶ of it: the
            // f64 sum rounds onto the f32 midpoint for every m < 362.
            let m = 1 + (bits >> 20) as u32 % 300;
            let scale = (origin.to_bits() >> 23 & 0xff) as i32 - 127 - 24;
            let half_ulp = 2.0_f32.powi(scale);
            let skew = m as f32 * f32::EPSILON;
            for (a, b) in [(1.0 + skew, 1.0 - skew), (-(1.0 + skew), 1.0 - skew)] {
                cases.push((a * half_ulp, b, origin));
            }
        }
        for (slope, t, origin) in cases {
            let fused = slope.mul_add(t, origin);
            let evaluated = fused_via_f64(f64::from(slope), f64::from(origin), t);
            assert_eq!(
                evaluated.to_bits(),
                fused.to_bits(),
                "{slope:e} * {t:e} + {origin:e}: {evaluated:e} versus {fused:e}"
            );
        }
    }

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
