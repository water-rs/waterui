//! SIMD span kernels for the gradient paints: sixteen lanes of device
//! columns at a time through the device→content map, the parameter
//! projection and `extend_t`/`eval_stops`' vector counterparts — the
//! same expressions the scalar path evaluates per pixel. Each lane
//! depends only on its own column, never on its position in the span,
//! and agrees with `PaintData::eval` to within what a viewer can see:
//! every multiply-add is the level's own `mul_add`, fused on NEON and
//! AVX2 and a separate product and sum elsewhere. No expression depends
//! on the fused rounding to stay finite — the radial quadratic rescales
//! lanes whose content point would overflow its terms.
//!
//! [`project_span`], [`eval_stops_span`] and [`premul_span`] are
//! generic over the vector width: `f32x16` for a full block, `f32x4`
//! for the driver's one-pixel path. Radials shorter than one block
//! solve scalar points without constructing block state; both paths
//! agree within visual rounding tolerance. Each kind decides
//! its span-invariant state once into a [`SpanEval`]; `eval_block`,
//! `eval_pixel` and the [`SpanKernel`] blanket impl are generic over
//! it — one kernel body shared by every gradient kind. The
//! kind-agnostic driver, lane helpers, extend modes and stores live in
//! `super::span`.
//!
//! No closure and nothing that takes one (`Option::map_or`,
//! `[T; N]::map`, iterator adapters) may appear on the kernel path: a
//! closure is its own function and does not inherit the dispatched
//! level's target features, so an intrinsic it holds compiles into an
//! out-of-line call on x86. Write `match`, explicit arrays and plain
//! loops instead, and where clippy suggests the combinator silence it
//! with an `expect` stating this reason.

use std::{marker::PhantomData, mem::MaybeUninit};

use cherenkov::{Extend, Interpolation};
use fearless_simd::{Select, Simd, SimdFloat, SimdMask, f32x4, f32x16};

use super::paint::{
    LinearPaint, PaintData, Project, RADIAL_RESCALE_ABOVE, RadialGeometry, RadialPaint, Stop,
    to_premul,
};
use super::span::{
    ExtendSpan, LANES, NoneSpan, PadSpan, ReflectSpan, RepeatSpan, SpanKernel, apply_span,
};

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
/// — the scalar evaluator's expression: one `mul_add`, one `mul`, one
/// `div`. Called only for a non-degenerate gradient;
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
/// four-channel `mul_add`, the block lane's expression. `stops`
/// holds at least two entries.
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

/// The span-invariant state every gradient kind's evaluation shares,
/// decided once per span by one constructor: the paint the
/// `Transformed` chain walk starts from (each link's f32 inverse lives
/// on `PaintData` since lowering), the leaf's device→content map, the
/// extend mode, the stops as a window walk or a constant colour, and
/// the premultiplication branch.
#[derive(Clone, Copy)]
struct SpanBase<'a> {
    /// The paint the chain walk starts from.
    outer: &'a PaintData,
    /// Device-to-content transform.
    inv: [f32; 6],
    /// The continuation mode.
    extend: Extend,
    /// `Ok` takes the window walk (two or more stops); `Err` is the
    /// span's constant premultiplied colour (zero or one stop).
    stops: Result<&'a [Stop], [f32; 4]>,
    /// `interpolation == Interpolation::Working`.
    working: bool,
}

impl<'a> SpanBase<'a> {
    /// Decides the shared span-invariant choices once: the
    /// constant-colour stop lists and the premultiplication branch.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn new(
        outer: &'a PaintData,
        inv: [f32; 6],
        stops: &'a std::sync::Arc<[Stop]>,
        extend: Extend,
        interpolation: Interpolation,
    ) -> Self {
        Self {
            outer,
            inv,
            extend,
            stops: match &**stops {
                [] => Err([0.0; 4]),
                [s] => Err(to_premul(s.color, interpolation)),
                stops => Ok(stops),
            },
            working: interpolation == Interpolation::Working,
        }
    }
}

