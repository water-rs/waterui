//! Four-pixel evaluation of the existing analytic shadow quadrature.

use cherenkov_simd::math::erf4;
use fearless_simd::{Simd, f32x4};

/// One in-box y sample's horizontal interval and Gaussian weight.
#[derive(Clone, Copy, Default)]
pub(super) struct Sample {
    pub left: f32,
    pub right: f32,
    pub weight: f32,
}

/// The 8-point Gauss–Legendre rule applied to each corner band, as
/// `(node, weight)` of the symmetric pair `mid ± hw·node`. The values
/// mirror `shadow` in `gpu/src/render/shader.wgsl` verbatim.
#[expect(
    clippy::excessive_precision,
    clippy::unreadable_literal,
    reason = "the WGSL constants verbatim; the f32 values are the same"
)]
pub(super) const GL8: [(f32, f32); 4] = [
    (0.1834346425, 0.3626837834),
    (0.5255324099, 0.3137066459),
    (0.7966664774, 0.2223810345),
    (0.9602898565, 0.1012285363),
];

/// A row's table bound: one analytic sample for the straight band plus
/// both corner bands' 8-node Gauss–Legendre rules.
pub(super) const MAX_SAMPLES: usize = 1 + 2 * (2 * GL8.len());

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
