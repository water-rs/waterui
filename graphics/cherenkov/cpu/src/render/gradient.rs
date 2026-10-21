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
//! width: `f32x16` for a full block, `f32x4` for a short span's
//! one-pixel path. The level's ops are the same at every width —
//! `mul_add` is fused or unfused at both — so a lone pixel agrees with
//! the block lanes bit for bit. They operate on a vector of
//! parameters rather than a single paint kind: the radial and sweep
//! kernels reuse them over their own parameter projections.
//!
//! No closure and nothing that takes one (`Option::map_or`,
//! `[T; N]::map`, iterator adapters) may appear on the kernel path: a
//! closure is its own function and does not inherit the dispatched
//! level's target features, so an intrinsic it holds compiles into an
//! out-of-line call on x86. Write `match`, explicit arrays and plain
//! loops instead, and where clippy suggests the combinator silence it
//! with an `expect` stating this reason.

use std::mem::MaybeUninit;

use cherenkov::{Extend, Interpolation};
use fearless_simd::{Select, Simd, SimdFloat, SimdMask, f32x4, f32x16, mask32x16};

use super::paint::{LinearPaint, PaintData, Project, Stop, to_premul};

/// Lanes per gradient-kernel block; a run's remainder goes through the
/// one-pixel path or one padded block.
const LANES: usize = 16;

/// A span below this many pixels takes the one-pixel path, inline in
/// the caller: a padded block breaks even against per-pixel evaluation
/// at nine pixels.
const SHORT_SPAN: usize = 9;

/// A block path's remainder below this many pixels takes the one-pixel
/// path: the block's fixed cost is already paid.
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
    let mut mixed = [
        V::splat(simd, first.color[0]),
        V::splat(simd, first.color[1]),
        V::splat(simd, first.color[2]),
        V::splat(simd, first.color[3]),
    ];
    let mut done = t.simd_le(V::splat(simd, first.offset));
    let take_last = t.simd_ge(V::splat(simd, last.offset)) & !done;
    for (c, out) in last.color.iter().zip(mixed.iter_mut()) {
        *out = take_last.select(V::splat(simd, *c), *out);
    }
    done |= take_last;
    for w in stops.windows(2) {
        // Each lane keeps its first matching window. Once every
        // lane is resolved, later windows cannot change the result.
        if done.all_true() {
            return mixed;
        }
        let (a, b) = (&w[0], &w[1]);
        let inside =
            t.simd_ge(V::splat(simd, a.offset)) & t.simd_le(V::splat(simd, b.offset)) & !done;
        let span = b.offset - a.offset;
        let f = if span > 0.0 {
            (t - V::splat(simd, a.offset)) / V::splat(simd, span)
        } else {
            V::splat(simd, 0.0)
        };
        for (i, out) in mixed.iter_mut().enumerate() {
            *out = inside.select(
                f.mul_add(
                    V::splat(simd, b.color[i] - a.color[i]),
                    V::splat(simd, a.color[i]),
                ),
                *out,
            );
        }
        done |= inside;
    }
    // The scalar loop's fall-through: a lane matching nothing — a NaN
    // parameter — yields the last stop.
    let remaining = !done;
    for (c, out) in last.color.iter().zip(mixed.iter_mut()) {
        *out = remaining.select(V::splat(simd, *c), *out);
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
        [
            V::from_slice(simd, &out[0][..V::N]),
            V::from_slice(simd, &out[1][..V::N]),
            V::from_slice(simd, &out[2][..V::N]),
            V::from_slice(simd, &out[3][..V::N]),
        ]
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

/// `premul_span`'s working-space arm on one pixel's channel vector:
/// `[r·a, g·a, b·a, a]` as one multiply — the same IEEE products — and
/// a blend restoring the alpha lane. The encoded space stays scalar,
/// `to_premul`, as the block's per-lane conversion does.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn premul_pixel<S: Simd>(simd: S, c: f32x4<S>, working: bool) -> f32x4<S> {
    if working {
        let premul = c * simd.splat_f32x4(c[3]);
        // Lane 3 holds `a·a`; restore `a` itself.
        simd.load_array_mask32x4([0, 0, 0, -1]).select(c, premul)
    } else {
        simd.load_array_f32x4(to_premul(c.into(), Interpolation::SrgbEncoded))
    }
}

/// `eval_stops`' early-exit window search on one pixel: the control
/// flow is scalar — `t ≤ first` / `t ≥ last` first, then the first
/// matching window, the NaN fall-through to the last stop — `f` is the
/// exactly rounded scalar quotient, and the window mix is one
/// four-channel `mul_add`, the block lane's expression. `stops` holds
/// at least two entries.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_stops_pixel<S: Simd>(simd: S, stops: &[Stop], t: f32, working: bool) -> f32x4<S> {
    let first = &stops[0];
    if t <= first.offset {
        return premul_pixel(simd, simd.load_array_f32x4(first.color), working);
    }
    let last = &stops[stops.len() - 1];
    if t >= last.offset {
        return premul_pixel(simd, simd.load_array_f32x4(last.color), working);
    }
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t >= a.offset && t <= b.offset {
            let span = b.offset - a.offset;
            let f = if span > 0.0 {
                (t - a.offset) / span
            } else {
                0.0
            };
            let a4 = simd.load_array_f32x4(a.color);
            let b4 = simd.load_array_f32x4(b.color);
            let mixed = simd.splat_f32x4(f).mul_add(b4 - a4, a4);
            return premul_pixel(simd, mixed, working);
        }
    }
    premul_pixel(simd, simd.load_array_f32x4(last.color), working)
}