/// A gradient kind's per-span evaluation: the [`SpanBase`] everything
/// shares, plus the per-kind parameter each lane's content point
/// yields and the mask of lanes it is solvable for. [`eval_block`]
/// and [`eval_pixel`] are generic over it, and the [`SpanKernel`]
/// blanket impl below drives it through the shared span driver.
trait SpanEval<'a, S: Simd> {
    /// The shared span-invariant state.
    fn base(&self) -> &SpanBase<'a>;
    /// The gradient parameter of each lane's content point, before
    /// extension.
    fn parameter<V>(&self, simd: S, cx: V, cy: V) -> V
    where
        V: SimdFloat<S, Element = f32>,
        V::Mask: Select<V>;
    /// The lanes whose parameter is solvable, tested before extension:
    /// radial's no-solution `NaN`. `None` for a kind whose parameter
    /// has no such case — a compile-time fact, so its block and pixel
    /// paths carry no mask and no branch.
    fn solvable<V>(&self, t: V) -> Option<V::Mask>
    where
        V: SimdFloat<S, Element = f32>;
}

/// The span-invariant linear evaluation, decided once per span: the
/// shared base and the projection — `None` for a degenerate gradient
/// (`len2 == 0.0`), fixing the parameter at 0.
#[derive(Clone, Copy)]
struct LinearEval<'a> {
    /// The shared span-invariant state.
    base: SpanBase<'a>,
    /// `Some` projects to `t`; `None` fixes it at 0.
    project: Option<Project>,
}

impl<'a> LinearEval<'a> {
    /// The projection — computed at lowering — over the shared base.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn new(outer: &'a PaintData, params: &'a LinearPaint) -> Self {
        Self {
            base: SpanBase::new(
                outer,
                params.inv,
                &params.stops,
                params.extend,
                params.interpolation,
            ),
            project: params.project,
        }
    }
}

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
impl<'a, S: Simd> SpanEval<'a, S> for LinearEval<'a> {
    #[inline(always)]
    fn base(&self) -> &SpanBase<'a> {
        &self.base
    }
    #[expect(
        clippy::option_if_let_else,
        reason = "a map_or_else closure is compiled without the level's target features"
    )]
    #[inline(always)]
    fn parameter<V>(&self, simd: S, cx: V, cy: V) -> V
    where
        V: SimdFloat<S, Element = f32>,
        V::Mask: Select<V>,
    {
        match self.project {
            Some(p) => project_span(simd, &p, cx, cy),
            None => V::splat(simd, 0.0),
        }
    }
    #[inline(always)]
    fn solvable<V>(&self, _t: V) -> Option<V::Mask>
    where
        V: SimdFloat<S, Element = f32>,
    {
        None
    }
}

/// Which equation a radial span's lanes solve — decided once per
/// span, as the scalar `radial_t` decides per pixel. [`radial_span`]
/// matches it once; each arm runs a span body generic over the
/// variant's [`RadialSolve`] type, so neither loop re-tests the form.
#[derive(Clone, Copy)]
enum RadialForm {
    /// The circles are identical: the parameter is `-∞` on and inside
    /// `r0`, `+∞` outside.
    Identical,
    /// `|a| < 1e-12`: the quadratic degenerates to the `-c / b`
    /// linear solve.
    Degenerate,
    /// The quadratic `a·t² + b·t + c = 0`.
    Quadratic,
}

/// The span-invariant radial evaluation, decided once per span: the
/// shared base, the quadratic's constants and its decided form.
#[derive(Clone, Copy)]
struct RadialEval<'a> {
    /// The shared span-invariant state.
    base: SpanBase<'a>,
    /// `centres[0]` and `centres[1]`: the point `radial_t` measures
    /// from.
    x0: f32,
    y0: f32,
    /// `radii[0]` and `radii[1] - radii[0]`.
    r0: f32,
    dr: f32,
    /// `centres[2] - centres[0]` and `centres[3] - centres[1]`.
    dcx: f32,
    dcy: f32,
    /// `4·a` and `2·a`, in the scalar expression's rounding.
    a4: f32,
    a2: f32,
    /// The equation the span solves.
    form: RadialForm,
}

