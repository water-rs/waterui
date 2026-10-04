//! Planning for projective layers (#84), shared by both backends.
//!
//! It covers the plane homography, homogeneous clipping against the
//! destination viewport, the conservative local raster density, the
//! local image layout with its mip chain, and the per-sample anisotropic
//! filter parameters.
//!
//! A projective layer is flattened: its subtree renders into a
//! layer-local image at density `ρ` texels per layer unit, with the
//! existing affine rasterizers, and the image is projected when the layer
//! composes into its parent raster (the surface, or an enclosing
//! projective layer's image). All planning arithmetic is `f64`.

use kurbo::{Affine, Rect};

use crate::{FillRule, LayerId, Projective, RenderError, ShapeData, SurfaceTree};

/// Feature name: a projective layer without a clip has no finite local
/// source domain.
pub const UNCLIPPED: &str = "projective-unclipped";
/// Feature name: a projective layer that is itself a backdrop member.
pub const BACKDROP_MEMBER: &str = "projective-backdrop-member";
/// Feature name: a backdrop group whose members lie in different
/// composition spaces (inside and outside a projective layer).
pub const BACKDROP_CROSS_SPACE: &str = "projective-backdrop-cross-space";

/// Destination pixels a projected sample can reach beyond the projected
/// domain: the antialiasing and reconstruction footprint.
pub const FOOTPRINT: f64 = 2.0;

/// The largest number of leaf regions the density bound evaluates.
const MAX_LEAVES: usize = 64;

/// The largest number of taps the anisotropic filter integrates.
pub const MAX_TAPS: u32 = 16;

/// How far `a / b_eff` may exceed an integer and still take that many taps.
///
/// It is the reconstruction's precision, 1/256. Rounding in
/// the homography makes an isotropic footprint's ratio `1 + ε`, and
/// `ceil` would turn that noise into a second tap along an arbitrary
/// direction; spacing the taps up to `1 + 1/256` times `b_eff` apart is
/// below that precision.
pub const TAP_SLACK: f64 = 1.0 / 256.0;

/// Resource limits a backend admits for one local image.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The largest image dimension, in texels.
    pub max_dimension: u32,
    /// The largest image, base level plus mips, in bytes.
    pub max_bytes: u64,
}

/// A 3×3 homography on column vectors, `(X, Y, W) = H · (x, y, 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Homography {
    /// The homography `pose` induces on the `z = 0` plane.
    #[must_use]
    pub const fn plane(pose: &Projective) -> Self {
        Self(pose.plane_homography())
    }

    /// The affine map as a homography.
    #[must_use]
    #[expect(
        clippy::many_single_char_names,
        reason = "a..f are the conventional affine coefficient names"
    )]
    pub const fn affine(t: Affine) -> Self {
        let [a, b, c, d, e, f] = t.as_coeffs();
        Self([[a, c, e], [b, d, f], [0., 0., 1.]])
    }

    /// `self · rhs`: `rhs` applies first.
    #[must_use]
    pub fn then(self, lhs: Self) -> Self {
        let (a, b) = (lhs.0, self.0);
        Self(std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                a[i][2].mul_add(b[2][j], a[i][0].mul_add(b[0][j], a[i][1] * b[1][j]))
            })
        }))
    }

    /// `H · (x, y, 1)`.
    #[must_use]
    pub fn map(&self, x: f64, y: f64) -> [f64; 3] {
        self.0.map(|r| r[2] + r[0].mul_add(x, r[1] * y))
    }

    /// The determinant.
    #[must_use]
    pub fn determinant(&self) -> f64 {
        let m = &self.0;
        let minor =
            |r0: usize, c0: usize, c1: usize| m[r0][c0].mul_add(m[2][c1], -(m[r0][c1] * m[2][c0]));
        m[0][2].mul_add(
            minor(1, 0, 1),
            m[0][0].mul_add(minor(1, 1, 2), -(m[0][1] * minor(1, 0, 2))),
        )
    }

    /// The adjugate, scaled by the sign of the determinant: for a
    /// destination point `d`, `inverse · (d, 1) = (x', y', w')` with the
    /// source point `(x'/w', y'/w')`, and the point lies in front
    /// (positive forward `W`) exactly when `w' > 0`.
    #[must_use]
    pub fn front_inverse(&self) -> Self {
        let m = &self.0;
        let cof = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0].mul_add(m[r1][c1], -(m[r0][c1] * m[r1][c0]))
        };
        let adj = [
            [cof(1, 2, 1, 2), -cof(0, 2, 1, 2), cof(0, 1, 1, 2)],
            [-cof(1, 2, 0, 2), cof(0, 2, 0, 2), -cof(0, 1, 0, 2)],
            [cof(1, 2, 0, 1), -cof(0, 2, 0, 1), cof(0, 1, 0, 1)],
        ];
        let sign = self.determinant().signum();
        Self(adj.map(|row| row.map(|v| v * sign)))
    }
}

