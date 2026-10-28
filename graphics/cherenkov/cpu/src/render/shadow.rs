//! Four-pixel evaluation of the analytic shadow quadrature.

use std::mem::MaybeUninit;

use cherenkov_simd::math::erf4;
use fearless_simd::{Simd, f32x4};

use super::raster::BAND_H;

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

/// Which edges' `erf` a four-pixel group evaluates. An edge at least the
/// saturation margin from every pixel centre of the group has `erf` at
/// exactly ±1, the same saturation the interior columns rely on.
#[derive(Clone, Copy)]
pub(super) enum Sides {
    /// Both edges of every interval.
    Both,
    /// Left edges only: every right edge is saturated at `+1`.
    Left,
    /// Right edges only: every left edge is saturated at `-1`.
    Right,
}

/// Consecutive rows of one band whose samples have bit-identical x
/// intervals — every row's straight-band sample, and the corner-band nodes
/// of rows whose windows cover the same corner-band span — so each
/// interval's x integral is evaluated once per four pixels for all of
/// them, and only the weights are per row.
pub(super) struct Run {
    left: [f32; MAX_SAMPLES],
    right: [f32; MAX_SAMPLES],
    n: usize,
    /// The intervals' most-inset left and right edges, which bound the
    /// interior columns where every `erf` is saturated.
    pub xlm: f32,
    pub xrm: f32,
    /// The band row of the run's first row.
    first: usize,
    len: usize,
    /// Per row: its weight sum, and its `n` weights. The weights are
    /// written by `push` before any read and never zero-filled: the
    /// table is a whole band's worth, filled per shadow item.
    sums: [f32; BAND_H],
    weights: [[MaybeUninit<f32>; MAX_SAMPLES]; BAND_H],
}

impl Run {
    pub(super) const fn new() -> Self {
        Self {
            left: [0.0; MAX_SAMPLES],
            right: [0.0; MAX_SAMPLES],
            n: 0,
            xlm: 0.0,
            xrm: 0.0,
            first: 0,
            len: 0,
            sums: [0.0; BAND_H],
            weights: [[MaybeUninit::uninit(); MAX_SAMPLES]; BAND_H],
        }
    }

    pub(super) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether band row `y` with `samples` continues this run: it is the
    /// next row and has exactly the run's intervals, bit for bit.
    pub(super) fn continues(&self, y: usize, samples: &[Sample]) -> bool {
        !self.is_empty()
            && y == self.first + self.len
            && samples.len() == self.n
            && samples
                .iter()
                .zip(&self.left)
                .zip(&self.right)
                .all(|((s, l), r)| {
                    s.left.to_bits() == l.to_bits() && s.right.to_bits() == r.to_bits()
                })
    }

    /// Empties the run and starts it at band row `y`, on `samples`'
    /// intervals.
    pub(super) fn start(&mut self, y: usize, samples: &[Sample], xlm: f32, xrm: f32) {
        for (k, s) in samples.iter().enumerate() {
            self.left[k] = s.left;
            self.right[k] = s.right;
        }
        self.n = samples.len();
        self.xlm = xlm;
        self.xrm = xrm;
        self.first = y;
        self.len = 0;
    }

    /// Appends the next row, with weight sum `sum` and `samples` on the
    /// run's intervals (`start` or `continues` established them).
    pub(super) fn push(&mut self, sum: f32, samples: &[Sample]) {
        assert_eq!(samples.len(), self.n, "a run row has the run's intervals");
        self.sums[self.len] = sum;
        for (w, s) in self.weights[self.len].iter_mut().zip(samples) {
            w.write(s.weight);
        }
        self.len += 1;
    }

    /// The run's rows: band row and weight sum.
    pub(super) fn rows(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        (self.first..).zip(self.sums[..self.len].iter().copied())
    }

    /// Row `i`'s weights.
    fn weights(&self, i: usize) -> &[f32] {
        assert!(i < self.len, "row {i} of a {}-row run", self.len);
        // SAFETY: `push` wrote the first `n` weights of every row below
        // `len`, and `start` resets `len` whenever `n` changes.
        unsafe { self.weights[i][..self.n].assume_init_ref() }
    }

