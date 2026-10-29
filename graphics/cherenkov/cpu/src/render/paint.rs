//! Paint evaluation, matching `oracle/src/paint.rs` in f32: the paint is
//! sampled at the pixel centre mapped by the item's inverse transform into
//! content space, then premultiplied.

use std::mem::MaybeUninit;

use cherenkov::kurbo::Affine;
use cherenkov::{ColorStop, Extend, Interpolation, Paint};
use rustc_hash::FxHashMap;

use cherenkov::RenderError;
use fearless_simd::{Simd, SimdInterleaved, f32x4};
use filtrate_core::space::{P3_TO_SRGB, SRGB_TO_P3, srgb_decode, srgb_encode, transform};

use crate::names;

/// A gradient stop in interpolation-space straight-alpha components.
#[derive(Clone, Copy, Debug)]
pub struct Stop {
    /// The stop offset.
    pub offset: f32,
    /// Straight-alpha components in the interpolation space.
    pub color: [f32; 4],
}

/// Image-only mappings and retained pixels, kept out of ordinary paint storage.
#[derive(Clone, Debug)]
pub struct ImagePaintData {
    /// Content-to-image mapping retained at f64 precision.
    mapping: Affine,
    /// Device-to-image-pixel-space transform.
    pub(super) inv: [f32; 6],
    /// The registered image.
    pub(super) image: std::sync::Arc<super::image::CpuImage>,
    /// Horizontal continuation.
    pub(super) extend_x: Extend,
    /// Vertical continuation.
    pub(super) extend_y: Extend,
    /// Sampling.
    pub(super) sampling: cherenkov::Sampling,
}

/// A resolved paint: the inverse device-to-content transform plus the
/// parameters the evaluator needs.
#[derive(Clone, Debug)]
pub enum PaintData {
    /// A shared prepared paint, its shape-to-paint coordinate map and
    /// that map's f32 conversion behind the link's `Arc`, stored so
    /// evaluators never reconvert.
    Transformed(std::sync::Arc<TransformedPaint>),
    /// Prepared bilinear patches shared across placements.
    Mesh(super::mesh::Mesh),
    /// A registered image pattern.
    Image(Box<ImagePaintData>),
    /// A sweep (conic) gradient.
    Sweep(SweepPaint),
    /// Premultiplied solid colour.
    Solid([f32; 4]),
    /// A linear gradient.
    Linear(LinearPaint),
    /// A two-point radial gradient.
    Radial(RadialPaint),
}

/// A [`PaintData::Linear`] payload: the resolved parameters the
/// evaluators share, with the projection decided once at lowering.
#[derive(Clone, Debug)]
pub struct LinearPaint {
    /// Device-to-content transform.
    pub inv: [f32; 6],
    /// The projection's span constants; `None` for a degenerate
    /// gradient (`len2 == 0.0`), whose parameter is fixed at 0.
    pub project: Option<Project>,
    /// Sorted stops.
    pub stops: std::sync::Arc<[Stop]>,
    /// The continuation mode.
    pub extend: Extend,
    /// The interpolation space.
    pub interpolation: Interpolation,
}

/// A [`PaintData::Radial`] payload: the resolved parameters the
/// evaluators share, with the identical-circles decision made once at
/// lowering.
#[derive(Clone, Debug)]
pub struct RadialPaint {
    /// Device-to-content transform.
    pub inv: [f32; 6],
    /// The circle equation's paint-invariant coefficients.
    pub geometry: RadialGeometry,
    /// The circles are identical: the parameter is `-∞` inside `r0` and
    /// `+∞` outside.
    pub identical: bool,
    /// Sorted stops.
    pub stops: std::sync::Arc<[Stop]>,
    /// The continuation mode.
    pub extend: Extend,
    /// The interpolation space.
    pub interpolation: Interpolation,
}

/// A [`PaintData::Sweep`] payload: the resolved parameters the
/// evaluators share.
#[derive(Clone, Debug)]
pub struct SweepPaint {
    /// Device-to-content transform.
    pub inv: [f32; 6],
    /// The centre in content space.
    pub center: [f32; 2],
    /// Start angle in radians.
    pub start: f32,
    /// Angular span in radians, adjusted to `> 0` like the oracle.
    pub span: f32,
    /// Sorted stops.
    pub stops: std::sync::Arc<[Stop]>,
    /// The continuation mode.
    pub extend: Extend,
    /// The interpolation space.
    pub interpolation: Interpolation,
}

/// The content-space distance from the start centre past which the
/// radial solve divides the point and `r0` by that distance: below it
/// every term of the quadratic is far from f32 overflow and the solve
/// is the plain expression; above it, unscaled, `px²` and `4a·c`
/// overflow (near 1.8e19 and earlier) and the pixel drops out.
pub(super) const RADIAL_RESCALE_ABOVE: f32 = 65536.0;

/// The two-circle equation, prepared once when the paint is lowered.
#[derive(Clone, Copy, Debug)]
pub struct RadialGeometry {
    /// Start centre's x coordinate.
    pub x0: f32,
    /// Start centre's y coordinate.
    pub y0: f32,
    /// Start radius.
    pub r0: f32,
    /// Radius delta.
    pub dr: f32,
    /// Centre delta's x coordinate.
    pub dcx: f32,
    /// Centre delta's y coordinate.
    pub dcy: f32,
    /// Quadratic coefficient `dc·dc - dr²`.
    pub a: f32,
}

impl RadialGeometry {
    fn new(centres: [f32; 4], radii: [f32; 2]) -> Self {
        let (x0, y0) = (centres[0], centres[1]);
        let (dcx, dcy) = (centres[2] - x0, centres[3] - y0);
        let (r0, dr) = (radii[0], radii[1] - radii[0]);
        Self {
            x0,
            y0,
            r0,
            dr,
            dcx,
            dcy,
            a: dr.mul_add(-dr, dcy.mul_add(dcy, dcx * dcx)),
        }
    }
}

/// The non-degenerate linear projection's span constants.
#[derive(Clone, Copy, Debug)]
pub struct Project {
    /// The start point in content space.
    pub sx: f32,
    /// The start point's y.
    pub sy: f32,
    /// The start-to-end x delta.
    pub ddx: f32,
    /// The start-to-end y delta.
    pub ddy: f32,
    /// The line's squared length.
    pub len2: f32,
}

/// A [`PaintData::Transformed`] link's payload: the prepared inner
/// paint and its shape-to-paint coordinate map in both precisions.
/// Kept behind the variant's `Arc` so `PaintData` — and the lowered
/// item embedding it — stays a pointer wide.
#[derive(Clone, Debug)]
pub struct TransformedPaint {
    /// The prepared inner paint.
    pub paint: PaintData,
    /// The shape-to-paint coordinate map.
    pub inverse: Affine,
    /// `inverse` converted to f32 once at lowering.
    pub inverse_f32: [f32; 6],
}

/// An affine as six f32 coefficients `[a, b, c, d, e, f]`.
#[expect(clippy::cast_possible_truncation, reason = "geometry is f32")]
pub const fn affine_f32(t: Affine) -> [f32; 6] {
    let c = t.as_coeffs();
    [
        c[0] as f32,
        c[1] as f32,
        c[2] as f32,
        c[3] as f32,
        c[4] as f32,
        c[5] as f32,
    ]
}

/// Applies `[a, b, c, d, e, f]` to `(x, y)`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
const fn apply(m: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    (
        m[0].mul_add(x, m[2].mul_add(y, m[4])),
        m[1].mul_add(x, m[3].mul_add(y, m[5])),
    )
}

/// f64 to f32.
#[expect(clippy::cast_possible_truncation)]
const fn f32_f64(v: f64) -> f32 {
    v as f32
}

