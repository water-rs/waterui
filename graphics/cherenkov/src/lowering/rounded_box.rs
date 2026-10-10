//! The centred rounded box both backends evaluate a primitive shape as.
//!
//! Every primitive [`ShapeData`] with area is a box centred at the origin
//! with per-corner radii and a Lamé corner exponent, placed by a local
//! affine. The GPU packs the box into its instance data and evaluates its
//! signed distance in WGSL; the CPU evaluates the same distance in Rust,
//! [`RoundedBox::sdf_sample`]. Both read the box from [`box_form`], so the
//! two engines cannot drift apart in how a shape maps to it. The oracle
//! keeps its own independent `f64` mapping.

use kurbo::{Affine, RoundedRectRadii};

use crate::ShapeData;

/// A rounded box centred at the origin, in `f32` like the engines'
/// distance fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedBox {
    /// Half extents of the box.
    pub half: [f32; 2],
    /// Corner radius aspect: the y radius over the x radius.
    pub aspect: f32,
    /// The corner curve's Lamé exponent: `2.0` for a circular or
    /// elliptical corner, larger for a continuous corner.
    pub exponent: f32,
    /// Corner radii along x: top-left, top-right, bottom-right,
    /// bottom-left.
    pub radii: [f32; 4],
}

impl RoundedBox {
    /// A sharp box of half extents `half`.
    #[must_use]
    pub const fn rect(half: [f32; 2]) -> Self {
        Self {
            half,
            aspect: 1.0,
            exponent: 2.0,
            radii: [0.0; 4],
        }
    }
}

/// A shape's analytic form as a centred rounded box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxForm {
    /// The shape is `shape` placed by the local affine `extra` (the
    /// centre translation, plus the rotation of an ellipse).
    Box {
        /// Local placement of the centred box.
        extra: Affine,
        /// The centred box.
        shape: RoundedBox,
    },
    /// The shape has no area: a line, or a circle or ellipse with a zero
    /// radius.
    Empty,
    /// A path, which has no analytic box.
    Path,
}

/// Whether `shape` encloses nothing: a line, or a circle or ellipse whose
/// radius is not positive. Filled it draws nothing, and as a clip it cuts
/// its whole subtree away, so every engine skips that subtree and the
/// plane planner never promotes a layer inside it.
#[must_use]
pub fn encloses_nothing(shape: &ShapeData) -> bool {
    match shape {
        ShapeData::Line(_) => true,
        ShapeData::Circle(c) => c.radius <= 0.0,
        ShapeData::Ellipse(e) => {
            let radii = e.radii();
            radii.x <= 0.0 || radii.y <= 0.0
        }
        ShapeData::Rect(_)
        | ShapeData::RoundedRect(_)
        | ShapeData::Continuous(_)
        | ShapeData::Path { .. } => false,
    }
}

/// `shape` as a centred rounded box.
///
/// Corner radii clamp to the smaller half extent, a continuous corner's
/// smoothing maps to its Lamé exponent `2 + 2·smoothing`, and an ellipse
/// is a box whose corners are its quarter arcs.
#[must_use]
pub fn box_form(shape: &ShapeData) -> BoxForm {
    if encloses_nothing(shape) {
        return BoxForm::Empty;
    }
    let (extra, shape) = match shape {
        ShapeData::Rect(r) => (
            Affine::translate(r.center().to_vec2()),
            RoundedBox::rect([f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)]),
        ),
        ShapeData::RoundedRect(rr) => {
            let r = rr.rect();
            let half = [f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)];
            (
                Affine::translate(r.center().to_vec2()),
                RoundedBox {
                    half,
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: clamped_radii(rr.radii(), half),
                },
            )
        }
        ShapeData::Continuous(c) => {
            let r = c.rect;
            let half = [f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)];
            (
                Affine::translate(r.center().to_vec2()),
                RoundedBox {
                    half,
                    aspect: 1.0,
                    exponent: f32_f64(c.smoothing).clamp(0.0, 1.0).mul_add(2.0, 2.0),
                    radii: clamped_radii(c.radii, half),
                },
            )
        }
        ShapeData::Circle(c) => {
            let r = f32_f64(c.radius);
            (
                Affine::translate(c.center.to_vec2()),
                RoundedBox {
                    half: [r, r],
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: [r; 4],
                },
            )
        }
        ShapeData::Ellipse(e) => {
            let radii = e.radii();
            let (a, b) = (radii.x, radii.y);
            (
                Affine::translate(e.center().to_vec2()) * Affine::rotate(e.rotation()),
                RoundedBox {
                    half: [f32_f64(a), f32_f64(b)],
                    aspect: f32_f64(b / a),
                    exponent: 2.0,
                    radii: [f32_f64(a); 4],
                },
            )
        }
        ShapeData::Line(_) => unreachable!("a line encloses nothing"),
        ShapeData::Path { .. } => return BoxForm::Path,
    };
    BoxForm::Box { extra, shape }
}

