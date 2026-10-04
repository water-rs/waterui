//! Reference shading rule: every paint is evaluated analytically at the
//! *pixel centre* and multiplied by the pixel's exact coverage.
//!
//! The sample point is `(x + 0.5, y + 0.5)` in device space, transformed
//! into the draw command's user space. The oracle never integrates a paint
//! over the pixel area; the centre sample times exact coverage is the
//! reference.

use cherenkov_scene::{
    Color, ColorSpace, Extend, GradientStop, ImagePaint, LinearGradient, Paint, RadialGradient,
    Sampling, SweepGradient,
};
use kurbo::Point;

use crate::color::{linear_p3_to_linear_srgb, srgb_encode, to_working};
use crate::resources::Resources;

/// Apply a gradient/pattern extend mode to `t`, `0.0..=1.0`.
/// Returns `None` when [`Extend::None`] leaves `t` outside the ramp.
#[must_use]
pub fn extend_t(t: f64, extend: Extend) -> Option<f64> {
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

/// Convert a scene colour into components of `space` (encoded or linear per
/// that space), for gradient interpolation in the declared space.
fn to_space(color: &Color, space: ColorSpace) -> [f64; 4] {
    // Working (premultiplied P3) -> straight linear P3.
    let [r, g, b, a] = to_working(color);
    let straight = if a > 0.0 {
        [r / a, g / a, b / a]
    } else {
        [0.0; 3]
    };
    let lin = match space {
        ColorSpace::LinearP3 | ColorSpace::DisplayP3 => straight,
        ColorSpace::LinearSrgb | ColorSpace::Srgb => linear_p3_to_linear_srgb(straight),
        ColorSpace::Rec2020 => {
            // P3 -> XYZ -> Rec. 2020.
            let xyz = crate::color::mat3_mul(&crate::color::P3_TO_XYZ, straight);
            crate::color::mat3_mul(&crate::color::mat3_inv(&crate::color::REC2020_TO_XYZ), xyz)
        }
    };
    let components = match space {
        ColorSpace::Srgb | ColorSpace::DisplayP3 => [
            srgb_encode(lin[0]),
            srgb_encode(lin[1]),
            srgb_encode(lin[2]),
        ],
        _ => lin,
    };
    [components[0], components[1], components[2], a]
}

/// Convert components in `space` (declared interpolation space) into
/// premultiplied linear Display P3.
/// `Color::components` is `[f32; 4]`; the scene's declared-space colour
/// is re-widened into the working space.
#[expect(
    clippy::cast_possible_truncation,
    reason = "scene colours are authored f32; the f64 pipeline widens them"
)]
fn from_space(components: [f64; 4], space: ColorSpace) -> [f64; 4] {
    to_working(&Color::new(
        space,
        [
            components[0] as f32,
            components[1] as f32,
            components[2] as f32,
            components[3] as f32,
        ],
    ))
}

/// Evaluate `stops` at `t ∈ [0,1]` in `interpolation` space, returning
/// premultiplied linear Display P3. End behaviour is pad inside the ramp.
fn eval_stops(stops: &[GradientStop], t: f64, interpolation: ColorSpace) -> [f64; 4] {
    let t = t.clamp(0.0, 1.0);
    if stops.is_empty() {
        return [0.0; 4];
    }
    if stops.len() == 1 || t <= f64::from(stops[0].offset) {
        return to_working(&stops[0].color);
    }
    let last = &stops[stops.len() - 1];
    if t >= f64::from(last.offset) {
        return to_working(&last.color);
    }
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t >= f64::from(a.offset) && t <= f64::from(b.offset) {
            let span = f64::from(b.offset - a.offset);
            let f = if span > 0.0 {
                (t - f64::from(a.offset)) / span
            } else {
                0.0
            };
            let ca = to_space(&a.color, interpolation);
            let cb = to_space(&b.color, interpolation);
            let mixed = [
                f.mul_add(cb[0] - ca[0], ca[0]),
                f.mul_add(cb[1] - ca[1], ca[1]),
                f.mul_add(cb[2] - ca[2], ca[2]),
                f.mul_add(cb[3] - ca[3], ca[3]),
            ];
            return from_space(mixed, interpolation);
        }
    }
    to_working(&last.color)
}

/// `t` along the linear gradient for point `p` (unnormalized, before extend).
#[must_use]
pub fn linear_t(p: Point, g: &LinearGradient) -> f64 {
    let dx = g.end.x - g.start.x;
    let dy = g.end.y - g.start.y;
    let len2 = dy.mul_add(dy, dx * dx);
    if len2 == 0.0 {
        return 0.0;
    }
    (p.y - g.start.y).mul_add(dy, (p.x - g.start.x) * dx) / len2
}

/// `t` along the two-point radial gradient for point `p`.
///
/// The parameter satisfies `|p - (c0 + t·dc)| = r0 + t·dr`; the larger real
/// root is taken, matching the CSS radial gradient definition. Degenerate
/// coincident circles use the relative distance from the centre.
#[must_use]
#[allow(clippy::many_single_char_names)] // quadratic notation mirrors the spec
pub fn radial_t(p: Point, g: &RadialGradient) -> f64 {
    let (px, py) = (p.x - g.center0.x, p.y - g.center0.y);
    let (dcx, dcy) = (g.center1.x - g.center0.x, g.center1.y - g.center0.y);
    let dr = g.r1 - g.r0;
    let a = dr.mul_add(-dr, dcy.mul_add(dcy, dcx * dcx));
    // |c0 + t·dc - p|² = (r0 + t·dr)² → a·t² + b·t + c = 0 with
    // b = -2·((p - c0)·dc + r0·dr).
    let b = -2.0 * g.r0.mul_add(dr, dcy.mul_add(py, dcx * px));
    let c = g.r0.mul_add(-g.r0, py.mul_add(py, px * px));
    if a.abs() < 1e-12 {
        if b.abs() < 1e-12 {
            // Coincident circles: distance relative to r0.
            return if g.r0.abs() < 1e-12 {
                0.0
            } else {
                (px.hypot(py) - g.r0) / g.r0.abs()
            };
        }
        return -c / b;
    }
    let disc = (4.0 * a).mul_add(-c, b * b);
    if disc < 0.0 {
        return f64::NAN;
    }
    let sq = disc.sqrt();
    // The cone answer is the larger root; when `a` is negative that is the
    // smaller numerator, so compare the roots themselves.
    let (r1, r2) = ((-b + sq) / (2.0 * a), (-b - sq) / (2.0 * a));
    r1.max(r2)
}

