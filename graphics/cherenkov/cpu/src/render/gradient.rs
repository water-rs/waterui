//! SIMD span kernels for the gradient paints: sixteen lanes of device
//! columns at a time through the device→content map, the parameter
//! projection and `extend_t`/`eval_stops`' vector counterparts — the
//! same expressions the scalar path evaluates per pixel. Each lane
//! depends only on its own column, never on its position in the span,
//! and agrees with `PaintData::eval` to within rounding: the vector
//! `mul_add` is whatever the level offers, fused or not.
//!
//! [`apply_span`], [`extend_t_span`] and [`eval_stops_span`] operate on
//! a vector of parameters rather than a single paint kind: the radial
//! and sweep kernels reuse them over their own parameter projections.

use std::mem::MaybeUninit;

use cherenkov::{Extend, Interpolation};
use fearless_simd::{Simd, f32x4, f32x16, mask32x16};

use super::paint::{Linear, PaintData, Stop, affine_f32, to_premul};

/// Lanes per gradient-kernel block; the tail is evaluated in the same
/// lanes and copied only up to the run's length.
const LANES: usize = 16;

/// `apply` lane-wise: the same two multiply-adds per lane.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn apply_span<S: Simd>(simd: S, m: [f32; 6], x: f32x16<S>, y: f32x16<S>) -> (f32x16<S>, f32x16<S>) {
    (
        simd.mul_add_f32x16(
            simd.splat_f32x16(m[0]),
            x,
            simd.mul_add_f32x16(simd.splat_f32x16(m[2]), y, simd.splat_f32x16(m[4])),
        ),
        simd.mul_add_f32x16(
            simd.splat_f32x16(m[1]),
            x,
            simd.mul_add_f32x16(simd.splat_f32x16(m[3]), y, simd.splat_f32x16(m[5])),
        ),
    )
}

/// `extend_t` lane-wise: the extended parameter and the mask of lanes
/// the gradient covers — every lane for `Pad`/`Repeat`/`Reflect`, the
/// `[0, 1]` lanes for `Extend::None`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn extend_t_span<S: Simd>(simd: S, t: f32x16<S>, extend: Extend) -> (f32x16<S>, mask32x16<S>) {
    let zero = simd.splat_f32x16(0.0);
    let one = simd.splat_f32x16(1.0);
    let all = simd.simd_eq_f32x16(zero, zero);
    match extend {
        // f32::clamp: a strict comparison falls through to `t`, so NaN
        // and the zero sign survive.
        Extend::Pad => {
            let t = simd.select_f32x16(simd.simd_lt_f32x16(t, zero), zero, t);
            (simd.select_f32x16(simd.simd_gt_f32x16(t, one), one, t), all)
        }
        Extend::Repeat => (simd.sub_f32x16(t, simd.floor_f32x16(t)), all),
        Extend::Reflect => {
            let m = simd.mul_add_f32x16(
                simd.floor_f32x16(simd.mul_f32x16(t, simd.splat_f32x16(0.5))),
                simd.splat_f32x16(-2.0),
                t,
            );
            (
                simd.select_f32x16(
                    simd.simd_gt_f32x16(m, one),
                    simd.sub_f32x16(simd.splat_f32x16(2.0), m),
                    m,
                ),
                all,
            )
        }
        Extend::None => (
            t,
            simd.and_mask32x16(simd.simd_ge_f32x16(t, zero), simd.simd_le_f32x16(t, one)),
        ),
    }
}

/// `eval_stops` lane-wise over `t`, returning premultiplied linear P3
/// channels. The first matching window wins, in the scalar loop's
/// window order; a lane no window matches — a NaN parameter — takes the
/// last stop, the scalar fall-through.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_stops_span<S: Simd>(
    simd: S,
    stops: &[Stop],
    t: f32x16<S>,
    interpolation: Interpolation,
) -> [f32x16<S>; 4] {
    let Some(first) = stops.first() else {
        return [simd.splat_f32x16(0.0); 4];
    };
    if stops.len() == 1 {
        let c = to_premul(first.color, interpolation);
        return [
            simd.splat_f32x16(c[0]),
            simd.splat_f32x16(c[1]),
            simd.splat_f32x16(c[2]),
            simd.splat_f32x16(c[3]),
        ];
    }
    let last = &stops[stops.len() - 1];
    let splat = |v: f32| simd.splat_f32x16(v);
    let mut mixed = first.color.map(|c| simd.splat_f32x16(c));
    let mut done = simd.simd_le_f32x16(t, splat(first.offset));
    let take_last = simd.and_mask32x16(
        simd.simd_ge_f32x16(t, splat(last.offset)),
        simd.not_mask32x16(done),
    );
    for (c, out) in last.color.iter().zip(mixed.iter_mut()) {
        *out = simd.select_f32x16(take_last, splat(*c), *out);
    }
    done = simd.or_mask32x16(done, take_last);
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let inside = simd.and_mask32x16(
            simd.and_mask32x16(
                simd.simd_ge_f32x16(t, splat(a.offset)),
                simd.simd_le_f32x16(t, splat(b.offset)),
            ),
            simd.not_mask32x16(done),
        );
        let span = b.offset - a.offset;
        let f = if span > 0.0 {
            simd.div_f32x16(simd.sub_f32x16(t, splat(a.offset)), splat(span))
        } else {
            simd.splat_f32x16(0.0)
        };
        for (i, out) in mixed.iter_mut().enumerate() {
            let mixed_lane =
                simd.mul_add_f32x16(f, splat(b.color[i] - a.color[i]), splat(a.color[i]));
            *out = simd.select_f32x16(inside, mixed_lane, *out);
        }
        done = simd.or_mask32x16(done, inside);
    }
    // The scalar loop's fall-through: a lane matching nothing — a NaN
    // parameter — yields the last stop.
    let remaining = simd.not_mask32x16(done);
    for (c, out) in last.color.iter().zip(mixed.iter_mut()) {
        *out = simd.select_f32x16(remaining, splat(*c), *out);
    }
    premul_span(simd, mixed, interpolation)
}