    /// Fills `out` with interval `k`'s x integral
    /// `½(erf((right-px)·is) - erf((left-px)·is))` at the four pixel
    /// centres, for every interval, evaluating only the edges `sides`
    /// names.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched SIMD level"
    )]
    #[inline(always)]
    pub(super) fn integrals4<S: Simd>(
        &self,
        simd: S,
        pixels: f32x4<S>,
        inv_sqrt2_sigma: f32,
        sides: Sides,
        out: &mut Integrals<S>,
    ) {
        match sides {
            Sides::Both => self.integrate::<S, true, true>(simd, pixels, inv_sqrt2_sigma, out),
            Sides::Left => self.integrate::<S, true, false>(simd, pixels, inv_sqrt2_sigma, out),
            Sides::Right => self.integrate::<S, false, true>(simd, pixels, inv_sqrt2_sigma, out),
        }
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched SIMD level"
    )]
    #[inline(always)]
    fn integrate<S: Simd, const LEFT: bool, const RIGHT: bool>(
        &self,
        simd: S,
        pixels: f32x4<S>,
        inv_sqrt2_sigma: f32,
        out: &mut Integrals<S>,
    ) {
        let scale = simd.splat_f32x4(inv_sqrt2_sigma);
        let half = simd.splat_f32x4(0.5);
        // Consecutive intervals can share the evaluated edges (the
        // straight wall, a zero-radius corner): reuse the integral only
        // when those edges have identical bits.
        let mut previous = None;
        for (k, slot) in out.values.iter_mut().enumerate().take(self.n) {
            let key = (
                if LEFT { self.left[k].to_bits() } else { 0 },
                if RIGHT { self.right[k].to_bits() } else { 0 },
            );
            let integral = match previous {
                Some((seen, integral)) if seen == key => integral,
                _ => {
                    let right = if RIGHT {
                        erf4(
                            simd,
                            simd.mul_f32x4(
                                simd.sub_f32x4(simd.splat_f32x4(self.right[k]), pixels),
                                scale,
                            ),
                        )
                    } else {
                        simd.splat_f32x4(1.0)
                    };
                    let left = if LEFT {
                        erf4(
                            simd,
                            simd.mul_f32x4(
                                simd.sub_f32x4(simd.splat_f32x4(self.left[k]), pixels),
                                scale,
                            ),
                        )
                    } else {
                        simd.splat_f32x4(-1.0)
                    };
                    let integral = simd.mul_f32x4(half, simd.sub_f32x4(right, left));
                    previous = Some((key, integral));
                    integral
                }
            };
            slot.write(integral);
        }
        out.n = self.n;
    }

    /// Row `i`'s coverage at the four pixels: the weighted sum of
    /// `integrals` in sample order.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched SIMD level"
    )]
    #[inline(always)]
    pub(super) fn coverage4<S: Simd>(
        &self,
        simd: S,
        i: usize,
        integrals: &Integrals<S>,
    ) -> f32x4<S> {
        let mut coverage = simd.splat_f32x4(0.0);
        for (integral, w) in integrals.values().iter().zip(self.weights(i)) {
            coverage = simd.add_f32x4(coverage, simd.mul_f32x4(*integral, simd.splat_f32x4(*w)));
        }
        coverage
    }
}

/// A run's interval x integrals at four pixel centres, in interval order.
/// `Run::integrals4` writes the first `n` values and only then sets `n`;
/// the rest are never read, so the table is never zero-filled.
pub(super) struct Integrals<S: Simd> {
    values: [MaybeUninit<f32x4<S>>; MAX_SAMPLES],
    n: usize,
}

impl<S: Simd> Integrals<S> {
    pub(super) const fn new() -> Self {
        Self {
            values: [const { MaybeUninit::uninit() }; MAX_SAMPLES],
            n: 0,
        }
    }

    fn values(&self) -> &[f32x4<S>] {
        // SAFETY: `n` is 0 or was set by `Run::integrate` right after it
        // wrote the first `n` values.
        unsafe { self.values[..self.n].assume_init_ref() }
    }
}