/// Converts stops into interpolation-space straight-alpha components,
/// sorted by offset.
fn stops(stops: &[ColorStop], interpolation: Interpolation) -> std::sync::Arc<[Stop]> {
    let mut sorted = stops.to_vec();
    sorted.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    sorted
        .iter()
        .map(|s| {
            let [r, g, b, a] = s.color.components;
            let color = if interpolation == Interpolation::SrgbEncoded {
                let [r, g, b] = transform(&P3_TO_SRGB, [r, g, b]).map(srgb_encode);
                [r, g, b, a]
            } else {
                [r, g, b, a]
            };
            Stop {
                offset: s.offset,
                color,
            }
        })
        .collect()
}

/// Samples an image at pixel-space `(u, v)` — the oracle's
/// `sample_image` in f32 (centre-based: texel centres at integer + 0.5).
/// The scalar evaluator's image arm; the span kernel is the lane-wise
/// `super::image` expression of the same formula, sharing
/// [`super::image::nearest_texel`]/[`super::image::linear_taps`]' index
/// math.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn sample_image(
    img: &super::image::CpuImage,
    u: f32,
    v: f32,
    sampling: cherenkov::Sampling,
) -> [f32; 4] {
    let (w, h) = (img.width as usize, img.height as usize);
    match sampling {
        cherenkov::Sampling::Nearest => {
            img.pixels[super::image::nearest_texel(v, h) * w + super::image::nearest_texel(u, w)]
        }
        cherenkov::Sampling::Linear => {
            let (x0, x1, tx) = super::image::linear_taps(u, w);
            let (y0, y1, ty) = super::image::linear_taps(v, h);
            let (c00, c10, c01, c11) = (
                img.pixels[y0 * w + x0],
                img.pixels[y0 * w + x1],
                img.pixels[y1 * w + x0],
                img.pixels[y1 * w + x1],
            );
            let mut out = [0.0; 4];
            for i in 0..4 {
                let top = tx.mul_add(c10[i] - c00[i], c00[i]);
                let bot = tx.mul_add(c11[i] - c01[i], c01[i]);
                out[i] = ty.mul_add(bot - top, top);
            }
            out
        }
    }
}

/// Lowers a `Paint` into [`PaintData`]. `inv` maps device space into the
/// content space the gradient parameters live in.
///
/// # Errors
/// [`RenderError::Render`] on invalid paint parameters,
/// [`RenderError::Image`] on a non-invertible image transform or an
/// unregistered image, [`RenderError::IdenticalRadialCircles`] when an
/// identical-circles radial repeats or reflects, and
/// [`RenderError::Unsupported`] for a shader paint.
#[expect(
    clippy::many_single_char_names,
    reason = "r/g/b/a are the channel names"
)]
pub fn paint_data(
    paint: &Paint,
    inv: Affine,
    images: &FxHashMap<u64, std::sync::Arc<super::image::CpuImage>>,
) -> Result<PaintData, RenderError> {
    Ok(match paint {
        Paint::Transformed(_) => {
            let mut transform = Affine::IDENTITY;
            let mut inner = paint;
            while let Paint::Transformed(mapped) = inner {
                if !mapped.transform.is_finite() || !mapped.transform.inverse().is_finite() {
                    return Err(RenderError::Render(
                        "paint transform must be finite and invertible".into(),
                    ));
                }
                transform *= mapped.transform;
                inner = &mapped.paint;
            }
            let inverse = transform.inverse();
            if !transform.is_finite() || !inverse.is_finite() {
                return Err(RenderError::Render(
                    "composed paint transform must be finite and invertible".into(),
                ));
            }
            if let Paint::Image(pattern) = inner {
                let mut pattern = pattern.clone();
                pattern.transform = transform * pattern.transform;
                return paint_data(&Paint::Image(pattern), inv, images);
            }
            if transform == Affine::IDENTITY {
                return paint_data(inner, inv, images);
            }
            let inner = paint_data(inner, Affine::IDENTITY, images)?;
            let inverse = inverse * inv;
            PaintData::Transformed(std::sync::Arc::new(TransformedPaint {
                paint: inner,
                inverse,
                inverse_f32: affine_f32(inverse),
            }))
        }
        Paint::Solid(c) => {
            let [r, g, b, a] = c.components;
            PaintData::Solid([r * a, g * a, b * a, a])
        }
        Paint::Linear(g) => {
            let (sx, sy, ex, ey) = (
                f32_f64(g.start.x),
                f32_f64(g.start.y),
                f32_f64(g.end.x),
                f32_f64(g.end.y),
            );
            let (ddx, ddy) = (ex - sx, ey - sy);
            let len2 = ddy.mul_add(ddy, ddx * ddx);
            PaintData::Linear(LinearPaint {
                inv: affine_f32(inv),
                project: (len2 != 0.0).then_some(Project {
                    sx,
                    sy,
                    ddx,
                    ddy,
                    len2,
                }),
                stops: stops(&g.stops, g.interpolation),
                extend: g.extend,
                interpolation: g.interpolation,
            })
        }
        Paint::Radial(g) => {
            let identical = g.has_identical_circles();
            if identical && matches!(g.extend, Extend::Repeat | Extend::Reflect) {
                return Err(RenderError::IdenticalRadialCircles(g.extend));
            }
            PaintData::Radial(RadialPaint {
                inv: affine_f32(inv),
                geometry: RadialGeometry::new(
                    [
                        f32_f64(g.start_center.x),
                        f32_f64(g.start_center.y),
                        f32_f64(g.end_center.x),
                        f32_f64(g.end_center.y),
                    ],
                    [f32_f64(g.start_radius), f32_f64(g.end_radius)],
                ),
                identical,
                stops: stops(&g.stops, g.interpolation),
                extend: g.extend,
                interpolation: g.interpolation,
            })
        }
        _ => return extra_paint(paint, inv, images),
    })
}

fn extra_paint(
    paint: &Paint,
    inv: Affine,
    images: &FxHashMap<u64, std::sync::Arc<super::image::CpuImage>>,
) -> Result<PaintData, RenderError> {
    Ok(match paint {
        Paint::Sweep(gradient) => {
            let raw_span = gradient.end_angle - gradient.start_angle;
            if !raw_span.is_finite()
                || !gradient.start_angle.is_finite()
                || !gradient.center.is_finite()
            {
                return Err(RenderError::Render(
                    "sweep angles and center must be finite".into(),
                ));
            }
            let span = if raw_span > 0.0 {
                raw_span
            } else {
                let wrapped = raw_span.rem_euclid(std::f64::consts::TAU);
                if wrapped == 0.0 {
                    std::f64::consts::TAU
                } else {
                    wrapped
                }
            };
            if !f32_f64(span).is_finite() || f32_f64(span) == 0.0 {
                return Err(RenderError::Render(
                    "sweep span exceeds raster precision".into(),
                ));
            }
            PaintData::Sweep(SweepPaint {
                inv: affine_f32(inv),
                center: [f32_f64(gradient.center.x), f32_f64(gradient.center.y)],
                start: f32_f64(gradient.start_angle.rem_euclid(std::f64::consts::TAU)),
                span: f32_f64(span),
                stops: stops(&gradient.stops, gradient.interpolation),
                extend: gradient.extend,
                interpolation: gradient.interpolation,
            })
        }
        Paint::Mesh(mesh) => {
            if mesh.points().iter().any(|point| !point.is_finite()) {
                return Err(RenderError::Render("mesh points must be finite".into()));
            }
            PaintData::Mesh(super::mesh::Mesh::new(mesh, inv))
        }
        Paint::Image(pattern) => {
            let image = images.get(&pattern.image.raw()).ok_or_else(|| {
                RenderError::Image(format!("unregistered image {}", pattern.image.raw()))
            })?;
            let mapping = pattern.transform.inverse();
            if !pattern.transform.is_finite() || !mapping.is_finite() {
                return Err(RenderError::Image(
                    "image transform must be finite and invertible".into(),
                ));
            }
            PaintData::Image(Box::new(ImagePaintData {
                mapping,
                inv: affine_f32(mapping * inv),
                image: std::sync::Arc::clone(image),
                extend_x: pattern.extend_x,
                extend_y: pattern.extend_y,
                sampling: pattern.sampling,
            }))
        }
        Paint::Shader(_) => return Err(RenderError::Unsupported(names::SHADER)),
        _ => unreachable!("common paint compiler handles solid, ordinary gradients and transforms"),
    })
}

