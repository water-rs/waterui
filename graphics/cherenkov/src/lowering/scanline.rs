//! Scanline boundary evaluation shared by the backends' coverage rasterizers.
//!
//! Both backends accumulate a polygon edge's signed area row by row, and
//! every row needs the x where the edge crosses the row's top and bottom.
//! Advancing an absolute `f32` x by repeated addition rounds once per row,
//! so a tall edge's coverage would depend on how many rows lie above the
//! pixel. A rasterizer instead evaluates every boundary from the edge's
//! origin, as `origin + slope * t` rounded once to `f32` — the value
//! `slope.mul_add(t, origin)` computes. [`walk_rows`] runs a rasterization
//! pass with the fused multiply-add instruction where the CPU has it and
//! with an exact `f64` evaluation where it does not, so coverage is the
//! same bits on every CPU.

/// An edge's x as a function of the height `t` above its origin.
pub trait EdgeX: Copy {
    /// The edge with x `origin` at `t = 0` and `slope` = dx/dy.
    fn new(slope: f32, origin: f32) -> Self;

    /// `origin + slope * t`, rounded once to `f32`.
    fn at(self, t: f32) -> f32;
}

/// A rasterization pass generic over its boundary evaluator, run by
/// [`walk_rows`].
pub trait RowWalk {
    /// What the pass returns.
    type Output;

    /// Runs the pass with `E`. An implementation is `#[inline(always)]`
    /// down to every [`EdgeX::at`] call, so the instantiation
    /// [`walk_rows`] compiles with the instruction evaluates with it.
    fn walk<E: EdgeX>(self) -> Self::Output;
}

/// Runs `walk` with the fastest exact evaluator the running CPU has: the
/// fused multiply-add instruction when the target guarantees it or the CPU
/// reports it, [`Widened`] otherwise. Both produce identical bits.
#[inline(always)]
#[expect(
    clippy::inline_always,
    reason = "the selection must cost a pass nothing on targets with the instruction"
)]
pub fn walk_rows<W: RowWalk>(walk: W) -> W::Output {
    #[cfg(any(target_arch = "aarch64", target_feature = "fma"))]
    {
        walk.walk::<Fused>()
    }
    #[cfg(all(
        any(target_arch = "x86", target_arch = "x86_64"),
        not(target_feature = "fma")
    ))]
    {
        if std::arch::is_x86_feature_detected!("fma") {
            // SAFETY: the running CPU implements FMA, checked just above.
            unsafe { walk_fused(walk) }
        } else {
            walk.walk::<Widened>()
        }
    }
    #[cfg(not(any(
        target_arch = "aarch64",
        target_arch = "x86",
        target_arch = "x86_64",
        target_feature = "fma"
    )))]
    {
        walk.walk::<Widened>()
    }
}

/// `walk` with [`Fused`], compiled with FMA for a CPU that has it on a
/// target that does not guarantee it.
#[cfg(all(
    any(target_arch = "x86", target_arch = "x86_64"),
    not(target_feature = "fma")
))]
#[target_feature(enable = "fma")]
fn walk_fused<W: RowWalk>(walk: W) -> W::Output {
    walk.walk::<Fused>()
}

/// [`f32::mul_add`]: one instruction where the code is compiled with FMA,
/// a library call per evaluation where it is not, so [`walk_rows`] selects
/// it only in the first case.
#[derive(Debug, Clone, Copy)]
pub struct Fused {
    slope: f32,
    origin: f32,
}

impl EdgeX for Fused {
    #[inline]
    fn new(slope: f32, origin: f32) -> Self {
        Self { slope, origin }
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "evaluated once per raster row")]
    fn at(self, t: f32) -> f32 {
        self.slope.mul_add(t, self.origin)
    }
}

/// The fused result formed exactly in `f64`, for CPUs without the
/// instruction.
#[derive(Debug, Clone, Copy)]
pub struct Widened {
    slope: f64,
    origin: f64,
}

impl EdgeX for Widened {
    #[inline]
    fn new(slope: f32, origin: f32) -> Self {
        Self {
            slope: f64::from(slope),
            origin: f64::from(origin),
        }
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "evaluated once per raster row")]
    fn at(self, t: f32) -> f32 {
        fused_via_f64(self.slope, self.origin, t)
    }
}

/// `slope * t + origin` rounded once to `f32`, for `slope` and `origin`
/// widened from `f32`: the product of two `f32` significands (24 + 24
/// bits) is exact in `f64`, and the rounded `f64` sum rounds to the fused
/// `f32` result unless it sits exactly on an `f32` rounding midpoint
/// without being exact, which the sum's exact residual resolves.
#[inline(always)]
#[expect(
    clippy::inline_always,
    reason = "evaluated once per raster row; its cold path stays out of line"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "rounding the f64 sum to f32 is the evaluation"
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
    if sum != f64::midpoint(f64::from(rounded), f64::from(other)) {
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
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        reason = "random bits shaped into f32 fields"
    )]
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
            let slope = f32::from_bits(
                (bits as u32 & 0x807f_ffff) | ((112 + (bits >> 32) as u32 % 30) << 23),
            );
            let origin = f32::from_bits(
                (next() as u32 & 0x807f_ffff) | ((120 + (bits >> 40) as u32 % 20) << 23),
            );
            let t = f32::from_bits(
                (next() as u32 & 0x007f_ffff) | ((120 + (bits >> 48) as u32 % 20) << 23),
            );
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
            let evaluated = Widened::new(slope, origin).at(t);
            assert_eq!(
                evaluated.to_bits(),
                fused.to_bits(),
                "{slope:e} * {t:e} + {origin:e}: {evaluated:e} versus {fused:e}"
            );
        }
    }
}
