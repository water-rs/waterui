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

use std::any::TypeId;
use std::mem::MaybeUninit;

use cherenkov::Extend;
use fearless_simd::{Select, Simd, SimdFloat, f32x4, f32x16, mask32x16};

/// Lanes per span-kernel block; a run's remainder goes through the
/// one-pixel path or one padded block.
pub(super) const LANES: usize = 16;

/// A span below this many pixels takes the one-pixel path, inline in
/// the caller: a padded block breaks even against per-pixel evaluation
/// at nine pixels.
const SHORT_SPAN: usize = 9;

/// A block path's remainder below this many pixels takes the one-pixel
/// path: the block's fixed cost is already paid.
const SHORT_TAIL: usize = 8;

/// `a·b + c` with the scalar evaluator's single fused rounding on every
/// level — the level's `mul_add` where it fuses (NEON, AVX2), per-lane
/// `f32::mul_add` elsewhere: an unfused level's separate product rounds
/// a finite `a·b` to `±∞`, and `a·b + c` then yields `NaN` where the
/// fused expression stays `±∞` — which of those a lane sees decides
/// whether a pixel paints at all, not just its last bit. The fallback
/// is an explicit loop, not `from_fn`, per the no-closure rule above.
///
/// The branch tests the token's type, not `simd.level()`: a `Fallback`
/// token reports the compile-time baseline level, which is NEON on
/// aarch64 and AVX2 on a statically AVX2 x86 build, while its own
/// `mul_add` stays unfused. The comparison folds away per
/// monomorphization.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub(super) fn fused_mul_add<S, V>(simd: S, a: V, b: V, c: V) -> V
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
{
    #[cfg(target_arch = "aarch64")]
    if TypeId::of::<S>() == TypeId::of::<fearless_simd::Neon>() {
        return a.mul_add(b, c);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if TypeId::of::<S>() == TypeId::of::<fearless_simd::Avx2>() {
        return a.mul_add(b, c);
    }
    let mut out = V::splat(simd, 0.0);
    for i in 0..V::N {
        out[i] = a[i].mul_add(b[i], c[i]);
    }
    out
}

/// What the span driver needs of a kind: one [`LANES`]-wide block from
/// device x-centres `xs` on row `dy`'s centre, and the one-pixel path
/// for what the blocks leave. `'a` is the borrowed-parameters lifetime
/// an evaluation carries.
pub(super) trait SpanKernel<'a, S: Simd> {
    /// The block's four premultiplied channels.
    fn block(&self, simd: S, xs: f32x16<S>, dy: f32x16<S>) -> [f32x16<S>; 4];
    /// The one-pixel path over `out`, `x` stepping the device pixel
    /// centres by `1.0` — the same value the block lanes form,
    /// `(x0 + base) as f32 + i + 0.5`, exact below 2²³.
    fn run(&self, simd: S, x: f32, y: f32, out: &mut [MaybeUninit<[f32; 4]>]);
}

/// `apply` lane-wise: the same two [`fused_mul_add`]s per lane — the
/// scalar expression's single-rounding map.
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
        fused_mul_add(
            simd,
            V::splat(simd, m[0]),
            x,
            fused_mul_add(simd, V::splat(simd, m[2]), y, V::splat(simd, m[4])),
        ),
        fused_mul_add(
            simd,
            V::splat(simd, m[1]),
            x,
            fused_mul_add(simd, V::splat(simd, m[3]), y, V::splat(simd, m[5])),
        ),
    )
}