/// One projective layer's plan for this frame.
#[derive(Clone, Debug)]
pub struct Projected {
    /// The projective layer.
    pub layer: LayerId,
    /// The enclosing projective layer whose image this one composes into,
    /// or `None` for the surface.
    pub parent: Option<LayerId>,
    /// The local image layout, or `None` when the layer contributes no
    /// area this frame (behind the viewer, edge-on, or off the viewport):
    /// its subtree is not rendered.
    pub image: Option<LocalImage>,
}

/// A projective layer's local image and how it maps to its parent raster.
#[derive(Clone, Debug)]
pub struct LocalImage {
    /// Texels per layer unit: a power of two, at least one.
    pub density: f64,
    /// Layer space (the layer's own, pre-scroll coordinates) to base
    /// texels. The subtree renders under this transform in place of the
    /// layer's placement.
    pub local_to_texel: Affine,
    /// The base level size in texels.
    pub size: (u32, u32),
    /// Every level's size, base first, down to `1 × 1`.
    pub levels: Vec<(u32, u32)>,
    /// Parent raster pixels to base texels (see
    /// [`Homography::front_inverse`]).
    pub inverse: Homography,
    /// Base texels to parent raster pixels.
    pub forward: Homography,
    /// Layer space to parent raster pixels.
    pub to_parent: Homography,
    /// The parent-raster pixels to shade, `[x0, y0, x1, y1)`.
    pub bounds: [u32; 4],
}

impl LocalImage {
    /// Bytes of the base level plus every mip at 8 bytes a texel
    /// (premultiplied RGBA16F): `8 · Σ w·h`.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        image_bytes(&self.levels)
    }
}

fn image_bytes(levels: &[(u32, u32)]) -> u64 {
    levels
        .iter()
        .map(|&(w, h)| 8 * u64::from(w) * u64::from(h))
        .sum()
}

/// Every mip level size from `size` down to `1 × 1`: each dimension is
/// `max(1, floor(previous / 2))` — the hardware mip chain's sizes — and
/// every level spans the same source extent.
#[must_use]
pub fn mip_levels(size: (u32, u32)) -> Vec<(u32, u32)> {
    let mut levels = vec![size];
    let (mut w, mut h) = size;
    while (w, h) != (1, 1) {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        levels.push((w, h));
    }
    levels
}

/// Plans every projective layer of `tree` on a surface of `size` pixels,
/// innermost first: a nested projective layer's image is realized before
/// the image it composes into.
///
/// # Errors
/// [`RenderError::ProjectivePose`] for an invalid composed pose,
/// [`RenderError::Unsupported`] for an unclipped projective layer or an
/// unsupported backdrop arrangement, [`RenderError::ProjectiveUnsupported`]
/// when a visible image exceeds `limits` or no finite bound exists.
pub fn plan(
    tree: &SurfaceTree,
    size: (u32, u32),
    limits: Limits,
) -> Result<Vec<Projected>, RenderError> {
    let mut out = Vec::new();
    if !tree.has_projective() {
        return Ok(out);
    }
    let mut walk = Walk {
        tree,
        limits,
        out: &mut out,
        groups: rustc_hash::FxHashMap::default(),
    };
    walk.layer(tree.root(), Affine::IDENTITY, size, None)?;
    Ok(out)
}

struct Walk<'a> {
    tree: &'a SurfaceTree,
    limits: Limits,
    out: &'a mut Vec<Projected>,
    /// The composition space each backdrop group's members were found in.
    groups: rustc_hash::FxHashMap<u64, Option<LayerId>>,
}