impl RoundedBox {
    /// The signed distance and unit outward normal at the box-local point
    /// `(x, y)` — the shader's `sdf_sample` in `f32`, less the radius of
    /// curvature at the foot, which no CPU SDF read uses. A sharp corner's
    /// axis-aligned distance, a circular corner's closed form, or a Lamé
    /// corner's nearest-point solve.
    #[must_use]
    pub fn sdf_sample(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let (sx, sy) = (
            if x >= 0.0 { 1.0 } else { -1.0 },
            if y >= 0.0 { 1.0 } else { -1.0 },
        );
        let radius = match (x > 0.0, y > 0.0) {
            (false, false) => self.radii[0],
            (true, false) => self.radii[1],
            (true, true) => self.radii[2],
            (false, true) => self.radii[3],
        };
        let rx = radius.max(0.0);
        let ry = rx * self.aspect;
        let (ax, ay) = (x.abs() - self.half[0], y.abs() - self.half[1]);
        if rx <= 0.0 || ry <= 0.0 {
            let d = length(ax.max(0.0), ay.max(0.0)) + ax.max(ay).min(0.0);
            let (gx, gy) = if ax > 0.0 && ay > 0.0 {
                let l = length(ax, ay);
                (ax / l, ay / l)
            } else if ax > ay {
                (1.0, 0.0)
            } else {
                (0.0, 1.0)
            };
            return (d, sx * gx, sy * gy);
        }
        let (qx, qy) = (ax + rx, ay + ry);
        if (self.exponent - 2.0).abs() < 1e-4 && (self.aspect - 1.0).abs() < 1e-4 {
            if qx > 0.0 && qy > 0.0 {
                let (ux, uy) = (qx / rx, qy / ry);
                let len = length(ux, uy);
                let grad = length(ux / rx, uy / ry) / len.max(1e-6);
                let d = (len - 1.0) / grad.max(1e-6);
                let (vx, vy) = (qx / (rx * rx), qy / (ry * ry));
                let vlen = length(vx, vy).max(1e-12);
                return (d, sx * vx / vlen, sy * vy / vlen);
            }
        } else if qx >= 0.0 && qy >= 0.0 {
            // A Lamé corner owns the lines through its centre too: on an
            // ellipse's major axis, a point deeper than the vertex's radius
            // of curvature is nearest to the curve on either side of the
            // axis, not to the vertex the straight-edge distance measures.
            let (d, nx, ny) = lame_corner((qx, qy), (rx, ry), self.exponent);
            return (d, sx * nx, sy * ny);
        }
        let (gx, gy) = if ax > ay { (1.0, 0.0) } else { (0.0, 1.0) };
        (ax.max(ay), sx * gx, sy * gy)
    }
}

/// Distance evaluations `lame_corner` spends on one point.
const LAME_EVALS: usize = 6;
/// `QUARTER_PI` in `shared.wgsl`.
const QUARTER_PI: f32 = std::f32::consts::FRAC_PI_4;
/// `SQRT_HALF` in `shared.wgsl`.
const SQRT_HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The Euclidean length of `(x, y)` — WGSL's `length`.
#[expect(clippy::imprecise_flops, reason = "the WGSL `length` verbatim")]
fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// WGSL's `sign`: zero at zero, unlike `f32::signum`.
fn sign(x: f32) -> f32 {
    if x == 0.0 { 0.0 } else { x.signum() }
}

#[expect(clippy::imprecise_flops, reason = "the WGSL `cbrt` verbatim")]
fn cbrt(x: f32) -> f32 {
    sign(x) * x.abs().max(1e-30).powf(1.0 / 3.0)
}

/// The largest real root of `c3·x³ + c1·x + c0 = 0` with `c3 > 0`:
/// Cardano's form where the cubic has one real root, the trigonometric
/// form where it has three.
#[expect(
    clippy::manual_clamp,
    clippy::suboptimal_flops,
    reason = "the WGSL `cubic_root` verbatim: WGSL `clamp` is `min(max(·, lo), hi)`"
)]
fn cubic_root(c3: f32, c1: f32, c0: f32) -> f32 {
    let p = c1 / c3;
    let h = 0.5 * c0 / c3;
    let disc = h * h + p * p * p / 27.0;
    if disc >= 0.0 {
        let e = disc.sqrt();
        return cbrt(e - h) + cbrt(-e - h);
    }
    let m = (-p / 3.0).sqrt();
    2.0 * m * ((-h / (m * m * m)).max(-1.0).min(1.0).acos() / 3.0).cos()
}

