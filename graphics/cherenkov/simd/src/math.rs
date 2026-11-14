//! Vector math evaluated under the caller's dispatched target features.
use fearless_simd::{Bytes, Select, Simd, SimdBase, SimdFloat, f32x4, i32x4};

/// Exponential under the dispatched SIMD level.
///
/// Range reduction by ln(2) and a seventh-degree Taylor polynomial on
/// [-ln(2)/2, ln(2)/2]. The normal-range relative error is
/// below 5e-7; subnormals are rounded by the final scaling multiply.
#[expect(clippy::inline_always, reason = "inherits dispatched target features")]
#[inline(always)]
pub fn exp4<S: Simd>(simd: S, x: f32x4<S>) -> f32x4<S> {
    let one = simd.splat_f32x4(1.0);
    let zero = simd.splat_f32x4(0.0);
    let bounded = x.max(simd.splat_f32x4(-104.0)).min(simd.splat_f32x4(89.0));
    let n = (bounded * core::f32::consts::LOG2_E).round_ties_even();
    let r = (-n).mul_add(simd.splat_f32x4(0.693_359_4), bounded);
    let r = (-n).mul_add(simd.splat_f32x4(-0.000_212_194_44), r);
    let p = simd
        .splat_f32x4(1.0 / 5040.0)
        .mul_add(r, simd.splat_f32x4(1.0 / 720.0))
        .mul_add(r, simd.splat_f32x4(1.0 / 120.0))
        .mul_add(r, simd.splat_f32x4(1.0 / 24.0))
        .mul_add(r, simd.splat_f32x4(1.0 / 6.0))
        .mul_add(r, simd.splat_f32x4(0.5))
        .mul_add(r, one)
        .mul_add(r, one);
    // Split the power to keep both factors normal, including results near
    // zero and the overflow boundary. Final multiplication rounds once.
    let low = n.max(simd.splat_f32x4(-126.0)).min(simd.splat_f32x4(127.0));
    let high = n - low;
    let low = (simd.cvt_i32_f32x4(low) + simd.splat_i32x4(127)) << 23;
    let high = (simd.cvt_i32_f32x4(high) + simd.splat_i32x4(127)) << 23;
    let result = p * high.bitcast::<f32x4<S>>() * low.bitcast::<f32x4<S>>();
    let result = x
        .simd_lt(simd.splat_f32x4(-103.972_08))
        .select(zero, result);
    let result = x
        .simd_gt(simd.splat_f32x4(88.722_84))
        .select(simd.splat_f32x4(f32::INFINITY), result);
    x.simd_eq(x).select(result, x)
}

/// Natural logarithm under the dispatched SIMD level.
///
/// Cephes' single-precision `logf`: the exponent split from a mantissa
/// in `[√½, √2)`, a ninth-degree polynomial in `m − 1`, and `ln 2` added
/// back in two parts. The domain is the positive normal floats, where the
/// error is below 2e-7 of `max(1, |ln x|)`; `+∞` and NaN pass through.
/// Zero, negative and subnormal inputs are outside the domain and give
/// unspecified finite values, so callers select those lanes away.
#[expect(clippy::inline_always, reason = "inherits dispatched target features")]
#[inline(always)]
pub fn ln4<S: Simd>(simd: S, x: f32x4<S>) -> f32x4<S> {
    let splat = |v| simd.splat_f32x4(v);
    let one = splat(1.0);
    let bits = x.bitcast::<i32x4<S>>();
    let exponent = simd.cvt_f32_i32x4(simd.shr_i32x4(bits, 23) - simd.splat_i32x4(126));
    let mantissa = ((bits & 0x007f_ffff) | 0x3f00_0000).bitcast::<f32x4<S>>();
    let small = mantissa.simd_lt(splat(core::f32::consts::FRAC_1_SQRT_2));
    let exponent = exponent - small.select(one, splat(0.0));
    let reduced = small.select(mantissa + mantissa - one, mantissa - one);
    let squared = reduced * reduced;
    let poly = splat(7.037_683_6e-2)
        .mul_add(reduced, splat(-1.151_461e-1))
        .mul_add(reduced, splat(1.167_699_9e-1))
        .mul_add(reduced, splat(-1.242_014_1e-1))
        .mul_add(reduced, splat(1.424_932_3e-1))
        .mul_add(reduced, splat(-1.666_805_8e-1))
        .mul_add(reduced, splat(2.000_071_4e-1))
        .mul_add(reduced, splat(-2.499_999_4e-1))
        .mul_add(reduced, splat(3.333_333e-1));
    let tail = exponent.mul_add(splat(-2.121_944_4e-4), poly * reduced * squared);
    let tail = squared.mul_add(splat(-0.5), tail);
    let result = exponent.mul_add(splat(0.693_359_4), reduced + tail);
    let result = x.simd_eq(splat(f32::INFINITY)).select(x, result);
    x.simd_eq(x).select(result, x)
}

/// Abramowitz–Stegun 7.1.26 error-function approximation.
///
/// Uses the dispatched exponential and multiply-add. Native-level checks
/// against f64 erf bound absolute error by 6e-7; scene acceptance is visual.
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
pub fn erf4<S: Simd>(simd: S, x: f32x4<S>) -> f32x4<S> {
    let splat = |v| simd.splat_f32x4(v);
    let one = splat(1.0);
    let a = simd.abs_f32x4(x);
    let t = simd.div_f32x4(one, simd.mul_add_f32x4(splat(0.327_591_1), a, one));
    let p = simd.mul_add_f32x4(splat(1.061_405_429), t, splat(-1.453_152_027));
    let p = simd.mul_add_f32x4(p, t, splat(1.421_413_741));
    let p = simd.mul_add_f32x4(p, t, splat(-0.284_496_736));
    let p = simd.mul_add_f32x4(p, t, splat(0.254_829_592));
    let argument = simd.mul_f32x4(simd.neg_f32x4(a), a);
    let exponential = exp4(simd, argument);
    let y = simd.sub_f32x4(one, simd.mul_f32x4(simd.mul_f32x4(p, t), exponential));
    let sign = one.copysign(x);
    simd.mul_f32x4(sign, y)
}