/// Applies `extend` to `t`; `None` outside `[0, 1]` is transparent.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn extend_t(t: f32, extend: Extend) -> Option<f32> {
    match extend {
        Extend::Pad => Some(t.clamp(0.0, 1.0)),
        Extend::Repeat => Some(t - t.floor()),
        Extend::Reflect => {
            let m = (t * 0.5).floor().mul_add(-2.0, t);
            Some(if m > 1.0 { 2.0 - m } else { m })
        }
        Extend::None => (0.0..=1.0).contains(&t).then_some(t),
    }
}

/// Premultiplies interpolation-space straight components into linear P3.
#[expect(
    clippy::inline_always,
    reason = "keep premultiplication inside the dispatched per-pixel loop"
)]
#[inline(always)]
#[expect(
    clippy::many_single_char_names,
    reason = "r/g/b/a are the channel names"
)]
pub(super) fn to_premul(c: [f32; 4], interpolation: Interpolation) -> [f32; 4] {
    let [r, g, b, a] = c;
    let [pr, pg, pb] = if interpolation == Interpolation::SrgbEncoded {
        transform(&SRGB_TO_P3, [r, g, b].map(srgb_decode))
    } else {
        [r, g, b]
    };
    [pr * a, pg * a, pb * a, a]
}

/// Evaluates `stops` at `t`, returning premultiplied linear P3.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_stops(stops: &[Stop], t: f32, interpolation: Interpolation) -> [f32; 4] {
    let Some(first) = stops.first() else {
        return [0.0; 4];
    };
    if stops.len() == 1 || t <= first.offset {
        return to_premul(first.color, interpolation);
    }
    let last = &stops[stops.len() - 1];
    if t >= last.offset {
        return to_premul(last.color, interpolation);
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
            let (ca, cb) = (a.color, b.color);
            let mixed = [
                f.mul_add(cb[0] - ca[0], ca[0]),
                f.mul_add(cb[1] - ca[1], ca[1]),
                f.mul_add(cb[2] - ca[2], ca[2]),
                f.mul_add(cb[3] - ca[3], ca[3]),
            ];
            return to_premul(mixed, interpolation);
        }
    }
    to_premul(last.color, interpolation)
}

/// `t` along the two-point radial gradient for point `(px, py)` in content
/// space: the largest root whose radius `r0 + t·dr` is non-negative, `NaN`
/// when there is none. Identical circles give `-∞` on and inside the circle
/// and `+∞` outside, as the oracle's `radial_t` does.
#[allow(clippy::many_single_char_names)] // quadratic notation mirrors the spec
#[expect(
    clippy::suboptimal_flops,
    reason = "f32 squares are exact in f64, so the circle test needs no fused operation"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn radial_t(px: f32, py: f32, geometry: &RadialGeometry, identical: bool) -> f32 {
    let RadialGeometry {
        x0,
        y0,
        r0,
        dr,
        dcx,
        dcy,
        a,
    } = *geometry;
    let (px, py) = (px - x0, py - y0);
    if identical {
        // Squaring f32 coordinates in f64 cannot overflow. Comparing
        // squared distances avoids both a square root and a libm call.
        let (px, py, radius) = (f64::from(px), f64::from(py), f64::from(r0));
        return if radius >= 0.0 && px * px + py * py <= radius * radius {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        };
    }
    // The span kernel's lane scale: past `RADIAL_RESCALE_ABOVE` the
    // solve runs on `p/s` and `r0/s`, so `b`, `c` and the discriminant
    // stay finite, and the root is scaled back.
    // A NaN coordinate makes the solve NaN regardless of the scale,
    // so this comparison need not implement max's NaN-ignoring rule.
    let (ax, ay) = (px.abs(), py.abs());
    let m = if ax > ay { ax } else { ay };
    let (px, py, r0, s) = if m > RADIAL_RESCALE_ABOVE {
        let inv = 1.0 / m;
        (px * inv, py * inv, r0 * inv, m)
    } else {
        (px, py, r0, 1.0)
    };
    // Write the equation as a*t² - 2*b*t + c = 0. Its half-linear
    // coefficient avoids the cancelling factors of two in the roots.
    let b = r0.mul_add(dr, dcy.mul_add(py, dcx * px));
    let c = r0.mul_add(-r0, py.mul_add(py, px * px));
    if a.abs() < 1e-12 {
        let b = 2.0 * b;
        if (b * s).abs() < 1e-12 {
            return f32::NAN;
        }
        let t = c / b;
        return if t.mul_add(dr, r0) >= 0.0 {
            t * s
        } else {
            f32::NAN
        };
    }
    let disc = a.mul_add(-c, b * b);
    if disc < 0.0 {
        return f32::NAN;
    }
    // The denominator's sign orders the roots. Evaluate the larger
    // root first, and only divide for the smaller root if needed.
    // A NaN larger root also reaches the smaller root, just as max/min
    // ignore a NaN operand in the vector solver.
    let sq = disc.sqrt().copysign(a);
    let hi = (b + sq) / a;
    if hi.mul_add(dr, r0) >= 0.0 {
        return hi * s;
    }
    let lo = (b - sq) / a;
    if lo.mul_add(dr, r0) >= 0.0 {
        lo * s
    } else {
        f32::NAN
    }
}

impl PaintData {
    /// An image paint over `image` sampled through `transform`
    /// (image-space to device), `Pad` extend and linear sampling — the
    /// bitmap present path.
    #[must_use]
    pub fn bitmap(image: std::sync::Arc<super::image::CpuImage>, transform: Affine) -> Self {
        let mapping = transform.inverse();
        Self::Image(Box::new(ImagePaintData {
            mapping,
            inv: affine_f32(mapping),
            image,
            extend_x: Extend::Pad,
            extend_y: Extend::Pad,
            sampling: cherenkov::Sampling::Linear,
        }))
    }

    /// Apply sampled device placement without resolving or copying gradient stops.
    #[must_use]
    pub fn transformed(&self, transform: Affine) -> Self {
        let mut paint = self.clone();
        match &mut paint {
            Self::Transformed(link) => {
                return link.paint.transformed(link.inverse * transform);
            }
            Self::Solid(_) => {}
            Self::Mesh(mesh) => *mesh = mesh.transformed(transform),
            Self::Image(image) => image.inv = affine_f32(image.mapping * transform),
            Self::Linear(linear) => linear.inv = affine_f32(transform),
            Self::Radial(radial) => radial.inv = affine_f32(transform),
            Self::Sweep(sweep) => sweep.inv = affine_f32(transform),
        }
        paint
    }

    /// Evaluates the paint at a device-space pixel centre in premultiplied linear P3.
    #[cfg(test)]
    #[must_use]
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    pub fn eval(&self, dx: f32, dy: f32) -> [f32; 4] {
        self.evaluate(PixelCenter { dx, dy })
    }

    // Select the paint formula once, before the output iterates over pixel centres.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn evaluate<O: PaintOutput>(&self, output: O) -> O::Output {
        if matches!(self, Self::Transformed(..)) {
            let mut leaf = self;
            while let Self::Transformed(link) = leaf {
                leaf = &link.paint;
            }
            leaf.evaluate_leaf(TransformedOutput {
                output,
                paint: self,
            })
        } else {
            self.evaluate_leaf(output)
        }
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn evaluate_leaf<O: PaintOutput>(&self, output: O) -> O::Output {
        match self {
            Self::Transformed(..) => unreachable!("evaluate resolves the transform chain"),
            Self::Solid(c) => output.solid(*c),
            Self::Mesh(mesh) => output.sample(|dx, dy| mesh.eval(dx, dy)),
            Self::Image(image) => output.image(self, image),
            Self::Sweep(p) => output.sweep(self, p),

            Self::Linear(p) => output.linear(self, p),
            Self::Radial(p) => output.radial(self, p),
        }
    }

