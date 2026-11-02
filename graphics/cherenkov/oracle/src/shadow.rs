//! Gaussian blur of a coverage field — the oracle shadow model: the exact
//! coverage of the (clip-intersected) shape convolved with a true Gaussian
//! kernel in `f64`, separable, edge-clamped.
//!
//! Kernel weights are the Gaussian *integrated* over each pixel-wide tap
//! — `w_d = ½ (erf((d+½)/(σ√2)) − erf((d−½)/(σ√2)))` — rather than
//! point-sampled, and the tail is truncated at `⌈6σ⌉` where the outside
//! mass is below 10⁻⁹, so the convolution error stays under 10⁻⁴.

/// Separable Gaussian convolution of `src` (`width`×`height`).
/// `sigma <= 0` returns a copy.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "kernel radius and pixel indices are small non-negative values"
)]
pub fn gaussian_blur(src: &[f64], width: usize, height: usize, sigma: f64) -> Vec<f64> {
    if sigma <= 1e-9 || src.is_empty() {
        return src.to_vec();
    }
    let radius = (6.0 * sigma).ceil() as usize;
    let width_k = 2 * radius + 1;
    let inv = 1.0 / (sigma * std::f64::consts::SQRT_2);
    let mut kernel = Vec::with_capacity(width_k);
    let mut sum = 0.0;
    for i in 0..width_k {
        let d = i as f64 - radius as f64;
        // The Gaussian integrated over the tap's one-pixel footprint.
        let w = 0.5 * (libm::erf((d + 0.5) * inv) - libm::erf((d - 0.5) * inv));
        kernel.push(w);
        sum += w;
    }
    for w in &mut kernel {
        *w /= sum;
    }

    let mut tmp = vec![0.0; src.len()];
    for y in 0..height {
        for x in 0..width {
            let mut acc = 0.0;
            for (i, &w) in kernel.iter().enumerate() {
                let dx = i as i64 - radius as i64;
                let xx = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
                acc = w.mul_add(src[y * width + xx], acc);
            }
            tmp[y * width + x] = acc;
        }
    }
    let mut out = vec![0.0; src.len()];
    for y in 0..height {
        for x in 0..width {
            let mut acc = 0.0;
            for (i, &w) in kernel.iter().enumerate() {
                let dy = i as i64 - radius as i64;
                let yy = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
                acc = w.mul_add(tmp[yy * width + x], acc);
            }
            out[y * width + x] = acc;
        }
    }
    out
}