impl<'a> RadialEval<'a> {
    /// Selects the form from the prepared coefficients: `|a| < 1e-12`
    /// selects the linear solve, `identical` the ±∞ edge.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn new(outer: &'a PaintData, params: &'a RadialPaint) -> Self {
        let RadialGeometry {
            x0,
            y0,
            r0,
            dr,
            dcx,
            dcy,
            a,
        } = params.geometry;
        Self {
            base: SpanBase::new(
                outer,
                params.inv,
                &params.stops,
                params.extend,
                params.interpolation,
            ),
            x0,
            y0,
            r0,
            dr,
            dcx,
            dcy,
            a4: 4.0 * a,
            a2: 2.0 * a,
            form: if params.identical {
                RadialForm::Identical
            } else if a.abs() < 1e-12 {
                RadialForm::Degenerate
            } else {
                RadialForm::Quadratic
            },
        }
    }
}

/// A `RadialForm` variant as a type: `solve` evaluates `t` for a
/// block and for a lane — the scalar expressions are identical at
/// every width, so one generic method serves both. [`radial_span`]
/// picks the type once per span; no form test runs inside a loop.
trait RadialSolve {
    /// Whether `solve` can yield `NaN`, the scalar evaluator's
    /// unpainted pixel; a form that never does skips the per-pixel gate.
    const MAY_FAIL: bool;

    /// `radial_t` lane-wise over `p − centre₀` for this form.
    fn solve<S, V>(simd: S, eval: &RadialEval<'_>, px: V, py: V) -> V
    where
        S: Simd,
        V: SimdFloat<S, Element = f32>,
        V::Mask: Select<V>;
}

/// The lane-scale `b`, `c`, `r0` and `s` every solving form shares:
/// `s` is `1` while `max(|px|, |py|)` is at most
/// [`RADIAL_RESCALE_ABOVE`], that bound beyond it. `p` and `r0`
/// multiplied by the lane reciprocal `1/s` — one divide per lane
/// instead of three — scales `b` by `1/s`, `c` and the discriminant
/// by `1/s²` and every root by `1/s`, so no term overflows for a
/// finite content point — unscaled, `px²` overflows f32 near 1.8e19
/// and `4a·c` before that, and the pixel paints nothing where the
/// oracle paints. At `s = 1` the expressions are unchanged.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn scaled_terms<S, V>(simd: S, eval: &RadialEval<'_>, px: V, py: V) -> (V, V, V, V)
where
    S: Simd,
    V: SimdFloat<S, Element = f32>,
    V::Mask: Select<V>,
{
    let one = V::splat(simd, 1.0);
    let hi = px.abs().max(py.abs());
    let s = hi
        .simd_gt(V::splat(simd, RADIAL_RESCALE_ABOVE))
        .select(hi, one);
    let inv = one / s;
    let (px, py, r0) = (px * inv, py * inv, V::splat(simd, eval.r0) * inv);
    // b = -2·(r0·dr + dcx·px + dcy·py), c = px² + py² − r0², at scale s.
    let inner = V::splat(simd, eval.dcy).mul_add(py, V::splat(simd, eval.dcx) * px);
    let b = V::splat(simd, -2.0) * r0.mul_add(V::splat(simd, eval.dr), inner);
    let c = r0.mul_add(-r0, py.mul_add(py, px * px));
    (b, c, r0, s)
}

/// `RadialForm::Identical`: the parameter is `-∞` on and inside `r0`,
/// `+∞` outside, by `hypot(p) ≤ r0`.
struct IdenticalSolve;

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
impl RadialSolve for IdenticalSolve {
    const MAY_FAIL: bool = false;

    #[inline(always)]
    fn solve<S, V>(simd: S, eval: &RadialEval<'_>, px: V, py: V) -> V
    where
        S: Simd,
        V: SimdFloat<S, Element = f32>,
        V::Mask: Select<V>,
    {
        let zero = V::splat(simd, 0.0);
        let one = V::splat(simd, 1.0);
        let hi = px.abs().max(py.abs());
        // `f32::hypot` lane-wise: `hi·sqrt(1 + (lo/hi)²)`. `hi == 0`
        // is 0, and an infinite component already reads as `> r0`.
        let lo = px.abs().min(py.abs());
        let r = lo / hi;
        let h = hi * r.mul_add(r, one).sqrt();
        let h = hi.simd_eq(zero).select(zero, h);
        h.simd_le(V::splat(simd, eval.r0)).select(
            V::splat(simd, f32::NEG_INFINITY),
            V::splat(simd, f32::INFINITY),
        )
    }
}

/// `RadialForm::Degenerate` — `|a| < 1e-12`: the quadratic reduces to
/// the `-c / b` linear solve.
struct DegenerateSolve;

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
impl RadialSolve for DegenerateSolve {
    const MAY_FAIL: bool = true;

