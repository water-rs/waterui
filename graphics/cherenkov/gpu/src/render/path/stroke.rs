//! The CPU stroke encoder of the compute stroker (water-rs/waterui#2544).
//!
//! It reads a path's centerline once and emits [`GpuStrokeRecord`]s: which
//! pieces exist, how curves are subdivided, and how many chords every offset,
//! join and cap takes, so that an undashed stroke's segment slots are exact
//! and a dashed stroke's buffers are sized by conservative bounds. Decisions
//! that move only geometry inside those counts are made by the kernels in
//! f32. The outline never exists on the CPU.
//!
//! Every CPU decision below fixes a count or a buffer size, and none is kept
//! to match kurbo's output. Where a port of kurbo 0.13.1 is kept, it is
//! because it defines the case.

use std::f64::consts::{PI, SQRT_2};
use std::sync::Arc;

use cherenkov::{RenderError, ShapeData};
use kurbo::common::solve_quadratic;
use kurbo::{
    Affine, BezPath, Cap, CubicBez, Join, ParamCurve, ParamCurveArclen, ParamCurveDeriv,
    ParamCurveNearest, PathEl, Point, QuadBez, Rect, Stroke, Vec2,
};

use super::{GpuBound, GpuPath, encodable, shape_path, sigma_max};
use crate::render::stroke_abi::{
    CAP_BUTT, CAP_END, CAP_ROUND, CAP_SQUARE, CAP_START, CLOSE, JOIN_BEVEL, JOIN_MASK, JOIN_MITER,
    JOIN_ROUND, JOIN_SHARED, JOIN_SHIFT, JOIN_START, LINEAR_END, N_LEN_MAX, N_OFF_MAX, PIECE_CURVE,
    PIECE_JOIN_ONLY, PIECE_LINE, PIECE_MASK, PIECE_POINT_ARC, SLOT_BE, SLOT_BS, SLOT_FE, SLOT_FS,
    SLOTS,
};

/// One stroke piece of the centerline, 96 bytes: the layout of the stroke
/// record ABI (design section 5.2). The comments name each `vec4`'s lanes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuStrokeRecord {
    /// `(flags, n | stage_cap, path index (stamped at upload), slot | stage_base)`.
    /// An undashed record's `n` is its exact chord count and `slot` its
    /// path-relative segment slot. A dashed record's are its staging capacity
    /// and the path-relative staging base.
    pub head: [u32; 4],
    /// `(prev_fe, prev_be, fs, bs)`: vertex slots `SLOTS * record + slot`,
    /// relative to the path's first record. `prev_*` is `u32::MAX` for
    /// `JOIN_START`.
    pub link: [u32; 4],
    /// `(k_join | n_off << 16, n_len, first_rel, last_rel)`. `first_rel` and
    /// `last_rel` (the subpath's first and last records) are set on dashed
    /// records only.
    pub aux: [u32; 4],
    /// `(P0.x, P0.y, T0.x, T0.y)`: start centre and tangent.
    pub p: [f32; 4],
    /// `(P1.x, P1.y, T1.x, T1.y)`: end centre and tangent.
    pub q: [f32; 4],
    /// CURVE: the sub-cubic's `(p1, p2)`. `POINT_ARC`: `(psi, 0, 0, 0)`,
    /// or `(psi, length)` for a dashed one: the sub-cubic's f64 arc
    /// length in fixed point as two u32 bit patterns. Dashed LINE: the
    /// length in fixed point as two u32 bit patterns (integer part,
    /// fraction), then 0, 0.
    pub c: [f32; 4],
}

/// The per-path stroke parameters the GPU path record carries (design
/// section 8). All zero for a fill.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StrokeParams {
    /// Half the stroke width, in user units.
    pub half_width: f32,
    /// The miter limit.
    pub miter_limit: f32,
    /// The start cap, one of the `CAP_*` codes.
    pub start_cap: u32,
    /// The end cap, one of the `CAP_*` codes.
    pub end_cap: u32,
    /// The chord count of a round cap, `arc_chords(pi, tol / 2)`.
    pub k_cap: u32,
}

/// A dashed stroke's pattern in fixed point, and the capacities the encoder
/// proved for it (design section 6.6). Fixed point has `2^-32` user-unit
/// resolution and is stored as `[integer part, fraction]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashTable {
    /// The cumulative boundaries `cum[0] = 0 ..= cum[E] = P` of the effective
    /// pattern (doubled when its length is odd).
    pub cum: Box<[[u32; 2]]>,
    /// The initial phase `phi`.
    pub phase: [u32; 2],
    /// A dash running through `ClosePath` merges with the first one.
    pub merge_start: bool,
    /// `Σ stage_cap`: staging entries of the whole path.
    pub stage_total: usize,
    /// `F`: bound on the path's dash fragments.
    pub fragments: usize,
    /// `D`: bound on the path's dashes.
    pub dashes: usize,
}

/// Relative margin on every device-space variation: it covers the kernels'
/// f32 evaluation of the same points (design section 6.5).
const VARIATION_MARGIN: f64 = 1.0 / 65536.0;

/// Relative margin on control-polygon lengths: it covers the f32 arc length
/// (design section 6.6).
const POLYGON_MARGIN: f64 = 1.0 / 1_048_576.0;

/// Positions and periods are fixed point with 32 fractional bits, so they must
/// stay below `2^31` user units.
const FIXED_LIMIT: f64 = 2_147_483_648.0;

/// Accuracy of the f64 arc length a dashed `POINT_ARC` carries in `c.yz`:
/// below the `2^-32` fixed-point unit.
const POINT_ARC_LENGTH_ACCURACY: f64 = 1e-10;

/// Subdivision depth at which a curve is declared not to converge.
const MAX_DEPTH: u32 = 24;

/// Largest value of a 16-bit record field.
const FIELD_MAX: u32 = 0xFFFF;

/// Encodes `stroke` of `shape` for the compute stroker under `raster`, at the
/// user-space tolerance `tol`, bound against `surface`. `stroke`'s dash
/// pattern has been validated by `Lowerer::stroke` (D12).
///
/// `raster` follows [`super::prepare_gpu`]'s convention: only the linear part
/// when called from the automatic route (placement comes from
/// [`GpuPath::placed`]), the full transform from the forced route. `tol` is
/// the value `OutlineRef::content` and `build` use. The centerline is read
/// once, and no `BezPath` is built for the outline or for dashes.
pub fn prepare(
    shape: &ShapeData,
    stroke: &Stroke,
    raster: Affine,
    tol: f64,
    surface: (f64, f64),
) -> Result<GpuPath, RenderError> {
    let [.., e, f] = raster.as_coeffs();
    let origin = Vec2::new(e, f);
    // Non-finite parameters draw nothing, as the sweep draws kurbo's
    // all-NaN outline (water-rs/waterui#3156).
    if !(stroke.width.is_finite()
        && stroke.miter_limit.is_finite()
        && stroke.dash_offset.is_finite()
        && stroke.dash_pattern.iter().all(|entry| entry.is_finite()))
    {
        return Ok(empty(origin, surface));
    }
    let dash = DashPlan::new(stroke)?;
    // No outline at zero width; a pattern of only zero-length dashes still
    // draws their cap dots.
    if stroke.width == 0.0 {
        return Ok(empty(origin, surface));
    }
    if !(stroke.width > 0.0 && tol.is_finite() && tol > 0.0) {
        return Err(RenderError::Unsupported("invalid stroke parameters"));
    }
    let centerline = shape_path(shape, tol);
    let mut encoder = Encoder::new(stroke, raster, tol, dash);
    encoder.walk(&centerline)?;
    Ok(encoder.finish_path(origin, surface))
}

/// The path of a stroke that draws nothing: no records and an empty bound.
fn empty(origin: Vec2, surface: (f64, f64)) -> GpuPath {
    GpuPath {
        elements: Arc::from([]),
        strokes: Arc::from([]),
        stroke: StrokeParams::default(),
        dash: None,
        hull: Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        origin,
        bound: GpuBound::default(),
    }
    .placed(origin, surface)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a rounded-up nonnegative count, saturating at usize::MAX"
)]
const fn ceil_count(x: f64) -> usize {
    x.ceil().max(0.0) as usize
}

fn count_u32(n: usize, what: &'static str) -> Result<u32, RenderError> {
    u32::try_from(n).map_err(|_| RenderError::Unsupported(what))
}

#[expect(clippy::cast_possible_truncation, reason = "records store f32")]
const fn f32x2(p: Point) -> [f32; 2] {
    [p.x as f32, p.y as f32]
}

#[expect(clippy::cast_possible_truncation, reason = "records store f32")]
const fn f32x2v(v: Vec2) -> [f32; 2] {
    [v.x as f32, v.y as f32]
}

/// A nonnegative `x < 2^31` in `2^-32` fixed point: `[integer, fraction]`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "x is below 2^31, so x * 2^32 fits u64 and the halves fit u32"
)]
fn fixed(x: f64) -> [u32; 2] {
    let v = (x * 4_294_967_296.0).round() as u64;
    [(v >> 32) as u32, v as u32]
}

/// kurbo 0.13.1 `PathSeg::tangents` for a quad.
fn tangents_quad(q: QuadBez) -> (Vec2, Vec2) {
    const EPS: f64 = 1e-12;
    let d01 = q.p1 - q.p0;
    let d0 = if d01.hypot2() > EPS { d01 } else { q.p2 - q.p0 };
    let d12 = q.p2 - q.p1;
    let d1 = if d12.hypot2() > EPS { d12 } else { q.p2 - q.p0 };
    (d0, d1)
}

/// kurbo 0.13.1 `PathSeg::tangents` for a cubic.
fn tangents_cubic(c: CubicBez) -> (Vec2, Vec2) {
    const EPS: f64 = 1e-12;
    let d01 = c.p1 - c.p0;
    let d0 = if d01.hypot2() > EPS {
        d01
    } else {
        let d02 = c.p2 - c.p0;
        if d02.hypot2() > EPS { d02 } else { c.p3 - c.p0 }
    };
    let d23 = c.p3 - c.p2;
    let d1 = if d23.hypot2() > EPS {
        d23
    } else {
        let d13 = c.p3 - c.p1;
        if d13.hypot2() > EPS { d13 } else { c.p3 - c.p0 }
    };
    (d0, d1)
}

