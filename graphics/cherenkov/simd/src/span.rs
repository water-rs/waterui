//! Shared pixel span blocks, tails and interleaved storage.
use core::mem::MaybeUninit;
use fearless_simd::{Simd, SimdBase, SimdInterleaved, f32x4, f32x16};

/// Lanes per span-kernel block; a run's remainder goes through the
/// one-pixel path or one padded block.
pub const LANES: usize = 16;

/// A span below this many pixels takes the one-pixel path, inline in
/// the caller: a padded block breaks even against per-pixel evaluation
/// at nine pixels.
const SHORT_SPAN: usize = 9;

/// A block path's remainder below this many pixels takes the one-pixel
/// path: the block's fixed cost is already paid.
const SHORT_TAIL: usize = 8;

/// A block kernel and its short-span path.
///
/// One [`LANES`]-wide block from device x-centres `xs` on row `dy`'s centre, and the one-pixel path
/// for what the blocks leave. `'a` is the borrowed-parameters lifetime
/// an evaluation carries.
///
/// # Safety
/// `run` must initialize every pixel in its output slice.
pub unsafe trait SpanKernel<'a, S: Simd> {
    /// The block's four premultiplied channels.
    fn block(&self, simd: S, xs: f32x16<S>, dy: f32x16<S>) -> [f32x16<S>; 4];
    /// The one-pixel path over `out`, `x` stepping the device pixel
    /// centres by `1.0` — the same value the block lanes form,
    /// `(x0 + base) as f32 + i + 0.5`, exact below 2²³.
    fn run(&self, simd: S, x: f32, y: f32, out: &mut [MaybeUninit<[f32; 4]>]);
}

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
/// [`SimdInterleaved::store_four_interleaved`].
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn block_group<S: Simd>(simd: S, px: &[f32x16<S>; 4], k: usize) -> [f32x4<S>; 4] {
    [
        lane4(simd, px[0], k),
        lane4(simd, px[1], k),
        lane4(simd, px[2], k),
        lane4(simd, px[3], k),
    ]
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
        f32x4::store_four_interleaved(block_group(simd, &px, k), group);
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
        f32x4::store_four_interleaved(block_group(simd, px, K), group);
    } else if count != 0 {
        let mut stage = [0.0f32; 16];
        f32x4::store_four_interleaved(block_group(simd, px, K), &mut stage);
        let dst = out.cast::<f32>();
        // SAFETY: `stage` is initialized and `count * 4` floats fit
        // in the caller's remainder.
        unsafe {
            match count {
                1 => core::ptr::copy_nonoverlapping(stage.as_ptr(), dst, 4),
                2 => core::ptr::copy_nonoverlapping(stage.as_ptr(), dst, 8),
                _ => core::ptr::copy_nonoverlapping(stage.as_ptr(), dst, 12),
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

/// Evaluate a pixel span with shared block and tail handling.
///
/// Full blocks store straight into `out`, a
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
pub fn span<'a, 'o, S, K>(
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
        let ramp = f32x16::load_array(
            simd,
            [
                0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5, 12.5, 13.5, 14.5,
                15.5,
            ],
        );
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
