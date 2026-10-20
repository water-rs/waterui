//! SIMD span kernels for the gradient paints: sixteen lanes of device
//! columns at a time through the device→content map, the parameter
//! projection and `extend_t`/`eval_stops`' vector counterparts — the
//! same expressions the scalar path evaluates per pixel. Each lane
//! depends only on its own column, never on its position in the span,
//! and agrees with `PaintData::eval` to within rounding: the vector
//! `mul_add` is whatever the level offers, fused or not.
//!
//! [`apply_span`], [`extend_t_span`], [`project_span`],
//! [`eval_stops_span`] and [`premul_span`] are generic over the vector
//! width: `f32x16` for a full block, `f32x4` for a short remainder's
//! one-lane evaluation. The level's ops are the same at every width —
//! `mul_add` is fused or unfused at both — so a remainder lane agrees
//! with the block lanes bit for bit. They operate on a vector of
//! parameters rather than a single paint kind: the radial and sweep
//! kernels reuse them over their own parameter projections.

use std::mem::MaybeUninit;

use cherenkov::{Extend, Interpolation};
use fearless_simd::{Select, Simd, SimdFloat, f32x4, f32x16, mask32x16};

use super::paint::{Linear, PaintData, Stop, to_premul};

/// Lanes per gradient-kernel block; a run's remainder goes through the
/// one-lane path or one padded block.
const LANES: usize = 16;

/// A remainder below this many pixels takes the one-lane path; at or
/// above it one padded block costs less — a block breaks even against
/// per-pixel evaluation around eight pixels.
const SHORT_TAIL: usize = 8;

/// `apply` lane-wise: the same two multiply-adds per lane.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn apply_span<S, V>(simd: S, m: [f32; 6], x: V, y: V) -> (V, V)
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
{
    (
        V::splat(simd, m[0]).mul_add(x, V::splat(simd, m[2]).mul_add(y, V::splat(simd, m[4]))),
        V::splat(simd, m[1]).mul_add(x, V::splat(simd, m[3]).mul_add(y, V::splat(simd, m[5]))),
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
            let m = (t * V::splat(simd, 0.5))
                .floor()
                .mul_add(V::splat(simd, -2.0), t);
            (m.simd_gt(one).select(V::splat(simd, 2.0) - m, m), all)
        }
        Extend::None => (t, t.simd_ge(zero) & t.simd_le(one)),
    }
}

/// `eval_stops`' window walk lane-wise over `t`, returning
/// interpolation-space straight-alpha channels for [`premul_span`].
/// The first matching window wins, in the scalar loop's window order;
/// a lane no window matches — a NaN parameter — takes the last stop,
/// the scalar fall-through. `stops` holds at least two entries;
/// shorter lists are decided once per span by [`LinearEval`].
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_stops_span<S, V>(simd: S, stops: &[Stop], t: V) -> [V; 4]
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
    V::Mask: Select<V>,
{
    let first = &stops[0];
    let last = &stops[stops.len() - 1];
    let splat = |v: f32| V::splat(simd, v);
    let mut mixed = first.color.map(&splat);
    let mut done = t.simd_le(splat(first.offset));
    let take_last = t.simd_ge(splat(last.offset)) & !done;
    for (c, out) in last.color.iter().zip(mixed.iter_mut()) {
        *out = take_last.select(splat(*c), *out);
    }
    done |= take_last;
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let inside = t.simd_ge(splat(a.offset)) & t.simd_le(splat(b.offset)) & !done;
        let span = b.offset - a.offset;
        let f = if span > 0.0 {
            (t - splat(a.offset)) / splat(span)
        } else {
            splat(0.0)
        };
        for (i, out) in mixed.iter_mut().enumerate() {
            *out = inside.select(
                f.mul_add(splat(b.color[i] - a.color[i]), splat(a.color[i])),
                *out,
            );
        }
        done |= inside;
    }
    // The scalar loop's fall-through: a lane matching nothing — a NaN
    // parameter — yields the last stop.
    let remaining = !done;
    for (c, out) in last.color.iter().zip(mixed.iter_mut()) {
        *out = remaining.select(splat(*c), *out);
    }
    mixed
}