impl Walk<'_> {
    /// Visits `id`, whose parent space maps to raster pixels by
    /// `to_raster`; `space` is the enclosing projective layer.
    fn layer(
        &mut self,
        id: LayerId,
        to_raster: Affine,
        raster: (u32, u32),
        space: Option<LayerId>,
    ) -> Result<(), RenderError> {
        let node = self.tree.layer(id);
        if let Some(sample) = &node.backdrop {
            let seen = *self.groups.entry(sample.group().raw()).or_insert(space);
            if seen != space {
                return Err(RenderError::Unsupported(BACKDROP_CROSS_SPACE));
            }
        }
        let Some(pose) = self.tree.projective_pose(id) else {
            let children = to_raster * node.content_transform();
            for child in &node.children {
                self.layer(*child, children, raster, space)?;
            }
            return Ok(());
        };
        let pose = pose.map_err(|error| RenderError::ProjectivePose { layer: id, error })?;
        if node.backdrop.is_some() {
            return Err(RenderError::Unsupported(BACKDROP_MEMBER));
        }
        let clip = node
            .clip
            .as_ref()
            .ok_or(RenderError::Unsupported(UNCLIPPED))?;
        let domain = clip.bounds();
        let to_parent = Homography::plane(&pose).then(Homography::affine(to_raster));
        let image = local_image(id, &to_parent, domain, raster, self.limits)?;
        if let Some(image) = &image {
            let children = image.local_to_texel * kurbo::Affine::translate(-node.scroll_offset);
            for child in &node.children {
                self.layer(*child, children, image.size, Some(id))?;
            }
        }
        self.out.push(Projected {
            layer: id,
            parent: space,
            image,
        });
        Ok(())
    }
}

/// A homogeneous polygon vertex: the local point and its destination
/// homogeneous coordinates.
#[derive(Clone, Copy, Debug)]
struct Vertex {
    local: [f64; 2],
    q: [f64; 3],
}

/// Clips `poly` against the half-space `plane · q ≥ 0`, intersecting
/// edges before division and carrying local coordinates through.
fn clip_plane(poly: &[Vertex], plane: [f64; 3]) -> Vec<Vertex> {
    let dist = |v: &Vertex| plane[2].mul_add(v.q[2], plane[0].mul_add(v.q[0], plane[1] * v.q[1]));
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (i, a) in poly.iter().enumerate() {
        let b = &poly[(i + 1) % poly.len()];
        let (da, db) = (dist(a), dist(b));
        if da >= 0.0 {
            out.push(*a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let lerp = |x: f64, y: f64| t.mul_add(y - x, x);
            out.push(Vertex {
                local: [lerp(a.local[0], b.local[0]), lerp(a.local[1], b.local[1])],
                q: [
                    lerp(a.q[0], b.q[0]),
                    lerp(a.q[1], b.q[1]),
                    lerp(a.q[2], b.q[2]),
                ],
            });
        }
    }
    out
}

/// The part of the local rectangle `domain` whose image lies in front of
/// the viewer and inside `viewport` (destination pixels): clipped in
/// homogeneous coordinates against `W > 0` and the viewport's four edges
/// before any division.
fn visible(h: &Homography, domain: Rect, viewport: Rect) -> Vec<Vertex> {
    let corners = [
        [domain.x0, domain.y0],
        [domain.x1, domain.y0],
        [domain.x1, domain.y1],
        [domain.x0, domain.y1],
    ];
    clip_polygon(h, &corners, viewport)
}

/// Clips the local polygon `points` under `h` against `W > 0` and the
/// `viewport` edges, in homogeneous coordinates.
fn clip_polygon(h: &Homography, points: &[[f64; 2]], viewport: Rect) -> Vec<Vertex> {
    let mut poly: Vec<Vertex> = points
        .iter()
        .map(|&[x, y]| Vertex {
            local: [x, y],
            q: h.map(x, y),
        })
        .collect();
    for plane in [
        [0.0, 0.0, 1.0],
        [1.0, 0.0, -viewport.x0],
        [-1.0, 0.0, viewport.x1],
        [0.0, 1.0, -viewport.y0],
        [0.0, -1.0, viewport.y1],
    ] {
        if poly.is_empty() {
            break;
        }
        poly = clip_plane(&poly, plane);
    }
    // A vertex exactly on W = 0 has no finite projection; the viewport
    // planes admit one only at the homogeneous origin.
    poly.retain(|v| v.q[2] > 0.0);
    poly
}

/// Projects the closed local outline `path` into the parent raster.
///
/// Each subpath is flattened to `tolerance` layer units, clipped against
/// `W > 0` and `viewport` before division, and divided. The result bounds
/// the projected area for coverage (a destructive blend's operator
/// domain); it is never used as source alpha.
#[must_use]
pub fn project_outline(
    h: &Homography,
    path: &kurbo::BezPath,
    tolerance: f64,
    viewport: Rect,
) -> kurbo::BezPath {
    let mut subpaths: Vec<Vec<[f64; 2]>> = Vec::new();
    kurbo::flatten(path, tolerance, |el| match el {
        kurbo::PathEl::MoveTo(p) => subpaths.push(vec![[p.x, p.y]]),
        kurbo::PathEl::LineTo(p) => {
            if let Some(sub) = subpaths.last_mut() {
                sub.push([p.x, p.y]);
            }
        }
        _ => {}
    });
    let mut out = kurbo::BezPath::new();
    for sub in subpaths {
        let poly = clip_polygon(h, &sub, viewport);
        if poly.len() < 3 {
            continue;
        }
        for (i, v) in poly.iter().enumerate() {
            let point = kurbo::Point::new(v.q[0] / v.q[2], v.q[1] / v.q[2]);
            if i == 0 {
                out.move_to(point);
            } else {
                out.line_to(point);
            }
        }
        out.close_path();
    }
    out
}

/// A destructive blend's operator domain in the parent raster.
///
/// The projective layer clip `clip` is flattened to `tolerance` layer
/// units and projected by `h` into `viewport` (see [`project_outline`]),
/// keeping its fill rule. `None` for a clip without area (a line).
#[must_use]
pub fn project_clip(
    h: &Homography,
    clip: &ShapeData,
    tolerance: f64,
    viewport: Rect,
) -> Option<(kurbo::BezPath, FillRule)> {
    let (path, rule) = super::shape_outline(clip, tolerance)?;
    Some((project_outline(h, &path, tolerance, viewport), rule))
}

/// The largest singular value of the row-major 2×2 `m`.
fn sigma_max(m: [f64; 4]) -> f64 {
    let sum = m[3].mul_add(m[3], m[2].mul_add(m[2], m[0].mul_add(m[0], m[1] * m[1])));
    let det = m[0].mul_add(m[3], -(m[1] * m[2]));
    let disc = sum.mul_add(sum, -4.0 * det * det).max(0.0);
    f64::midpoint(sum, disc.sqrt()).sqrt()
}

/// The largest density exponent: `2^20` texels per layer unit.
const MAX_EXPONENT: u32 = 20;

/// The density bucket exponent for a bound `s`: the least `k` with
/// `2^k ≥ max(1, s)`, or `None` above [`MAX_EXPONENT`].
fn bucket(s: f64) -> Option<u32> {
    (0..=MAX_EXPONENT).find(|&k| f64::from(1_u32 << k) >= s)
}

/// Clips the local convex polygon `poly` to the axis-aligned `r`.
fn clip_rect(poly: &[[f64; 2]], r: Rect) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = poly.to_vec();
    for (axis, bound, keep_above) in [
        (0, r.x0, true),
        (0, r.x1, false),
        (1, r.y0, true),
        (1, r.y1, false),
    ] {
        if out.is_empty() {
            break;
        }
        let inside = |p: &[f64; 2]| {
            if keep_above {
                p[axis] >= bound
            } else {
                p[axis] <= bound
            }
        };
        let mut next = Vec::with_capacity(out.len() + 1);
        for (i, a) in out.iter().enumerate() {
            let b = &out[(i + 1) % out.len()];
            if inside(a) {
                next.push(*a);
            }
            if inside(a) != inside(b) {
                let t = (bound - a[axis]) / (b[axis] - a[axis]);
                next.push([t.mul_add(b[0] - a[0], a[0]), t.mul_add(b[1] - a[1], a[1])]);
            }
        }
        out = next;
    }
    out
}