enum CuspType {
    Loop,
    DoubleInflection,
}

/// kurbo 0.13.1 `CubicBez::detect_cusp`.
#[expect(
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    reason = "line-for-line port of kurbo 0.13.1"
)]
fn detect_cusp(c: &CubicBez, dimension: f64) -> Option<CuspType> {
    let d01 = c.p1 - c.p0;
    let d02 = c.p2 - c.p0;
    let d03 = c.p3 - c.p0;
    let d12 = c.p2 - c.p1;
    let d23 = c.p3 - c.p2;
    let det_012 = d01.cross(d02);
    let det_123 = d12.cross(d23);
    let det_013 = d01.cross(d03);
    let det_023 = d02.cross(d03);
    if det_012 * det_123 > 0.0 && det_012 * det_013 < 0.0 && det_012 * det_023 < 0.0 {
        let q = c.deriv();
        // The accuracy is not used for a quadratic's nearest point.
        let nearest = q.nearest(Point::ORIGIN, 1e-9);
        // Whether the curvature at the minimum derivative exceeds
        // 1 / dimension, without division.
        let d = q.eval(nearest.t);
        let d2 = q.deriv().eval(nearest.t);
        let cross = d.to_vec2().cross(d2.to_vec2());
        if nearest.distance_sq.powi(3) <= (cross * dimension).powi(2) {
            let a = 3. * det_012 + det_023 - 2. * det_013;
            let b = -3. * det_012 + det_013;
            let c = det_012;
            let d = b * b - 4. * a * c;
            return Some(if d > 0.0 {
                CuspType::DoubleInflection
            } else {
                CuspType::Loop
            });
        }
    }
    None
}

/// kurbo 0.13.1 `CubicBez::regularize_cusp`: moves the centerline by at most
/// `dimension` so that the hodograph does not pass through the origin.
fn regularize_cusp(c: CubicBez, dimension: f64) -> CubicBez {
    let mut c = c;
    if let Some(cusp_type) = detect_cusp(&c, dimension) {
        let d01 = c.p1 - c.p0;
        let d01h = d01.hypot();
        let d23 = c.p3 - c.p2;
        let d23h = d23.hypot();
        match cusp_type {
            CuspType::Loop => {
                c.p1 += (dimension / d01h) * d01;
                c.p2 -= (dimension / d23h) * d23;
            }
            CuspType::DoubleInflection => {
                // Avoid making the control distance smaller than the dimension.
                if d01h > 2.0 * dimension {
                    c.p1 -= (dimension / d01h) * d01;
                }
                if d23h > 2.0 * dimension {
                    c.p2 += (dimension / d23h) * d23;
                }
            }
        }
    }
    c
}

/// A cubic whose control points are nearly colinear, with the projection onto
/// its reference chord (kurbo 0.13.1 `do_cubic`'s `do_linear` inputs).
struct Linear {
    projected: [f64; 4],
    ref_pt: Point,
    ref_vec: Vec2,
}

/// kurbo 0.13.1 `StrokeCtx::do_cubic`'s near-colinear test.
#[expect(
    clippy::suboptimal_flops,
    reason = "line-for-line port of kurbo 0.13.1"
)]
fn near_linear(c: CubicBez, tolerance: f64) -> Option<Linear> {
    const ENDPOINT_D: f64 = 0.01;
    // Ordinarily this is the direction of the chord, but if the chord is very
    // short, the longer control arm.
    let chord = c.p3 - c.p0;
    let mut chord_ref = chord;
    let mut chord_ref_hypot2 = chord_ref.hypot2();
    let d01 = c.p1 - c.p0;
    if d01.hypot2() > chord_ref_hypot2 {
        chord_ref = d01;
        chord_ref_hypot2 = chord_ref.hypot2();
    }
    let d23 = c.p3 - c.p2;
    if d23.hypot2() > chord_ref_hypot2 {
        chord_ref = d23;
        chord_ref_hypot2 = chord_ref.hypot2();
    }
    // Project the Bezier onto the chord.
    let p0 = c.p0.to_vec2().dot(chord_ref);
    let p1 = c.p1.to_vec2().dot(chord_ref);
    let p2 = c.p2.to_vec2().dot(chord_ref);
    let p3 = c.p3.to_vec2().dot(chord_ref);
    if p3 <= p0 || p1 > p2 || p1 < p0 + ENDPOINT_D * (p3 - p0) || p2 > p3 - ENDPOINT_D * (p3 - p0) {
        // Potentially a cusp inside.
        let x01 = d01.cross(chord_ref);
        let x23 = d23.cross(chord_ref);
        let x03 = chord.cross(chord_ref);
        let thresh = tolerance.powi(2) * chord_ref_hypot2;
        if x01 * x01 < thresh && x23 * x23 < thresh && x03 * x03 < thresh {
            let midpoint = c.p0.midpoint(c.p3);
            // The map back from the projection onto the reference chord.
            let ref_vec = chord_ref / chord_ref_hypot2;
            let ref_pt = midpoint - f64::midpoint(p0, p3) * ref_vec;
            return Some(Linear {
                projected: [p0, p1, p2, p3],
                ref_pt,
                ref_vec,
            });
        }
    }
    None
}

/// The distance from the origin to the segment `a..b`.
fn origin_segment_distance(a: Vec2, b: Vec2) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 > 0.0 {
        (-a.dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t).hypot()
}

/// The distance from the origin to the convex hull of the hodograph control
/// points, 0 when the origin is inside or on it.
fn origin_hull_distance(h: [Vec2; 3]) -> f64 {
    let side = |a: Vec2, b: Vec2| (b - a).cross(-a);
    let (s0, s1, s2) = (side(h[0], h[1]), side(h[1], h[2]), side(h[2], h[0]));
    let area = (h[1] - h[0]).cross(h[2] - h[0]);
    if area != 0.0
        && ((s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0))
    {
        return 0.0;
    }
    origin_segment_distance(h[0], h[1])
        .min(origin_segment_distance(h[1], h[2]))
        .min(origin_segment_distance(h[2], h[0]))
}

/// The largest pairwise angle among the non-zero hodograph control points.
fn aperture(h: [Vec2; 3]) -> f64 {
    let mut widest = 0.0_f64;
    for (i, a) in h.iter().enumerate() {
        for b in &h[i + 1..] {
            if *a != Vec2::ZERO && *b != Vec2::ZERO {
                widest = widest.max(a.cross(*b).abs().atan2(a.dot(*b)));
            }
        }
    }
    widest
}

/// The chord count of an arc of `phi` radians on a circle of radius `hw`
/// whose chords have sagitta at most `e` (design section 6.4).
fn arc_chords(phi: f64, e: f64, hw: f64) -> Result<u32, RenderError> {
    let step = 2.0 * (1.0 - e / hw).clamp(-1.0, 1.0).acos();
    let chords = (phi.abs() / step).ceil().max(1.0);
    if chords > f64::from(FIELD_MAX) {
        return Err(RenderError::Unsupported("stroke arc chord count"));
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the count is a whole number in 1..=FIELD_MAX"
    )]
    Ok(chords as u32)
}

/// The diagonal of the bounding box of a cubic's control points.
fn diameter(c: &CubicBez) -> f64 {
    Rect::from_points(c.p0, c.p3)
        .union_pt(c.p1)
        .union_pt(c.p2)
        .size()
        .to_vec2()
        .hypot()
}

/// A curve piece the subdivision settled on.
struct Sub {
    cubic: CubicBez,
    /// `None` for a CURVE piece; the signed sweep of a `POINT_ARC` piece.
    psi: Option<f64>,
    n_off: u32,
    n_len: u32,
}

/// The inputs of the subdivision that do not change with the sub-cubic.
#[derive(Clone, Copy)]
struct SubdivideCtx {
    hw: f64,
    tol: f64,
    dashed: bool,
}

/// The sweep of a `POINT_ARC` piece, `None` where the piece must keep
/// splitting (design section 6.3).
fn point_arc_sweep(c: CubicBez, h: [Vec2; 3], m: f64) -> Option<f64> {
    let defined = m > 0.0
        || (h[0] == Vec2::ZERO && origin_segment_distance(h[1], h[2]) > 0.0)
        || (h[2] == Vec2::ZERO && origin_segment_distance(h[0], h[1]) > 0.0);
    defined.then(|| {
        let (d0, d1) = tangents_cubic(c);
        d0.cross(d1).atan2(d0.dot(d1))
    })
}

/// `n_len` for a dashed CURVE piece: the sub-interval count of the 8-point
/// Gauss-Legendre arc length rule (design section 6.3).
fn gauss_intervals(m: f64, m2: f64, m3: f64) -> f64 {
    // y0 is the positive root of y*M2 + y^2*M3/2 = (sqrt(2) - 1)*m, and
    // n_len = ceil(1 / y0).
    let reach = (SQRT_2 - 1.0) * m;
    ((m2 + (2.0 * m3).mul_add(reach, m2 * m2).sqrt()) / (2.0 * reach))
        .ceil()
        .max(1.0)
}