/// `extend_t` lane-wise: the extended parameter and the mask of lanes
/// the domain covers — every lane for `Pad`/`Repeat`/`Reflect`, the
/// `[0, 1]` lanes for `Extend::None`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn extend_t_span<S, V>(simd: S, t: V, extend: Extend) -> (V, V::Mask)
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
    V::Mask: Select<V>,
{
    let zero = V::splat(simd, 0.0);
    let one = V::splat(simd, 1.0);
    let all = zero.simd_eq(zero);
    match extend {
        // f32::clamp: a strict comparison falls through to `t`, so NaN
        // and the zero sign survive.
        Extend::Pad => {
            let t = t.simd_lt(zero).select(zero, t);
            (t.simd_gt(one).select(one, t), all)
        }
        Extend::Repeat => (t - t.floor(), all),
        Extend::Reflect => {
            let m = fused_mul_add(
                simd,
                (t * V::splat(simd, 0.5)).floor(),
                V::splat(simd, -2.0),
                t,
            );
            (m.simd_gt(one).select(V::splat(simd, 2.0) - m, m), all)
        }
        Extend::None => (t, t.simd_ge(zero) & t.simd_le(one)),
    }
}

/// `extend` folded to a constant per instantiation: `block` is
/// [`extend_t_span`] at `LANES`, `lane` the same extension on one
/// scalar parameter — the modes' strict compares are exact in scalar
/// code, and `Reflect`'s multiply-add is `f32::mul_add`, the same
/// fused rounding [`fused_mul_add`] gives the block lanes.
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
    // `floor(t·½)·(−2) + t` — `f32::mul_add`, the same fused rounding
    // the block lanes' `fused_mul_add` gives on every level.
    let m = (-2.0f32).mul_add((t * 0.5).floor(), t);
    (if m > 1.0 { 2.0 - m } else { m }, true)
});
extend_span!(NoneSpan, Extend::None, |_simd, t| {
    (t, (0.0..=1.0).contains(&t))
});

/// Lanes `4k..4k+4` of a sixteen-lane vector.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn lane4<S: Simd>(simd: S, v: f32x16<S>, k: usize) -> f32x4<S> {
    let (lo, hi) = simd.split_f32x16(v);
    let (q0, q1) = simd.split_f32x8(lo);
    let (q2, q3) = simd.split_f32x8(hi);
    match k {
        0 => q0,
        1 => q1,
        2 => q2,
        _ => q3,
    }
}

/// The interleaved sixteen-float group holding lanes `4k..4k+4` of a
/// block's premultiplied channels — pixels `4k..4k+4`, ready for
/// [`Simd::store_interleaved_128_f32x16`].
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn block_group<S: Simd>(simd: S, px: &[f32x16<S>; 4], k: usize) -> f32x16<S> {
    simd.combine_f32x8(
        simd.combine_f32x4(lane4(simd, px[0], k), lane4(simd, px[1], k)),
        simd.combine_f32x4(lane4(simd, px[2], k), lane4(simd, px[3], k)),
    )
}

/// The premultiplied channels of a block transposed into `stage` as
/// sixteen `[r, g, b, a]` pixels — the solid kernel's interleaved
/// store, one 128-bit group per four lanes. A full block's `stage`
/// aliases `out` directly.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn store_block<S: Simd>(simd: S, px: [f32x16<S>; 4], stage: &mut [f32; 64]) {
    for (k, group) in stage.as_chunks_mut::<16>().0.iter_mut().enumerate() {
        simd.store_interleaved_128_f32x16(block_group(simd, &px, k), group);
    }
}

/// Store up to four pixels from a statically selected quarter of a
/// block. Keeping the quarter constant lets the level's registers
/// feed the transpose directly instead of indexing spilled vectors.
///
/// # Safety
/// `K < 4` and `out` is writable for `count.min(4)` pixels.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
unsafe fn store_quarter<S: Simd, const K: usize>(
    simd: S,
    px: &[f32x16<S>; 4],
    out: *mut MaybeUninit<[f32; 4]>,
    count: usize,
) {
    if count >= 4 {
        // SAFETY: the caller supplies at least `count` writable pixels.
        let group = unsafe { &mut *out.cast::<[f32; 16]>() };
        simd.store_interleaved_128_f32x16(block_group(simd, px, K), group);
    } else if count != 0 {
        let mut stage = [0.0f32; 16];
        simd.store_interleaved_128_f32x16(block_group(simd, px, K), &mut stage);
        let dst = out.cast::<f32>();
        // SAFETY: `stage` is initialized and `count * 4` floats fit
        // in the caller's remainder.
        unsafe {
            match count {
                1 => std::ptr::copy_nonoverlapping(stage.as_ptr(), dst, 4),
                2 => std::ptr::copy_nonoverlapping(stage.as_ptr(), dst, 8),
                _ => std::ptr::copy_nonoverlapping(stage.as_ptr(), dst, 12),
            }
        }
    }
}