    /// Evaluates pixel centres into `out`, agreeing with [`Self::eval`] to
    /// within rounding; a pixel never depends on its span alignment.
    /// Every element is initialized. Solid spans use four-pixel native stores;
    /// other paints select their scalar formula once for the whole span.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    pub fn eval_span<'a, S: Simd>(
        &self,
        simd: S,
        x0: usize,
        py: usize,
        out: &'a mut [MaybeUninit<[f32; 4]>],
    ) -> &'a mut [[f32; 4]] {
        self.evaluate(PixelSpan { simd, x0, py, out })
    }
}

// The output owns iteration and initialization; paint formulas are shared by
// scalar sampling and span sampling without a per-pixel enum dispatch.
trait PaintOutput: Sized {
    type Output;

    fn sample(self, evaluate: impl FnMut(f32, f32) -> [f32; 4]) -> Self::Output;
    fn solid(self, color: [f32; 4]) -> Self::Output;

    /// The linear-gradient leaf: `outer` supplies the `Transformed`
    /// chain whose inverses apply before `params.inv` — for a leaf
    /// reached directly it is the leaf itself and the walk finds no
    /// links.
    fn linear(self, outer: &PaintData, params: &LinearPaint) -> Self::Output;

    /// The radial-gradient leaf, `outer` as in [`Self::linear`].
    fn radial(self, outer: &PaintData, params: &RadialPaint) -> Self::Output;

    /// The sweep-gradient leaf, `outer` as in [`Self::linear`].
    fn sweep(self, outer: &PaintData, params: &SweepPaint) -> Self::Output;

    /// The image leaf, `outer` supplying the `Transformed` chain as in
    /// [`PaintOutput::linear`].
    fn image(self, outer: &PaintData, image: &ImagePaintData) -> Self::Output;
}

/// The image leaf's scalar evaluation: `outer`'s transform chain, then
/// `image`'s map, the per-axis extends and the sampling — the same
/// expression the span kernel evaluates per lane, and the one-pixel
/// path for spans shorter than one block.
#[expect(
    clippy::cast_precision_loss,
    reason = "image dimensions are bounded by memory budget"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn image_at(outer: &PaintData, image: &ImagePaintData, mut x: f32, mut y: f32) -> [f32; 4] {
    let mut link = outer;
    while let PaintData::Transformed(inner) = link {
        (x, y) = apply(inner.inverse_f32, x, y);
        link = &inner.paint;
    }
    let (qx, qy) = apply(image.inv, x, y);
    let (iw, ih) = (image.image.width as f32, image.image.height as f32);
    let Some(u) = extend_t(qx / iw, image.extend_x).map(|t| t * iw) else {
        return [0.0; 4];
    };
    let Some(v) = extend_t(qy / ih, image.extend_y).map(|t| t * ih) else {
        return [0.0; 4];
    };
    sample_image(&image.image, u, v, image.sampling)
}

/// The linear leaf's scalar evaluation: `outer`'s transform chain, then
/// `params`' map, projection, extend and stops — the same expression the
/// span kernel evaluates per lane.
#[cfg(test)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn linear_at(outer: &PaintData, params: &LinearPaint, mut x: f32, mut y: f32) -> [f32; 4] {
    let mut link = outer;
    while let PaintData::Transformed(inner) = link {
        (x, y) = apply(inner.inverse_f32, x, y);
        link = &inner.paint;
    }
    let (px, py) = apply(params.inv, x, y);
    let t = params.project.map_or(0.0, |p| {
        (py - p.sy).mul_add(p.ddy, (px - p.sx) * p.ddx) / p.len2
    });
    extend_t(t, params.extend).map_or([0.0; 4], |t| {
        eval_stops(&params.stops, t, params.interpolation)
    })
}

/// The radial leaf's scalar evaluation: `outer`'s transform chain, then
/// `params`' map, `radial_t`, extend and stops — the same expression the
/// span kernel evaluates per lane, including the no-solution `NaN`
/// mapping to transparent before extension.
#[expect(
    clippy::option_if_let_else,
    reason = "keep stop interpolation inside the dispatched target features, without a closure boundary"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn radial_at(outer: &PaintData, params: &RadialPaint, mut x: f32, mut y: f32) -> [f32; 4] {
    let mut link = outer;
    while let PaintData::Transformed(inner) = link {
        (x, y) = apply(inner.inverse_f32, x, y);
        link = &inner.paint;
    }
    let (px, py) = apply(params.inv, x, y);
    let t = radial_t(px, py, &params.geometry, params.identical);
    if t.is_nan() {
        return [0.0; 4];
    }
    match extend_t(t, params.extend) {
        Some(t) => eval_stops(&params.stops, t, params.interpolation),
        None => [0.0; 4],
    }
}

/// The shared `atan2` on one pixel, for the scalar reference: the
/// kernel's polynomial at `Fallback` precision, whose multiply-adds
/// are unfused. It agrees with the dispatched kernel within rounding.
#[cfg(test)]
fn atan2_f32(y: f32, x: f32) -> f32 {
    let simd = fearless_simd::Fallback::new();
    cherenkov_simd::math::atan2(simd, simd.splat_f32x4(y), simd.splat_f32x4(x))[0]
}

/// The sweep leaf's scalar reference: `outer`'s transform chain, then
/// `params`' map, [`atan2_f32`], angular wrap, extend and stops. It
/// agrees with the span kernel within rounding.
#[cfg(test)]
fn sweep_at(outer: &PaintData, params: &SweepPaint, mut x: f32, mut y: f32) -> [f32; 4] {
    let mut link = outer;
    while let PaintData::Transformed(inner) = link {
        (x, y) = apply(inner.inverse_f32, x, y);
        link = &inner.paint;
    }
    let (px, py) = apply(params.inv, x, y);
    let raw = atan2_f32(py - params.center[1], px - params.center[0]) - params.start;
    // Lowering normalizes start to [0, TAU]; atan2 is in [-PI, PI].
    // At most two additions wrap the angle without a per-pixel fmod.
    let turn = std::f32::consts::TAU;
    let raw = if raw < -turn { raw + turn } else { raw };
    let angle = if raw < 0.0 { raw + turn } else { raw };
    let t = angle / params.span;
    extend_t(t, params.extend).map_or([0.0; 4], |t| {
        eval_stops(&params.stops, t, params.interpolation)
    })
}

/// Applies each mapping separately, preserving the scalar evaluator's f32
/// rounding at every link. The terminal paint formula is already selected.
struct TransformedOutput<'a, O> {
    output: O,
    paint: &'a PaintData,
}

impl<O: PaintOutput> PaintOutput for TransformedOutput<'_, O> {
    type Output = O::Output;

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn sample(self, mut evaluate: impl FnMut(f32, f32) -> [f32; 4]) -> Self::Output {
        self.output.sample(|mut x, mut y| {
            let mut paint = self.paint;
            while let PaintData::Transformed(inner) = paint {
                (x, y) = apply(inner.inverse_f32, x, y);
                paint = &inner.paint;
            }
            evaluate(x, y)
        })
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn solid(self, color: [f32; 4]) -> Self::Output {
        self.output.solid(color)
    }

    /// The chain lives in `self.paint`; the leaf's own `self` carries
    /// none — forward the transform-owning paint.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn linear(self, _outer: &PaintData, params: &LinearPaint) -> Self::Output {
        self.output.linear(self.paint, params)
    }

    /// As [`Self::linear`].
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn radial(self, _outer: &PaintData, params: &RadialPaint) -> Self::Output {
        self.output.radial(self.paint, params)
    }

    /// As [`Self::linear`].
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn sweep(self, _outer: &PaintData, params: &SweepPaint) -> Self::Output {
        self.output.sweep(self.paint, params)
    }

    /// As [`Self::linear`].
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn image(self, _outer: &PaintData, image: &ImagePaintData) -> Self::Output {
        self.output.image(self.paint, image)
    }
}