/// Splits `c` into pieces whose offset curves sample within `ctx.tol` at
/// their chord counts, in parameter order (design section 6.3).
fn subdivide(
    cubic: CubicBez,
    ctx: SubdivideCtx,
    depth: u32,
    out: &mut Vec<Sub>,
) -> Result<(), RenderError> {
    let hodograph = [
        3.0 * (cubic.p1 - cubic.p0),
        3.0 * (cubic.p2 - cubic.p1),
        3.0 * (cubic.p3 - cubic.p2),
    ];
    let gap = origin_hull_distance(hodograph);
    if gap > 0.0 {
        let m2 = (6.0 * (cubic.p2.to_vec2() - 2.0 * cubic.p1.to_vec2() + cubic.p0.to_vec2()))
            .hypot()
            .max(
                (6.0 * (cubic.p3.to_vec2() - 2.0 * cubic.p2.to_vec2() + cubic.p1.to_vec2()))
                    .hypot(),
            );
        let m3 = (6.0
            * (cubic.p3.to_vec2() - 3.0 * cubic.p2.to_vec2() + 3.0 * cubic.p1.to_vec2()
                - cubic.p0.to_vec2()))
        .hypot();
        let bound = ctx.hw.mul_add(3.0 * m2 * m2 / (gap * gap) + m3 / gap, m2);
        let chords = (bound / (8.0 * ctx.tol)).sqrt().ceil().max(1.0);
        let intervals = if ctx.dashed {
            gauss_intervals(gap, m2, m3)
        } else {
            1.0
        };
        if chords <= f64::from(N_OFF_MAX) && intervals <= f64::from(N_LEN_MAX) {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "both counts are whole numbers within their small maxima"
            )]
            out.push(Sub {
                cubic,
                psi: None,
                n_off: chords as u32,
                n_len: intervals as u32,
            });
            return Ok(());
        }
    }
    if diameter(&cubic) <= ctx.tol / 2.0
        && let Some(psi) = point_arc_sweep(cubic, hodograph, gap)
    {
        out.push(Sub {
            cubic,
            psi: Some(psi),
            n_off: arc_chords(psi, ctx.tol / 2.0, ctx.hw)?,
            n_len: 1,
        });
        return Ok(());
    }
    if depth == MAX_DEPTH {
        return Err(RenderError::Unsupported(
            "stroke offset subdivision did not converge",
        ));
    }
    let (left, right) = cubic.subdivide();
    subdivide(left, ctx, depth + 1, out)?;
    subdivide(right, ctx, depth + 1, out)
}

/// A dash pattern normalized as `kurbo::dash` does, with its initial state
/// (design section 6.6).
struct DashPlan {
    /// `cum[0] = 0 ..= cum[E] = P` of the effective pattern.
    cum: Vec<f64>,
    period: f64,
    /// Entries at even index: every on-interval, positive-length or not —
    /// a zero-length one draws its cap dot.
    on_entries: usize,
    phase: f64,
    merge_start: bool,
}

impl DashPlan {
    #[expect(
        clippy::while_float,
        reason = "port of kurbo 0.13.1 `dash_iter`'s initial-state loop"
    )]
    fn new(stroke: &Stroke) -> Result<Option<Self>, RenderError> {
        cherenkov::lowering::check_dashes(stroke)?;
        if stroke.dash_pattern.is_empty() {
            return Ok(None);
        }
        // An odd-length pattern is doubled so that dashes and gaps do not
        // swap as the offset grows (kurbo 0.13.1 `dash_iter`).
        let mut entries: Vec<f64> = stroke.dash_pattern.to_vec();
        if entries.len() % 2 == 1 {
            entries.extend_from_within(..);
        }
        let mut cum = Vec::with_capacity(entries.len() + 1);
        cum.push(0.0);
        for entry in &entries {
            cum.push(cum[cum.len() - 1] + entry);
        }
        let period = cum[entries.len()];
        if period >= FIXED_LIMIT {
            return Err(RenderError::Unsupported("dash period too long"));
        }
        let on_entries = entries.len() / 2;
        // kurbo 0.13.1 `dash_iter`'s normalization and initial state. A dash
        // that begins in a dash with positive remaining length merges
        // through `ClosePath`.
        let phase = stroke.dash_offset.rem_euclid(period);
        let mut ix = 0;
        let mut remaining = entries[ix] - phase;
        let mut active = true;
        while remaining < 0.0 {
            ix = (ix + 1) % entries.len();
            remaining += entries[ix];
            active = !active;
        }
        Ok(Some(Self {
            cum,
            period,
            on_entries,
            phase,
            merge_start: active && remaining > 0.0,
        }))
    }

    /// `n_on * ceil(length / P)`: how many on-intervals — dashes and
    /// zero-length cap dots — can start in a window of arc length `length`
    /// (design section 6.6).
    fn starts(&self, length: f64) -> usize {
        self.on_entries
            .saturating_mul(ceil_count(length * (1.0 + POLYGON_MARGIN) / self.period))
    }
}

/// The join the next piece starts with.
#[derive(Clone, Copy)]
struct JoinRec {
    code: u32,
    /// Chords per side of a round join's arc.
    k: u32,
    /// The encoder's f64 turn angle.
    phi: f64,
    /// The tangent the join turns to.
    tan: Vec2,
    at: Point,
}

enum Body {
    Line,
    Curve {
        cubic: CubicBez,
        n_off: u32,
        n_len: u32,
    },
    PointArc {
        cubic: CubicBez,
        psi: f64,
        n_off: u32,
    },
    JoinOnly,
}

struct Piece {
    body: Body,
    p0: Point,
    t0: Vec2,
    p1: Point,
    t1: Vec2,
}

/// The record fields a piece body decides: its kind, offset chord count,
/// `n_len` and the `c` lane.
const fn body_fields(body: &Body) -> (u32, u32, u32, [f32; 4]) {
    match *body {
        Body::Line => (PIECE_LINE, 1, 1, [0.0; 4]),
        Body::Curve {
            cubic,
            n_off,
            n_len,
        } => {
            let [x1, y1] = f32x2(cubic.p1);
            let [x2, y2] = f32x2(cubic.p2);
            (PIECE_CURVE, n_off, n_len, [x1, y1, x2, y2])
        }
        Body::PointArc { psi, n_off, .. } => {
            #[expect(clippy::cast_possible_truncation, reason = "records store f32")]
            let psi = psi as f32;
            (PIECE_POINT_ARC, n_off, 1, [psi, 0.0, 0.0, 0.0])
        }
        Body::JoinOnly => (PIECE_JOIN_ONLY, 0, 1, [0.0; 4]),
    }
}

/// The length of a piece's control polygon, the bound on its arc length.
fn polygon_length(piece: &Piece) -> f64 {
    match piece.body {
        Body::Line => (piece.p1 - piece.p0).hypot(),
        Body::Curve { cubic, .. } | Body::PointArc { cubic, .. } => {
            (cubic.p1 - cubic.p0).hypot()
                + (cubic.p2 - cubic.p1).hypot()
                + (cubic.p3 - cubic.p2).hypot()
        }
        Body::JoinOnly => 0.0,
    }
}

/// The exact f64 arc length a dashed piece's record carries in fixed
/// point (design section 6.6): a LINE's control polygon is its length;
/// a `POINT_ARC` stores its sub-cubic's so the section-3 drift bound
/// holds at cusps too.
fn dash_piece_length(piece: &Piece, polygon: f64, dashed: bool) -> Option<f64> {
    if !dashed {
        return None;
    }
    match piece.body {
        Body::Line => Some(polygon),
        Body::PointArc { cubic, .. } => Some(cubic.arclen(POINT_ARC_LENGTH_ACCURACY)),
        _ => None,
    }
}

/// The cap chord counts and the bound row of a cap style.
struct CapRow {
    chords: usize,
    /// Device variation per axis, before the path's `s * hw` factor.
    reach: f64,
}

struct Encoder<'a> {
    stroke: &'a Stroke,
    raster: Affine,
    tol: f64,
    hw: f64,
    /// `sigma_max(raster)`.
    s: f64,
    join_thresh: f64,
    k_cap: u32,
    dash: Option<DashPlan>,
    records: Vec<GpuStrokeRecord>,
    /// `2 * (j + o)` chords of each record, before caps.
    body_chords: Vec<usize>,
    // The walk.
    start_pt: Point,
    start_tan: Vec2,
    last_pt: Point,
    last_tan: Vec2,
    has_forward: bool,
    first: usize,
    cur_fe: u32,
    cur_be: u32,
    pending: Option<JoinRec>,
    // The bound.
    segments: usize,
    bins: usize,
    fragments: usize,
    hull: Rect,
    // The dash capacities.
    subpath_polygon: f64,
    stage_total: usize,
    dashes: usize,
}

impl<'a> Encoder<'a> {
    fn new(stroke: &'a Stroke, raster: Affine, tol: f64, dash: Option<DashPlan>) -> Self {
        let hw = stroke.width / 2.0;
        Self {
            stroke,
            raster,
            tol,
            hw,
            s: sigma_max(raster),
            join_thresh: 2.0 * tol / stroke.width,
            k_cap: 0,
            dash,
            records: Vec::new(),
            body_chords: Vec::new(),
            start_pt: Point::ORIGIN,
            start_tan: Vec2::ZERO,
            last_pt: Point::ORIGIN,
            last_tan: Vec2::ZERO,
            has_forward: false,
            first: 0,
            cur_fe: 0,
            cur_be: 0,
            pending: None,
            segments: 0,
            bins: 0,
            fragments: 0,
            hull: Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            subpath_polygon: 0.0,
            stage_total: 0,
            dashes: 0,
        }
    }

    /// The walk of design section 6.2, following kurbo's `stroke_undashed`.
    fn walk(&mut self, path: &BezPath) -> Result<(), RenderError> {
        self.k_cap = arc_chords(PI, self.tol / 2.0, self.hw)?;
        for el in path.iter() {
            let p0 = self.last_pt;
            // An element with a non-finite point is skipped, the cursor still
            // advancing to its end, as `prepare_gpu` skips it.
            let r = self.raster;
            let checked = match el {
                PathEl::MoveTo(_) => None,
                PathEl::LineTo(p1) => Some((p1, encodable(r, &[p0, p1]))),
                PathEl::QuadTo(p1, p2) => Some((p2, encodable(r, &[p0, p1, p2]))),
                PathEl::CurveTo(p1, p2, p3) => Some((p3, encodable(r, &[p0, p1, p2, p3]))),
                PathEl::ClosePath => Some((self.start_pt, encodable(r, &[p0, self.start_pt]))),
            };
            if let Some((end, false)) = checked {
                self.last_pt = end;
                continue;
            }
            match el {
                PathEl::MoveTo(p) => {
                    self.finish(false)?;
                    self.start_pt = p;
                    self.last_pt = p;
                }
                PathEl::LineTo(p1) => {
                    if p1 != p0 {
                        let t = p1 - p0;
                        self.join(t, self.stroke.join)?;
                        self.line(p0, p1, t)?;
                        self.last_tan = t;
                    }
                }
                PathEl::QuadTo(p1, p2) => {
                    if p1 != p0 || p2 != p0 {
                        let q = QuadBez::new(p0, p1, p2);
                        let (t0, t1) = tangents_quad(q);
                        self.join(t0, self.stroke.join)?;
                        self.cubic(q.raise(), t0, t1)?;
                        self.last_tan = t1;
                    }
                }
                PathEl::CurveTo(p1, p2, p3) => {
                    if p1 != p0 || p2 != p0 || p3 != p0 {
                        let c = CubicBez::new(p0, p1, p2, p3);
                        let (t0, t1) = tangents_cubic(c);
                        self.join(t0, self.stroke.join)?;
                        self.cubic(c, t0, t1)?;
                        self.last_tan = t1;
                    }
                }
                PathEl::ClosePath => {
                    if p0 != self.start_pt {
                        let t = self.start_pt - p0;
                        self.join(t, self.stroke.join)?;
                        self.line(p0, self.start_pt, t)?;
                        self.last_tan = t;
                    }
                    self.finish_closed()?;
                }
            }
        }
        self.finish(false)
    }