/// `extend` folded to a constant per instantiation: `block` is
/// [`extend_t_span`] at `LANES`, `lane` the same extension on one
/// scalar parameter — the modes' strict compares are exact in scalar
/// code, and `Reflect`'s `mul_add` goes through the level's own op on
/// lane 0 of an `f32x4`.
trait ExtendSpan<S: Simd> {
    /// [`extend_t_span`] at block width.
    fn block(simd: S, t: f32x16<S>) -> (f32x16<S>, mask32x16<S>);
    /// The same extension on one parameter, with its coverage flag —
    /// `Extend::None` outside `[0, 1]` yields transparent.
    fn lane(simd: S, t: f32) -> (f32, bool);
}

macro_rules! extend_span {
    ($name:ident, $extend:expr, |$simd:ident, $t:ident| $lane:expr) => {
        /// The mode's [`ExtendSpan`] instantiation.
        struct $name;
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
extend_span!(ReflectSpan, Extend::Reflect, |simd, t| {
    // `floor(t·½)·(−2) + t` through the level's own `mul_add` on lane
    // 0, the fused-or-not rounding the block lanes get.
    let m = simd
        .splat_f32x4((t * 0.5).floor())
        .mul_add(simd.splat_f32x4(-2.0), simd.splat_f32x4(t))[0];
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

/// The span-invariant evaluation, decided once per span: the
/// device→content map after the `Transformed` chain (each link's f32
/// inverse lives on `PaintData` since lowering), the projection —
/// absent for a degenerate gradient, whose parameter is fixed at 0 —
/// the stops as a window walk or a constant colour, and the
/// premultiplication branch.
#[derive(Clone, Copy)]
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
    /// Decides every span-invariant choice once: the projection —
    /// computed at lowering — the constant-colour stop lists and the
    /// premultiplication branch.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn new(outer: &'a PaintData, params: &'a LinearPaint) -> Self {
        Self {
            outer,
            inv: params.inv,
            project: params.project,
            stops: match &*params.stops {
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
/// extend mode as a runtime choice on one block copy and the
/// span-decided stops — premultiplied channels, zero where
/// `Extend::None` leaves the domain.
#[expect(
    clippy::option_if_let_else,
    reason = "a map_or closure is compiled without the level's target features"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_block<S: Simd>(
    simd: S,
    eval: &LinearEval<'_>,
    xs: f32x16<S>,
    dy: f32x16<S>,
    extend: Extend,
) -> [f32x16<S>; 4] {
    let (mut cx, mut cy) = (xs, dy);
    let mut link = eval.outer;
    while let PaintData::Transformed(inner) = link {
        (cx, cy) = apply_span(simd, inner.inverse_f32, cx, cy);
        link = &inner.paint;
    }
    let (cx, cy) = apply_span(simd, eval.inv, cx, cy);
    let zero = simd.splat_f32x16(0.0);
    let t = match eval.project {
        Some(p) => project_span(simd, &p, cx, cy),
        None => zero,
    };
    let (t, active) = match extend {
        Extend::Pad => PadSpan::block(simd, t),
        Extend::Repeat => RepeatSpan::block(simd, t),
        Extend::Reflect => ReflectSpan::block(simd, t),
        Extend::None => NoneSpan::block(simd, t),
    };
    let mut px = match eval.stops {
        Ok(stops) => premul_span(simd, eval_stops_span(simd, stops, t), eval.working),
        Err(c) => [
            simd.splat_f32x16(c[0]),
            simd.splat_f32x16(c[1]),
            simd.splat_f32x16(c[2]),
            simd.splat_f32x16(c[3]),
        ],
    };
    for c in &mut px {
        *c = active.select(*c, zero);
    }
    px
}

/// One pixel of the block expression in channel-vector form: `outer`'s
/// chain — present only in the `CHAINED` instantiation — and `inv`
/// applied with the level's own `mul_add` on lane 0 of an `f32x4`, the
/// same arithmetic, fused or not, the block lanes get; the scalar
/// extend and early-exit stop search, one four-channel mix and
/// premultiply, zero where `Extend::None` leaves the domain.
#[expect(
    clippy::option_if_let_else,
    reason = "a map_or closure is compiled without the level's target features"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_pixel<S, E, const CHAINED: bool>(simd: S, eval: LinearEval<'_>, x: f32, y: f32) -> f32x4<S>
where
    S: Simd,
    E: ExtendSpan<S>,
{
    let (mut cx, mut cy) = (simd.splat_f32x4(x), simd.splat_f32x4(y));
    if CHAINED {
        let mut link = eval.outer;
        while let PaintData::Transformed(inner) = link {
            (cx, cy) = apply_span(simd, inner.inverse_f32, cx, cy);
            link = &inner.paint;
        }
    }
    let (cx, cy) = apply_span(simd, eval.inv, cx, cy);
    let t = match eval.project {
        Some(p) => project_span(simd, &p, cx, cy)[0],
        None => 0.0,
    };
    let (t, active) = E::lane(simd, t);
    if !active {
        return simd.splat_f32x4(0.0);
    }
    match eval.stops {
        Ok(stops) => eval_stops_pixel(simd, stops, t, eval.working),
        Err(c) => simd.load_array_f32x4(c),
    }
}

/// `eval_pixel` over a run of `out`, `x` stepping the device pixel
/// centres by `1.0` — the same value the block lanes form,
/// `(x0 + base) as f32 + i + 0.5`, exact below 2²³.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_pixels<S, E, const CHAINED: bool>(
    simd: S,
    eval: LinearEval<'_>,
    mut x: f32,
    y: f32,
    out: &mut [MaybeUninit<[f32; 4]>],
) where
    S: Simd,
    E: ExtendSpan<S>,
{
    for px in out.iter_mut() {
        // SAFETY: the store writes all four lanes of `px`.
        simd.store_array_f32x4(eval_pixel::<S, E, CHAINED>(simd, eval, x, y), unsafe {
            &mut *px.as_mut_ptr()
        });
        x += 1.0;
    }
}

/// The one-pixel path over a whole run, shared by the inline short
/// span and a block's short remainder: the `Transformed` test is
/// decided once, outside the loop, so the common leaf case carries no
/// chain walk.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_run<S, E>(simd: S, eval: LinearEval<'_>, x: f32, y: f32, out: &mut [MaybeUninit<[f32; 4]>])
where
    S: Simd,
    E: ExtendSpan<S>,
{
    if matches!(eval.outer, PaintData::Transformed(..)) {
        eval_pixels::<S, E, true>(simd, eval, x, y, out);
    } else {
        eval_pixels::<S, E, false>(simd, eval, x, y, out);
    }
}

/// The per-extend [`eval_run`] instances behind one call site: one
/// four-arm `match` so the mode's choice does not multiply the
/// one-pixel path — a short span's whole `out` or a block's short
/// remainder reaches it as `out[base..]`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_run_extend<S: Simd>(
    simd: S,
    eval: LinearEval<'_>,
    extend: Extend,
    x: f32,
    y: f32,
    out: &mut [MaybeUninit<[f32; 4]>],
) {
    match extend {
        Extend::Pad => eval_run::<S, PadSpan>(simd, eval, x, y, out),
        Extend::Repeat => eval_run::<S, RepeatSpan>(simd, eval, x, y, out),
        Extend::Reflect => eval_run::<S, ReflectSpan>(simd, eval, x, y, out),
        Extend::None => eval_run::<S, NoneSpan>(simd, eval, x, y, out),
    }
}

/// `PaintData::Linear` over `out`: `outer`'s `Transformed` chain
/// inverses then `params.inv` map each lane's device centre into
/// content space, one affine apply per link so the chain keeps the
/// scalar rounding; the projection, extend mode and shared stop
/// evaluation follow. Every choice that holds for the whole span is
/// decided here or in [`LinearEval`]. A span under [`SHORT_SPAN`]
/// pixels evaluates inline by [`eval_run`]; a longer span runs
/// `LANES`-wide blocks inline — one block body, one call site, the
/// extend mode chosen at runtime per block — full blocks storing
/// straight into `out`, a [`SHORT_TAIL`]-or-wider remainder taking a
/// padded block through the same body. What the blocks leave —
/// a short span's whole `out` or a remainder under [`SHORT_TAIL`]
/// pixels — takes the one-pixel path, also once.
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
pub(super) fn linear_span<'a, S: Simd>(
    simd: S,
    outer: &PaintData,
    params: &LinearPaint,
    x0: usize,
    py: usize,
    out: &'a mut [MaybeUninit<[f32; 4]>],
) -> &'a mut [[f32; 4]] {
    let eval = LinearEval::new(outer, params);
    let mut base = 0;
    if out.len() >= SHORT_SPAN {
        let dy = simd.splat_f32x16((py as i64) as f32 + 0.5);
        let ramp = simd.load_array_f32x16([
            0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5, 12.5, 13.5, 14.5, 15.5,
        ]);
        // One `eval_block` call site: the run's full blocks and a
        // padded remainder share one inlined f32x16 block body.
        loop {
            let remaining = out.len() - base;
            let xs = simd.add_f32x16(simd.splat_f32x16(((x0 + base) as i64) as f32), ramp);
            let px = eval_block(simd, &eval, xs, dy, params.extend);
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
    // One `eval_run_extend` call site: a short span's whole `out`, or
    // the remainder the blocks left.
    if base < out.len() {
        let (x, y) = (((x0 + base) as i64) as f32 + 0.5, (py as i64) as f32 + 0.5);
        eval_run_extend(simd, eval, params.extend, x, y, &mut out[base..]);
    }
    // SAFETY: the blocks and the one-pixel path together initialized
    // every element of `out`.
    unsafe { out.assume_init_mut() }
}
