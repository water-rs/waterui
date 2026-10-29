//! The span driver every span kernel shares: sixteen lanes of device
//! columns at a time through the kind's [`SpanKernel`] — one block
//! body, one call site — a [`store_partial`] padded remainder, and the
//! one-pixel path for a short run or a short tail. [`apply_span`]'s
//! affine map, [`extend_t_span`]'s per-mode extension and the
//! interleaved stores are the lane helpers every kind reuses; nothing
//! here knows a kind's own shape — gradients, images and sweeps plug in
//! through [`SpanKernel`] alone.
//!
//! No closure and nothing that takes one (`Option::map_or`,
//! `[T; N]::map`, iterator adapters) may appear on the kernel path: a
//! closure is its own function and does not inherit the dispatched
//! level's target features, so an intrinsic it holds compiles into an
//! out-of-line call on x86. Write `match`, explicit arrays and plain
//! loops instead, and where clippy suggests the combinator silence it
//! with an `expect` stating this reason.

use cherenkov::Extend;
use fearless_simd::{Select, Simd, SimdFloat, f32x16, mask32x16};

pub(super) use cherenkov_simd::span::{LANES, SpanKernel, span};

/// `apply` lane-wise: the same two multiply-adds per lane, through the
/// level's own `mul_add` — fused on NEON and AVX2, a separate product
/// and sum elsewhere; the difference is a last-bit rounding no viewer
/// can see.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub(super) fn apply_span<S, V>(simd: S, m: [f32; 6], x: V, y: V) -> (V, V)
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
{
    (
        V::splat(simd, m[0]).mul_add(x, V::splat(simd, m[2]).mul_add(y, V::splat(simd, m[4]))),
        V::splat(simd, m[1]).mul_add(x, V::splat(simd, m[3]).mul_add(y, V::splat(simd, m[5]))),
    )
}

/// `f32::clamp` lane-wise against span-constant bounds: each strict
/// comparison falls through to `v`, so a NaN lane survives the way the
/// scalar clamp returns it.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
pub(super) fn clamp_span<S, V>(simd: S, v: V, lo: f32, hi: f32) -> V
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
    V::Mask: Select<V>,
{
    let lo = V::splat(simd, lo);
    let hi = V::splat(simd, hi);
    let v = v.simd_lt(lo).select(lo, v);
    v.simd_gt(hi).select(hi, v)
}

/// `extend_t` lane-wise: the extended parameter and the mask of lanes
/// the domain covers — every lane for `Pad`/`Repeat`/`Reflect`, the
/// `[0, 1]` lanes for `Extend::None`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub(super) fn extend_t_span<S, V>(simd: S, t: V, extend: Extend) -> (V, V::Mask)
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
    V::Mask: Select<V>,
{
    let zero = V::splat(simd, 0.0);
    let one = V::splat(simd, 1.0);
    let all = zero.simd_eq(zero);
    match extend {
        Extend::Pad => (clamp_span(simd, t, 0.0, 1.0), all),
        Extend::Repeat => (t - t.floor(), all),
        // `t − 2·floor(t/2)`: the product is exact, so the one rounding
        // is the subtraction's — what a fused multiply-add would give.
        Extend::Reflect => {
            let m = t - (t * V::splat(simd, 0.5)).floor() * V::splat(simd, 2.0);
            (m.simd_gt(one).select(V::splat(simd, 2.0) - m, m), all)
        }
        Extend::None => (t, t.simd_ge(zero) & t.simd_le(one)),
    }
}

/// `extend` folded to a constant per instantiation: `block` is
/// [`extend_t_span`] at `LANES`, `lane` the same extension on one
/// scalar parameter — the modes' strict compares are exact in scalar
/// code, and `Reflect`'s `t − 2·floor(t/2)` is exact in both.
pub(super) trait ExtendSpan<S: Simd> {
    /// [`extend_t_span`] at block width.
    fn block(simd: S, t: f32x16<S>) -> (f32x16<S>, mask32x16<S>);
    /// The same extension on one parameter, with its coverage flag —
    /// `Extend::None` outside `[0, 1]` yields transparent.
    fn lane(simd: S, t: f32) -> (f32, bool);
}

macro_rules! extend_span {
    ($name:ident, $extend:expr, |$simd:ident, $t:ident| $lane:expr) => {
        /// The mode's [`ExtendSpan`] instantiation.
        pub(super) struct $name;
        impl<S: Simd> ExtendSpan<S> for $name {
            #[inline(always)]
            fn block(simd: S, t: f32x16<S>) -> (f32x16<S>, mask32x16<S>) {
                extend_t_span(simd, t, $extend)
            }
            #[inline(always)]
            fn lane($simd: S, $t: f32) -> (f32, bool) {
                $lane
            }
        }
    };
}

extend_span!(PadSpan, Extend::Pad, |_simd, t| {
    // The block's selects as scalar `if`s: a strict comparison falls
    // through to `t`, so NaN and the zero sign survive.
    let t = if t < 0.0 { 0.0 } else { t };
    (if t > 1.0 { 1.0 } else { t }, true)
});
extend_span!(RepeatSpan, Extend::Repeat, |_simd, t| {
    (t - t.floor(), true)
});
extend_span!(ReflectSpan, Extend::Reflect, |_simd, t| {
    // The block's `t − 2·floor(t/2)`, exact as there.
    #[expect(
        clippy::suboptimal_flops,
        reason = "the product is exact, so mul_add rounds identically; without the level's fma feature f32::mul_add is a libm call"
    )]
    let m = t - (t * 0.5).floor() * 2.0;
    (if m > 1.0 { 2.0 - m } else { m }, true)
});
extend_span!(NoneSpan, Extend::None, |_simd, t| {
    (t, (0.0..=1.0).contains(&t))
});
