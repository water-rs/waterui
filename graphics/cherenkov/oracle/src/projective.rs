//! Projective layers (#84), independently of the engine.
//!
//! A projective layer is flattened: its content, clip, filter and children
//! render into a layer-local image at `ρ` texels per layer unit, the image
//! gets an area-average mip chain, and each destination pixel reconstructs
//! the image through the anisotropic filter. Everything here is `f64`.
//!
//! The model, restated from the plan of record:
//!
//! - **Pose.** `M = embed(B) · T(p) · P · T(0, 0, z) · Ry(v) · Rx(u) ·
//!   T(−p)` on column vectors; the plane homography is `M`'s rows and
//!   columns 0, 1 and 3. A point is visible where `W > 0`.
//! - **Visible domain.** The clip bounds, clipped in homogeneous
//!   coordinates against `W > 0` and the destination viewport widened by
//!   the two-pixel footprint, before any division.
//! - **Density.** `ρ = 2^k` for the least `k ≥ 0` with `2^k` at least an
//!   upper bound of `σmax` of the local-to-destination Jacobian over the
//!   visible domain. The bound is interval arithmetic on the Jacobian's
//!   entries (midpoint singular value plus the Frobenius norm of the
//!   radius), refined by halving the worst leaf's longer side until the
//!   bound and the leaf-centroid sample agree on `k`, or 64 leaves were
//!   evaluated. A constant denominator takes the exact singular value.
//! - **Local grid.** Texel `(i, j)` covers layer units
//!   `[(x0 + i) / ρ, (x0 + i + 1) / ρ)` with `x0 = floor(bounds.x0 · ρ)`,
//!   and the image extends to `ceil(bounds.x1 · ρ)`.
//! - **Mips.** Each level is `max(1, floor(previous / 2))` per axis over the same
//!   extent; texels are area-overlap averages of the previous level.
//! - **Reconstruction.** With the inverse map's Jacobian singular values
//!   `a ≥ b` (base texels per destination pixel) and major direction `e`:
//!   `b_eff = max(1, b, a / 16)`, level of detail `log2(b_eff)` blended
//!   linearly between the two nearest levels, and `N = clamp(ceil(a /
//!   b_eff − 1/256), 1, 16)` equal-weight bilinear taps at the centres of `N`
//!   equal parts of the length-`a` major axis. Texels outside a level are
//!   transparent.

use cherenkov_scene::{Projection, Shape};
use kurbo::{Affine, Rect, Shape as _};

use crate::clip::Segment;
use crate::path::Polyline;

/// Destination pixels a projected sample reaches beyond the projected
/// domain.
const FOOTPRINT: f64 = 2.0;
/// The most leaf regions the density bound evaluates.
const LEAVES: usize = 64;
/// The most taps along the major axis.
const TAPS: f64 = 16.0;
/// How far `a / b_eff` may exceed an integer and still take that many
/// taps: rounding must not turn an isotropic footprint into two taps.
const TAP_SLACK: f64 = 1.0 / 256.0;
/// The largest density exponent considered.
const MAX_EXPONENT: i32 = 20;

type M3 = [[f64; 3]; 3];
type M4 = [[f64; 4]; 4];

fn mul4(a: &M4, b: &M4) -> M4 {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..4).fold(0.0, |acc, k| a[i][k].mul_add(b[k][j], acc));
        }
    }
    out
}

fn mul3(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).fold(0.0, |acc, k| a[i][k].mul_add(b[k][j], acc));
        }
    }
    out
}

fn apply3(m: &M3, p: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[2].mul_add(p[2], r[0].mul_add(p[0], r[1] * p[1])))
}

/// `a + t·(b − a)`.
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    t.mul_add(b - a, a)
}

/// `a·b − c·d`.
fn cross(a: f64, b: f64, c: f64, d: f64) -> f64 {
    a.mul_add(b, -(c * d))
}

const fn affine3(t: Affine) -> M3 {
    let [xx, yx, xy, yy, x, y] = t.as_coeffs();
    [[xx, xy, x], [yx, yy, y], [0.0, 0.0, 1.0]]
}