/// `to_premul` lane-wise with the premultiplication branch already
/// decided for the span: working-space premultiplication vectorizes;
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
fn premul_span<S, V>(simd: S, mixed: [V; 4], working: bool) -> [V; 4]
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
{
    let [r, g, b, a] = mixed;
    if working {
        [r * a, g * a, b * a, a]
    } else {
        let mut out = [[0.0f32; LANES]; 4];
        for lane in 0..V::N {
            let p = to_premul(
                [r[lane], g[lane], b[lane], a[lane]],
                Interpolation::SrgbEncoded,
            );
            for (ch, dst) in out.iter_mut().enumerate() {
                dst[lane] = p[ch];
            }
        }
        out.map(|c| V::from_slice(simd, &c[..V::N]))
    }
}

/// The projection lane-wise — `((cy − sy)·ddy + (cx − sx)·ddx) / len2`
/// — the scalar evaluator's expression: one fused-or-not `mul_add`,
/// one `mul`, one `div`. Called only for a non-degenerate gradient;
/// [`LinearEval`] decides `len2 == 0.0`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn project_span<S, V>(simd: S, p: &Project, cx: V, cy: V) -> V
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
{
    (cy - V::splat(simd, p.sy)).mul_add(
        V::splat(simd, p.ddy),
        (cx - V::splat(simd, p.sx)) * V::splat(simd, p.ddx),
    ) / V::splat(simd, p.len2)
}

/// `extend_t_span` on lane 0 of a four-lane splat — the same ops as the
/// block expression, scalarized by the lane-0-only use — with the
/// coverage mask read back as a flag.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn extend_t_lane<S: Simd>(simd: S, t: f32, extend: Extend) -> (f32, bool) {
    let (t, active) = extend_t_span(simd, simd.splat_f32x4(t), extend);
    (t[0], active[0] != 0)
}

/// `premul_span` on lane 0 of a four-lane splat.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn premul_lane<S: Simd>(simd: S, c: [f32; 4], working: bool) -> [f32; 4] {
    premul_span(simd, c.map(|ch| simd.splat_f32x4(ch)), working).map(|v| v[0])
}

/// `eval_stops`' early-exit window search at one lane: the control
/// flow is scalar — `t ≤ first` / `t ≥ last` first, then the first
/// matching window, the NaN fall-through to the last stop — and the
/// window arithmetic is `eval_stops_span`'s per-lane expression on
/// lane 0 of `f32x4`, so a remainder pixel rounds exactly like the
/// block lanes. `stops` holds at least two entries.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_stops_lane<S: Simd>(simd: S, stops: &[Stop], t: f32, working: bool) -> [f32; 4] {
    let first = &stops[0];
    if t <= first.offset {
        return premul_lane(simd, first.color, working);
    }
    let last = &stops[stops.len() - 1];
    if t >= last.offset {
        return premul_lane(simd, last.color, working);
    }
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t >= a.offset && t <= b.offset {
            let span = b.offset - a.offset;
            let f = if span > 0.0 {
                (simd.splat_f32x4(t) - simd.splat_f32x4(a.offset)) / simd.splat_f32x4(span)
            } else {
                simd.splat_f32x4(0.0)
            };
            let mut mixed = [0.0f32; 4];
            for (i, out) in mixed.iter_mut().enumerate() {
                *out = f.mul_add(
                    simd.splat_f32x4(b.color[i] - a.color[i]),
                    simd.splat_f32x4(a.color[i]),
                )[0];
            }
            return premul_lane(simd, mixed, working);
        }
    }
    premul_lane(simd, last.color, working)
}

/// `extend` folded to a constant per instantiation: `block` is
/// [`extend_t_span`] at `LANES`, `lane` the same expression on one lane
/// of a four-lane splat — LLVM scalarizes the lane-0-only use.
trait ExtendSpan<S: Simd> {
    /// [`extend_t_span`] at block width.
    fn block(simd: S, t: f32x16<S>) -> (f32x16<S>, mask32x16<S>);
    /// The same expression on one lane, with its coverage flag.
    fn lane(simd: S, t: f32) -> (f32, bool);
}

macro_rules! extend_span {
    ($name:ident, $extend:expr) => {
        /// The mode's [`ExtendSpan`] instantiation.
        struct $name;
        impl<S: Simd> ExtendSpan<S> for $name {
            #[inline(always)]
            fn block(simd: S, t: f32x16<S>) -> (f32x16<S>, mask32x16<S>) {
                extend_t_span(simd, t, $extend)
            }
            #[inline(always)]
            fn lane(simd: S, t: f32) -> (f32, bool) {
                extend_t_lane(simd, t, $extend)
            }
        }
    };
}