    /// Sets the join the next piece starts with (kurbo `do_join`'s
    /// classification; the branch decisions that move only geometry are the
    /// kernels').
    fn join(&mut self, tan0: Vec2, style: Join) -> Result<(), RenderError> {
        // A second join at one vertex (a `do_linear` cubic) cannot attach to
        // the next piece, so the first becomes a piece of its own.
        if let Some(earlier) = self.pending.take() {
            self.join_only(earlier, 0, None)?;
        }
        let at = self.last_pt;
        if !self.has_forward {
            self.start_tan = tan0;
            self.pending = Some(JoinRec {
                code: JOIN_START,
                k: 0,
                phi: 0.0,
                tan: tan0,
                at,
            });
            return Ok(());
        }
        let ab = self.last_tan;
        let cross = ab.cross(tan0);
        let dot = ab.dot(tan0);
        let hypot = cross.hypot(dot);
        let phi = cross.atan2(dot);
        let (code, k) = if dot > 0.0 && cross.abs() < hypot * self.join_thresh {
            (JOIN_SHARED, 0)
        } else {
            match style {
                Join::Bevel => (JOIN_BEVEL, 0),
                Join::Miter => (JOIN_MITER, 0),
                Join::Round => (JOIN_ROUND, arc_chords(phi, self.tol / 2.0, self.hw)?),
            }
        };
        self.pending = Some(JoinRec {
            code,
            k,
            phi,
            tan: tan0,
            at,
        });
        Ok(())
    }

    /// kurbo `do_line`.
    fn line(&mut self, p0: Point, p1: Point, t: Vec2) -> Result<(), RenderError> {
        self.push_piece(&Piece {
            body: Body::Line,
            p0,
            t0: t,
            p1,
            t1: t,
        })?;
        self.last_pt = p1;
        Ok(())
    }

    /// kurbo `do_cubic`, with the sub-pieces of design section 6.3.
    fn cubic(&mut self, c: CubicBez, tan0: Vec2, tan1: Vec2) -> Result<(), RenderError> {
        if let Some(linear) = near_linear(c, self.tol) {
            return self.linear(c, &linear);
        }
        let regular = regularize_cusp(c, self.tol * 0.25);
        let mut subs = Vec::new();
        let ctx = SubdivideCtx {
            hw: self.hw,
            tol: self.tol,
            dashed: self.dash.is_some(),
        };
        subdivide(regular, ctx, 0, &mut subs)?;
        let last = subs.len() - 1;
        let mut prev_t1 = tan0;
        for (k, sub) in subs.into_iter().enumerate() {
            let (mut t0, mut t1) = tangents_cubic(sub.cubic);
            if k == 0 {
                t0 = tan0;
            } else {
                // An interior boundary is classified like any vertex: a
                // smooth one resolves to `SHARED`, a reversal (an exact
                // cusp) gets the stroke's own join, as kurbo applies it at
                // the split cusp `cherenkov_stroke` hands it — a miter at a
                // 180-degree reversal exceeds the limit and bevels.
                self.last_pt = sub.cubic.p0;
                self.last_tan = prev_t1;
                self.join(t0, self.stroke.join)?;
            }
            if k == last {
                t1 = tan1;
            }
            let body = sub.psi.map_or(
                Body::Curve {
                    cubic: sub.cubic,
                    n_off: sub.n_off,
                    n_len: sub.n_len,
                },
                |psi| Body::PointArc {
                    cubic: sub.cubic,
                    psi,
                    n_off: sub.n_off,
                },
            );
            self.push_piece(&Piece {
                body,
                p0: sub.cubic.p0,
                t0,
                p1: sub.cubic.p3,
                t1,
            })?;
            prev_t1 = t1;
        }
        self.last_pt = c.p3;
        Ok(())
    }