/// `atan2` lane-wise: `f32::atan2` has no vector form, so the sweep
/// kernel evaluates this polynomial at every width.
///
/// Cephes' single-precision `atan` reduction:
/// `t ∈ [0, 1]` from |min|/|max|, `t > tan(π/8)` folded through
/// `(t − 1)/(t + 1)` with a π/4 bias, the four-coefficient minimax on
/// `|u| ≤ tan(π/8)`, then the quadrant rebuilt from the arguments'
/// signs. The IEEE corners fall out of the same selects: `±0`/`±π` for
/// a zero pair, `±π/4`/`±3π/4` for `±∞,±∞`, NaN in gives NaN out. The
/// absolute error is under 3×10⁻⁷ rad (≈1.1 ulp at π), as measured
/// over the domain; `f32::atan2` itself is good to ≈0.5 ulp.
#[expect(
    clippy::many_single_char_names,
    reason = "x/y/t/u/z/r are the atan2 reduction's own names"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub fn atan2<S, V>(simd: S, y: V, x: V) -> V
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
    V::Mask: Select<V>,
{
    let zero = V::splat(simd, 0.0);
    let one = V::splat(simd, 1.0);
    let (ax, ay) = (x.abs(), y.abs());
    let (lo, hi) = (ax.min(ay), ax.max(ay));
    // `lo / hi` is NaN when both are ∞ — atan(±1) — or both are 0;
    // the first is corrected here, the second below.
    let t = lo
        .simd_eq(V::splat(simd, f32::INFINITY))
        .select(one, lo / hi);
    let big = t.simd_gt(V::splat(simd, 0.414_213_57));
    let u = big.select((t - one) / (t + one), t);
    let z = u * u;
    // atan(u) = u + u³·P(u²), P the cephes f32 coefficients.
    let p = V::splat(simd, 8.053_744_5e-2)
        .mul_add(z, V::splat(simd, -1.387_768_5e-1))
        .mul_add(z, V::splat(simd, 1.997_771_1e-1))
        .mul_add(z, V::splat(simd, -3.333_295e-1));
    let mut r =
        (u * z).mul_add(p, u) + big.select(V::splat(simd, core::f32::consts::FRAC_PI_4), zero);
    r = ay
        .simd_gt(ax)
        .select(V::splat(simd, core::f32::consts::FRAC_PI_2) - r, r);
    r = x
        .simd_lt(zero)
        .select(V::splat(simd, core::f32::consts::PI) - r, r);
    let r = r.copysign(y);
    // atan2(±0, ±0): ±π for a negative-signed x, ±0 otherwise.
    let corner = x
        .simd_lt(zero)
        .select(V::splat(simd, core::f32::consts::PI), zero)
        .copysign(y);
    let r = hi.simd_eq(zero).select(corner, r);
    // NaN in either argument gives NaN, whatever min/max picked.
    (!(x.simd_eq(x)) | !(y.simd_eq(y))).select(V::splat(simd, f32::NAN), r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_and_erf_follow_f64() {
        fn check<S: Simd>(simd: S) {
            for i in -10_000_i16..=10_000 {
                let x = f32::from(i) * 0.008;
                let actual = exp4(simd, simd.splat_f32x4(x))[0];
                let expected = f64::from(x).exp();
                assert!(
                    (f64::from(actual) / expected - 1.0).abs() < 5e-7,
                    "exp({x})={actual}, expected {expected}"
                );
                let x = f32::from(i) * 0.001;
                let actual = erf4(simd, simd.splat_f32x4(x))[0];
                let expected = libm::erf(f64::from(x));
                assert!(
                    (f64::from(actual) - expected).abs() < 6e-7,
                    "erf({x})={actual}, expected {expected}"
                );
            }
            let x = f32x4::load_array(simd, [f32::NEG_INFINITY, 0.0, f32::INFINITY, f32::NAN]);
            let result = exp4(simd, x).to_array();
            assert_eq!(result[..3], [0.0, 1.0, f32::INFINITY]);
            assert!(result[3].is_nan());
            // Every 4099th bit pattern over the positive normal floats.
            let mut bits = f32::MIN_POSITIVE.to_bits();
            while bits < f32::INFINITY.to_bits() {
                let x = f32::from_bits(bits);
                let actual = ln4(simd, simd.splat_f32x4(x))[0];
                let expected = f64::from(x).ln();
                assert!(
                    (f64::from(actual) - expected).abs() < 2e-7 * expected.abs().max(1.0),
                    "ln({x})={actual}, expected {expected}"
                );
                bits += 4099;
            }
            let result = ln4(
                simd,
                f32x4::load_array(simd, [f32::INFINITY, f32::NAN, 1.0, 1.0]),
            );
            assert_eq!(result[0], f32::INFINITY);
            assert!(result[1].is_nan());
        }
        let native = crate::detect();
        let levels = [
            Some(native),
            Some(crate::Level::baseline()),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            native.as_sse2().map(Simd::level),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            native.as_sse4_2().map(Simd::level),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            native.as_avx2().map(Simd::level),
        ];
        for level in levels.into_iter().flatten() {
            crate::dispatch!(level, simd => check(simd));
        }
    }
}