fn det3(m: &M3) -> f64 {
    m[0][2].mul_add(
        cross(m[1][0], m[2][1], m[1][1], m[2][0]),
        m[0][0].mul_add(
            cross(m[1][1], m[2][2], m[1][2], m[2][1]),
            -(m[0][1] * cross(m[1][0], m[2][2], m[1][2], m[2][0])),
        ),
    )
}

/// The inverse of `m` by cofactors; `None` when singular.
fn inverse3(m: &M3) -> Option<M3> {
    let det = det3(m);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let c = |r0: usize, r1: usize, c0: usize, c1: usize| {
        cross(m[r0][c0], m[r1][c1], m[r0][c1], m[r1][c0])
    };
    Some([
        [
            c(1, 2, 1, 2) / det,
            -c(0, 2, 1, 2) / det,
            c(0, 1, 1, 2) / det,
        ],
        [
            -c(1, 2, 0, 2) / det,
            c(0, 2, 0, 2) / det,
            -c(0, 1, 0, 2) / det,
        ],
        [
            c(1, 2, 0, 1) / det,
            -c(0, 2, 0, 1) / det,
            c(0, 1, 0, 1) / det,
        ],
    ])
}

/// The determinant of a 4×4 matrix by Laplace expansion along row 0.
fn det4(m: &M4) -> f64 {
    (0..4)
        .map(|j| {
            let minor: M3 = std::array::from_fn(|r| {
                let cols: Vec<usize> = (0..4).filter(|&c| c != j).collect();
                std::array::from_fn(|c| m[r + 1][cols[c]])
            });
            let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
            sign * m[0][j] * det3(&minor)
        })
        .sum()
}

