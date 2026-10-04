//! Shadow-kernel math shared by the backends' silhouette capture path.
//!
//! A silhouette shadow convolves the captured coverage with a morphology
//! pass for the spread and a separable Gaussian pass for the blur. Both
//! backends build the same taps here; only the buffer layout and the
//! sampler (CPU rows or the WGSL kernel) stay per backend.

use kurbo::Affine;

use crate::RenderError;

/// Reject a negative or non-finite blur `sigma`.
///
/// A silhouette convolves by `sigma` and the analytic path integrates
/// `sqrt(sigma² + 1/12)`; a negative sigma is not a blur and a non-finite
/// one cannot produce a bounded kernel.
///
/// # Errors
/// Returns [`RenderError::Render`] when `sigma` is negative or non-finite.
#[inline]
pub fn check_sigma(sigma: f64) -> Result<(), RenderError> {
    if !sigma.is_finite() || sigma < 0.0 {
        return Err(RenderError::Render("invalid shadow sigma".into()));
    }
    Ok(())
}

/// The capture padding in device pixels a silhouette shadow needs around
/// the surface: the spread's reach plus `6σ` of blur support plus a 2 px
/// rasterization margin, along each device axis.
#[must_use]
#[inline]
pub fn capture_padding(transform: Affine, sigma: f64, spread: f64) -> (f64, f64) {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let spread = spread.abs();
    let px = (spread.mul_add(a.hypot(c), 6.0 * sigma * (a.abs() + c.abs())) + 2.0).ceil();
    let py = (spread.mul_add(b.hypot(d), 6.0 * sigma * (b.abs() + d.abs())) + 2.0).ceil();
    (px, py)
}

/// The validated tap count for a kernel that must stay within `limit` taps.
///
/// # Errors
/// Returns [`RenderError::Render`] when `count` is non-finite, below one, or
/// exceeds `limit`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "finite bounded count"
)]
#[inline]
fn kernel_count(count: f64, limit: f64) -> Result<usize, RenderError> {
    if !count.is_finite() || count < 1.0 || count > limit {
        return Err(RenderError::Render(
            "shadow kernel exceeds addressable storage".into(),
        ));
    }
    Ok(count as usize)
}

/// Gaussian kernel taps along `axis` for a blur of `sigma`.
///
/// `(x, y, weight)` samples at unit offsets over `±6σ` of the axis length,
/// normalized to a unit sum. The empty kernel for `sigma <= 0` is a single
/// identity tap.
///
/// # Errors
/// Returns [`RenderError::Render`] when the kernel would exceed `limit`
/// taps.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "bounded kernel indices and f32 taps"
)]
#[inline]
pub fn gaussian_taps(axis: [f64; 2], sigma: f64, limit: f64) -> Result<Vec<[f32; 3]>, RenderError> {
    let length = axis[0].hypot(axis[1]);
    let sigma = sigma * length;
    if sigma <= 1e-9 {
        return Ok(vec![[0.0, 0.0, 1.0]]);
    }
    let radius = (6.0 * sigma).ceil();
    let count = kernel_count(2.0f64.mul_add(radius, 1.0), limit)?;
    let inv = 1.0 / (sigma * std::f64::consts::SQRT_2);
    let mut taps = Vec::with_capacity(count);
    let mut total = 0.0;
    for i in 0..count {
        let d = i as f64 - radius;
        let weight = 0.5 * (libm::erf((d + 0.5) * inv) - libm::erf((d - 0.5) * inv));
        total += weight;
        taps.push([
            (axis[0] / length * d) as f32,
            (axis[1] / length * d) as f32,
            weight as f32,
        ]);
    }
    for tap in &mut taps {
        tap[2] /= total as f32;
    }
    Ok(taps)
}

/// Morphology kernel taps for a `spread` under `matrix` (the transform's
/// 2×2 part as `[a, b, c, d]`): `(x, y, 1)` samples covering the spread
/// disk in device space.
///
/// # Errors
/// Returns [`RenderError::Render`] when the kernel would exceed `limit`
/// taps.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bounded morphology footprint and f32 taps"
)]
#[inline]
pub fn spread_taps(
    matrix: [f64; 4],
    spread: f64,
    limit: f64,
) -> Result<Vec<[f32; 3]>, RenderError> {
    let [a, b, c, d] = matrix;
    let radius = spread.abs();
    let steps = (radius * a.hypot(b).max(c.hypot(d)).max(1.0) * 2.0)
        .ceil()
        .max(1.0);
    let count = kernel_count(2.0f64.mul_add(steps, 1.0).powi(2), limit)?;
    let side = 2.0f64.mul_add(steps, 1.0) as usize;
    let mut taps = Vec::with_capacity(count);
    for y in 0..side {
        for x in 0..side {
            let px = (x as f64 - steps) / steps * radius;
            let py = (y as f64 - steps) / steps * radius;
            if px.hypot(py) <= radius {
                taps.push([
                    a.mul_add(px, c * py) as f32,
                    b.mul_add(px, d * py) as f32,
                    1.0,
                ]);
            }
        }
    }
    Ok(taps)
}
