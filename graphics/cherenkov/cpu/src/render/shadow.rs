//! Four-pixel evaluation of the existing analytic shadow quadrature.

use fearless_simd::{Simd, SimdBase, f32x4};

/// One in-box y sample's horizontal interval and Gaussian weight.
#[derive(Clone, Copy, Default)]
pub(super) struct Sample {
    pub left: f32,
    pub right: f32,
    pub weight: f32,
}

/// The existing Abramowitz–Stegun 7.1.26 polynomial, with a library vector
/// exponential. Every multiply-add is the level's own `mul_add`: fused on
/// NEON and AVX2, a separate product and sum elsewhere. Remainders use this
/// same kernel.
///
/// The published approximation error does not include f32 rounding. A
/// dense native-level check against f64 erf measured 4.77e-7 maximum absolute
/// error for both this formula and the previous scalar formula; full-scene
/// oracle comparisons remain the acceptance criterion.
#[expect(clippy::excessive_precision, reason = "the existing A&S coefficients")]
#[expect(
    clippy::many_single_char_names,
    reason = "the A&S formula and its coefficients are cited verbatim"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched SIMD level"
)]
#[inline(always)]
pub(super) fn erf4<S: Simd>(simd: S, x: f32x4<S>) -> f32x4<S> {
    let splat = |v| simd.splat_f32x4(v);
    let one = splat(1.0);
    let a = simd.abs_f32x4(x);
    let t = simd.div_f32x4(one, simd.mul_add_f32x4(splat(0.327_591_1), a, one));
    let p = simd.mul_add_f32x4(splat(1.061_405_429), t, splat(-1.453_152_027));
    let p = simd.mul_add_f32x4(p, t, splat(1.421_413_741));
    let p = simd.mul_add_f32x4(p, t, splat(-0.284_496_736));
    let p = simd.mul_add_f32x4(p, t, splat(0.254_829_592));
    let argument = simd.mul_f32x4(simd.neg_f32x4(a), a);
    // The typed array is the boundary between runtime-dispatched arithmetic
    // and wide's maintained exponential implementation; no approximation is
    // copied into the renderer.
    let exponential: [f32; 4] = wide::f32x4::from(argument.to_array()).exp().into();
    let exponential = f32x4::load_array(simd, exponential);
    let y = simd.sub_f32x4(one, simd.mul_f32x4(simd.mul_f32x4(p, t), exponential));
    let sign = f32x4::load_array(simd, x.to_array().map(f32::signum));
    simd.mul_f32x4(sign, y)
}

/// Integrate four adjacent pixel centres in the original y-sample order.
/// Each lane uses the scalar expression's separate products and sum.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched SIMD level"
)]
#[inline(always)]
pub(super) fn integral4<S: Simd>(
    simd: S,
    pixels: f32x4<S>,
    inv_sqrt2_sigma: f32,
    samples: &[Sample],
) -> f32x4<S> {
    let scale = simd.splat_f32x4(inv_sqrt2_sigma);
    let half = simd.splat_f32x4(0.5);
    let mut coverage = simd.splat_f32x4(0.0);
    // Consecutive samples can share the straight-wall interval. Reuse its
    // x integral only when both endpoints have identical bits; retain each
    // sample's weight and addition in the original quadrature order.
    let mut previous = None;
    for sample in samples {
        let left_bits = sample.left.to_bits();
        let right_bits = sample.right.to_bits();
        let difference = match previous {
            Some((left, right, difference)) if left_bits == left && right_bits == right => {
                difference
            }
            _ => {
                let right = simd.mul_f32x4(
                    simd.sub_f32x4(simd.splat_f32x4(sample.right), pixels),
                    scale,
                );
                let left =
                    simd.mul_f32x4(simd.sub_f32x4(simd.splat_f32x4(sample.left), pixels), scale);
                let difference = simd.sub_f32x4(erf4(simd, right), erf4(simd, left));
                previous = Some((left_bits, right_bits, difference));
                difference
            }
        };
        let weighted = simd.mul_f32x4(
            simd.mul_f32x4(half, difference),
            simd.splat_f32x4(sample.weight),
        );
        coverage = simd.add_f32x4(coverage, weighted);
    }
    coverage
}