/// The Jacobian of `p(x, y) = (X/W, Y/W)` at the local point `at`.
fn jacobian(h: &Homography, at: [f64; 2]) -> [f64; 4] {
    derivative(h, h.map(at[0], at[1]))
}

/// The Jacobian of the division `(X/W, Y/W)` of `h · (x, y, 1)` with
/// respect to `(x, y)`, given the homogeneous image `q`.
fn derivative(h: &Homography, q: [f64; 3]) -> [f64; 4] {
    let m = &h.0;
    let w2 = q[2] * q[2];
    [
        m[0][0].mul_add(q[2], -(q[0] * m[2][0])) / w2,
        m[0][1].mul_add(q[2], -(q[0] * m[2][1])) / w2,
        m[1][0].mul_add(q[2], -(q[1] * m[2][0])) / w2,
        m[1][1].mul_add(q[2], -(q[1] * m[2][1])) / w2,
    ]
}

/// Upper and sampled lower bounds of `σmax(J)` over the local convex
/// polygon `poly`: interval arithmetic on the Jacobian's entries (each
/// numerator is affine, `W` is affine and positive), then the midpoint's
/// singular value plus the Frobenius norm of the radius.
fn leaf_bounds(h: &Homography, poly: &[[f64; 2]]) -> (f64, f64) {
    let m = &h.0;
    // Numerators H_ij·W − X_i·H_2j are affine in (x, y).
    let numerators = [(0usize, 0usize), (0, 1), (1, 0), (1, 1)].map(|(i, j)| {
        let (row, other) = (m[i], m[2]);
        [
            // coefficient of x, y, constant
            row[j].mul_add(other[0], -(row[0] * other[j])),
            row[j].mul_add(other[1], -(row[1] * other[j])),
            row[j].mul_add(other[2], -(row[2] * other[j])),
        ]
    });
    let range = |f: &dyn Fn(&[f64; 2]) -> f64| {
        poly.iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                let v = f(p);
                (lo.min(v), hi.max(v))
            })
    };
    let (w_lo, w_hi) = range(&|p| h.map(p[0], p[1])[2]);
    let (inv_lo, inv_hi) = (1.0 / (w_hi * w_hi), 1.0 / (w_lo * w_lo));
    let mut mid = [0.0; 4];
    let mut rad2 = 0.0;
    for (k, n) in numerators.iter().enumerate() {
        let (lo, hi) = range(&|p| n[2] + n[0].mul_add(p[0], n[1] * p[1]));
        let products = [lo * inv_lo, lo * inv_hi, hi * inv_lo, hi * inv_hi];
        let e_lo = products.iter().copied().fold(f64::INFINITY, f64::min);
        let e_hi = products.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        mid[k] = f64::midpoint(e_lo, e_hi);
        let r = 0.5 * (e_hi - e_lo);
        rad2 = r.mul_add(r, rad2);
    }
    let upper = sigma_max(mid) + rad2.sqrt();
    #[expect(clippy::cast_precision_loss, reason = "a polygon has few vertices")]
    let n = poly.len() as f64;
    let centroid = poly
        .iter()
        .fold([0.0, 0.0], |c, p| [c[0] + p[0] / n, c[1] + p[1] / n]);
    let j = jacobian(h, centroid);
    (upper, sigma_max(j))
}