    #[inline(always)]
    fn solve<S, V>(simd: S, eval: &RadialEval<'_>, px: V, py: V) -> V
    where
        S: Simd,
        V: SimdFloat<S, Element = f32>,
        V::Mask: Select<V>,
    {
        let (b, c, r0, s) = scaled_terms(simd, eval, px, py);
        let dr = V::splat(simd, eval.dr);
        let t = -c / b;
        // |b| < 1e-12 has no solution; `t·dr + r0 ≥ 0` keeps the
        // solve's radius non-negative — both scale-free.
        let ok = (b * s).abs().simd_ge(V::splat(simd, 1e-12))
            & t.mul_add(dr, r0).simd_ge(V::splat(simd, 0.0));
        ok.select(t * s, V::splat(simd, f32::NAN))
    }
}

/// `RadialForm::Quadratic`: `a·t² + b·t + c = 0`, keeping the largest
/// root whose radius `r0 + t·dr` is non-negative — `NaN` when there is
/// none.
struct QuadraticSolve;

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
impl RadialSolve for QuadraticSolve {
    const MAY_FAIL: bool = true;

    #[inline(always)]
    fn solve<S, V>(simd: S, eval: &RadialEval<'_>, px: V, py: V) -> V
    where
        S: Simd,
        V: SimdFloat<S, Element = f32>,
        V::Mask: Select<V>,
    {
        let (b, c, r0, s) = scaled_terms(simd, eval, px, py);
        let zero = V::splat(simd, 0.0);
        let nan = V::splat(simd, f32::NAN);
        let dr = V::splat(simd, eval.dr);
        let disc = V::splat(simd, eval.a4).mul_add(-c, b * b);
        let sq = disc.sqrt();
        let den = V::splat(simd, eval.a2);
        let r1 = (-b + sq) / den;
        let r2 = (-b - sq) / den;
        // `f32::max`/`min` ignore a NaN operand; NEON's `vmaxq`/`vminq`
        // propagate it, and an infinite `b`/`sq` leaves one root NaN and
        // the other infinite, so the precise variants are required.
        let (hi, lo) = (r1.max_precise(r2), r1.min_precise(r2));
        // radius_ok(hi) ? hi : radius_ok(lo) ? lo : NaN. A negative
        // discriminant's `NaN` roots already fail the `≥ 0` test, so the
        // discriminant select only mirrors the scalar early return.
        let t = hi
            .mul_add(dr, r0)
            .simd_ge(zero)
            .select(hi, lo.mul_add(dr, r0).simd_ge(zero).select(lo, nan));
        disc.simd_lt(zero).select(nan, t * s)
    }
}

/// A radial span with the form folded to a type parameter —
/// `parameter` calls the variant's [`RadialSolve`] directly, so the
/// block and one-pixel paths carry no runtime form test.
struct RadialFormEval<'a, F> {
    /// The span-invariant radial state.
    eval: RadialEval<'a>,
    /// The form marker — zero-sized, decided by [`radial_span`]'s match.
    form: PhantomData<F>,
}

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
impl<'a, S, F> SpanEval<'a, S> for RadialFormEval<'a, F>
where
    S: Simd,
    F: RadialSolve,
{
    #[inline(always)]
    fn base(&self) -> &SpanBase<'a> {
        &self.eval.base
    }
    #[inline(always)]
    fn parameter<V>(&self, simd: S, cx: V, cy: V) -> V
    where
        V: SimdFloat<S, Element = f32>,
        V::Mask: Select<V>,
    {
        F::solve(
            simd,
            &self.eval,
            cx - V::splat(simd, self.eval.x0),
            cy - V::splat(simd, self.eval.y0),
        )
    }
    #[inline(always)]
    fn solvable<V>(&self, t: V) -> Option<V::Mask>
    where
        V: SimdFloat<S, Element = f32>,
    {
        // The scalar evaluator's `t.is_nan()` gate, lane-wise; identical
        // circles only produce ±∞, so their span carries no gate.
        F::MAY_FAIL.then(|| t.simd_eq(t))
    }
}

/// One `LANES`-wide block from device x-centres `xs` on row `dy`'s
/// centre: the base's chain inverses and `inv`, the parameter, the
/// extend mode as a runtime choice on one block copy and the
/// span-decided stops — premultiplied channels, zero where
/// `Extend::None` leaves the domain or the parameter has no solution.
#[expect(
    clippy::option_if_let_else,
    reason = "a map_or closure is compiled without the level's target features"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_block<'a, S, E>(simd: S, eval: &E, xs: f32x16<S>, dy: f32x16<S>) -> [f32x16<S>; 4]
where
    S: Simd,
    E: SpanEval<'a, S>,
{
    let base = eval.base();
    let (mut cx, mut cy) = (xs, dy);
    let mut link = base.outer;
    while let PaintData::Transformed(inner) = link {
        (cx, cy) = apply_span(simd, inner.inverse_f32, cx, cy);
        link = &inner.paint;
    }
    let (cx, cy) = apply_span(simd, base.inv, cx, cy);
    let zero = simd.splat_f32x16(0.0);
    let t = eval.parameter(simd, cx, cy);
    let solvable = eval.solvable(t);
    let (t, active) = match base.extend {
        Extend::Pad => PadSpan::block(simd, t),
        Extend::Repeat => RepeatSpan::block(simd, t),
        Extend::Reflect => ReflectSpan::block(simd, t),
        Extend::None => NoneSpan::block(simd, t),
    };
    let active = match solvable {
        Some(solvable) => active & solvable,
        None => active,
    };
    let mut px = match base.stops {
        Ok(stops) => premul_span(simd, eval_stops_span(simd, stops, t), base.working),
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

/// One pixel of the block expression in channel-vector form: the
/// base's chain — present only in the `CHAINED` instantiation — and
/// `inv` applied through `apply_span`'s `f32x4` lane, the same
/// arithmetic the block lanes get;
/// the scalar extend and early-exit stop search, one four-channel mix
/// and premultiply, zero where `Extend::None` leaves the domain or the
/// parameter has no solution.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_pixel<'a, S, X, E, const CHAINED: bool>(simd: S, eval: &E, x: f32, y: f32) -> f32x4<S>
where
    S: Simd,
    X: ExtendSpan<S>,
    E: SpanEval<'a, S>,
{
    let base = eval.base();
    let (mut cx, mut cy) = (simd.splat_f32x4(x), simd.splat_f32x4(y));
    if CHAINED {
        let mut link = base.outer;
        while let PaintData::Transformed(inner) = link {
            (cx, cy) = apply_span(simd, inner.inverse_f32, cx, cy);
            link = &inner.paint;
        }
    }
    let (cx, cy) = apply_span(simd, base.inv, cx, cy);
    let t = eval.parameter(simd, cx, cy);
    // `solvable`'s lane 0 — the splat point makes every lane identical.
    if let Some(solvable) = eval.solvable(t)
        && solvable[0] == 0
    {
        return simd.splat_f32x4(0.0);
    }
    let (t, active) = X::lane(simd, t[0]);
    if !active {
        return simd.splat_f32x4(0.0);
    }
    match base.stops {
        Ok(stops) => eval_stops_pixel(simd, stops, t, base.working),
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
fn eval_pixels<'a, S, X, E, const CHAINED: bool>(
    simd: S,
    eval: &E,
    mut x: f32,
    y: f32,
    out: &mut [MaybeUninit<[f32; 4]>],
) where
    S: Simd,
    X: ExtendSpan<S>,
    E: SpanEval<'a, S>,
{
    for px in out.iter_mut() {
        // SAFETY: the store writes all four lanes of `px`.
        simd.store_array_f32x4(eval_pixel::<S, X, E, CHAINED>(simd, eval, x, y), unsafe {
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
fn eval_run<'a, S, X, E>(simd: S, eval: &E, x: f32, y: f32, out: &mut [MaybeUninit<[f32; 4]>])
where
    S: Simd,
    X: ExtendSpan<S>,
    E: SpanEval<'a, S>,
{
    if matches!(eval.base().outer, PaintData::Transformed(..)) {
        eval_pixels::<S, X, E, true>(simd, eval, x, y, out);
    } else {
        eval_pixels::<S, X, E, false>(simd, eval, x, y, out);
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
fn eval_run_extend<'a, S, E>(simd: S, eval: &E, x: f32, y: f32, out: &mut [MaybeUninit<[f32; 4]>])
where
    S: Simd,
    E: SpanEval<'a, S>,
{
    match eval.base().extend {
        Extend::Pad => eval_run::<S, PadSpan, E>(simd, eval, x, y, out),
        Extend::Repeat => eval_run::<S, RepeatSpan, E>(simd, eval, x, y, out),
        Extend::Reflect => eval_run::<S, ReflectSpan, E>(simd, eval, x, y, out),
        Extend::None => eval_run::<S, NoneSpan, E>(simd, eval, x, y, out),
    }
}

/// Every [`SpanEval`] is a [`SpanKernel`]: `block` is [`eval_block`]'s
/// one inlined body, `run` the extend-dispatched one-pixel path — the
/// pieces [`super::span::span`] drives.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
impl<'a, S, E> SpanKernel<'a, S> for E
where
    S: Simd,
    E: SpanEval<'a, S>,
{
    #[inline(always)]
    fn block(&self, simd: S, xs: f32x16<S>, dy: f32x16<S>) -> [f32x16<S>; 4] {
        eval_block(simd, self, xs, dy)
    }
    #[inline(always)]
    fn run(&self, simd: S, x: f32, y: f32, out: &mut [MaybeUninit<[f32; 4]>]) {
        eval_run_extend(simd, self, x, y, out);
    }
}

/// `PaintData::Linear` over `out`: the span driver with the
/// span-invariant choices [`LinearEval`] decides.
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
    super::span::span(simd, &LinearEval::new(outer, params), x0, py, out)
}

/// `PaintData::Radial` over `out`: the span driver with the
/// span-invariant choices [`RadialEval`] decides for full blocks.
/// The caller handles short runs before that setup. The block form is
/// unswitched once per span — each arm runs a span body generic over
/// the variant's [`RadialSolve`] type, so neither the block loop nor
/// the one-pixel loop re-tests it.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub(super) fn radial_span<'a, S: Simd>(
    simd: S,
    outer: &PaintData,
    params: &RadialPaint,
    x0: usize,
    py: usize,
    out: &'a mut [MaybeUninit<[f32; 4]>],
) -> &'a mut [[f32; 4]] {
    let eval = RadialEval::new(outer, params);
    match eval.form {
        RadialForm::Identical => radial_span_form::<S, IdenticalSolve>(simd, eval, x0, py, out),
        RadialForm::Degenerate => radial_span_form::<S, DegenerateSolve>(simd, eval, x0, py, out),
        RadialForm::Quadratic => radial_span_form::<S, QuadraticSolve>(simd, eval, x0, py, out),
    }
}

/// One form's span body, entered through `vectorize` so it compiles
/// under the level's target features even though it is not inlined —
/// `#[inline(never)]` keeps only one form's body live per stack frame;
/// the earlier all-inlined attempt's three copies overflowed the
/// debug test stack.
#[inline(never)]
fn radial_span_form<'o, S, F>(
    simd: S,
    eval: RadialEval<'_>,
    x0: usize,
    py: usize,
    out: &'o mut [MaybeUninit<[f32; 4]>],
) -> &'o mut [[f32; 4]]
where
    S: Simd,
    F: RadialSolve,
{
    simd.vectorize(
        #[inline(always)]
        || {
            super::span::span(
                simd,
                &RadialFormEval::<F> {
                    eval,
                    form: PhantomData,
                },
                x0,
                py,
                out,
            )
        },
    )
}