    /// kurbo `do_linear`: a cubic that is a line with reversals, drawn as
    /// lines joined by round joins.
    #[expect(
        clippy::suboptimal_flops,
        reason = "line-for-line port of kurbo 0.13.1 `do_linear`"
    )]
    fn linear(&mut self, c: CubicBez, linear: &Linear) -> Result<(), RenderError> {
        // Cusps right at the endpoints are discarded.
        const EPSILON: f64 = 1e-6;
        let (tan0, tan1) = tangents_cubic(c);
        self.last_tan = tan0;
        let p = linear.projected;
        let c0 = p[1] - p[0];
        let c1 = 2.0 * p[2] - 4.0 * p[1] + 2.0 * p[0];
        let c2 = p[3] - 3.0 * p[2] + 3.0 * p[1] - p[0];
        for t in solve_quadratic(c0, c1, c2) {
            if t > EPSILON && t < 1.0 - EPSILON {
                let mt = 1.0 - t;
                let z = mt * (mt * mt * p[0] + 3.0 * t * (mt * p[1] + t * p[2])) + t * t * t * p[3];
                let point = linear.ref_pt + z * linear.ref_vec;
                self.round_line(point)?;
            }
        }
        self.round_line(c.p3)?;
        self.join(tan1, Join::Round)?;
        let last = self.pending.take().expect("join sets the pending join");
        self.join_only(last, LINEAR_END, None)
    }

    /// One line of a `do_linear` cubic, to `to`, with a round join before it.
    /// A zero-length line has no direction and is skipped, as every
    /// zero-length segment is.
    fn round_line(&mut self, to: Point) -> Result<(), RenderError> {
        let from = self.last_pt;
        let tan = to - from;
        if tan == Vec2::ZERO {
            return Ok(());
        }
        self.join(tan, Join::Round)?;
        self.line(from, to, tan)?;
        self.last_tan = tan;
        Ok(())
    }

    /// A piece that is a join and nothing else: at `join.at`, with the tangent
    /// of the join.
    fn join_only(
        &mut self,
        join: JoinRec,
        flags: u32,
        close_to: Option<usize>,
    ) -> Result<(), RenderError> {
        self.append(
            &Piece {
                body: Body::JoinOnly,
                p0: join.at,
                t0: join.tan,
                p1: join.at,
                t1: join.tan,
            },
            flags,
            Some(join),
            close_to,
        )
    }

    /// A piece with the join pending before it, if any: the second and later
    /// sub-pieces of one cubic have none and share the previous end vertices.
    fn push_piece(&mut self, piece: &Piece) -> Result<(), RenderError> {
        let join = self.pending.take();
        self.append(piece, 0, join, None)
    }

    /// Appends one record (design sections 5.2 and 6.2): resolves its vertex
    /// slots, counts its chords and adds its groups to the bound.
    fn append(
        &mut self,
        piece: &Piece,
        extra: u32,
        join: Option<JoinRec>,
        close_to: Option<usize>,
    ) -> Result<(), RenderError> {
        let i = self.records.len();
        let slot = |record: usize, slot: u32| -> Result<u32, RenderError> {
            let base = count_u32(record, "stroke record count")?;
            base.checked_mul(SLOTS)
                .and_then(|v| v.checked_add(slot))
                .ok_or(RenderError::Unsupported("stroke record count"))
        };
        let code = join.map_or(JOIN_SHARED, |j| j.code);
        let k_join = join.map_or(0, |j| j.k);
        let prev = if code == JOIN_START {
            [u32::MAX; 2]
        } else {
            [self.cur_fe, self.cur_be]
        };
        let (fs, bs) = if let Some(first) = close_to {
            (slot(first, SLOT_FS)?, slot(first, SLOT_BS)?)
        } else if code == JOIN_SHARED {
            (self.cur_fe, self.cur_be)
        } else {
            (slot(i, SLOT_FS)?, slot(i, SLOT_BS)?)
        };
        let join_only = matches!(piece.body, Body::JoinOnly);
        let (fe, be) = if join_only {
            (fs, bs)
        } else {
            (slot(i, SLOT_FE)?, slot(i, SLOT_BE)?)
        };
        let (kind, n_off, n_len, c) = body_fields(&piece.body);
        let join_chords = match code {
            JOIN_BEVEL => 1,
            JOIN_MITER => 2,
            JOIN_ROUND => k_join,
            _ => 0,
        };
        let body_chords = 2 * (join_chords as usize + n_off as usize);
        let polygon = polygon_length(piece);
        let (stage_cap, stage_base) = if let Some(plan) = &self.dash {
            let cap = if join_only {
                // Its join and, when it ends the subpath, the cap dot of a
                // zero-length dash there.
                2
            } else {
                1 + plan.starts(polygon)
            };
            (cap, self.stage_total)
        } else {
            (0, 0)
        };
        // A dashed LINE carries its length in `c.xy`, a dashed POINT_ARC
        // its sub-cubic's in `c.yz` (design section 6.6).
        let c = dash_piece_length(piece, polygon, self.dash.is_some()).map_or(c, |length| {
            let [integer, fraction] = fixed(length);
            let (x, y) = (f32::from_bits(integer), f32::from_bits(fraction));
            if matches!(piece.body, Body::Line) {
                [x, y, 0.0, 0.0]
            } else {
                [c[0], x, y, c[3]]
            }
        });
        let [p0x, p0y] = f32x2(piece.p0);
        let [t0x, t0y] = f32x2v(piece.t0);
        let [p1x, p1y] = f32x2(piece.p1);
        let [t1x, t1y] = f32x2v(piece.t1);
        let flags = kind | (code << JOIN_SHIFT) | extra;
        debug_assert_eq!(flags & PIECE_MASK, kind);
        debug_assert_eq!((flags >> JOIN_SHIFT) & JOIN_MASK, code);
        self.records.push(GpuStrokeRecord {
            head: [
                flags,
                count_u32(stage_cap, "dash staging capacity")?,
                0,
                count_u32(stage_base, "dash staging capacity")?,
            ],
            link: [prev[0], prev[1], fs, bs],
            aux: [k_join | (n_off << 16), n_len, 0, 0],
            p: [p0x, p0y, t0x, t0y],
            q: [p1x, p1y, t1x, t1y],
            c,
        });
        self.body_chords.push(body_chords);
        self.stage_total += stage_cap;
        self.subpath_polygon += polygon;
        if self.dash.is_some() {
            self.segments += body_chords;
        }
        self.bound_piece(piece, join);
        if code == JOIN_START {
            self.first = i;
        }
        self.cur_fe = fe;
        self.cur_be = be;
        self.has_forward = true;
        Ok(())
    }

    /// The device variation per axis of the user vector `v`.
    fn device(&self, v: Vec2) -> (f64, f64) {
        let [xx, yx, xy, yy, _, _] = self.raster.as_coeffs();
        (
            xx.mul_add(v.x, xy * v.y).abs(),
            yx.mul_add(v.x, yy * v.y).abs(),
        )
    }

    /// The bin and fragment contribution of a chord group (design section
    /// 6.5): `chords` chords with device variation `(vx, vy)`.
    fn group(&self, chords: usize, vx: f64, vy: f64) -> (usize, usize) {
        let margin = |v: f64| self.s.mul_add(self.tol, v * (1.0 + VARIATION_MARGIN));
        let (vx, vy) = (margin(vx), margin(vy));
        (
            2 * chords + ceil_count(vy / 4.0),
            3 * chords + ceil_count(vx + vy),
        )
    }

    fn add_group(&mut self, times: usize, chords: usize, vx: f64, vy: f64) {
        let (bins, fragments) = self.group(chords, vx, vy);
        self.bins += times * bins;
        self.fragments += times * fragments;
    }

    /// Adds a piece's offset and join groups to the bound, and the piece to
    /// the hull. A join is drawn on either side, so every join row applies to
    /// both.
    fn bound_piece(&mut self, piece: &Piece, join: Option<JoinRec>) {
        let sh = self.s * self.hw;
        match &piece.body {
            Body::Line => {
                let (vx, vy) = self.device(piece.p1 - piece.p0);
                self.add_group(2, 1, vx, vy);
            }
            Body::Curve { cubic, n_off, .. } => {
                let (mut vx, mut vy) = (0.0, 0.0);
                for (a, b) in [
                    (cubic.p0, cubic.p1),
                    (cubic.p1, cubic.p2),
                    (cubic.p2, cubic.p3),
                ] {
                    let (dx, dy) = self.device(b - a);
                    vx += dx;
                    vy += dy;
                }
                let h = [
                    3.0 * (cubic.p1 - cubic.p0),
                    3.0 * (cubic.p2 - cubic.p1),
                    3.0 * (cubic.p3 - cubic.p2),
                ];
                let turn = sh * 3.0 * aperture(h);
                self.add_group(2, *n_off as usize, vx + turn, vy + turn);
            }
            Body::PointArc {
                cubic, psi, n_off, ..
            } => {
                let v = (2.0 * self.s).mul_add(diameter(cubic), sh * psi.abs());
                self.add_group(2, *n_off as usize, v, v);
            }
            Body::JoinOnly => {}
        }
        if let Some(join) = join {
            let (chords, v) = match join.code {
                JOIN_BEVEL => (1, sh * join.phi.abs()),
                JOIN_MITER => (2, 2.0 * sh * self.stroke.miter_limit),
                JOIN_ROUND => (join.k as usize, sh * join.phi.abs()),
                _ => (0, 0.0),
            };
            if chords > 0 {
                self.add_group(2, chords, v, v);
            }
        }
        let mut dev = Rect::from_points(self.raster * piece.p0, self.raster * piece.p1);
        if let Body::Curve { cubic, .. } = &piece.body {
            dev = dev
                .union_pt(self.raster * cubic.p1)
                .union_pt(self.raster * cubic.p2);
        }
        self.hull = self.hull.union(dev);
    }

    /// The chord count and bound row of a cap style (design section 6.5).
    fn cap_row(&self, cap: Cap) -> CapRow {
        let sh = self.s * self.hw;
        match cap {
            Cap::Butt => CapRow {
                chords: 1,
                reach: 2.0 * sh,
            },
            Cap::Square => CapRow {
                chords: 3,
                reach: 4.0 * sh,
            },
            Cap::Round => CapRow {
                chords: self.k_cap as usize,
                reach: sh * PI,
            },
        }
    }

    /// Closes the current subpath (kurbo `finish`): caps and slots for an
    /// undashed stroke, capacities for a dashed one.
    fn finish(&mut self, closed: bool) -> Result<(), RenderError> {
        if !self.has_forward {
            return Ok(());
        }
        let first = self.first;
        let last = self.records.len() - 1;
        if let Some(plan) = &self.dash {
            if self.subpath_polygon >= FIXED_LIMIT {
                return Err(RenderError::Unsupported("dashed subpath too long"));
            }
            // `dash_fragments` computes `u1 = φ + s0 + L` and a following
            // boundary `b` up to a period past `u1`, so a subpath's
            // positions must stay below `2^32 - P` in the kernel's `2^-32`
            // fixed point or they wrap.
            let [pi, pf] = fixed(plan.period);
            let [li, lf] = fixed(self.subpath_polygon);
            let period = u128::from(pi) << 32 | u128::from(pf);
            let length = u128::from(li) << 32 | u128::from(lf);
            if period + length >= (1u128 << 64) - period {
                return Err(RenderError::Unsupported("dashed subpath too long"));
            }
            let (first_rel, last_rel) = (
                count_u32(first, "stroke records")?,
                count_u32(last, "stroke records")?,
            );
            for record in &mut self.records[first..] {
                record.aux[2] = first_rel;
                record.aux[3] = last_rel;
            }
            self.dashes = self
                .dashes
                .saturating_add(plan.starts(self.subpath_polygon).saturating_add(1));
        } else {
            let (start, end) = (
                self.cap_row(self.stroke.start_cap),
                self.cap_row(self.stroke.end_cap),
            );
            if !closed {
                self.records[first].head[0] |= CAP_START;
                self.records[last].head[0] |= CAP_END;
                self.add_group(1, start.chords, start.reach, start.reach);
                self.add_group(1, end.chords, end.reach, end.reach);
            }
            for i in first..=last {
                let flags = self.records[i].head[0];
                let mut n = self.body_chords[i];
                if flags & CAP_START != 0 {
                    n += start.chords;
                }
                if flags & CAP_END != 0 {
                    n += end.chords;
                }
                self.records[i].head[1] = count_u32(n, "stroke chord count")?;
                self.records[i].head[3] = count_u32(self.segments, "stroke segment count")?;
                self.segments += n;
            }
        }
        self.has_forward = false;
        self.subpath_polygon = 0.0;
        Ok(())
    }

    /// kurbo `finish_closed`: the closing join is a piece of its own, from the
    /// last end vertices to the subpath's first start vertices.
    fn finish_closed(&mut self) -> Result<(), RenderError> {
        if !self.has_forward {
            return Ok(());
        }
        self.join(self.start_tan, self.stroke.join)?;
        let mut join = self.pending.take().expect("join sets the pending join");
        // kurbo still draws a straight line per side where it omits the join.
        if join.code == JOIN_SHARED {
            join.code = JOIN_BEVEL;
        }
        join.at = self.last_pt;
        self.join_only(join, CLOSE, Some(self.first))?;
        self.finish(true)
    }

    /// Assembles the path: the records, the stroke parameters, the dash table
    /// and the bound (design sections 6.5 and 6.6).
    fn finish_path(mut self, origin: Vec2, surface: (f64, f64)) -> GpuPath {
        let sh = self.s * self.hw;
        let (start, end) = (
            self.cap_row(self.stroke.start_cap),
            self.cap_row(self.stroke.end_cap),
        );
        let dash = if let Some(plan) = self.dash.take() {
            let records = self.records.len();
            let fragments = self.dashes.saturating_add(records);
            // Dash fragments take one more chord and one more rounding per
            // side, and every dash draws at most one start and one end cap.
            self.bins += 2 * 3 * fragments;
            self.fragments += 2 * 4 * fragments;
            self.add_group(self.dashes, start.chords, start.reach, start.reach);
            self.add_group(self.dashes, end.chords, end.reach, end.reach);
            self.segments += 2 * fragments + self.dashes * (start.chords + end.chords);
            Some(Arc::new(DashTable {
                cum: plan.cum.iter().map(|&x| fixed(x)).collect(),
                phase: fixed(plan.phase),
                merge_start: plan.merge_start,
                stage_total: self.stage_total,
                fragments,
                dashes: self.dashes,
            }))
        } else {
            None
        };
        let miter = if self.stroke.join == Join::Miter {
            self.stroke.miter_limit
        } else {
            1.0
        };
        let square = if self.stroke.start_cap == Cap::Square || self.stroke.end_cap == Cap::Square {
            SQRT_2
        } else {
            1.0
        };
        let reach = sh * 1.0_f64.max(miter).max(square);
        let hull = self.hull.inflate(reach, reach);
        let bound = GpuBound {
            segments: self.segments,
            bins: self.bins,
            fragments: self.fragments,
            ..GpuBound::default()
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the GPU path record stores f32"
        )]
        let stroke = StrokeParams {
            half_width: self.hw as f32,
            miter_limit: self.stroke.miter_limit as f32,
            start_cap: cap_code(self.stroke.start_cap),
            end_cap: cap_code(self.stroke.end_cap),
            k_cap: self.k_cap,
        };
        GpuPath {
            elements: Arc::from([]),
            strokes: self.records.into(),
            stroke,
            dash,
            hull,
            origin,
            bound,
        }
        .placed(origin, surface)
    }
}