/// The layer's pose: `embed(base) · T(p) · P · T(0, 0, z) · Ry(v) · Rx(u)
/// · T(−p)`.
///
/// # Errors
/// A message when a coefficient is not finite or the pose is singular.
pub fn pose(projection: &Projection, base: Affine) -> Result<M4, String> {
    let [xx, yx, xy, yy, x0, y0] = base.as_coeffs();
    let translate = |x: f64, y: f64, z: f64| -> M4 {
        [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, y],
            [0.0, 0.0, 1.0, z],
            [0.0, 0.0, 0.0, 1.0],
        ]
    };
    let (sy, cy) = projection.tilt.y.sin_cos();
    let (sx, cx) = projection.tilt.x.sin_cos();
    let ry: M4 = [
        [cy, 0.0, sy, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [-sy, 0.0, cy, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let rx: M4 = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, cx, -sx, 0.0],
        [0.0, sx, cx, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let embed: M4 = [
        [xx, xy, 0.0, x0],
        [yx, yy, 0.0, y0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let pivot = projection.pivot;
    let chain = [
        embed,
        translate(pivot.x, pivot.y, 0.0),
        projection.matrix,
        translate(0.0, 0.0, projection.depth),
        ry,
        rx,
        translate(-pivot.x, -pivot.y, 0.0),
    ];
    let matrix = chain[1..]
        .iter()
        .fold(chain[0], |acc, next| mul4(&acc, next));
    if !matrix.iter().flatten().all(|v| v.is_finite()) {
        return Err("projective pose has a non-finite coefficient".into());
    }
    if det4(&matrix) == 0.0 {
        return Err("projective pose is not invertible".into());
    }
    Ok(matrix)
}

/// The homography the pose induces on the `z = 0` plane, followed by the
/// parent's affine map into its raster.
#[must_use]
pub fn to_raster(pose: &M4, raster: Affine) -> M3 {
    let plane = [
        [pose[0][0], pose[0][1], pose[0][3]],
        [pose[1][0], pose[1][1], pose[1][3]],
        [pose[3][0], pose[3][1], pose[3][3]],
    ];
    mul3(&affine3(raster), &plane)
}

/// The smallest rectangle containing `shape`'s area.
///
/// It is the layer's local source domain: exact for every analytic shape, never the bounds of a
/// curve approximation, which can overshoot by a rounding error and move
/// the texel grid by a whole texel.
#[must_use]
pub fn domain(shape: &Shape) -> Rect {
    match shape {
        Shape::Rect(rect) => *rect,
        Shape::RoundedRect(rounded) => rounded.rect(),
        Shape::Continuous(continuous) => continuous.rect,
        Shape::Circle(circle) => circle.bounding_box(),
        Shape::Ellipse(ellipse) => ellipse.bounding_box(),
        Shape::Line(line) => line.bounding_box(),
        Shape::Path { path } => path.bounding_box(),
    }
}

/// A projective layer's local image placement for one render.
#[derive(Debug)]
pub struct Placement {
    /// Layer space to base texels.
    pub local_to_texel: Affine,
    /// Base size in texels.
    pub width: usize,
    /// Base size in texels.
    pub height: usize,
    /// Base texels to parent raster, homogeneous.
    pub forward: M3,
    /// Parent raster to base texels, homogeneous: `inverse · (d, 1)` has
    /// the sign of the forward `W`.
    pub inverse: M3,
}

/// The visible part of the local polygon `poly` under `h`: clipped in
/// homogeneous coordinates against `W > 0` and `view`, returned as local
/// points with their positive-`W` homogeneous images.
fn clip_visible(h: &M3, poly: &[[f64; 2]], view: Rect) -> Vec<([f64; 2], [f64; 3])> {
    let mut verts: Vec<([f64; 2], [f64; 3])> = poly
        .iter()
        .map(|&p| (p, apply3(h, [p[0], p[1], 1.0])))
        .collect();
    let planes: [[f64; 3]; 5] = [
        [0.0, 0.0, 1.0],
        [1.0, 0.0, -view.x0],
        [-1.0, 0.0, view.x1],
        [0.0, 1.0, -view.y0],
        [0.0, -1.0, view.y1],
    ];
    for plane in planes {
        let side = |q: &[f64; 3]| plane[2].mul_add(q[2], plane[0].mul_add(q[0], plane[1] * q[1]));
        let mut next = Vec::new();
        for i in 0..verts.len() {
            let (pa, qa) = verts[i];
            let (pb, qb) = verts[(i + 1) % verts.len()];
            let (sa, sb) = (side(&qa), side(&qb));
            if sa >= 0.0 {
                next.push((pa, qa));
            }
            if (sa >= 0.0) != (sb >= 0.0) {
                let t = sa / (sa - sb);
                next.push((
                    [lerp(t, pa[0], pb[0]), lerp(t, pa[1], pb[1])],
                    [
                        lerp(t, qa[0], qb[0]),
                        lerp(t, qa[1], qb[1]),
                        lerp(t, qa[2], qb[2]),
                    ],
                ));
            }
        }
        verts = next;
        if verts.is_empty() {
            break;
        }
    }
    verts.retain(|(_, q)| q[2] > 0.0);
    verts
}

/// `σmax` of the row-major 2×2 `m`.
fn largest_singular(m: [f64; 4]) -> f64 {
    let squares = m.iter().fold(0.0, |acc, v| v.mul_add(*v, acc));
    let det = cross(m[0], m[3], m[1], m[2]);
    let disc = squares.mul_add(squares, -4.0 * det * det).max(0.0);
    f64::midpoint(squares, disc.sqrt()).sqrt()
}

/// `h`'s row `row` applied to the local point `p`.
const fn row_at(h: &M3, row: usize, p: [f64; 2]) -> f64 {
    h[row][0].mul_add(p[0], h[row][1].mul_add(p[1], h[row][2]))
}

/// The Jacobian of `(X/W, Y/W)` for `(X, Y, W) = h · (x, y, 1)` at `p`.
fn jacobian(h: &M3, p: [f64; 2]) -> [f64; 4] {
    let q = apply3(h, [p[0], p[1], 1.0]);
    let w2 = q[2] * q[2];
    [
        cross(h[0][0], q[2], q[0], h[2][0]) / w2,
        cross(h[0][1], q[2], q[0], h[2][1]) / w2,
        cross(h[1][0], q[2], q[1], h[2][0]) / w2,
        cross(h[1][1], q[2], q[1], h[2][1]) / w2,
    ]
}

/// Upper bound and centroid sample of `σmax(J)` over the convex local
/// polygon `poly`, on which `W > 0`.
fn bounds_over(h: &M3, poly: &[[f64; 2]]) -> (f64, f64) {
    let extent = |f: &dyn Fn([f64; 2]) -> f64| {
        let values: Vec<f64> = poly.iter().map(|&p| f(p)).collect();
        (
            values.iter().copied().fold(f64::INFINITY, f64::min),
            values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        )
    };
    let (w_min, w_max) = extent(&|p| row_at(h, 2, p));
    let (inv_lo, inv_hi) = (1.0 / (w_max * w_max), 1.0 / (w_min * w_min));
    let mut mid = [0.0; 4];
    let mut radius2 = 0.0;
    for (k, (i, j)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
        // H_ij · W − X_i · H_2j: affine in (x, y).
        let numerator = |p: [f64; 2]| cross(h[i][j], row_at(h, 2, p), row_at(h, i, p), h[2][j]);
        let (lo, hi) = extent(&numerator);
        let corners = [lo * inv_lo, lo * inv_hi, hi * inv_lo, hi * inv_hi];
        let e_lo = corners.iter().copied().fold(f64::INFINITY, f64::min);
        let e_hi = corners.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        mid[k] = f64::midpoint(e_lo, e_hi);
        let radius = (e_hi - e_lo) / 2.0;
        radius2 = radius.mul_add(radius, radius2);
    }
    #[expect(clippy::cast_precision_loss, reason = "few vertices")]
    let n = poly.len() as f64;
    let centroid = poly
        .iter()
        .fold([0.0, 0.0], |c, p| [c[0] + p[0] / n, c[1] + p[1] / n]);
    (
        largest_singular(mid) + radius2.sqrt(),
        largest_singular(jacobian(h, centroid)),
    )
}

/// The least `k ≥ 0` with `2^k ≥ s`, or `None` past the largest density.
fn exponent(s: f64) -> Option<i32> {
    (0..=MAX_EXPONENT).find(|&k| 2f64.powi(k) >= s)
}

/// Clips the convex polygon `poly` to `r`.
fn within(poly: &[[f64; 2]], r: Rect) -> Vec<[f64; 2]> {
    let mut out = poly.to_vec();
    let edges: [(usize, f64, f64); 4] = [
        (0, r.x0, 1.0),
        (0, r.x1, -1.0),
        (1, r.y0, 1.0),
        (1, r.y1, -1.0),
    ];
    for (axis, at, dir) in edges {
        let mut next = Vec::new();
        for i in 0..out.len() {
            let (a, b) = (out[i], out[(i + 1) % out.len()]);
            let (da, db) = ((a[axis] - at) * dir, (b[axis] - at) * dir);
            if da >= 0.0 {
                next.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = (at - a[axis]) / (b[axis] - a[axis]);
                next.push([lerp(t, a[0], b[0]), lerp(t, a[1], b[1])]);
            }
        }
        out = next;
        if out.is_empty() {
            break;
        }
    }
    out
}

/// The density exponent for the visible local polygon.
fn density_exponent(h: &M3, poly: &[[f64; 2]]) -> Option<i32> {
    if h[2][0] == 0.0 && h[2][1] == 0.0 {
        let w = h[2][2];
        return exponent(largest_singular([
            h[0][0] / w,
            h[0][1] / w,
            h[1][0] / w,
            h[1][1] / w,
        ]));
    }
    let mut bbox = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in poly {
        bbox = bbox.union_pt(kurbo::Point::new(p[0], p[1]));
    }
    let mut leaves: Vec<(Rect, f64, f64)> = Vec::new();
    let mut evaluated = 0;
    let add = |r: Rect, leaves: &mut Vec<(Rect, f64, f64)>, evaluated: &mut usize| {
        let part = within(poly, r);
        if part.len() >= 3 {
            *evaluated += 1;
            let (upper, lower) = bounds_over(h, &part);
            leaves.push((r, upper, lower));
        }
    };
    add(bbox, &mut leaves, &mut evaluated);
    loop {
        let upper = leaves.iter().map(|l| l.1).fold(0.0, f64::max);
        let lower = leaves.iter().map(|l| l.2).fold(0.0, f64::max);
        if !upper.is_finite() {
            return None;
        }
        let k = exponent(upper)?;
        if exponent(lower)? == k || evaluated >= LEAVES {
            return Some(k);
        }
        let worst = (0..leaves.len()).max_by(|&a, &b| leaves[a].1.total_cmp(&leaves[b].1))?;
        let (r, ..) = leaves.swap_remove(worst);
        let halves = if r.width() >= r.height() {
            let x = f64::midpoint(r.x0, r.x1);
            [
                Rect::new(r.x0, r.y0, x, r.y1),
                Rect::new(x, r.y0, r.x1, r.y1),
            ]
        } else {
            let y = f64::midpoint(r.y0, r.y1);
            [
                Rect::new(r.x0, r.y0, r.x1, y),
                Rect::new(r.x0, y, r.x1, r.y1),
            ]
        };
        add(halves[0], &mut leaves, &mut evaluated);
        add(halves[1], &mut leaves, &mut evaluated);
    }
}

/// How the oracle reconstructs a projected local image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reconstruction {
    /// The specified model: the conservative density, the mip chain and
    /// the 16-tap anisotropic filter — the conformance reference.
    #[default]
    Model,
    /// The quality reference: the local source at `refine` times the
    /// model density, and every destination pixel the box average of a
    /// `grid × grid` lattice of bilinear base-level samples of its
    /// preimage. Refining both until the image stops changing gives the
    /// converged projection a model can be judged against.
    Supersampled {
        /// The density multiple, a power of two.
        refine: u32,
        /// Samples per pixel axis.
        grid: u32,
    },
}

impl Reconstruction {
    /// The factor the model density is multiplied by.
    #[must_use]
    pub fn refine(self) -> f64 {
        match self {
            Self::Model => 1.0,
            Self::Supersampled { refine, .. } => f64::from(refine),
        }
    }
}

/// Places the local image of a layer's `domain` under `h`.
///
/// `h` maps the layer plane into a `raster`-pixel parent; the density is
/// the model's times `refine`. `None` when the layer contributes nothing
/// (no area, edge-on, behind the viewer, or off the raster).
///
/// # Errors
/// A message when no finite density bound exists.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "texel extents of corpus layers are small and non-negative"
)]
pub fn place(
    h: &M3,
    domain: Rect,
    raster: (usize, usize),
    refine: f64,
) -> Result<Option<Placement>, String> {
    if !(domain.width() > 0.0 && domain.height() > 0.0) || det3(h) == 0.0 {
        return Ok(None);
    }
    #[expect(clippy::cast_precision_loss, reason = "raster sizes are small")]
    let view = Rect::new(0.0, 0.0, raster.0 as f64, raster.1 as f64).inflate(FOOTPRINT, FOOTPRINT);
    let corners = [
        [domain.x0, domain.y0],
        [domain.x1, domain.y0],
        [domain.x1, domain.y1],
        [domain.x0, domain.y1],
    ];
    let visible = clip_visible(h, &corners, view);
    if visible.len() < 3 {
        return Ok(None);
    }
    let local: Vec<[f64; 2]> = visible.iter().map(|v| v.0).collect();
    let k = density_exponent(h, &local).ok_or("no finite raster density bound")?;
    let rho = 2f64.powi(k) * refine;
    let (x0, y0) = ((domain.x0 * rho).floor(), (domain.y0 * rho).floor());
    let (x1, y1) = ((domain.x1 * rho).ceil(), (domain.y1 * rho).ceil());
    let local_to_texel = Affine::new([rho, 0.0, 0.0, rho, -x0, -y0]);
    let forward = mul3(h, &affine3(local_to_texel.inverse()));
    let inverse = inverse3(&forward).ok_or("singular projective placement")?;
    Ok(Some(Placement {
        local_to_texel,
        width: (x1 - x0) as usize,
        height: (y1 - y0) as usize,
        forward,
        inverse,
    }))
}

/// One mip level: premultiplied texels.
#[derive(Debug)]
struct Level {
    width: usize,
    height: usize,
    texels: Vec<[f64; 4]>,
}

/// A local image with its complete area-average mip chain.
#[derive(Debug)]
pub struct Mipmapped {
    levels: Vec<Level>,
}

/// Area-overlap weights resampling `from` texels onto `to` texels over the
/// same extent.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "level sizes are small"
)]
fn overlap_weights(from: usize, to: usize) -> Vec<Vec<(usize, f64)>> {
    let scale = from as f64 / to as f64;
    (0..to)
        .map(|i| {
            let (lo, hi) = (i as f64 * scale, (i + 1) as f64 * scale);
            (lo.floor() as usize..(hi.ceil() as usize).min(from))
                .map(|j| (j, (hi.min((j + 1) as f64) - lo.max(j as f64)) / scale))
                .filter(|&(_, w)| w > 0.0)
                .collect()
        })
        .collect()
}