/// The conservative density for the visible local polygon: the bucket of
/// an upper bound on `σmax(J)`, refined by subdividing the longest local
/// axis until the upper and sampled lower bounds choose the same bucket
/// or [`MAX_LEAVES`] leaves were evaluated. `None` when no finite bound
/// exists.
fn density(h: &Homography, poly: &[[f64; 2]]) -> Option<f64> {
    bucket_bound(h, poly).map(|k| f64::from(1_u32 << k))
}

/// [`density`]'s bucket exponent.
fn bucket_bound(h: &Homography, poly: &[[f64; 2]]) -> Option<u32> {
    let m = &h.0;
    if m[2][0] == 0.0 && m[2][1] == 0.0 {
        // A constant denominator: J is constant and exact.
        let w = m[2][2];
        return bucket(sigma_max(
            [m[0][0], m[0][1], m[1][0], m[1][1]].map(|v| v / w),
        ));
    }
    let bbox = poly.iter().fold(
        Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |r, p| {
            Rect::new(
                r.x0.min(p[0]),
                r.y0.min(p[1]),
                r.x1.max(p[0]),
                r.y1.max(p[1]),
            )
        },
    );
    let mut leaves: Vec<(Rect, f64, f64)> = Vec::new();
    let mut evaluated = 0;
    let push = |r: Rect, leaves: &mut Vec<(Rect, f64, f64)>, evaluated: &mut usize| {
        let part = clip_rect(poly, r);
        if part.len() >= 3 {
            *evaluated += 1;
            let (upper, lower) = leaf_bounds(h, &part);
            leaves.push((r, upper, lower));
        }
    };
    push(bbox, &mut leaves, &mut evaluated);
    loop {
        let upper = leaves.iter().map(|l| l.1).fold(0.0, f64::max);
        let lower = leaves.iter().map(|l| l.2).fold(0.0, f64::max);
        if !upper.is_finite() {
            return None;
        }
        let hi = bucket(upper)?;
        if bucket(lower)? == hi || evaluated >= MAX_LEAVES {
            return Some(hi);
        }
        let worst = leaves
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
            .map(|(i, _)| i)?;
        let (leaf, ..) = leaves.swap_remove(worst);
        let (first, second) = if leaf.width() >= leaf.height() {
            let x = f64::midpoint(leaf.x0, leaf.x1);
            (
                Rect::new(leaf.x0, leaf.y0, x, leaf.y1),
                Rect::new(x, leaf.y0, leaf.x1, leaf.y1),
            )
        } else {
            let y = f64::midpoint(leaf.y0, leaf.y1);
            (
                Rect::new(leaf.x0, leaf.y0, leaf.x1, y),
                Rect::new(leaf.x0, y, leaf.x1, leaf.y1),
            )
        };
        push(first, &mut leaves, &mut evaluated);
        push(second, &mut leaves, &mut evaluated);
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "texel coordinates are checked against the dimension limit first"
)]
fn local_image(
    layer: LayerId,
    to_parent: &Homography,
    domain: Rect,
    raster: (u32, u32),
    limits: Limits,
) -> Result<Option<LocalImage>, RenderError> {
    let unsupported = |reason: String| RenderError::ProjectiveUnsupported { layer, reason };
    if !(domain.width() > 0.0 && domain.height() > 0.0) || to_parent.determinant() == 0.0 {
        // No local area, or a plane seen exactly edge-on.
        return Ok(None);
    }
    let raster_rect = Rect::new(0.0, 0.0, f64::from(raster.0), f64::from(raster.1));
    let poly = visible(to_parent, domain, raster_rect.inflate(FOOTPRINT, FOOTPRINT));
    if poly.len() < 3 {
        return Ok(None);
    }
    let mut dest = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for v in &poly {
        let (x, y) = (v.q[0] / v.q[2], v.q[1] / v.q[2]);
        dest = Rect::new(
            dest.x0.min(x),
            dest.y0.min(y),
            dest.x1.max(x),
            dest.y1.max(y),
        );
    }
    if !(dest.x0.is_finite() && dest.y0.is_finite() && dest.x1.is_finite() && dest.y1.is_finite()) {
        return Err(unsupported("no finite projected bound".into()));
    }
    let dest = dest.inflate(FOOTPRINT, FOOTPRINT).intersect(raster_rect);
    let bounds = [
        dest.x0.floor().max(0.0) as u32,
        dest.y0.floor().max(0.0) as u32,
        dest.x1.ceil().min(raster_rect.x1) as u32,
        dest.y1.ceil().min(raster_rect.y1) as u32,
    ];
    if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
        return Ok(None);
    }
    let local: Vec<[f64; 2]> = poly.iter().map(|v| v.local).collect();
    let rho = density(to_parent, &local)
        .ok_or_else(|| unsupported("no finite raster density bound".into()))?;
    let (x0, y0) = ((domain.x0 * rho).floor(), (domain.y0 * rho).floor());
    let (x1, y1) = ((domain.x1 * rho).ceil(), (domain.y1 * rho).ceil());
    let max = f64::from(limits.max_dimension);
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w <= max && h <= max) {
        return Err(unsupported(format!(
            "{w}x{h} texels at density {rho} exceeds the {max} texel dimension limit"
        )));
    }
    let size = (w as u32, h as u32);
    let levels = mip_levels(size);
    let bytes = image_bytes(&levels);
    if bytes > limits.max_bytes {
        return Err(unsupported(format!(
            "{}x{} texels at density {rho} needs {bytes} bytes; {} are admitted",
            size.0, size.1, limits.max_bytes
        )));
    }
    let local_to_texel = Affine::new([rho, 0.0, 0.0, rho, -x0, -y0]);
    let forward = Homography::affine(local_to_texel.inverse()).then(*to_parent);
    Ok(Some(LocalImage {
        density: rho,
        local_to_texel,
        size,
        levels,
        inverse: forward.front_inverse(),
        forward,
        to_parent: *to_parent,
        bounds,
    }))
}