extend_span!(PadSpan, Extend::Pad);
extend_span!(RepeatSpan, Extend::Repeat);
extend_span!(ReflectSpan, Extend::Reflect);
extend_span!(NoneSpan, Extend::None);

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
        let planes = simd.combine_f32x8(
            simd.combine_f32x4(lane4(simd, px[0], k), lane4(simd, px[1], k)),
            simd.combine_f32x4(lane4(simd, px[2], k), lane4(simd, px[3], k)),
        );
        simd.store_interleaved_128_f32x16(planes, group);
    }
}

/// The non-degenerate projection's span constants.
#[derive(Clone, Copy)]
struct Project {
    sx: f32,
    sy: f32,
    ddx: f32,
    ddy: f32,
    len2: f32,
}

/// The span-invariant evaluation, decided once per span: the
/// device→content map after the `Transformed` chain (each link's f32
/// inverse lives on `PaintData` since lowering), the projection —
/// absent for a degenerate gradient, whose parameter is fixed at 0 —
/// the stops as a window walk or a constant colour, and the
/// premultiplication branch.
struct LinearEval<'a> {
    /// The paint the chain walk starts from.
    outer: &'a PaintData,
    /// Device-to-content transform.
    inv: [f32; 6],
    /// `Some` projects to `t`; `None` is a degenerate gradient
    /// (`len2 == 0.0`), fixing `t` at 0.
    project: Option<Project>,
    /// `Ok` takes the window walk (two or more stops); `Err` is the
    /// span's constant premultiplied colour (zero or one stop).
    stops: Result<&'a [Stop], [f32; 4]>,
    /// `interpolation == Interpolation::Working`.
    working: bool,
}

impl<'a> LinearEval<'a> {
    /// Decides every span-invariant choice once: the degenerate
    /// projection, the constant-colour stop lists and the
    /// premultiplication branch.
    fn new(outer: &'a PaintData, params: &Linear<'a>) -> Self {
        let (sx, sy, ex, ey) = params.end_points.into();
        let (ddx, ddy) = (ex - sx, ey - sy);
        let len2 = ddy.mul_add(ddy, ddx * ddx);
        Self {
            outer,
            inv: params.inv,
            project: (len2 != 0.0).then_some(Project {
                sx,
                sy,
                ddx,
                ddy,
                len2,
            }),
            stops: match params.stops {
                [] => Err([0.0; 4]),
                [s] => Err(to_premul(s.color, params.interpolation)),
                stops => Ok(stops),
            },
            working: params.interpolation == Interpolation::Working,
        }
    }
}

/// One `LANES`-wide block from device x-centres `xs` on row `dy`'s
/// centre: `outer`'s chain inverses and `inv`, the projection, the
/// monomorphized extend and the span-decided stops — premultiplied
/// channels, zero where `Extend::None` leaves the domain.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_block<S, E>(simd: S, eval: &LinearEval<'_>, xs: f32x16<S>, dy: f32x16<S>) -> [f32x16<S>; 4]
where
    S: Simd,
    E: ExtendSpan<S>,
{
    let (mut cx, mut cy) = (xs, dy);
    let mut link = eval.outer;
    while let PaintData::Transformed(inner, _, inverse) = link {
        (cx, cy) = apply_span(simd, *inverse, cx, cy);
        link = inner;
    }
    let (cx, cy) = apply_span(simd, eval.inv, cx, cy);
    let zero = simd.splat_f32x16(0.0);
    let t = eval
        .project
        .map_or(zero, |p| project_span(simd, &p, cx, cy));
    let (t, active) = E::block(simd, t);
    let mut px = match eval.stops {
        Ok(stops) => premul_span(simd, eval_stops_span(simd, stops, t), eval.working),
        Err(c) => c.map(|ch| simd.splat_f32x16(ch)),
    };
    for c in &mut px {
        *c = active.select(*c, zero);
    }
    px
}