impl Mipmapped {
    /// The chain of `base` (`width × height`) down to `1 × 1`.
    #[must_use]
    pub fn new(base: Vec<[f64; 4]>, width: usize, height: usize) -> Self {
        let mut levels = vec![Level {
            width,
            height,
            texels: base,
        }];
        while let Some(prev) = levels.last().filter(|l| (l.width, l.height) != (1, 1)) {
            let (w, h) = ((prev.width / 2).max(1), (prev.height / 2).max(1));
            let (wx, wy) = (
                overlap_weights(prev.width, w),
                overlap_weights(prev.height, h),
            );
            let mut texels = vec![[0.0; 4]; w * h];
            for (y, ys) in wy.iter().enumerate() {
                for (x, xs) in wx.iter().enumerate() {
                    let mut acc = [0.0; 4];
                    for &(sy, fy) in ys {
                        for &(sx, fx) in xs {
                            let t = prev.texels[sy * prev.width + sx];
                            for (sum, value) in acc.iter_mut().zip(t) {
                                *sum = (fx * fy).mul_add(value, *sum);
                            }
                        }
                    }
                    texels[y * w + x] = acc;
                }
            }
            levels.push(Level {
                width: w,
                height: h,
                texels,
            });
        }
        Self { levels }
    }

    /// Bilinear sample of `level` at base-texel point `at`; texels
    /// outside the level are transparent.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tap indices are range-checked"
    )]
    fn bilinear(&self, level: usize, at: [f64; 2]) -> [f64; 4] {
        let (base, lv) = (&self.levels[0], &self.levels[level]);
        let col = at[0] * lv.width as f64 / base.width as f64 - 0.5;
        let row = at[1] * lv.height as f64 / base.height as f64 - 0.5;
        let (left, upper) = (col.floor(), row.floor());
        let (fx, fy) = (col - left, row - upper);
        let mut out = [0.0; 4];
        for (dy, wy) in [(0.0, 1.0 - fy), (1.0, fy)] {
            for (dx, wx) in [(0.0, 1.0 - fx), (1.0, fx)] {
                let (x, y) = (left + dx, upper + dy);
                if x < 0.0 || y < 0.0 || x >= lv.width as f64 || y >= lv.height as f64 {
                    continue;
                }
                let texel = lv.texels[y as usize * lv.width + x as usize];
                for (sum, value) in out.iter_mut().zip(texel) {
                    *sum = (wx * wy).mul_add(value, *sum);
                }
            }
        }
        out
    }

    /// The box average over the destination pixel whose top-left corner
    /// is `corner`: `grid × grid` bilinear base-level samples at the
    /// preimages of a uniform lattice, each transparent without a
    /// front-facing preimage.
    #[must_use]
    pub fn box_sample(&self, inverse: &M3, corner: [f64; 2], grid: u32) -> [f64; 4] {
        let n = f64::from(grid);
        let mut acc = [0.0; 4];
        for j in 0..grid {
            for i in 0..grid {
                let dest = [
                    corner[0] + (f64::from(i) + 0.5) / n,
                    corner[1] + (f64::from(j) + 0.5) / n,
                ];
                let pre = apply3(inverse, [dest[0], dest[1], 1.0]);
                if pre[2] <= 0.0 || pre[2].is_nan() {
                    continue;
                }
                let texel = self.bilinear(0, [pre[0] / pre[2], pre[1] / pre[2]]);
                for (sum, value) in acc.iter_mut().zip(texel) {
                    *sum += value;
                }
            }
        }
        acc.map(|v| v / (n * n))
    }

    /// The anisotropic reconstruction at destination point `dest` under
    /// `inverse` (parent raster to base texels): transparent where `dest`
    /// has no front-facing preimage.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the level index and tap count are small and clamped"
    )]
    pub fn sample(&self, inverse: &M3, dest: [f64; 2]) -> [f64; 4] {
        let pre = apply3(inverse, [dest[0], dest[1], 1.0]);
        if pre[2] <= 0.0 || pre[2].is_nan() {
            return [0.0; 4];
        }
        let center = [pre[0] / pre[2], pre[1] / pre[2]];
        let jac = jacobian(inverse, dest);
        // Singular values of J from JJᵀ, and the major direction in source
        // (texel) space: the eigenvector of JJᵀ for its larger eigenvalue.
        let xx = jac[0].mul_add(jac[0], jac[1] * jac[1]);
        let xy = jac[0].mul_add(jac[2], jac[1] * jac[3]);
        let yy = jac[2].mul_add(jac[2], jac[3] * jac[3]);
        let half_trace = f64::midpoint(xx, yy);
        let spread = ((xx - yy) / 2.0).hypot(xy);
        let major = (half_trace + spread).sqrt();
        let minor = (half_trace - spread).max(0.0).sqrt();
        let theta = if spread > 0.0 {
            (2.0 * xy).atan2(xx - yy) / 2.0
        } else {
            0.0
        };
        let (sin, cos) = theta.sin_cos();
        let b_eff = 1f64.max(minor).max(major / TAPS);
        let taps = (major / b_eff - TAP_SLACK).ceil().clamp(1.0, TAPS);
        let top = (self.levels.len() - 1) as f64;
        let lod = b_eff.log2().clamp(0.0, top);
        let fine = lod.floor();
        let blend = lod - fine;
        let coarse = (fine + 1.0).min(top);
        let mut acc = [0.0; 4];
        for tap in 0..taps as usize {
            let offset = (tap as f64 + 0.5 - taps / 2.0) * major / taps;
            let at = [
                offset.mul_add(cos, center[0]),
                offset.mul_add(sin, center[1]),
            ];
            let near = self.bilinear(fine as usize, at);
            let far = if coarse > fine {
                self.bilinear(coarse as usize, at)
            } else {
                near
            };
            for ((sum, lo), hi) in acc.iter_mut().zip(near).zip(far) {
                *sum += lerp(blend, lo, hi);
            }
        }
        acc.map(|v| v / taps)
    }
}

/// The texel-space polylines `outline` projected by `forward` as edges.
///
/// The parent raster is `raster` pixels; each closed polyline is clipped
/// against `W > 0` and the raster (widened by a pixel) before division.
#[must_use]
pub fn project_edges(forward: &M3, outline: &[Polyline], raster: (usize, usize)) -> Vec<Segment> {
    #[expect(clippy::cast_precision_loss, reason = "raster sizes are small")]
    let view = Rect::new(0.0, 0.0, raster.0 as f64, raster.1 as f64).inflate(1.0, 1.0);
    let mut out = Vec::new();
    for line in outline {
        if !line.closed {
            continue;
        }
        let points: Vec<[f64; 2]> = line.points.iter().map(|&p| p.into()).collect();
        let poly = clip_visible(forward, &points, view);
        let n = poly.len();
        if n < 3 {
            continue;
        }
        for i in 0..n {
            let (qa, qb) = (poly[i].1, poly[(i + 1) % n].1);
            out.push((qa[0] / qa[2], qa[1] / qa[2], qb[0] / qb[2], qb[1] / qb[2]));
        }
    }
    out
}