/// The anisotropic reconstruction at one destination sample: where to
/// sample, at which level, with how many taps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    /// The source point in base texels.
    pub center: [f64; 2],
    /// The major-axis step between taps, in base texels.
    pub step: [f64; 2],
    /// The tap count, `1..=16`.
    pub taps: u32,
    /// The level of detail, `log2(b_eff)`.
    pub lod: f64,
}

/// The footprint of destination point `d` under `inverse` (destination to
/// base texels), or `None` when `d` has no front-facing preimage.
///
/// With the inverse map's Jacobian singular values `a ≥ b` and major-axis
/// direction `e`: `b_eff = max(1, b, a/16)`, `lod = log2(b_eff)`,
/// `N = clamp(ceil(a / b_eff − 1/256), 1, 16)` (see [`TAP_SLACK`]), and
/// the `N` equal-weight taps sit
/// at the centres of `N` equal parts of the length-`a` major axis.
#[must_use]
pub fn footprint(inverse: &Homography, d: [f64; 2]) -> Option<Footprint> {
    footprint_at(inverse, inverse.map(d[0], d[1]))
}

/// [`footprint`] from the homogeneous preimage `q = inverse · (d, 1)`.
///
/// It serves callers that step `q` incrementally along a scanline: `q` is
/// affine in `d`, so adding `inverse`'s first column per pixel keeps the
/// numerators and denominator exact up to rounding, and only the division
/// is per sample.
#[must_use]
pub fn footprint_at(inverse: &Homography, q: [f64; 3]) -> Option<Footprint> {
    if q[2] <= 0.0 {
        return None;
    }
    let center = [q[0] / q[2], q[1] / q[2]];
    let (a, b, e) = singular(derivative(inverse, q));
    let b_eff = 1.0_f64.max(b).max(a / f64::from(MAX_TAPS));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the ratio is clamped to 1..=16"
    )]
    let taps = (a / b_eff - TAP_SLACK)
        .ceil()
        .clamp(1.0, f64::from(MAX_TAPS)) as u32;
    let step = a / f64::from(taps);
    Some(Footprint {
        center,
        step: [e[0] * step, e[1] * step],
        taps,
        lod: b_eff.log2(),
    })
}