/// `t` along the sweep gradient for point `p` (radians, unnormalized).
#[must_use]
#[allow(clippy::while_float)] // the float wrap loops are the clearest form
pub fn sweep_t(p: Point, g: &SweepGradient) -> f64 {
    let start = g.start_angle;
    let mut end = g.end_angle;
    while end <= start {
        end += std::f64::consts::TAU;
    }
    let span = end - start;
    let mut theta = (p.y - g.center.y).atan2(p.x - g.center.x);
    while theta < start {
        theta += std::f64::consts::TAU;
    }
    while theta >= start + std::f64::consts::TAU {
        theta -= std::f64::consts::TAU;
    }
    (theta - start) / span
}

/// Evaluate a [`Paint`] at user-space point `p`, returning premultiplied
/// linear Display P3.
///
/// # Errors
/// [`SceneError`] if an image resource is missing or undecodable.
pub fn eval_paint(
    paint: &Paint,
    p: Point,
    resources: &mut Resources,
) -> Result<[f64; 4], cherenkov_scene::SceneError> {
    Ok(match paint {
        Paint::Transformed { paint, transform } => {
            let inverse = transform.inverse();
            if !transform.is_finite() || !inverse.is_finite() {
                return Err(cherenkov_scene::SceneError::PaintTransform);
            }
            eval_paint(paint, inverse * p, resources)?
        }
        Paint::Solid(c) => to_working(c),
        Paint::Linear(g) => extend_t(linear_t(p, g), g.extend)
            .map_or([0.0; 4], |t| eval_stops(&g.stops, t, g.interpolation)),
        Paint::Radial(g) => match radial_t(p, g) {
            t if t.is_finite() => {
                extend_t(t, g.extend).map_or([0.0; 4], |t| eval_stops(&g.stops, t, g.interpolation))
            }
            _ => [0.0; 4],
        },
        Paint::Sweep(g) => extend_t(sweep_t(p, g), g.extend)
            .map_or([0.0; 4], |t| eval_stops(&g.stops, t, g.interpolation)),
        Paint::Mesh(mesh) => crate::mesh::eval(mesh, p),
        Paint::Image(ip) => eval_image_paint(ip, p, resources)?,
    })
}

/// Sample an image pattern at user point `p`. `transform` maps image space
/// (pixels, `[0, w] × [0, h]`) into user space, so the sample coordinate is
/// `transform⁻¹ · p`.
#[allow(clippy::many_single_char_names)] // u/v/w/h are the natural names
#[expect(
    clippy::cast_precision_loss,
    reason = "image dimensions are far below 2^53"
)]
fn eval_image_paint(
    ip: &ImagePaint,
    p: Point,
    resources: &mut Resources,
) -> Result<[f64; 4], cherenkov_scene::SceneError> {
    let img = resources.image(ip.image, ip.encoding)?;
    let (w, h) = (img.width as f64, img.height as f64);
    let q = ip.transform.inverse() * p;
    let Some(u) = extend_t(q.x / w, ip.extend_x).map(|t| t * w) else {
        return Ok([0.0; 4]);
    };
    let Some(v) = extend_t(q.y / h, ip.extend_y).map(|t| t * h) else {
        return Ok([0.0; 4]);
    };
    Ok(sample_image(img, u, v, ip.sampling))
}

/// Sample `img` at pixel-space coordinate `(u, v)` (centre-based sampling:
/// texel centres are at integers + 0.5).
#[must_use]
#[allow(clippy::many_single_char_names)] // u/v/w/h/x/y are the natural names
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "texel coordinates are clamped into the image before indexing"
)]
pub fn sample_image(img: &crate::image::Image, u: f64, v: f64, sampling: Sampling) -> [f64; 4] {
    let (w, h) = (img.width, img.height);
    let at = |x: usize, y: usize| img.pixels[y.min(h - 1) * w + x.min(w - 1)];
    match sampling {
        Sampling::Nearest => {
            let x = (u - 0.5).round().clamp(0.0, w as f64 - 1.0) as usize;
            let y = (v - 0.5).round().clamp(0.0, h as f64 - 1.0) as usize;
            at(x, y)
        }
        Sampling::Bilinear => {
            // Clamp the *sample coordinate* into texel-centre space before
            // taking the fraction: outside the border texels every tap must
            // collapse onto the edge texel, not blend inward with a flipped
            // weight.
            let fx = (u - 0.5).clamp(0.0, w as f64 - 1.0);
            let fy = (v - 0.5).clamp(0.0, h as f64 - 1.0);
            let x0 = fx.floor() as usize;
            let y0 = fy.floor() as usize;
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            let tx = fx - x0 as f64;
            let ty = fy - y0 as f64;
            let (c00, c10, c01, c11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
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