/// Independent covariance convolution.
///
/// The kernel integrates the transformed Gaussian over each device pixel using
/// tensor Gauss-Legendre quadrature. Source coverage has transparent padding;
/// no viewport edge is replicated.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "finite corpus dimensions and bounded kernel coordinates"
)]
pub fn affine_blur(
    src: &[f64],
    width: usize,
    height: usize,
    sigma: f64,
    matrix: [f64; 4],
) -> Vec<f64> {
    if sigma <= 1e-9 {
        return src.to_vec();
    }
    let [ma, mb, mc, md] = matrix;
    let xx = mc.mul_add(mc, ma * ma) * sigma * sigma;
    let yy = md.mul_add(md, mb * mb) * sigma * sigma;
    let xy = mc.mul_add(md, ma * mb) * sigma * sigma;
    let det = xy.mul_add(-xy, xx * yy);
    if det <= 0.0 {
        return vec![0.0; src.len()];
    }
    let rx = (6.0 * xx.sqrt()).ceil() as i32;
    let ry = (6.0 * yy.sqrt()).ceil() as i32;
    let points: [f64; 4] = [
        -0.861_136_311_594_052_6,
        -0.339_981_043_584_856_3,
        0.339_981_043_584_856_3,
        0.861_136_311_594_052_6,
    ];
    let weights: [f64; 4] = [
        0.347_854_845_137_453_8,
        0.652_145_154_862_546_1,
        0.652_145_154_862_546_1,
        0.347_854_845_137_453_8,
    ];
    let mut kernel = Vec::new();
    let mut total = 0.0;
    for dy in -ry..=ry {
        for dx in -rx..=rx {
            let mut weight = 0.0;
            for (u, wu) in points.into_iter().zip(weights) {
                for (v, wv) in points.into_iter().zip(weights) {
                    let px = u.mul_add(0.5, f64::from(dx));
                    let py = v.mul_add(0.5, f64::from(dy));
                    weight = (wu * wv).mul_add(
                        (-0.5 * (xx * py).mul_add(py, (2.0 * xy * px).mul_add(-py, yy * px * px))
                            / det)
                            .exp(),
                        weight,
                    );
                }
            }
            total += weight;
            kernel.push((dx, dy, weight));
        }
    }
    for (_, _, weight) in &mut kernel {
        *weight /= total;
    }
    // A diagonal covariance is separable. Use the independent quadrature
    // kernel's marginals, retaining its finite support and normalization.
    // This is exact separability, not an approximation for small rotations.
    if xy == 0.0 {
        let mut horizontal = vec![0.0; (2 * rx + 1) as usize];
        let mut vertical = vec![0.0; (2 * ry + 1) as usize];
        for &(dx, dy, weight) in &kernel {
            horizontal[(dx + rx) as usize] += weight;
            vertical[(dy + ry) as usize] += weight;
        }
        let mut intermediate = vec![0.0; src.len()];
        convolve_lines(src, &mut intermediate, &horizontal, width, height, 1, width);
        let mut out = vec![0.0; src.len()];
        convolve_lines(&intermediate, &mut out, &vertical, height, width, width, 1);
        return out;
    }
    let mut out = vec![0.0; src.len()];
    for y in 0..height {
        for x in 0..width {
            let mut value = 0.0;
            for &(dx, dy, weight) in &kernel {
                let sx = x as i64 + i64::from(dx);
                let sy = y as i64 + i64::from(dy);
                if sx >= 0 && sy >= 0 && sx < width as i64 && sy < height as i64 {
                    value = weight.mul_add(src[sy as usize * width + sx as usize], value);
                }
            }
            out[y * width + x] = value;
        }
    }
    out
}

/// Convolve independent strided lines with transparent padding. Keeping the
/// layout explicit lets rows and columns use the same accumulation order.
fn convolve_lines(
    src: &[f64],
    dst: &mut [f64],
    kernel: &[f64],
    length: usize,
    lines: usize,
    stride: usize,
    line_stride: usize,
) {
    let radius = kernel.len() / 2;
    for line in 0..lines {
        let base = line * line_stride;
        for position in 0..length {
            let first = radius.saturating_sub(position);
            let end = kernel.len().min(length + radius - position);
            let mut value = 0.0;
            for (tap, &weight) in kernel.iter().enumerate().take(end).skip(first) {
                let source = position + tap - radius;
                value = weight.mul_add(src[base + source * stride], value);
            }
            dst[base + position * stride] = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::affine_blur;

    #[test]
    fn diagonal_covariance_matches_dense_limit() {
        for (width, height) in [(1, 1), (1, 7), (9, 1), (7, 9)] {
            let src: Vec<_> = (0..width * height)
                .map(|index| f64::from(u32::try_from((index * 7 + 3) % 19).unwrap()) / 19.0)
                .collect();
            for (x, y) in [(1.0, 1.0), (0.5, 2.0), (-2.0, 0.75)] {
                let diagonal = affine_blur(&src, width, height, 1.3, [x, 0.0, 0.0, y]);
                // A nonzero cross term exercises the independent dense
                // quadrature path while perturbing the covariance by <1e-13.
                let dense = affine_blur(&src, width, height, 1.3, [x, 1e-14, 0.0, y]);
                for (actual, expected) in diagonal.into_iter().zip(dense) {
                    assert!((actual - expected).abs() < 1e-12);
                }
            }
        }
    }
}
