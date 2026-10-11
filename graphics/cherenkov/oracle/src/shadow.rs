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
