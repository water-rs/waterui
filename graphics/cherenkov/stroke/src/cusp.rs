//! The exact cusp predicate.
//!
//! With the control arms `A = P1−P0`, `B = P2−P1`, `C = P3−P2` and
//! `α = B×C`, `β = C×A`, `γ = A×B`, a non-collinear cubic has an interior cusp
//! iff `α`, `β`, `γ` are nonzero with one sign and `β² = 4αγ`; the cusp is at
//! `t₀ = 2γ / (β + 2γ)`.
//!
//! Both tests are taken at the representation precision of the control
//! points, because a cubic built in exact arithmetic (a rotated cusp, a
//! rotated line) reaches us only with every coordinate rounded to `f64`.
//! With `u = 2⁻⁵³`, `M` the largest absolute coordinate and `s` the largest
//! absolute arm component, rounding moves each coordinate by at most
//! `u·|x| ≤ u·M`, so each arm component by at most `e = 2u·M`. Expanding
//! around the given arms, `α + Δα = (B + ΔB)×(C + ΔC)` gives
//! `|Δα| ≤ |ΔB×C| + |B×ΔC| + |ΔB×ΔC| ≤ 2es + 2es + 2e² = ε` with
//! `ε = 8u·(M·s + u·M²)`, and likewise for `β` and `γ`. Hence:
//!
//! - **Collinear.** `max(|α|, |β|, |γ|) ≤ ε`: the cubic is collinear to within
//!   its own rounding, the signs of `α`, `β`, `γ` are rounding noise, and it is
//!   not a cusp. It is left to `kurbo`'s collinear handling.
//! - **Near cusp.** With `|α|, |β|, |γ| ≤ 2s²`,
//!   `Δ(β² − 4αγ) = 2βΔβ + Δβ² − 4(αΔγ + γΔα + ΔαΔγ)`, so
//!   `|ΔD| ≤ 4s²ε + ε² + 16s²ε + 4ε² = 20s²ε + 5ε²`. A same-signed cubic with
//!   `|β² − 4αγ|` within that is a cusp to within its rounding and is split.
//!
//! Both comparisons are decided exactly over the `f64` inputs, so no
//! evaluation rounding moves them. The bounds depend on where the cubic sits
//! (`M`), not only on its shape, because that is where the rounding of its
//! coordinates comes from.

use kurbo::CubicBez;
use num_bigint::{BigInt, BigUint, Sign};

/// Upper bound, in units of `u·s⁴`, of the error of evaluating
/// `β² − 4αγ` in `f64`, with margin over the analysed ~200: rounding of each
/// arm (`u` relative to the arm), of each product and difference in the
/// crosses (`|α̂ − α| ≤ 8u·s²`), and of the fused `β̂·β̂ − 4α̂γ̂`
/// (`32 + 144 + 20 ≈ 196`).
const FILTER_BAND: f64 = 1024.0;

/// The parameter of `c`'s interior cusp, rounded to `f64`, if it has one
/// within the band.
pub fn parameter(c: &CubicBez) -> Option<f64> {
    let points = [c.p0, c.p1, c.p2, c.p3];
    if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    if !passes_filter(c) {
        return None;
    }
    exact_parameter(points.map(|p| [p.x, p.y]))
}

/// `false` only when `f64` evaluation proves the cubic is outside the band.
fn passes_filter(c: &CubicBez) -> bool {
    let first = c.p1 - c.p0;
    let second = c.p2 - c.p1;
    let third = c.p3 - c.p2;
    let largest = [first.x, first.y, second.x, second.y, third.x, third.y]
        .into_iter()
        .fold(0.0_f64, |m, v| m.max(v.abs()));
    if largest == 0.0 {
        return false;
    }
    let coordinate = [c.p0, c.p1, c.p2, c.p3]
        .into_iter()
        .fold(0.0_f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
    let alpha = second.cross(third);
    let beta = third.cross(first);
    let gamma = first.cross(second);
    let disc = beta.mul_add(beta, -4.0 * alpha * gamma);
    let unit_roundoff = 0.5 * f64::EPSILON;
    // The band `20s²ε + 5ε²` evaluated in `f64` is within a few `u` of its
    // exact value, so twice it bounds the exact band from above.
    let epsilon =
        8.0 * unit_roundoff * coordinate.mul_add(largest, unit_roundoff * coordinate * coordinate);
    let band = (20.0 * largest * largest).mul_add(epsilon, 5.0 * epsilon * epsilon);
    disc.abs() <= FILTER_BAND.mul_add(unit_roundoff * largest.powi(4), 2.0 * band)
}

/// An `f64` as `(mantissa, exponent)` with value `mantissa · 2^exponent`.
fn decompose(v: f64) -> (i64, i32) {
    let bits = v.to_bits();
    let exp_field = i32::try_from((bits >> 52) & 0x7ff).expect("11 bits");
    let fraction = i64::try_from(bits & ((1 << 52) - 1)).expect("52 bits");
    let (mantissa, exp) = if exp_field == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exp_field - 1075)
    };
    (
        if v.is_sign_negative() {
            -mantissa
        } else {
            mantissa
        },
        exp,
    )
}