/// A point on the Lamé quarter curve `(x/r.x)ⁿ + (y/r.y)ⁿ = 1` at the
/// parameter `w = (y/r.y) / (x/r.x)`, `w` in `[0, 1]` from the x axis to
/// the diagonal of the normalised curve: `c = r·(1, w)·s` with
/// `s = (1 + wⁿ)^(−1/n)`.
struct LamePoint {
    /// The curve point.
    c: (f32, f32),
    /// `(r.y, r.x·w^(n-1))`: the outward normal, unnormalised.
    normal: (f32, f32),
    /// `w^(n-2)`.
    wm2: f32,
    /// `(1 + wⁿ)^(−1/n)`.
    s: f32,
    /// `1 + wⁿ`.
    one: f32,
}

#[expect(
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    reason = "the WGSL `lame_point` verbatim"
)]
fn lame_point(r: (f32, f32), n: f32, w: f32) -> LamePoint {
    // `w^(n-2)` is 1 at `w = 0` for an ellipse (`n = 2`) and 0 for `n > 2`.
    let wm2 = w.max(1e-30).powf(n - 2.0);
    let p = w * wm2;
    let one = 1.0 + w * p;
    let s = one.powf(-1.0 / n);
    LamePoint {
        c: (r.0 * s, r.1 * w * s),
        normal: (r.1, r.0 * p),
        wm2,
        s,
        one,
    }
}

/// Where the solve of an elliptical corner starts: its `w = 0` end is a
/// vertex of the ellipse. There the foot condition is
/// `F(w) = r.y·q.y - w·(r.x·q.x - Δ/√(1 + w²))`, `Δ = r.x² - r.y²`. Across
/// a major vertex (`Δ > 0`) `-F` is convex and its Taylor cubic
/// `(Δ/2)w³ + (r.x·q.x - Δ)w - r.y·q.y` bounds it from above, so the
/// cubic's largest root lies just below the foot, with an error of order
/// `w⁵`: the evolute cusp, where the foot is a near-triple root that
/// Newton's method approaches only linearly, sits at small `w`. Across a
/// minor vertex `-F` is concave with a slope bounded away from zero and
/// the linear term is the start.
#[expect(
    clippy::manual_clamp,
    clippy::suboptimal_flops,
    reason = "the WGSL `ellipse_start` verbatim: WGSL `clamp` is `min(max(·, lo), hi)`"
)]
fn ellipse_start(q: (f32, f32), r: (f32, f32)) -> f32 {
    let delta = r.0 * r.0 - r.1 * r.1;
    let c1 = r.0 * q.0 - delta;
    if delta <= 0.0 {
        return (r.1 * q.1 / c1).max(0.0).min(1.0);
    }
    cubic_root(0.5 * delta, c1, -r.1 * q.1).max(0.0).min(1.0)
}

/// Where the solve of a continuous corner (`r.x = r.y = r`) starts: its
/// `w = 1` end is the corner's diagonal, a vertex whose evolute cusp lies
/// on the diagonal. The corner's support function about the diagonal
/// normal is `h(π/4 + ψ) = h0·(1 + K2·ψ² + K4·ψ⁴)` with `p = n/(n - 1)`
/// the dual exponent, so the stationarity of `q·m - h(ψ)` is, to third
/// order, `B - (A - h0 + ρ0)·ψ - ((h4 - A)/6)·ψ³ = 0` with `A`, `B` the
/// components of `q` along and across the diagonal,
/// `ρ0 = h0·(1 + 2·K2)` the radius of curvature and `h4 = 24·h0·K4`. Its
/// largest root is the normal angle `ψ` away from the diagonal, and
/// `w = tan(π/4 - ψ)^(1/(n - 1))`.
#[expect(
    clippy::manual_clamp,
    clippy::suboptimal_flops,
    reason = "the WGSL `diagonal_start` verbatim: WGSL `clamp` is `min(max(·, lo), hi)`"
)]
fn diagonal_start(q: (f32, f32), r: f32, n: f32) -> f32 {
    let p = n / (n - 1.0);
    let c2 = 0.5 * p * (p - 1.0);
    let c4 = p * (p - 1.0) * (p - 2.0) * (p - 3.0) / 24.0;
    let k2 = c2 / p - 0.5;
    let k4 =
        (2.0 * c2 / 3.0 + c4) / p + 0.5 * (1.0 / p - 1.0) * c2 * c2 / p - 0.5 * c2 / p + 1.0 / 24.0;
    let h0 = r * f32::exp2(1.0 / p - 0.5);
    let along = (q.0 + q.1) * SQRT_HALF;
    let across = (q.0 - q.1) * SQRT_HALF;
    let k3 = (24.0 * h0 * k4 - along) / 6.0;
    let k1 = along - h0 + h0 * (1.0 + 2.0 * k2);
    let mut psi = QUARTER_PI;
    if k3 > 0.0 {
        psi = cubic_root(k3, k1, -across);
    } else if k1 > 0.0 {
        psi = across / k1;
    }
    let theta = (QUARTER_PI - psi).max(0.0).min(QUARTER_PI);
    theta
        .tan()
        .max(1e-30)
        .powf(1.0 / (n - 1.0))
        .max(0.0)
        .min(1.0)
}