/// One pixel of the block expression on lane 0 of `f32x4`: the chain
/// inverses, `inv` and the projection through the level's own ops —
/// the same arithmetic, fused `mul_add` or not, the block lanes get —
/// with a scalar extend flag and the early-exit stop search.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_lane<S, E>(simd: S, eval: &LinearEval<'_>, x: f32, y: f32) -> [f32; 4]
where
    S: Simd,
    E: ExtendSpan<S>,
{
    let (mut cx, mut cy) = (simd.splat_f32x4(x), simd.splat_f32x4(y));
    let mut link = eval.outer;
    while let PaintData::Transformed(inner, _, inverse) = link {
        (cx, cy) = apply_span(simd, *inverse, cx, cy);
        link = inner;
    }
    let (cx, cy) = apply_span(simd, eval.inv, cx, cy);
    let t = eval
        .project
        .map_or(0.0, |p| project_span(simd, &p, cx, cy)[0]);
    let (t, active) = E::lane(simd, t);
    if !active {
        return [0.0; 4];
    }
    match eval.stops {
        Ok(stops) => eval_stops_lane(simd, stops, t, eval.working),
        Err(c) => c,
    }
}

/// The block loop, out of line and entered through `vectorize` so the
/// kernel's frame and register pressure stay out of the shared
/// `eval_span` dispatch while the body still compiles under the
/// level's target features. `E` folds the span's extend mode into the
/// instantiation. Full `LANES` blocks store straight into `out`; a
/// remainder under [`SHORT_TAIL`] pixels takes the one-lane path, a
/// wider one a single padded block through the stage.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel indices and band offsets are far below 2^24"
)]
#[inline(never)]
fn linear_blocks<S, E>(
    simd: S,
    eval: &LinearEval<'_>,
    x0: usize,
    py: usize,
    out: &mut [MaybeUninit<[f32; 4]>],
) where
    S: Simd,
    E: ExtendSpan<S>,
{
    simd.vectorize(
        #[inline(always)]
        || {
            let dy = simd.splat_f32x16(py as f32 + 0.5);
            let ramp = simd.load_array_f32x16([
                0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5, 12.5, 13.5, 14.5,
                15.5,
            ]);
            let mut base = 0;
            while base + LANES <= out.len() {
                let xs = simd.add_f32x16(simd.splat_f32x16((x0 + base) as f32), ramp);
                let px = eval_block::<S, E>(simd, eval, xs, dy);
                // SAFETY: `out[base..base + LANES]` is `LANES * 4`
                // contiguous uninitialised floats — every bit pattern
                // is a valid f32 — and `stage` is exclusive.
                let stage = unsafe { &mut *out[base..].as_mut_ptr().cast::<[f32; 64]>() };
                store_block(simd, px, stage);
                base += LANES;
            }
            let tail = out.len() - base;
            if tail == 0 {
                return;
            }
            if tail < SHORT_TAIL {
                for (i, px) in out[base..].iter_mut().enumerate() {
                    px.write(eval_lane::<S, E>(
                        simd,
                        eval,
                        (x0 + base + i) as f32 + 0.5,
                        py as f32 + 0.5,
                    ));
                }
            } else {
                let xs = simd.add_f32x16(simd.splat_f32x16((x0 + base) as f32), ramp);
                let px = eval_block::<S, E>(simd, eval, xs, dy);
                let mut stage = [0.0f32; 64];
                store_block(simd, px, &mut stage);
                // SAFETY: `stage` holds sixteen initialised pixels and
                // `out[base..base + tail]` is `tail * 4` contiguous
                // uninit floats.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        stage.as_ptr(),
                        out[base..].as_mut_ptr().cast::<f32>(),
                        tail * 4,
                    );
                }
            }
        },
    );
}

/// `PaintData::Linear` over `out`: `outer`'s `Transformed` chain
/// inverses then `params.inv` map each lane's device centre into
/// content space, one affine apply per link so the chain keeps the
/// scalar rounding; the projection, extend mode and shared stop
/// evaluation follow. Every choice that holds for the whole span is
/// decided here or in [`LinearEval`]; the extend mode monomorphizes
/// [`linear_blocks`].
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
    let eval = LinearEval::new(outer, params);
    match params.extend {
        Extend::Pad => linear_blocks::<S, PadSpan>(simd, &eval, x0, py, out),
        Extend::Repeat => linear_blocks::<S, RepeatSpan>(simd, &eval, x0, py, out),
        Extend::Reflect => linear_blocks::<S, ReflectSpan>(simd, &eval, x0, py, out),
        Extend::None => linear_blocks::<S, NoneSpan>(simd, &eval, x0, py, out),
    }
    // SAFETY: `linear_blocks` initialized every element of `out`.
    unsafe { out.assume_init_mut() }
}