fn exact_parameter(points: [[f64; 2]; 4]) -> Option<f64> {
    let parts = points.map(|p| p.map(decompose));
    let min_exp = parts
        .iter()
        .flatten()
        .filter(|(m, _)| *m != 0)
        .map(|&(_, e)| e)
        .min()?;
    // Every coordinate as an integer multiple of 2^min_exp.
    let int = parts.map(|p| {
        p.map(|(m, e)| {
            // A zero has no exponent of its own.
            let shift = if m == 0 { 0 } else { e - min_exp };
            BigInt::from(m) << usize::try_from(shift).expect("e ≥ min_exp")
        })
    });
    let arm = |i: usize| [&int[i + 1][0] - &int[i][0], &int[i + 1][1] - &int[i][1]];
    let (first, second, third) = (arm(0), arm(1), arm(2));
    let cross = |u: &[BigInt; 2], v: &[BigInt; 2]| &u[0] * &v[1] - &u[1] * &v[0];
    let alpha = cross(&second, &third);
    let beta = cross(&third, &first);
    let gamma = cross(&first, &second);
    let largest = [&first, &second, &third]
        .into_iter()
        .flatten()
        .map(|v| v.magnitude().clone())
        .max()
        .expect("six components");
    let coordinate = int
        .iter()
        .flatten()
        .map(|v| v.magnitude().clone())
        .max()
        .expect("eight coordinates");
    // `ε = 8u·(M·s + u·M²) = 2⁻¹⁰³·E` with `E = 2⁵³·M·s + M²`.
    let rounding = ((&coordinate * &largest) << 53usize) + &coordinate * &coordinate;
    let cross_max = [&alpha, &beta, &gamma]
        .into_iter()
        .map(BigInt::magnitude)
        .max()
        .expect("three crosses");
    if cross_max << 103usize <= rounding {
        return None;
    }
    let sign = alpha.sign();
    if sign == Sign::NoSign || beta.sign() != sign || gamma.sign() != sign {
        return None;
    }
    let disc = &beta * &beta - BigInt::from(4) * &alpha * &gamma;
    // `|D| ≤ 20s²ε + 5ε²`, scaled by `2²⁰⁶`.
    let band = ((BigUint::from(20u8) * &largest * &largest * &rounding) << 103usize)
        + BigUint::from(5u8) * &rounding * &rounding;
    if disc.magnitude() << 206usize > band {
        return None;
    }
    let numerator = gamma.magnitude() << 1usize;
    let denominator = beta.magnitude() + &numerator;
    let param = ratio_to_f64(&numerator, &denominator);
    (param > 0.0 && param < 1.0).then_some(param)
}

/// `numerator / denominator` rounded to `f64`, for `numerator < denominator`.
fn ratio_to_f64(numerator: &BigUint, denominator: &BigUint) -> f64 {
    // Scale so the quotient carries at least 64 significant bits.
    let scale = 66 + denominator.bits() - numerator.bits();
    let shifted = numerator << usize::try_from(scale).expect("scale fits");
    let quotient = &shifted / denominator;
    let mut sticky = (&shifted % denominator).bits() != 0;
    let excess = quotient.bits().saturating_sub(64);
    let top = &quotient >> usize::try_from(excess).expect("excess fits");
    if excess > 0 {
        sticky |= quotient.trailing_zeros().is_some_and(|z| z < excess);
    }
    let mantissa = top.iter_u64_digits().next().expect("nonzero quotient") | u64::from(sticky);
    let mut exp = i64::try_from(excess).expect("excess fits") - i64::try_from(scale).expect("fits");
    #[expect(
        clippy::cast_precision_loss,
        reason = "round-to-nearest with the sticky bit is the intended rounding"
    )]
    let mut value = mantissa as f64;
    // Scale by 2^exp in exactly representable power-of-two steps.
    while exp != 0 {
        let step = exp.clamp(-1000, 1000);
        value *= f64::from_bits(
            u64::try_from(1023 + step).expect("within the normal exponent range") << 52,
        );
        exp -= step;
    }
    value
}