#[cfg(test)]
struct PixelCenter {
    dx: f32,
    dy: f32,
}

#[cfg(test)]
impl PaintOutput for PixelCenter {
    type Output = [f32; 4];

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn sample(self, mut evaluate: impl FnMut(f32, f32) -> [f32; 4]) -> Self::Output {
        evaluate(self.dx, self.dy)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn solid(self, color: [f32; 4]) -> Self::Output {
        color
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn linear(self, outer: &PaintData, params: &LinearPaint) -> Self::Output {
        linear_at(outer, params, self.dx, self.dy)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn radial(self, outer: &PaintData, params: &RadialPaint) -> Self::Output {
        radial_at(outer, params, self.dx, self.dy)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn sweep(self, outer: &PaintData, params: &SweepPaint) -> Self::Output {
        sweep_at(outer, params, self.dx, self.dy)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn image(self, outer: &PaintData, image: &ImagePaintData) -> Self::Output {
        image_at(outer, image, self.dx, self.dy)
    }
}

/// The mesh and short-run sampling loop. Debug builds keep it out of
/// line: unoptimized code gives every inlined body its own stack
/// slots, and inlined next to every other kind in
/// `PaintData::eval_span` it overflowed the test stack. Optimized
/// builds inline it into the caller's dispatched `vectorize` region,
/// since the optimizer coalesces the frame and a call per span costs
/// short spans.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel indices and band offsets are far below 2^24"
)]
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(
    not(debug_assertions),
    expect(
        clippy::inline_always,
        reason = "an optimized build coalesces the frame, and a call per span costs short spans"
    ),
    inline(always)
)]
fn sample_span(
    x0: usize,
    py: usize,
    out: &mut [MaybeUninit<[f32; 4]>],
    mut evaluate: impl FnMut(f32, f32) -> [f32; 4],
) -> &mut [[f32; 4]] {
    let y = py as f32 + 0.5;
    let mut x = x0 as f32 + 0.5;
    for pixel in out.iter_mut() {
        pixel.write(evaluate(x, y));
        x += 1.0;
    }
    // SAFETY: every element was written by the loop above.
    unsafe { out.assume_init_mut() }
}

/// The solid fill, out of line in debug builds for the same reason as
/// [`sample_span`]. Entered through its own `vectorize`, which
/// isolates it from the paint evaluators' register pressure.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(
    not(debug_assertions),
    expect(
        clippy::inline_always,
        reason = "an optimized build coalesces the frame, and a call per span costs short spans"
    ),
    inline(always)
)]
fn solid_span<S: Simd>(simd: S, c: [f32; 4], out: &mut [MaybeUninit<[f32; 4]>]) -> &mut [[f32; 4]] {
    simd.vectorize(
        #[inline(always)]
        || {
            let planes = [
                simd.splat_f32x4(c[0]),
                simd.splat_f32x4(c[1]),
                simd.splat_f32x4(c[2]),
                simd.splat_f32x4(c[3]),
            ];
            let mut block = [0.0f32; 16];
            f32x4::store_four_interleaved(planes, &mut block);
            let count = out.len();
            let mut pos = 0;
            while pos + 4 <= count {
                // SAFETY: `out[pos..pos + 4]` is four contiguous
                // `MaybeUninit<[f32; 4]>` — 16 floats — and `block`
                // holds those 16 initialised floats.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        block.as_ptr(),
                        out[pos..].as_mut_ptr().cast::<f32>(),
                        16,
                    );
                }
                pos += 4;
            }
            for px in &mut out[pos..] {
                px.write(c);
            }
            // SAFETY: the four-pixel stores and scalar tail initialized every element.
            unsafe { out.assume_init_mut() }
        },
    )
}

struct PixelSpan<'a, S> {
    simd: S,
    x0: usize,
    py: usize,
    out: &'a mut [MaybeUninit<[f32; 4]>],
}