/// Singular values `a ≥ b` of the row-major 2×2 `j` and the unit
/// direction of the major axis in the output space of `j`.
fn singular(j: [f64; 4]) -> (f64, f64, [f64; 2]) {
    // J Jᵀ = [[xx, xy], [xy, yy]].
    let xx = j[0].mul_add(j[0], j[1] * j[1]);
    let xy = j[0].mul_add(j[2], j[1] * j[3]);
    let yy = j[2].mul_add(j[2], j[3] * j[3]);
    let root = (0.5 * (xx - yy)).hypot(xy);
    let mean = f64::midpoint(xx, yy);
    let major = mean + root;
    let minor = (mean - root).max(0.0);
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    let (sin, cos) = angle.sin_cos();
    (major.sqrt(), minor.sqrt(), [cos, sin])
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Vec2;

    fn tilted(tilt: Vec2, distance: f64, pivot: Vec2) -> Homography {
        let pose = crate::projective::Pose {
            base: Affine::IDENTITY,
            translation: Vec2::ZERO,
            pivot,
            rotation: 0.0,
            skew: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            projection: Projective::perspective(distance).unwrap(),
            tilt,
            depth: 0.0,
        }
        .matrix()
        .unwrap();
        Homography::plane(&pose)
    }

    const LIMITS: Limits = Limits {
        max_dimension: 16384,
        max_bytes: 1 << 30,
    };

    #[test]
    fn mip_levels_halve_up_to_one_texel_and_count_bytes_exactly() {
        assert_eq!(mip_levels((5, 2)), [(5, 2), (2, 1), (1, 1)]);
        assert_eq!(image_bytes(&mip_levels((5, 2))), 8 * (10 + 2 + 1));
        // As many levels as the hardware chain: floor(log2(257)) + 1.
        assert_eq!(mip_levels((257, 3)).len(), 9);
        // The plan's scale example: 640×400 plus its chain is ≈ 2.60 MiB.
        let bytes = image_bytes(&mip_levels((640, 400)));
        #[expect(clippy::cast_precision_loss, reason = "a test figure")]
        let mib = bytes as f64 / f64::from(1 << 20);
        assert!((mib - 2.60).abs() < 0.01, "{mib}");
    }

    #[test]
    fn the_front_inverse_round_trips_and_marks_the_front_half_space() {
        let h = tilted(Vec2::new(0.4, -0.3), 500.0, Vec2::new(50.0, 40.0));
        let inv = h.front_inverse();
        let [x, y, w] = h.map(12.0, 34.0);
        assert!(w > 0.0);
        let [sx, sy, sw] = inv.map(x / w, y / w);
        assert!(sw > 0.0);
        assert!((sx / sw - 12.0).abs() < 1e-9 && (sy / sw - 34.0).abs() < 1e-9);
        // Negating the homography flips the front half-space.
        let neg = Homography(h.0.map(|r| r.map(|v| -v)));
        assert!(neg.front_inverse().map(x / w, y / w)[2] < 0.0);
        // Positive homogeneous scaling does not.
        let scaled = Homography(h.0.map(|r| r.map(|v| v * 4.0)));
        assert!(scaled.front_inverse().map(x / w, y / w)[2] > 0.0);
    }

    #[test]
    fn clipping_happens_before_division_across_the_horizon() {
        // A plane tilted so its far half lies behind the camera.
        let h = tilted(Vec2::new(1.4, 0.0), 100.0, Vec2::new(0.0, 0.0));
        let domain = Rect::new(-200.0, -400.0, 200.0, 400.0);
        let poly = visible(&h, domain, Rect::new(0.0, 0.0, 640.0, 480.0));
        assert!(poly.len() >= 3);
        for v in &poly {
            assert!(v.q[2] > 0.0);
            let (x, y) = (v.q[0] / v.q[2], v.q[1] / v.q[2]);
            assert!(x.is_finite() && y.is_finite());
            assert!((-1e-9..=640.0 + 1e-9).contains(&x) && (-1e-9..=480.0 + 1e-9).contains(&y));
            // Local coordinates carried through intersections map back.
            let q = h.map(v.local[0], v.local[1]);
            for (a, b) in q.iter().zip(v.q) {
                assert!((a - b).abs() < 1e-9 * (1.0 + a.abs()));
            }
        }
    }

    #[test]
    fn density_is_one_for_an_untransformed_layer_and_grows_toward_the_viewer() {
        let identity = Homography::affine(Affine::IDENTITY);
        let square = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        assert_eq!(density(&identity, &square), Some(1.0));
        assert_eq!(
            density(&Homography::affine(Affine::scale(3.0)), &square),
            Some(4.0)
        );
        // A card tilted about its left edge brings its right edge toward
        // the viewer when the angle is negative: magnification above one.
        let h = tilted(Vec2::new(0.0, -1.0), 300.0, Vec2::new(0.0, 50.0));
        let rect = [[0.0, 0.0], [200.0, 0.0], [200.0, 100.0], [0.0, 100.0]];
        let rho = density(&h, &rect).unwrap();
        // The bound must dominate every sampled singular value.
        for i in 0..=20 {
            for k in 0..=10 {
                let j = jacobian(&h, [f64::from(i) * 10.0, f64::from(k) * 10.0]);
                assert!(sigma_max(j) <= rho);
            }
        }
        assert!(rho >= 2.0);
    }

    #[test]
    fn a_layer_behind_the_viewer_or_edge_on_contributes_nothing() {
        let layer = LayerId::new(1);
        let domain = Rect::new(0.0, 0.0, 100.0, 100.0);
        // Everything behind: w = 1 − z/d < 0 for the whole card.
        let mut behind = Homography::affine(Affine::IDENTITY).0;
        behind[2] = [0.0, 0.0, -1.0];
        let image = local_image(layer, &Homography(behind), domain, (64, 64), LIMITS).unwrap();
        assert!(image.is_none());
        let mut edge_on = Homography::affine(Affine::IDENTITY).0;
        edge_on[0] = [0.0, 0.0, 10.0];
        let image = local_image(layer, &Homography(edge_on), domain, (64, 64), LIMITS).unwrap();
        assert!(image.is_none());
    }

    #[test]
    fn oversized_images_are_explicitly_unsupported() {
        let h = Homography::affine(Affine::scale(3.0));
        let tiny = Limits {
            max_dimension: 64,
            max_bytes: 1 << 30,
        };
        let error = local_image(
            LayerId::new(7),
            &h,
            Rect::new(0.0, 0.0, 100.0, 10.0),
            (1000, 1000),
            tiny,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RenderError::ProjectiveUnsupported { layer, .. } if layer == LayerId::new(7)
        ));
    }

    #[test]
    fn the_footprint_follows_the_major_axis_and_caps_taps() {
        // Minify x by 8, keep y: a = 8 texels along x, b = 1.
        let inv = Homography::affine(Affine::scale_non_uniform(8.0, 1.0));
        let f = footprint(&inv, [2.0, 3.0]).unwrap();
        assert_eq!(f.taps, 8);
        assert!(f.lod.abs() < 1e-12);
        assert!((f.step[0].abs() - 1.0).abs() < 1e-12 && f.step[1].abs() < 1e-12);
        // Anisotropy above 16 widens the minor footprint instead.
        let inv = Homography::affine(Affine::scale_non_uniform(64.0, 1.0));
        let f = footprint(&inv, [0.0, 0.0]).unwrap();
        assert_eq!(f.taps, 16);
        assert!((f.lod - 2.0).abs() < 1e-12);
        // Rounding noise on an isotropic map stays one tap; a real 1%
        // anisotropy takes two.
        let noisy = Homography([
            [1.095, 0.0, 3.0],
            [3.4e-17, 1.095, 3.0],
            [5.3e-19, 0.0, 1.0],
        ]);
        assert_eq!(footprint(&noisy, [140.5, 125.5]).unwrap().taps, 1);
        let inv = Homography::affine(Affine::scale_non_uniform(1.01, 1.0));
        assert_eq!(footprint(&inv, [0.0, 0.0]).unwrap().taps, 2);
    }
}