/// Exact signed distance from `q` (corner-local, both components >= 0, in
/// the units of `r`) to the Lamé quarter curve `(q.x/r.x)ⁿ + (q.y/r.y)ⁿ = 1`,
/// for an elliptical corner (`n = 2`) or a continuous one (`r.x = r.y`,
/// `n > 2`). Returns `(d, unit normal in q space)` — the shader's
/// `lame_corner` in `f32`, less the radius of curvature at the foot.
///
/// The foot is the root of `F(w) = (q - c)·T`, `T` the tangent. Folding
/// the quarter at `w = 1` puts it in `[0, 1]` with `F(0) >= 0 >= F(1)`:
/// an ellipse quarter runs between two vertices, and a continuous corner
/// is symmetric about its diagonal, so the half that holds `q`'s foot has
/// monotone curvature and `F` changes sign on it once. The solve starts
/// from a closed-form model of `F` about the half's vertex, where the
/// evolute cusp makes the foot a near-triple root (`ellipse_start`,
/// `diagonal_start`), then takes Newton steps clamped to the sign
/// bracket, bisecting where `F` is not falling. The box is convex, so the
/// tangent at every curve point supports it: the result, the distance to
/// the tangent at the last iterate, never exceeds the box's signed
/// distance and equals it at the foot.
#[expect(
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    clippy::suspicious_operation_groupings,
    reason = "the WGSL `lame_corner` verbatim, names included"
)]
fn lame_corner(q_in: (f32, f32), r_in: (f32, f32), n: f32) -> (f32, f32, f32) {
    let k = f32::exp2(-1.0 / n);
    let swap = (q_in.0 - r_in.0 * k) * (-r_in.0) + (q_in.1 - r_in.1 * k) * r_in.1 > 0.0;
    let q = if swap { (q_in.1, q_in.0) } else { q_in };
    let r = if swap { (r_in.1, r_in.0) } else { r_in };
    let mut w = if n == 2.0 {
        ellipse_start(q, r)
    } else {
        diagonal_start(q, r.0, n)
    };
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    let mut foot = (0.0_f32, 0.0_f32, 0.0_f32);
    for _ in 0..LAME_EVALS {
        let pt = lame_point(r, n, w);
        let len = length(pt.normal.0, pt.normal.1);
        let normal = (pt.normal.0 / len, pt.normal.1 / len);
        let qc = (q.0 - pt.c.0, q.1 - pt.c.1);
        foot = (qc.0 * normal.0 + qc.1 * normal.1, normal.0, normal.1);
        let t = (-pt.normal.1, pt.normal.0);
        let f = qc.0 * t.0 + qc.1 * t.1;
        let df = -pt.s / pt.one * (t.0 * t.0 + t.1 * t.1) - qc.0 * r.0 * (n - 1.0) * pt.wm2;
        if f > 0.0 {
            lo = w;
        } else {
            hi = w;
        }
        w = if df < 0.0 {
            (w - f / df).max(lo).min(hi)
        } else {
            0.5 * (lo + hi)
        };
    }
    let (nx, ny) = if swap {
        (foot.2, foot.1)
    } else {
        (foot.1, foot.2)
    };
    (foot.0, nx, ny)
}

/// Corner radii clamped to `[0, min(half)]`.
fn clamped_radii(radii: RoundedRectRadii, half: [f32; 2]) -> [f32; 4] {
    let limit = f64::from(half[0].min(half[1]));
    [
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ]
    .map(|r| f32_f64(r.clamp(0.0, limit)))
}

/// `f64` to `f32`: the box is the engines' `f32` distance-field input.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the engines evaluate distance fields in f32"
)]
const fn f32_f64(v: f64) -> f32 {
    v as f32
}