impl<'a, S: Simd> PaintOutput for PixelSpan<'a, S> {
    type Output = &'a mut [[f32; 4]];

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn sample(self, evaluate: impl FnMut(f32, f32) -> [f32; 4]) -> Self::Output {
        sample_span(self.x0, self.py, self.out, evaluate)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn solid(self, c: [f32; 4]) -> Self::Output {
        solid_span(self.simd, c, self.out)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn linear(self, outer: &PaintData, params: &LinearPaint) -> Self::Output {
        super::gradient::linear_span(self.simd, outer, params, self.x0, self.py, self.out)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn radial(self, outer: &PaintData, params: &RadialPaint) -> Self::Output {
        if self.out.len() < super::span::LANES {
            self.sample(
                #[inline(always)]
                |x, y| radial_at(outer, params, x, y),
            )
        } else {
            super::gradient::radial_span(self.simd, outer, params, self.x0, self.py, self.out)
        }
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn sweep(self, outer: &PaintData, params: &SweepPaint) -> Self::Output {
        super::gradient::sweep_span(self.simd, outer, params, self.x0, self.py, self.out)
    }

    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn image(self, outer: &PaintData, image: &ImagePaintData) -> Self::Output {
        if self.out.len() < super::span::LANES {
            self.sample(
                #[inline(always)]
                |x, y| image_at(outer, image, x, y),
            )
        } else {
            super::image::image_span(self.simd, outer, image, self.x0, self.py, self.out)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::MaybeUninit;
    use std::sync::Arc;

    use cherenkov::kurbo::Affine;
    use cherenkov::{
        ColorStop, Extend, ImageColorSpace, ImageFormat, ImageId, ImagePattern, ImageUpload,
        Interpolation, LinearGradient, Paint, RadialGradient, Sampling, SweepGradient,
        TransformedPaint, WorkingColor,
    };

    use super::PaintData;
    use crate::testing::kernels;

    const IMAGE: ImageId = ImageId::new(1);

    fn stops() -> Vec<ColorStop> {
        [
            (0.0, [0.9, 0.1, 0.2, 1.0]),
            (0.35, [0.1, 0.8, 0.3, 1.0]),
            (0.7, [0.2, 0.3, 0.9, 0.8]),
            (1.0, [0.05, 0.05, 0.4, 1.0]),
        ]
        .into_iter()
        .map(|(offset, color)| ColorStop {
            offset,
            color: WorkingColor::new(color),
        })
        .collect()
    }

    fn linear() -> LinearGradient {
        let mut gradient = LinearGradient::new((0.0, 0.0), (1024.0, 0.0));
        gradient.stops = stops();
        gradient
    }

    fn sweep() -> SweepGradient {
        let mut gradient = SweepGradient::new((512.0, 256.0), 0.0, std::f64::consts::TAU);
        gradient.stops = stops();
        gradient
    }

    /// A deterministic `width`×`height` `LinearP3` upload whose texels
    /// all differ.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "texel bytes are taken mod 256"
    )]
    fn upload(width: u32, height: u32) -> ImageUpload {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[
                    (x * 37 % 256) as u8,
                    (y * 53 % 256) as u8,
                    ((x + y) * 19 % 256) as u8,
                    255,
                ]);
            }
        }
        ImageUpload {
            width,
            height,
            data: data.into(),
            color_space: ImageColorSpace::LinearP3,
            premultiplied: false,
            format: ImageFormat::Rgba8,
        }
    }

    /// The linear-gradient matrix the span kernel must match lane for
    /// lane: every extend mode and interpolation space, a diagonal and a
    /// zero-length line, stops sharing one offset, stops outside
    /// `[0, 1]`, a single stop and none, and inverse maps whose
    /// parameter overflows — huge magnitudes, `±∞` and `NaN` mid-span.
    fn linear_cases() -> Vec<PaintData> {
        let lower =
            |paint: &Paint, inv: Affine| kernels::paint(paint, inv, []).expect("test paints lower");
        let mut cases = Vec::new();
        for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect, Extend::None] {
            for interpolation in [Interpolation::Working, Interpolation::SrgbEncoded] {
                let mut gradient = linear();
                gradient.extend = extend;
                gradient.interpolation = interpolation;
                cases.push(lower(&Paint::Linear(gradient), Affine::IDENTITY));
            }
            // A diagonal line exercises both projection terms.
            let mut diagonal = LinearGradient::new((64.0, 8.0), (128.0, 40.0));
            diagonal.stops = stops();
            diagonal.extend = extend;
            cases.push(lower(&Paint::Linear(diagonal), Affine::IDENTITY));
            // A zero-length line: `len2 == 0.0` fixes the parameter at 0.
            let mut degenerate = LinearGradient::new((32.0, 13.0), (32.0, 13.0));
            degenerate.stops = stops();
            degenerate.extend = extend;
            cases.push(lower(&Paint::Linear(degenerate), Affine::IDENTITY));
        }
        // Two stops sharing an offset: the zero-span window's `f == 0`.
        let mut shared = linear();
        shared.stops = [
            (0.0, [0.9, 0.1, 0.2, 1.0]),
            (0.35, [0.1, 0.8, 0.3, 1.0]),
            (0.35, [0.6, 0.2, 0.7, 0.5]),
            (1.0, [0.05, 0.05, 0.4, 1.0]),
        ]
        .into_iter()
        .map(|(offset, color)| ColorStop {
            offset,
            color: WorkingColor::new(color),
        })
        .collect();
        cases.push(lower(&Paint::Linear(shared), Affine::IDENTITY));
        // Stop offsets outside `[0, 1]` and a single-stop gradient.
        for offsets in [[-0.5, 0.5, 1.5].as_slice(), &[0.4]] {
            let mut gradient = linear();
            gradient.stops = stops()
                .into_iter()
                .zip(offsets)
                .map(|(mut stop, &offset)| {
                    stop.offset = offset;
                    stop
                })
                .collect();
            cases.push(lower(&Paint::Linear(gradient), Affine::IDENTITY));
        }
        cases.push(lower(&Paint::Linear(linear()), Affine::IDENTITY));
        let mut empty = linear();
        empty.stops = Vec::new();
        cases.push(lower(&Paint::Linear(empty), Affine::IDENTITY));
        // Inverse scales carrying the projection to finite overflow,
        // `±∞` and `NaN` — and mixing all of them inside one block.
        for scale in [1e-30, 1e-37, 1e-38, -1e-38] {
            for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect, Extend::None] {
                let mut gradient = linear();
                gradient.extend = extend;
                cases.push(lower(&Paint::Linear(gradient), Affine::scale(scale)));
            }
        }
        cases
    }

    /// A two-point radial with moving centres — `b` and `c` vary per
    /// lane and the row crosses the cone's no-solution region.
    fn radial() -> RadialGradient {
        RadialGradient {
            start_center: (30.0, 13.5).into(),
            start_radius: 12.0,
            end_center: (150.0, 13.5).into(),
            end_radius: 70.0,
            stops: stops(),
            extend: Extend::Pad,
            interpolation: Interpolation::Working,
        }
    }

    /// A radial whose end circle contains the start: `a < 0`, and at
    /// `px = ±∞` the quadratic's roots split into one infinite and one
    /// NaN root.
    fn radial_contained() -> RadialGradient {
        RadialGradient {
            start_center: (30.0, 13.5).into(),
            start_radius: 5.0,
            end_center: (40.0, 13.5).into(),
            end_radius: 70.0,
            stops: stops(),
            extend: Extend::Pad,
            interpolation: Interpolation::Working,
        }
    }

    /// The radial-gradient matrix the span kernel must match lane for
    /// lane: every extend mode and interpolation space, the identical-
    /// circles hard edge, the degenerate `|a| < 1e-12` linear
    /// fallback, stop lists shared with `linear_cases`, and inverse
    /// maps carrying the parameter to `±∞` and `NaN` mid-span.
    fn radial_cases() -> Vec<PaintData> {
        let lower =
            |paint: &Paint, inv: Affine| kernels::paint(paint, inv, []).expect("test paints lower");
        let mut cases = Vec::new();
        for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect, Extend::None] {
            for interpolation in [Interpolation::Working, Interpolation::SrgbEncoded] {
                let mut gradient = radial();
                gradient.extend = extend;
                gradient.interpolation = interpolation;
                cases.push(lower(&Paint::Radial(gradient), Affine::IDENTITY));
            }
            // Identical circles: −∞ inside the circle, +∞ outside —
            // Repeat and Reflect fail at lowering, so only Pad and
            // None reach the kernel.
            for edge in [Extend::Pad, Extend::None] {
                let mut identical = radial();
                identical.end_center = identical.start_center;
                identical.end_radius = identical.start_radius;
                identical.extend = edge;
                cases.push(lower(&Paint::Radial(identical), Affine::IDENTITY));
            }
            // `dcx² + dcy² == dr²`: the quadratic degenerates to the
            // `-c / b` linear solve.
            let mut degenerate = radial();
            degenerate.start_center = (0.0, 0.0).into();
            degenerate.start_radius = 0.0;
            degenerate.end_center = (100.0, 0.0).into();
            degenerate.end_radius = 100.0;
            degenerate.extend = extend;
            cases.push(lower(&Paint::Radial(degenerate), Affine::IDENTITY));
        }
        // A shrinking radius can reject the larger root and accept
        // the smaller one, for either sign of the quadratic coefficient.
        for mut gradient in [radial(), radial_contained()] {
            std::mem::swap(&mut gradient.start_center, &mut gradient.end_center);
            std::mem::swap(&mut gradient.start_radius, &mut gradient.end_radius);
            cases.push(lower(&Paint::Radial(gradient), Affine::IDENTITY));
        }
        // A `Transformed` chain over a radial gradient.
        cases.push(lower(
            &Paint::Transformed(TransformedPaint::new(
                Paint::Radial(radial()),
                Affine::translate((4.0, -2.0)) * Affine::rotate(0.2),
            )),
            Affine::IDENTITY,
        ));
        // X-only inverse scales carrying `cx` past every unscaled
        // term's f32 range and to `±∞` mid-span, mixing rescaled,
        // unsolvable and infinite-parameter lanes inside one block,
        // over both circle-containment orders; `-1e38` on the
        // containing geometry splits the roots into one `NaN` and one
        // `+∞`, the `max`/`min` NaN case.
        for scale in RADIAL_SCALES {
            for base in [radial(), radial_contained()] {
                for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect, Extend::None] {
                    let mut gradient = base.clone();
                    gradient.extend = extend;
                    cases.push(lower(
                        &Paint::Radial(gradient),
                        Affine::scale_non_uniform(scale, 1.0),
                    ));
                }
            }
        }
        cases
    }

    /// The device→content x scales of the large-scale radial cases.
    const RADIAL_SCALES: [f64; 5] = [1e18, 1e19, 1e37, 1e38, -1e38];

    /// At content scales where the unscaled quadratic's terms overflow
    /// f32, a radial pixel paints exactly where the f64 oracle's
    /// parameter exists — over every pixel whose f32 content point is
    /// finite, under the detected level and the scalar fallback.
    #[test]
    fn radial_large_scale_paints_where_the_oracle_does() {
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            reason = "pixel indices are far below 2^24, and the f32 product is the kernel's own content point"
        )]
        fn check<S: fearless_simd::Simd>(
            simd: S,
            paint: &PaintData,
            oracle: &cherenkov_scene::RadialGradient,
            scale: f64,
        ) {
            let mut pixels = Vec::new();
            for len in [1, 15, REFERENCE_LEN] {
                for x0 in (0..REFERENCE_LEN).step_by(len) {
                    let count = len.min(REFERENCE_LEN - x0);
                    for (i, pixel) in span(simd, paint, x0, count).into_iter().enumerate() {
                        pixels.push((x0 + i, pixel));
                    }
                }
            }
            let y = PY as f64 + 0.5;
            for (x, px) in pixels {
                let x = x as f64 + 0.5;
                if !((scale as f32) * (x as f32)).is_finite() {
                    continue;
                }
                let t = cherenkov_oracle::paint::radial_t((scale * x, y).into(), oracle);
                assert_eq!(
                    px[3] > 0.0,
                    !t.is_nan(),
                    "scale {scale:e}, {oracle:?}, x={x}: alpha {} where the oracle's t is {t}",
                    px[3]
                );
            }
        }
        for scale in RADIAL_SCALES {
            for base in [radial(), radial_contained()] {
                let paint = kernels::paint(
                    &Paint::Radial(base.clone()),
                    Affine::scale_non_uniform(scale, 1.0),
                    [],
                )
                .expect("test paints lower");
                let oracle = cherenkov_scene::RadialGradient {
                    center0: (base.start_center.x, base.start_center.y).into(),
                    r0: base.start_radius,
                    center1: (base.end_center.x, base.end_center.y).into(),
                    r1: base.end_radius,
                    stops: Vec::new(),
                    extend: cherenkov_scene::Extend::Pad,
                    interpolation: cherenkov_scene::ColorSpace::default(),
                };
                cherenkov_simd::dispatch!(cherenkov_simd::detect(), simd => check(simd, &paint, &oracle, scale));
                check(fearless_simd::Fallback::new(), &paint, &oracle, scale);
            }
        }
    }

    /// The sweep-gradient matrix the span kernel must match within
    /// rounding: every extend mode and interpolation space, a non-zero
    /// start angle and a partial span, an off-axis centre, stops
    /// sharing one offset, stops outside `[0, 1]`, a single stop and
    /// none, a transform chain, and inverse maps whose mapped point
    /// underflows to the centre's `atan2(±0, ±0)` corner or lands in a
    /// negative quadrant.
    fn sweep_cases() -> Vec<PaintData> {
        let lower =
            |paint: &Paint, inv: Affine| kernels::paint(paint, inv, []).expect("test paints lower");
        let mut cases = Vec::new();
        for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect, Extend::None] {
            for interpolation in [Interpolation::Working, Interpolation::SrgbEncoded] {
                let mut gradient = sweep();
                gradient.extend = extend;
                gradient.interpolation = interpolation;
                cases.push(lower(&Paint::Sweep(gradient), Affine::IDENTITY));
            }
            // A non-zero start and a partial span exercise both wraps.
            let mut partial = SweepGradient::new((64.0, 8.0), 0.7, 4.0);
            partial.stops = stops();
            partial.extend = extend;
            cases.push(lower(&Paint::Sweep(partial), Affine::IDENTITY));
        }
        // Two stops sharing an offset: the zero-span window's `f == 0`.
        let mut shared = sweep();
        shared.stops = [
            (0.0, [0.9, 0.1, 0.2, 1.0]),
            (0.35, [0.1, 0.8, 0.3, 1.0]),
            (0.35, [0.6, 0.2, 0.7, 0.5]),
            (1.0, [0.05, 0.05, 0.4, 1.0]),
        ]
        .into_iter()
        .map(|(offset, color)| ColorStop {
            offset,
            color: WorkingColor::new(color),
        })
        .collect();
        cases.push(lower(&Paint::Sweep(shared), Affine::IDENTITY));
        // Stop offsets outside `[0, 1]` and a single-stop gradient.
        for offsets in [[-0.5, 0.5, 1.5].as_slice(), &[0.4]] {
            let mut gradient = sweep();
            gradient.stops = stops()
                .into_iter()
                .zip(offsets)
                .map(|(mut stop, &offset)| {
                    stop.offset = offset;
                    stop
                })
                .collect();
            cases.push(lower(&Paint::Sweep(gradient), Affine::IDENTITY));
        }
        cases.push(lower(&Paint::Sweep(sweep()), Affine::IDENTITY));
        let mut empty = sweep();
        empty.stops = Vec::new();
        cases.push(lower(&Paint::Sweep(empty), Affine::IDENTITY));
        // A transform chain over a sweep leaf.
        cases.push(lower(
            &Paint::Transformed(TransformedPaint::new(
                Paint::Sweep(sweep()),
                Affine::translate((20.0, 5.0)) * Affine::rotate(0.3),
            )),
            Affine::IDENTITY,
        ));
        // Inverse scales mapping every device point to the centre or a
        // negative quadrant: the `atan2(±0, ±0)` and `|y| > |x|`
        // corners, and a negative start angle.
        let mut at_centre = SweepGradient::new((0.0, 0.0), -1.0, std::f64::consts::TAU);
        at_centre.stops = stops();
        for scale in [0.0, 1e-38, -1e-38] {
            cases.push(lower(
                &Paint::Sweep(at_centre.clone()),
                Affine::scale(scale),
            ));
        }
        cases
    }

    const STRIP_W: ImageId = ImageId::new(2);
    const STRIP_H: ImageId = ImageId::new(3);
    const SINGLE: ImageId = ImageId::new(4);

    /// Image paints over the 64×64 image and the degenerate 1×1, 8×1
    /// and 1×8 images: one transform whose sample coordinates cross
    /// every image edge on the test row, under four extend pairs
    /// covering each mode on each axis; the transform shapes under
    /// pad; the degenerate images under all four modes — both
    /// samplings each.
    fn image_cases() -> Vec<PaintData> {
        let uploads = [
            (IMAGE.raw(), upload(64, 64)),
            (STRIP_W.raw(), upload(8, 1)),
            (STRIP_H.raw(), upload(1, 8)),
            (SINGLE.raw(), upload(1, 1)),
        ];
        let images: Vec<_> = uploads
            .iter()
            .map(|(id, up)| (*id, Arc::new(kernels::image(up).unwrap())))
            .collect();
        let lower =
            |paint: &Paint| kernels::paint(paint, Affine::IDENTITY, images.clone()).unwrap();
        let pattern = |image: ImageId,
                       transform: Affine,
                       extend_x: Extend,
                       extend_y: Extend,
                       sampling: Sampling| {
            Paint::Image(ImagePattern {
                image,
                transform,
                extend_x,
                extend_y,
                sampling,
            })
        };
        let mut paints = Vec::new();
        // One transform whose sample coordinates cross all four image
        // edges on the test row — u ∈ [-101, 141], v ∈ [-112, 92] —
        // under four extend pairs covering each mode on each axis.
        let all_edges = Affine::translate((40.0, 13.5))
            * Affine::scale(0.25)
            * Affine::rotate(-0.7)
            * Affine::translate((-20.0, 10.0));
        for (extend_x, extend_y) in [
            (Extend::Pad, Extend::Repeat),
            (Extend::Repeat, Extend::Reflect),
            (Extend::Reflect, Extend::None),
            (Extend::None, Extend::Pad),
        ] {
            for sampling in [Sampling::Nearest, Sampling::Linear] {
                paints.push(lower(&pattern(
                    IMAGE, all_edges, extend_x, extend_y, sampling,
                )));
            }
        }
        // Transform shapes under pad.
        for transform in [
            Affine::IDENTITY,
            Affine::scale(0.25),
            Affine::scale(4.0),
            Affine::rotate(0.3),
            Affine::skew(0.3, -0.2),
            Affine::translate((-3.0, -3.0)),
            Affine::translate((4.0, 4.0)) * Affine::rotate(0.17) * Affine::scale(0.5),
        ] {
            for sampling in [Sampling::Nearest, Sampling::Linear] {
                paints.push(lower(&pattern(
                    IMAGE,
                    transform,
                    Extend::Pad,
                    Extend::Pad,
                    sampling,
                )));
            }
        }
        // Degenerate images under all four modes: every lane collapses
        // to the one texel or the edge row/column.
        for (id, transform) in [
            (SINGLE, Affine::scale(0.5)),
            (STRIP_W, Affine::rotate(0.2)),
            (STRIP_H, Affine::rotate(0.2)),
            (STRIP_W, Affine::IDENTITY),
        ] {
            for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect, Extend::None] {
                for sampling in [Sampling::Nearest, Sampling::Linear] {
                    paints.push(lower(&pattern(id, transform, extend, extend, sampling)));
                }
            }
        }
        // A `Transformed` image paint lowers through the same Image
        // variant after the transform folds into the mapping.
        paints.push(lower(&Paint::Transformed(TransformedPaint::new(
            pattern(
                IMAGE,
                Affine::scale(1024.0 / 64.0),
                Extend::Pad,
                Extend::Pad,
                Sampling::Linear,
            ),
            Affine::rotate(0.3) * Affine::translate((2.0, -1.0)),
        ))));
        paints
    }

    /// Every paint kind the span table covers, lowered through the real
    /// `paint_data`/`CpuImage::decode` path a bench uses.
    fn each_kind() -> Vec<PaintData> {
        let images = [(
            IMAGE.raw(),
            Arc::new(kernels::image(&upload(64, 64)).unwrap()),
        )];
        let lower =
            |paint: &Paint| kernels::paint(paint, Affine::IDENTITY, images.clone()).unwrap();
        let radial = RadialGradient {
            start_center: (512.0, 256.0).into(),
            start_radius: 32.0,
            end_center: (640.0, 256.0).into(),
            end_radius: 400.0,
            stops: stops(),
            extend: Extend::Pad,
            interpolation: Interpolation::Working,
        };
        let mut identical = radial.clone();
        identical.end_center = identical.start_center;
        identical.end_radius = identical.start_radius;
        let mut sweep = SweepGradient::new((512.0, 256.0), 0.0, std::f64::consts::TAU);
        sweep.stops = stops();
        let pattern = |sampling| {
            Paint::Image(ImagePattern {
                image: IMAGE,
                transform: Affine::scale(1024.0 / 64.0),
                extend_x: Extend::Pad,
                extend_y: Extend::Pad,
                sampling,
            })
        };
        vec![
            lower(&Paint::Solid(WorkingColor::new([0.2, 0.5, 0.9, 1.0]))),
            lower(&Paint::Solid(WorkingColor::new([0.2, 0.5, 0.9, 0.6]))),
            lower(&Paint::Linear(linear())),
            lower(&Paint::Radial(radial)),
            lower(&Paint::Radial(identical)),
            lower(&Paint::Sweep(sweep)),
            lower(&pattern(Sampling::Nearest)),
            lower(&pattern(Sampling::Linear)),
            lower(&Paint::Transformed(TransformedPaint::new(
                Paint::Linear(linear()),
                Affine::translate((20.0, 5.0)) * Affine::rotate(0.3),
            ))),
            lower(&Paint::Transformed(TransformedPaint::new(
                Paint::Transformed(TransformedPaint::new(
                    Paint::Linear(linear()),
                    Affine::scale_non_uniform(1.03, 0.97) * Affine::rotate(0.17),
                )),
                Affine::translate((20.0, 5.0)) * Affine::rotate(0.3),
            ))),
        ]
    }

    fn scalar_reference(paint: &PaintData, x: f32, y: f32) -> [f32; 4] {
        if let PaintData::Transformed(link) = paint {
            let (x, y) = super::apply(link.inverse_f32, x, y);
            scalar_reference(&link.paint, x, y)
        } else {
            paint.eval(x, y)
        }
    }

    const PY: usize = 13;
    const REFERENCE_LEN: usize = 80;

    fn span<S: fearless_simd::Simd>(
        simd: S,
        paint: &PaintData,
        x0: usize,
        len: usize,
    ) -> Vec<[f32; 4]> {
        let mut out = vec![MaybeUninit::uninit(); len];
        paint.eval_span(simd, x0, PY, &mut out).to_vec()
    }

    /// `assert_close` over two spans' flattened pixels, naming the
    /// paint and the run when a channel exceeds the visual bound.
    #[track_caller]
    fn assert_spans_close(
        actual: &[[f32; 4]],
        expected: &[[f32; 4]],
        paint: &PaintData,
        x0: usize,
        len: usize,
    ) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cherenkov_oracle::metrics::assert_close(actual.as_flattened(), expected.as_flattened());
        }));
        assert!(result.is_ok(), "paint {paint:?}, run x0={x0} len={len}");
    }

    /// A pixel never visibly depends on its lane or the span's alignment:
    /// every sub-span agrees with a reference span covering x in 0..80
    /// within `assert_close`'s visual bound, over every run of length
    /// 0-67 starting at offsets 0-7, under the detected level and the
    /// scalar fallback.
    #[test]
    fn eval_span_is_alignment_invariant() {
        fn check<S: fearless_simd::Simd>(simd: S, paint: &PaintData) {
            let reference = span(simd, paint, 0, REFERENCE_LEN);
            for x0 in 0..=7usize {
                for len in 0..=67usize {
                    let sub = span(simd, paint, x0, len);
                    assert_spans_close(&sub, &reference[x0..x0 + len], paint, x0, len);
                }
            }
        }
        for paint in each_kind()
            .into_iter()
            .chain(linear_cases())
            .chain(radial_cases())
            .chain(sweep_cases())
            .chain(image_cases())
        {
            cherenkov_simd::dispatch!(cherenkov_simd::detect(), simd => check(simd, &paint));
            check(fearless_simd::Fallback::new(), &paint);
        }
    }

    /// `eval_span` agrees with per-pixel `eval` at the pixel centre
    /// within `assert_close`'s visual bound — no visible difference,
    /// whether or not the level fuses its multiply-adds as the scalar
    /// evaluator does — over every run of length 0-67 starting at
    /// offsets 0-7, under the detected level and the scalar fallback.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel indices and band offsets are far below 2^24"
    )]
    #[test]
    fn eval_span_agrees_with_scalar_eval_within_rounding() {
        fn check<S: fearless_simd::Simd>(simd: S, paint: &PaintData, x0: usize, len: usize) {
            let sub = span(simd, paint, x0, len);
            let scalars: Vec<[f32; 4]> = (0..len)
                .map(|i| scalar_reference(paint, (x0 + i) as f32 + 0.5, PY as f32 + 0.5))
                .collect();
            assert_spans_close(&sub, &scalars, paint, x0, len);
        }
        for paint in each_kind()
            .into_iter()
            .chain(linear_cases())
            .chain(radial_cases())
            .chain(image_cases())
        {
            for x0 in 0..=7usize {
                for len in 0..=67usize {
                    cherenkov_simd::dispatch!(cherenkov_simd::detect(), simd => check(
                        simd, &paint, x0, len
                    ));
                    check(fearless_simd::Fallback::new(), &paint, x0, len);
                }
            }
        }
    }

    /// The sweep kernel agrees with the scalar reference to the oracle
    /// crate's visual tolerance, over every run of length 0-67 at
    /// offsets 0-7, under the detected level and the scalar fallback.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel indices and band offsets are far below 2^24"
    )]
    #[test]
    fn sweep_span_agrees_with_scalar_reference() {
        fn check<S: fearless_simd::Simd>(simd: S, paint: &PaintData, x0: usize, len: usize) {
            let actual = span(simd, paint, x0, len)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let expected = (0..len)
                .flat_map(|i| scalar_reference(paint, (x0 + i) as f32 + 0.5, PY as f32 + 0.5))
                .collect::<Vec<_>>();
            cherenkov_oracle::metrics::assert_close(&actual, &expected);
        }
        for paint in sweep_cases() {
            for x0 in 0..=7usize {
                for len in 0..=67usize {
                    cherenkov_simd::dispatch!(cherenkov_simd::detect(), simd => check(
                        simd, &paint, x0, len
                    ));
                    check(fearless_simd::Fallback::new(), &paint, x0, len);
                }
            }
        }
    }
}