/// Store an eight-to-fifteen-pixel prefix with constant quarter
/// selections. The first two quarters are always complete.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn store_partial<S: Simd>(simd: S, px: [f32x16<S>; 4], out: &mut [MaybeUninit<[f32; 4]>]) {
    let dst = out.as_mut_ptr();
    // SAFETY: the prefix contains eight to fifteen pixels. Each
    // constant quarter is in range and writes only its live pixels;
    // a zero count does not dereference the one-past-the-end pointer.
    unsafe {
        store_quarter::<S, 0>(simd, &px, dst, 4);
        store_quarter::<S, 1>(simd, &px, dst.add(4), 4);
        store_quarter::<S, 2>(simd, &px, dst.add(8), out.len() - 8);
        if out.len() > 12 {
            store_quarter::<S, 3>(simd, &px, dst.add(12), out.len() - 12);
        }
    }
}

/// A span kernel over `out`: `LANES`-wide blocks inline — one block
/// body, one call site — full blocks storing straight into `out`, a
/// [`SHORT_TAIL`]-or-wider remainder taking a padded block through the
/// same body. A span under [`SHORT_SPAN`] pixels takes the one-pixel
/// path; so does a remainder under [`SHORT_TAIL`]. What the blocks
/// leave reaches `run` once.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel indices and band offsets are far below 2^24"
)]
#[expect(
    clippy::cast_possible_wrap,
    reason = "device coordinates are far below 2^24"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub(super) fn span<'a, 'o, S, K>(
    simd: S,
    kernel: &K,
    x0: usize,
    py: usize,
    out: &'o mut [MaybeUninit<[f32; 4]>],
) -> &'o mut [[f32; 4]]
where
    S: Simd,
    K: SpanKernel<'a, S>,
{
    let mut base = 0;
    if out.len() >= SHORT_SPAN {
        let dy = simd.splat_f32x16((py as i64) as f32 + 0.5);
        let ramp = simd.load_array_f32x16([
            0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5, 12.5, 13.5, 14.5, 15.5,
        ]);
        // One `block` call site: the run's full blocks and a padded
        // remainder share one inlined f32x16 block body.
        loop {
            let remaining = out.len() - base;
            let xs = simd.add_f32x16(simd.splat_f32x16(((x0 + base) as i64) as f32), ramp);
            let px = kernel.block(simd, xs, dy);
            if remaining < LANES {
                store_partial(simd, px, &mut out[base..]);
                base = out.len();
                break;
            }
            // SAFETY: `out[base..base + LANES]` is `LANES * 4`
            // contiguous uninitialised floats — every bit pattern
            // is a valid f32 — and `stage` is exclusive.
            let stage = unsafe { &mut *out[base..].as_mut_ptr().cast::<[f32; 64]>() };
            store_block(simd, px, stage);
            base += LANES;
            if out.len() - base < SHORT_TAIL {
                break;
            }
        }
    }
    // One `run` call site: a short span's whole `out`, or the
    // remainder the blocks left.
    if base < out.len() {
        let (x, y) = (((x0 + base) as i64) as f32 + 0.5, (py as i64) as f32 + 0.5);
        kernel.run(simd, x, y, &mut out[base..]);
    }
    // SAFETY: the blocks and the one-pixel path together initialized
    // every element of `out`.
    unsafe { out.assume_init_mut() }
}