/// `to_premul` lane-wise: working-space premultiplication vectorizes;
/// the sRGB-encoded space's per-channel `powf` stays scalar per lane,
/// the `convert_packed` pattern.
#[expect(
    clippy::many_single_char_names,
    reason = "r/g/b/a are the channel names"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn premul_span<S: Simd>(
    simd: S,
    mixed: [f32x16<S>; 4],
    interpolation: Interpolation,
) -> [f32x16<S>; 4] {
    if interpolation == Interpolation::Working {
        let [r, g, b, a] = mixed;
        [
            simd.mul_f32x16(r, a),
            simd.mul_f32x16(g, a),
            simd.mul_f32x16(b, a),
            a,
        ]
    } else {
        let [r, g, b, a] = mixed.map(|c| simd.as_array_f32x16(c));
        let mut out = [[0.0f32; 16]; 4];
        for lane in 0..LANES {
            let p = to_premul([r[lane], g[lane], b[lane], a[lane]], interpolation);
            for (ch, dst) in out.iter_mut().enumerate() {
                dst[lane] = p[ch];
            }
        }
        out.map(|c| simd.load_array_f32x16(c))
    }
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

/// The premultiplied channels of a block transposed into `stage` as
/// sixteen `[r, g, b, a]` pixels — the solid kernel's interleaved store,
/// one 128-bit group per four lanes.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn store_block<S: Simd>(simd: S, px: [f32x16<S>; 4], stage: &mut [f32; 64]) {
    for (k, group) in stage.as_chunks_mut::<16>().0.iter_mut().enumerate() {
        let planes = simd.combine_f32x8(
            simd.combine_f32x4(lane4(simd, px[0], k), lane4(simd, px[1], k)),
            simd.combine_f32x4(lane4(simd, px[2], k), lane4(simd, px[3], k)),
        );
        simd.store_interleaved_128_f32x16(planes, group);
    }
}

/// `PaintData::Linear` over `out`: `outer`'s `Transformed` chain
/// inverses then `params.inv` map each lane's device centre into
/// content space, one affine apply per link so the chain keeps the
/// scalar rounding; the projection, extend mode and shared stop
/// evaluation follow. Blocks run sixteen lanes and copy the run's
/// length, so the tail is the same evaluation, never a scalar variant.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel indices and band offsets are far below 2^24"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub(super) fn linear_span<'a, S: Simd>(
    simd: S,
    outer: &PaintData,
    params: &Linear<'_>,
    x0: usize,
    py: usize,
    out: &'a mut [MaybeUninit<[f32; 4]>],
) -> &'a mut [[f32; 4]] {
    let dy = simd.splat_f32x16(py as f32 + 0.5);
    let ramp = simd.load_array_f32x16([
        0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5, 12.5, 13.5, 14.5, 15.5,
    ]);
    let (sx, sy, ex, ey) = params.end_points.into();
    let (ddx, ddy) = (ex - sx, ey - sy);
    let len2 = ddy.mul_add(ddy, ddx * ddx);
    let zero = simd.splat_f32x16(0.0);
    let mut stage = [0.0f32; 64];
    let mut base = 0;
    while base < out.len() {
        let n = (out.len() - base).min(LANES);
        let xs = simd.add_f32x16(simd.splat_f32x16((x0 + base) as f32), ramp);
        let (mut cx, mut cy) = (xs, dy);
        let mut link = outer;
        while let PaintData::Transformed(inner, inverse) = link {
            (cx, cy) = apply_span(simd, affine_f32(*inverse), cx, cy);
            link = inner;
        }
        (cx, cy) = apply_span(simd, params.inv, cx, cy);
        let t = if len2 == 0.0 {
            zero
        } else {
            simd.div_f32x16(
                simd.mul_add_f32x16(
                    simd.sub_f32x16(cy, simd.splat_f32x16(sy)),
                    simd.splat_f32x16(ddy),
                    simd.mul_f32x16(
                        simd.sub_f32x16(cx, simd.splat_f32x16(sx)),
                        simd.splat_f32x16(ddx),
                    ),
                ),
                simd.splat_f32x16(len2),
            )
        };
        let (t, active) = extend_t_span(simd, t, params.extend);
        let mut px = eval_stops_span(simd, params.stops, t, params.interpolation);
        for c in &mut px {
            *c = simd.select_f32x16(active, *c, zero);
        }
        store_block(simd, px, &mut stage);
        // SAFETY: `stage` holds sixteen initialised pixels and
        // `out[base..base + n]` is `n * 4` contiguous uninit floats.
        unsafe {
            std::ptr::copy_nonoverlapping(
                stage.as_ptr(),
                out[base..].as_mut_ptr().cast::<f32>(),
                n * 4,
            );
        }
        base += n;
    }
    // SAFETY: every element of `out` was copied from an initialised stage.
    unsafe { out.assume_init_mut() }
}