const fn cap_code(cap: Cap) -> u32 {
    match cap {
        Cap::Butt => CAP_BUTT,
        Cap::Square => CAP_SQUARE,
        Cap::Round => CAP_ROUND,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use kurbo::ParamCurveArclen;

    use super::*;
    use crate::render::path::FLATTEN;
    use crate::render::stroke_abi::{CAP_BUTT, CAP_SQUARE};

    /// A seeded generator, so a failing case replays.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        /// Uniform in `[0, 1)`.
        fn unit(&mut self) -> f64 {
            f64::from(u32::try_from(self.next() >> 32).expect("32 bits")) / 4_294_967_296.0
        }

        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            (hi - lo).mul_add(self.unit(), lo)
        }

        fn below(&mut self, n: usize) -> usize {
            let n = u64::try_from(n).expect("a small bound");
            usize::try_from(self.next() % n).expect("below a usize bound")
        }

        fn point(&mut self) -> Point {
            Point::new(self.range(0.0, 100.0), self.range(0.0, 100.0))
        }
    }

    fn shape(path: &BezPath) -> ShapeData {
        ShapeData::Path {
            elements: path.elements().into(),
            rule: cherenkov::FillRule::NonZero,
        }
    }

    /// A random cubic of one of the shapes the encoder has to survive: generic,
    /// looped, cusped, `p1 == p0`, or colinear with reversals.
    fn random_cubic(rng: &mut Rng, p0: Point) -> CubicBez {
        match rng.below(6) {
            0 => CubicBez::new(p0, rng.point(), rng.point(), rng.point()),
            // A loop: the control points cross.
            1 => {
                let p3 = p0 + Vec2::new(rng.range(-5.0, 5.0), rng.range(-5.0, 5.0));
                let a = p0 + Vec2::new(rng.range(20.0, 40.0), rng.range(20.0, 40.0));
                let b = p3 + Vec2::new(rng.range(-40.0, -20.0), rng.range(20.0, 40.0));
                CubicBez::new(p0, a, b, p3)
            }
            // A true cusp: (0,0), (1,1), (0,1), (1,0), scaled and moved.
            2 => {
                let k = rng.range(1.0, 60.0);
                let at = |x: f64, y: f64| p0 + Vec2::new(x, y) * k;
                CubicBez::new(at(0.0, 0.0), at(1.0, 1.0), at(0.0, 1.0), at(1.0, 0.0))
            }
            // p1 == p0.
            3 => CubicBez::new(p0, p0, rng.point(), rng.point()),
            // Colinear, with a reversal.
            4 => {
                let dir = Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)).normalize();
                let at = |s: f64| p0 + dir * s;
                CubicBez::new(
                    p0,
                    at(rng.range(0.0, 60.0)),
                    at(rng.range(-20.0, 40.0)),
                    at(rng.range(-20.0, 60.0)),
                )
            }
            // A small cubic, down to the point-arc scale.
            _ => {
                let k = [1e-4, 1e-2, 0.5, 3.0][rng.below(4)];
                let d = |rng: &mut Rng| Vec2::new(rng.range(-k, k), rng.range(-k, k));
                CubicBez::new(p0, p0 + d(rng), p0 + d(rng), p0 + d(rng))
            }
        }
    }

    /// Random subpaths of lines, quads and cubics, with colinear runs,
    /// reversals, repeated points, and open and closed endings.
    fn random_path(rng: &mut Rng) -> BezPath {
        let mut path = BezPath::new();
        for _ in 0..=rng.below(3) {
            let mut at = rng.point();
            path.move_to(at);
            for _ in 0..=rng.below(7) {
                match rng.below(8) {
                    // A line, a colinear continuation, a reversal.
                    0 | 1 => at = rng.point(),
                    2 => at += (at - path_start(&path)) * rng.range(0.1, 1.0),
                    3 => at = path_start(&path) + (path_start(&path) - at) * rng.range(0.0, 1.0),
                    // A repeated point: a zero-length segment.
                    4 => {}
                    5 => {
                        let c = rng.point();
                        let p = rng.point();
                        path.quad_to(c, p);
                        at = p;
                        continue;
                    }
                    _ => {
                        let c = random_cubic(rng, at);
                        path.curve_to(c.p1, c.p2, c.p3);
                        at = c.p3;
                        continue;
                    }
                }
                path.line_to(at);
            }
            if rng.below(2) == 0 {
                path.close_path();
            }
        }
        path
    }

    /// The last move point: a pivot for the colinear and reversal cases.
    fn path_start(path: &BezPath) -> Point {
        path.elements()
            .iter()
            .rev()
            .find_map(|el| match el {
                PathEl::MoveTo(p) => Some(*p),
                _ => None,
            })
            .expect("a path under construction starts with a move")
    }

    const JOINS: [Join; 3] = [Join::Bevel, Join::Miter, Join::Round];
    const CAPS: [Cap; 3] = [Cap::Butt, Cap::Square, Cap::Round];

    fn style(rng: &mut Rng, join: Join, start: Cap, end: Cap) -> Stroke {
        let mut stroke = Stroke::new(rng.range(0.05, 9.0))
            .with_join(join)
            .with_miter_limit(rng.range(1.0, 6.0));
        stroke.start_cap = start;
        stroke.end_cap = end;
        stroke
    }

    /// The raster of a test: an anisotropic scale, so `sigma_max != 1`.
    fn raster(rng: &mut Rng) -> Affine {
        Affine::new([
            rng.range(0.5, 3.0),
            rng.range(-0.5, 0.5),
            rng.range(-0.5, 0.5),
            rng.range(0.5, 3.0),
            rng.range(0.0, 40.0),
            rng.range(0.0, 40.0),
        ])
    }

    const SURFACE: (f64, f64) = (512.0, 512.0);

    /// A small index or count as `u32`.
    fn small(n: usize) -> u32 {
        u32::try_from(n).expect("a small count")
    }

    /// A small index or count as `f64`.
    fn flt(n: usize) -> f64 {
        f64::from(small(n))
    }

    fn encode(path: &BezPath, stroke: &Stroke, raster: Affine) -> GpuPath {
        let tol = FLATTEN / sigma_max(raster).max(1e-12);
        prepare(&shape(path), stroke, raster, tol, SURFACE).expect("the stroke encodes")
    }

    // The decode of a record, written from the layout of design section 5.2
    // and independent of the encoder's own bookkeeping.
    fn piece(r: &GpuStrokeRecord) -> u32 {
        r.head[0] & 0b11
    }

    fn join_code(r: &GpuStrokeRecord) -> u32 {
        (r.head[0] >> 2) & 0b111
    }

    fn join_chords(r: &GpuStrokeRecord) -> usize {
        match join_code(r) {
            2 => 1,
            3 => 2,
            4 => (r.aux[0] & 0xFFFF) as usize,
            _ => 0,
        }
    }

    fn offset_chords(r: &GpuStrokeRecord) -> usize {
        match piece(r) {
            0 => 1,
            1 | 2 => (r.aux[0] >> 16) as usize,
            _ => 0,
        }
    }

    fn cap_chords(cap: u32, k_cap: u32) -> usize {
        match cap {
            CAP_BUTT => 1,
            CAP_SQUARE => 3,
            _ => k_cap as usize,
        }
    }

    /// The subpaths of a record list, as index ranges: each begins at a
    /// `JOIN_START` record.
    fn subpaths(records: &[GpuStrokeRecord]) -> Vec<std::ops::Range<usize>> {
        let starts: Vec<usize> = (0..records.len())
            .filter(|&i| join_code(&records[i]) == 0)
            .collect();
        assert!(records.is_empty() || starts.first() == Some(&0));
        starts
            .iter()
            .enumerate()
            .map(|(k, &s)| s..starts.get(k + 1).copied().unwrap_or(records.len()))
            .collect()
    }

    const CAP_START_FLAG: u32 = 1 << 5;
    const CAP_END_FLAG: u32 = 1 << 6;
    const CLOSE_FLAG: u32 = 1 << 7;

    /// Every encoder input of the structural tests: random paths under every
    /// join style and cap pair.
    fn for_each_encode(seed: u64, mut check: impl FnMut(&Stroke, &BezPath, &GpuPath)) {
        let mut rng = Rng(seed);
        for _ in 0..24 {
            let path = random_path(&mut rng);
            let raster = raster(&mut rng);
            for join in JOINS {
                for start in CAPS {
                    for end in CAPS {
                        let stroke = style(&mut rng, join, start, end);
                        let gpu = encode(&path, &stroke, raster);
                        check(&stroke, &path, &gpu);
                    }
                }
            }
        }
    }

    #[test]
    fn undashed_slots_are_exact_and_disjoint() {
        for_each_encode(0x2544_0001, |stroke, _, gpu| {
            let records = &gpu.strokes;
            let k_cap = gpu.stroke.k_cap;
            let mut next = 0usize;
            for range in subpaths(records) {
                let subpath = &records[range.clone()];
                let closed = subpath.last().is_some_and(|r| r.head[0] & CLOSE_FLAG != 0);
                for (k, r) in subpath.iter().enumerate() {
                    let start = !closed && k == 0;
                    let end = !closed && k == subpath.len() - 1;
                    assert_eq!(r.head[0] & CAP_START_FLAG != 0, start, "start cap flag");
                    assert_eq!(r.head[0] & CAP_END_FLAG != 0, end, "end cap flag");
                    let expected = 2 * (join_chords(r) + offset_chords(r))
                        + if start {
                            cap_chords(cap_code(stroke.start_cap), k_cap)
                        } else {
                            0
                        }
                        + if end {
                            cap_chords(cap_code(stroke.end_cap), k_cap)
                        } else {
                            0
                        };
                    assert_eq!(r.head[1] as usize, expected, "chord count");
                    // Contiguous inside the subpath, and the subpaths tile
                    // the slot range in order: disjoint by construction.
                    assert_eq!(r.head[3] as usize, next, "slot");
                    next += expected;
                }
            }
            assert_eq!(next, gpu.bound.segments, "the slots sum to the bound");
            assert!(gpu.dash.is_none());
        });
    }

    /// Whether record `r` owns vertex slot `slot` (design section 5.2).
    fn owns(r: &GpuStrokeRecord, slot: u32) -> bool {
        match slot {
            0 | 1 => join_code(r) != 1 && r.head[0] & CLOSE_FLAG == 0,
            _ => piece(r) != 3,
        }
    }

    /// Checks every vertex reference of the subpath records `range`: owned
    /// slots are their own, a shared join takes the previous end vertices, a
    /// closing join the subpath's first start vertices, and any other
    /// reference names an owned slot of an earlier record.
    fn check_references(records: &[GpuStrokeRecord], range: std::ops::Range<usize>) {
        let first = range.start;
        for i in range {
            let r = &records[i];
            let prev = [r.link[0], r.link[1]];
            let [fs, bs] = [r.link[2], r.link[3]];
            let code = join_code(r);
            let references = |v: u32| {
                let rec = usize::try_from(v / 4).expect("a record index");
                assert!(rec < i || rec == first, "reference to a later record");
                assert!(owns(&records[rec], v % 4), "reference to an unowned slot");
            };
            if code == 0 {
                assert_eq!(prev, [u32::MAX; 2]);
            } else {
                for v in prev {
                    references(v);
                }
            }
            if owns(r, 0) {
                assert_eq!([fs, bs], [small(i) * 4, small(i) * 4 + 1], "owned start");
            } else {
                references(fs);
                references(bs);
                if code == 1 {
                    assert_eq!([fs, bs], prev, "shared join");
                } else {
                    assert_eq!(fs, small(first) * 4, "closing join to the first FS");
                    assert_eq!(bs, small(first) * 4 + 1, "closing join to the first BS");
                }
            }
        }
    }

    /// The directed multigraph of chords the layout of design section 5.2
    /// implies: slots are nodes, interior points are fresh nodes, and a
    /// backward chord runs reversed.
    fn contour_edges(
        gpu: &GpuPath,
        range: std::ops::Range<usize>,
        fresh: &mut u32,
    ) -> Vec<(u32, u32)> {
        let mut edges = Vec::new();
        // `count` chords from `a` to `b` through fresh interior nodes.
        let mut chain = |a: u32, b: u32, count: usize| {
            let mut from = a;
            for k in 0..count {
                let to = if k + 1 == count {
                    b
                } else {
                    *fresh += 1;
                    *fresh
                };
                edges.push((from, to));
                from = to;
            }
        };
        let cap = |r: &GpuStrokeRecord, flag: u32, code: u32| {
            if r.head[0] & flag != 0 {
                cap_chords(code, gpu.stroke.k_cap)
            } else {
                0
            }
        };
        for i in range {
            let r = &gpu.strokes[i];
            let prev = [r.link[0], r.link[1]];
            let [fs, bs] = [r.link[2], r.link[3]];
            let (fe, be) = if piece(r) == 3 {
                (fs, bs)
            } else {
                (small(i) * 4 + 2, small(i) * 4 + 3)
            };
            let drawn_join = join_code(r) > 1;
            chain(bs, fs, cap(r, CAP_START_FLAG, gpu.stroke.start_cap));
            if drawn_join {
                chain(prev[0], fs, join_chords(r));
            }
            chain(fs, fe, offset_chords(r));
            if drawn_join {
                chain(bs, prev[1], join_chords(r));
            }
            chain(be, bs, offset_chords(r));
            chain(fe, be, cap(r, CAP_END_FLAG, gpu.stroke.end_cap));
        }
        edges
    }

    #[test]
    fn vertex_references_close_every_contour() {
        for_each_encode(0x2544_0002, |_, _, gpu| {
            let records = &gpu.strokes;
            let mut fresh = small(records.len()) * 4;
            let mut edges = Vec::new();
            for range in subpaths(records) {
                check_references(records, range.clone());
                edges.extend(contour_edges(gpu, range, &mut fresh));
            }
            // Every owned slot is a chord endpoint, and no chord touches a
            // slot nobody owns. A slot is `4 * record + lane`.
            let floor = small(records.len()) * 4;
            let owned: HashSet<u32> = (0..floor)
                .filter(|&s| owns(&records[usize::try_from(s / 4).expect("index")], s % 4))
                .collect();
            let touched: HashSet<u32> = edges.iter().flat_map(|&(a, b)| vec![a, b]).collect();
            for node in touched.iter().filter(|&&n| n < floor) {
                assert!(owned.contains(node), "chord on an unowned slot");
            }
            for slot in &owned {
                assert!(touched.contains(slot), "owned slot {slot} is on no chord");
            }
            let mut degree: HashMap<u32, i64> = HashMap::new();
            for (a, b) in edges {
                *degree.entry(a).or_default() += 1;
                *degree.entry(b).or_default() -= 1;
            }
            assert!(
                degree.values().all(|&d| d == 0),
                "a contour node has unequal in and out degree"
            );
        });
    }

    /// The exact f64 parallel curve of `c` at `t`, on the side `d` (`-hw` is
    /// the forward side).
    fn offset_at(c: &CubicBez, d: f64, t: f64) -> Point {
        let deriv = c.deriv().eval(t).to_vec2();
        c.eval(t) + Vec2::new(-deriv.y, deriv.x) * (d / deriv.hypot())
    }

    fn distance_to_polyline(p: Point, polyline: &[Point]) -> f64 {
        polyline
            .windows(2)
            .map(|w| {
                let ab = w[1] - w[0];
                let len2 = ab.hypot2();
                let t = if len2 > 0.0 {
                    ((p - w[0]).dot(ab) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (p - (w[0] + ab * t)).hypot()
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn curve_chords_meet_tolerance() {
        let mut rng = Rng(0x2544_0003);
        let (mut curves, mut arcs, mut linears) = (0, 0, 0);
        for _ in 0..400 {
            let p0 = rng.point();
            let c = random_cubic(&mut rng, p0);
            let raster = raster(&mut rng);
            let tol = FLATTEN / sigma_max(raster);
            let hw = rng.range(0.025, 5.0);
            let stroke = Stroke::new(2.0 * hw);
            let mut path = BezPath::new();
            path.move_to(c.p0);
            path.curve_to(c.p1, c.p2, c.p3);
            // No input returns the non-convergence error, the near-colinear
            // ones included.
            prepare(&shape(&path), &stroke, raster, tol, SURFACE).expect("the cubic encodes");
            if near_linear(c, tol).is_some() {
                linears += 1;
                continue;
            }
            let mut subs = Vec::new();
            let ctx = SubdivideCtx {
                hw,
                tol,
                dashed: false,
            };
            subdivide(regularize_cusp(c, tol * 0.25), ctx, 0, &mut subs).expect("converges");
            for sub in subs {
                if sub.psi.is_some() {
                    arcs += 1;
                    let diam = diameter(&sub.cubic);
                    assert!(diam <= tol / 2.0, "point arc of diameter {diam} at {tol}");
                    continue;
                }
                curves += 1;
                let n = sub.n_off as usize;
                assert!((1..=N_OFF_MAX as usize).contains(&n));
                for d in [-hw, hw] {
                    let chords: Vec<Point> = (0..=n)
                        .map(|k| offset_at(&sub.cubic, d, flt(k) / flt(n)))
                        .collect();
                    for j in 0..=16 * n {
                        let on_curve = offset_at(&sub.cubic, d, flt(j) / flt(16 * n));
                        let off = distance_to_polyline(on_curve, &chords);
                        // The bound is `tol` in exact arithmetic; the margin
                        // is the encoder's own f32 margin.
                        assert!(
                            off <= tol * (1.0 + VARIATION_MARGIN),
                            "offset curve {off} from its chords, tolerance {tol}"
                        );
                    }
                }
            }
        }
        assert!(
            curves > 100 && arcs > 0 && linears > 10,
            "{curves} curves, {arcs} point arcs, {linears} near-linear"
        );
    }

    /// The composite 8-point Gauss-Legendre rule on `intervals` equal
    /// sub-intervals, in f64.
    fn composite_arclen(c: &CubicBez, intervals: u32) -> f64 {
        let h = 1.0 / f64::from(intervals);
        let mut total = 0.0;
        for i in 0..intervals {
            let mid = (f64::from(i) + 0.5) * h;
            for &(w, x) in kurbo::common::GAUSS_LEGENDRE_COEFFS_8 {
                let speed = c.deriv().eval((0.5 * h).mul_add(x, mid)).to_vec2().hypot();
                total = (w * 0.5 * h).mul_add(speed, total);
            }
        }
        total
    }

    #[test]
    fn arc_length_rule_meets_its_bound() {
        let mut rng = Rng(0x2544_0004);
        let mut checked = 0;
        for _ in 0..400 {
            let p0 = rng.point();
            let c = random_cubic(&mut rng, p0);
            let raster = raster(&mut rng);
            let mut path = BezPath::new();
            path.move_to(c.p0);
            path.curve_to(c.p1, c.p2, c.p3);
            let mut stroke = Stroke::new(rng.range(0.05, 5.0)).with_dashes(0.0, [5.0, 2.0]);
            stroke.start_cap = Cap::Butt;
            let gpu = encode(&path, &stroke, raster);
            for r in gpu.strokes.iter().filter(|r| piece(r) == 1) {
                // The sub-cubic as the kernel sees it: its f32 control points.
                let at = |x: f32, y: f32| Point::new(f64::from(x), f64::from(y));
                let sub = CubicBez::new(
                    at(r.p[0], r.p[1]),
                    at(r.c[0], r.c[1]),
                    at(r.c[2], r.c[3]),
                    at(r.q[0], r.q[1]),
                );
                let n_len = r.aux[1];
                assert!((1..=N_LEN_MAX).contains(&n_len));
                let reference = sub.arclen(1e-12);
                let rule = composite_arclen(&sub, n_len);
                assert!(
                    (rule - reference).abs() <= 1e-9 * reference,
                    "rule {rule} against {reference} on {n_len} intervals"
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "{checked} curve pieces checked");
    }

    /// Positive-length dashes `kurbo::dash` produces: an independent model
    /// of the count, used for the bound only.
    fn kurbo_dash_count(path: &BezPath, offset: f64, pattern: &[f64]) -> usize {
        let dashed = BezPath::from_vec(kurbo::dash(path.iter(), offset, pattern).collect());
        let mut count = 0;
        let mut subpath = BezPath::new();
        let flush = |subpath: &mut BezPath, count: &mut usize| {
            let len: f64 = subpath
                .segments()
                .map(|s| kurbo::ParamCurveArclen::arclen(&s, 1e-9))
                .sum();
            if len > 1e-9 {
                *count += 1;
            }
            subpath.truncate(0);
        };
        for el in dashed.iter() {
            if matches!(el, PathEl::MoveTo(_)) {
                flush(&mut subpath, &mut count);
            }
            subpath.push(el);
        }
        flush(&mut subpath, &mut count);
        count
    }

    #[test]
    fn dash_bound_covers_every_subpath() {
        let mut rng = Rng(0x2544_0005);
        let mut compared = 0;
        for _ in 0..300 {
            // A pattern of whole numbers, so offsets can fall exactly on a
            // boundary. At least one on-entry is positive.
            let len = 1 + rng.below(5);
            let mut pattern: Vec<f64> = (0..len).map(|_| flt(rng.below(6))).collect();
            let doubled = |p: &[f64]| -> Vec<f64> {
                if p.len() % 2 == 1 {
                    p.iter().chain(p).copied().collect()
                } else {
                    p.to_vec()
                }
            };
            if !doubled(&pattern).iter().step_by(2).any(|x| *x > 0.0) {
                pattern[0] = 3.0;
            }
            let e = doubled(&pattern);
            let period: f64 = e.iter().sum();
            let offset = match rng.below(3) {
                0 => 0.0,
                1 => e.iter().take(rng.below(e.len() + 1)).sum(),
                _ => rng.range(-2.0 * period, 2.0 * period),
            };
            let path = if rng.below(2) == 0 {
                random_path(&mut rng)
            } else {
                // Integer polylines and rectangles, whose lengths the period
                // can divide.
                let mut p = BezPath::new();
                let n = 1 + rng.below(4);
                p.move_to((0.0, 0.0));
                for _ in 0..n {
                    let k = flt(1 + rng.below(8)) * period.max(1.0);
                    p.line_to(if rng.below(2) == 0 {
                        (k, 0.0)
                    } else {
                        (0.0, k)
                    });
                }
                if rng.below(2) == 0 {
                    p.close_path();
                }
                p
            };
            let mut stroke = Stroke::new(rng.range(0.1, 3.0)).with_dashes(offset, pattern.clone());
            stroke.join = Join::Miter;
            let gpu = encode(&path, &stroke, Affine::IDENTITY);
            let positive = e.iter().step_by(2).any(|x| *x > 0.0);
            assert!(positive);
            let table = gpu.dash.as_ref().expect("a dashed stroke has a table");
            for r in gpu.strokes.iter() {
                assert!(r.head[1] >= 1, "stage_cap is at least 1 per record");
            }
            assert_eq!(
                table.stage_total,
                gpu.strokes
                    .iter()
                    .map(|r| r.head[1] as usize)
                    .sum::<usize>()
            );
            assert!(table.fragments >= table.dashes);
            let reference = kurbo_dash_count(&path, offset, &pattern);
            assert!(
                reference <= table.dashes,
                "kurbo draws {reference} dashes against the bound {} for {pattern:?} at {offset}",
                table.dashes
            );
            compared += 1;
        }
        assert_eq!(compared, 300);
    }

    /// The merge rule of design section 6.6, stated from the pattern: a dash
    /// runs through the start of the cycle with positive remaining length.
    fn expected_merge(pattern: &[f64], offset: f64) -> bool {
        let e: Vec<f64> = if pattern.len() % 2 == 1 {
            pattern.iter().chain(pattern).copied().collect()
        } else {
            pattern.to_vec()
        };
        let period: f64 = e.iter().sum();
        let u = offset.rem_euclid(period);
        if u == 0.0 {
            return e[0] > 0.0;
        }
        let mut start = 0.0;
        for (j, entry) in e.iter().enumerate() {
            let end = start + entry;
            // Strictly inside an on-interval: at its start or end, kurbo's
            // remaining length is zero.
            if j % 2 == 0 && start < u && u < end {
                return true;
            }
            start = end;
        }
        false
    }

    #[test]
    fn merge_flag_follows_the_initial_state() {
        let mut square = BezPath::new();
        square.move_to((0.0, 0.0));
        square.line_to((40.0, 0.0));
        square.line_to((40.0, 40.0));
        square.line_to((0.0, 40.0));
        square.close_path();
        let patterns: [[f64; 4]; 3] = [
            [3.0, 2.0, 4.0, 1.0],
            [5.0, 0.0, 2.0, 3.0],
            [0.0, 4.0, 2.0, 2.0],
        ];
        let mut merged = 0;
        for [a, b, c, d] in patterns {
            let period = a + b + c + d;
            let mut offsets = vec![0.0, a, a + b, a + b + c, period, -a, period + a + b];
            for edge in [0.0, a, a + b, a + b + c, period] {
                offsets.extend([edge + 0.25, edge + 0.5, edge - 0.5]);
            }
            for offset in offsets {
                let stroke = Stroke::new(1.0).with_dashes(offset, [a, b, c, d]);
                let gpu = encode(&square, &stroke, Affine::IDENTITY);
                let table = gpu.dash.as_ref().expect("dashed");
                assert_eq!(
                    table.merge_start,
                    expected_merge(&[a, b, c, d], offset),
                    "[{a}, {b}, {c}, {d}] at offset {offset}"
                );
                merged += usize::from(table.merge_start);
            }
        }
        // An odd-length pattern is doubled.
        for offset in [0.0, 1.0, 5.0, 7.5, 9.0, 11.0] {
            let stroke = Stroke::new(1.0).with_dashes(offset, [5.0, 2.0, 1.0]);
            let gpu = encode(&square, &stroke, Affine::IDENTITY);
            let table = gpu.dash.as_ref().expect("dashed");
            assert_eq!(table.merge_start, expected_merge(&[5.0, 2.0, 1.0], offset));
            merged += usize::from(table.merge_start);
        }
        assert!(merged > 10, "the rule must be exercised both ways");
    }

    #[test]
    fn dash_rejects_fixed_point_overflow() {
        // A period just under the `2^31` limit: `dash_fragments`'s `u1`
        // and the next boundary `b` wrap the 64-bit fixed point once a
        // subpath's control-polygon length reaches `2^32 - P`, which the
        // `2^31` checks alone let through.
        let stroke = Stroke::new(1.0).with_dashes(0.0, [FIXED_LIMIT - 200.0, 100.0]);
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((200.0, 0.0));
        assert!(matches!(
            prepare(&shape(&path), &stroke, Affine::IDENTITY, FLATTEN, SURFACE),
            Err(RenderError::Unsupported(_))
        ));
        // Just below the sum the same stroke still encodes.
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 0.0));
        let gpu = encode(&path, &stroke, Affine::IDENTITY);
        assert!(gpu.dash.is_some());
    }

    #[test]
    fn degenerate_strokes_emit_nothing() {
        let line = {
            let mut p = BezPath::new();
            p.move_to((1.0, 1.0));
            p.line_to((50.0, 20.0));
            p
        };
        let mut lone_move = BezPath::new();
        lone_move.move_to((5.0, 5.0));
        let mut zero_length = BezPath::new();
        zero_length.move_to((5.0, 5.0));
        zero_length.line_to((5.0, 5.0));
        let mut zero_close = zero_length.clone();
        zero_close.close_path();
        let round = Stroke::new(4.0).with_caps(Cap::Round);
        let cases = [
            ("width 0", &line, Stroke::new(0.0)),
            ("lone move", &lone_move, round.clone()),
            ("zero-length subpath", &zero_length, round.clone()),
            ("zero-length closed subpath", &zero_close, round.clone()),
            ("NaN width", &line, Stroke::new(f64::NAN)),
            (
                "NaN miter limit",
                &line,
                round.clone().with_miter_limit(f64::NAN),
            ),
            (
                "NaN dash offset",
                &line,
                round.clone().with_dashes(f64::NAN, [3.0, 2.0]),
            ),
            (
                "NaN dash entry",
                &line,
                round.clone().with_dashes(0.0, [3.0, f64::NAN]),
            ),
        ];
        for (name, path, stroke) in cases {
            let gpu = encode(path, &stroke, Affine::IDENTITY);
            assert!(gpu.strokes.is_empty(), "{name}: records");
            assert!(gpu.elements.is_empty(), "{name}: elements");
            assert!(gpu.bound.empty, "{name}: bound");
            assert_eq!(gpu.bound.segments, 0, "{name}: segments");
            assert!(gpu.dash.is_none(), "{name}: dash table");
        }
        // A NaN point inside an otherwise finite path skips the elements
        // that touch it, as the fill encoder does; the rest still encodes
        // and the counts stay consistent with the bound.
        let mut holed = BezPath::new();
        holed.move_to((0.0, 0.0));
        holed.line_to((10.0, 0.0));
        holed.line_to((f64::NAN, 5.0));
        holed.line_to((20.0, 20.0));
        holed.line_to((30.0, 30.0));
        let gpu = encode(&holed, &round, Affine::IDENTITY);
        assert!(!gpu.strokes.is_empty() && !gpu.bound.empty);
        assert!(
            gpu.strokes
                .iter()
                .all(|r| { r.p.iter().chain(&r.q).chain(&r.c).all(|v| v.is_finite()) })
        );
        assert_eq!(
            gpu.strokes
                .iter()
                .map(|r| r.head[1] as usize)
                .sum::<usize>(),
            gpu.bound.segments
        );
        // The same strokes with something to draw do not collapse.
        let drawn = encode(&line, &Stroke::new(4.0), Affine::IDENTITY);
        assert!(!drawn.bound.empty && !drawn.strokes.is_empty());
    }
}
