//! Analytic signed-distance support for per-member backdrop effects.
//!
//! The distance is exact, not a port: the box boundary is decomposed into
//! straight segments and corner arcs (Lamé quarter curves), and the
//! signed distance is the true Euclidean distance to the closest piece
//! in device space, with the sign from the box-local inside test. The
//! scene `Shape` → box `Shape` mapping follows
//! `gpu::render::prepared::box_shape` (radii clamped to the half extents,
//! corner order top-left, top-right, bottom-right, bottom-left).

use cherenkov_scene::Shape;
use kurbo::Affine;

/// A rounded box centred at the origin, mirroring the WGSL `Shape`.
#[derive(Clone, Copy, Debug)]
pub struct BoxShape {
    /// Half extents of the box.
    pub half: [f64; 2],
    /// Corner radius aspect (y radius / x radius).
    pub aspect: f64,
    /// Lamé exponent of the corner curve; 2.0 for circular corners.
    pub exponent: f64,
    /// Corner radii along x: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f64; 4],
}

/// The clip shape mapped to the GPU's centred box form, and the local →
/// box-local affine `extra` (`transform * extra` maps clip space onto the
/// centred box). `None` for shapes with no analytic box (path, line).
#[must_use]
pub fn box_params(shape: &Shape) -> Option<(BoxShape, Affine)> {
    let (boxed, extra) = match shape {
        Shape::Rect(r) => {
            let half = [r.width() / 2.0, r.height() / 2.0];
            (
                BoxShape {
                    half,
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: [0.0; 4],
                },
                Affine::translate(r.center().to_vec2()),
            )
        }
        Shape::RoundedRect(rr) => {
            let r = rr.rect();
            let half = [r.width() / 2.0, r.height() / 2.0];
            (
                BoxShape {
                    half,
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: clamped_radii(rr.radii(), half),
                },
                Affine::translate(r.center().to_vec2()),
            )
        }
        Shape::Continuous(c) => {
            let r = c.rect;
            let half = [r.width() / 2.0, r.height() / 2.0];
            let limit = half[0].min(half[1]);
            (
                BoxShape {
                    half,
                    aspect: 1.0,
                    exponent: c.smoothing.clamp(0.0, 1.0).mul_add(2.0, 2.0),
                    radii: [c.corner_radius.clamp(0.0, limit); 4],
                },
                Affine::translate(r.center().to_vec2()),
            )
        }
        Shape::Circle(c) => {
            let r = c.radius;
            if r <= 0.0 {
                return None;
            }
            (
                BoxShape {
                    half: [r, r],
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: [r; 4],
                },
                Affine::translate(c.center.to_vec2()),
            )
        }
        Shape::Ellipse(e) => {
            let radii = e.radii();
            let (a, b) = (radii.x, radii.y);
            if a <= 0.0 || b <= 0.0 {
                return None;
            }
            (
                BoxShape {
                    half: [a, b],
                    aspect: b / a,
                    exponent: 2.0,
                    radii: [a; 4],
                },
                Affine::translate(e.center().to_vec2()) * Affine::rotate(e.rotation()),
            )
        }
        Shape::Line(_) | Shape::Path { .. } => return None,
    };
    Some((boxed, extra))
}

const fn clamped_radii(radii: kurbo::RoundedRectRadii, half: [f64; 2]) -> [f64; 4] {
    let limit = half[0].min(half[1]);
    [
        radii.top_left.clamp(0.0, limit),
        radii.top_right.clamp(0.0, limit),
        radii.bottom_right.clamp(0.0, limit),
        radii.bottom_left.clamp(0.0, limit),
    ]
}

/// One boundary piece of the box in box-local space.
#[derive(Clone, Copy, Debug)]
enum Piece {
    /// A straight segment between two arc endpoints, with its box-local
    /// outward unit normal.
    Segment([f64; 2], [f64; 2], [f64; 2]),
    /// A Lamé quarter arc `(u/rx)ⁿ + (v/ry)ⁿ = 1` centred at `cc` in the
    /// quadrant `(sx, sy)`, parameterised as
    /// `c(θ) = cc + (sx·rx·cos θ^(2/n), sy·ry·sin θ^(2/n))`.
    Arc {
        cc: [f64; 2],
        radii: [f64; 2],
        exponent: f64,
        signs: [f64; 2],
    },
}

/// The corner quadrants `(sx, sy)` in `radii` order: top-left, top-right,
/// bottom-right, bottom-left.
const CORNERS: [(f64, f64); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];

/// The arc's box-local point at `θ`.
#[allow(clippy::many_single_char_names)] // names follow the curve's formula
fn arc_point(arc: &(f64, f64, f64, [f64; 2], [f64; 2]), theta: f64) -> [f64; 2] {
    let &(sx, sy, n, cc, radii) = arc;
    let e = 2.0 / n;
    [
        (sx * radii[0]).mul_add(theta.cos().powf(e), cc[0]),
        (sy * radii[1]).mul_add(theta.sin().powf(e), cc[1]),
    ]
}

/// The arc's box-local outward unit normal at `θ`: the level-set
/// gradient `(u^(n-1)/rx, v^(n-1)/ry)` with the quadrant's signs.
#[allow(clippy::many_single_char_names)]
fn arc_normal(arc: &(f64, f64, f64, [f64; 2], [f64; 2]), theta: f64) -> [f64; 2] {
    let &(sx, sy, n, _cc, radii) = arc;
    let e = 2.0 * (n - 1.0) / n;
    let n_out = [
        sx * theta.cos().powf(e) / radii[0],
        sy * theta.sin().powf(e) / radii[1],
    ];
    let len = n_out[0].hypot(n_out[1]).max(1e-12);
    [n_out[0] / len, n_out[1] / len]
}

/// The box boundary as four straight segments plus four corner arcs, in
/// box-local space. Zero radii degenerate an arc to the corner point.
fn boundary_pieces(s: &BoxShape) -> Vec<Piece> {
    let (hx, hy) = s.half.into();
    // Arc centre and its two edge endpoints per corner.
    let mut arcs = Vec::with_capacity(4);
    for (i, (sx, sy)) in CORNERS.iter().enumerate() {
        let rx = s.radii[i].max(0.0);
        let ry = rx * s.aspect;
        arcs.push((
            *sx,
            *sy,
            s.exponent,
            [sx * (hx - rx), sy * (hy - ry)],
            [rx, ry],
        ));
    }
    // (sx*hx, sy*(hy-ry)) — the endpoint on the x = ±hx edge.
    let end_x = |i: usize| {
        let (sx, sy, _n, _cc, radii) = arcs[i];
        [sx * hx, sy * (hy - radii[1])]
    };
    // (sx*(hx-rx), sy*hy) — the endpoint on the y = ±hy edge.
    let end_y = |i: usize| {
        let (_sx, sy, _n, cc, _radii) = arcs[i];
        [cc[0], sy * hy]
    };
    let mut pieces = Vec::with_capacity(8);
    // Top edge (y = -hy): TL arc's end_y to TR arc's end_y.
    pieces.push(Piece::Segment(end_y(0), end_y(1), [0.0, -1.0]));
    // Right edge (x = hx): TR arc's end_x to BR arc's end_x.
    pieces.push(Piece::Segment(end_x(1), end_x(2), [1.0, 0.0]));
    // Bottom edge (y = hy): BR arc's end_y to BL arc's end_y.
    pieces.push(Piece::Segment(end_y(2), end_y(3), [0.0, 1.0]));
    // Left edge (x = -hx): BL arc's end_x to TL arc's end_x.
    pieces.push(Piece::Segment(end_x(3), end_x(0), [-1.0, 0.0]));
    for arc in &arcs {
        pieces.push(Piece::Arc {
            cc: arc.3,
            radii: arc.4,
            exponent: arc.2,
            signs: [arc.0, arc.1],
        });
    }
    pieces
}

/// Golden-section minimise of `f` on `[lo, hi]` until the interval is
/// under `eps` wide; returns the minimiser's parameter.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the interval only needs ~60 iterations"
)]
fn golden_section(f: impl Fn(f64) -> f64, mut lo: f64, mut hi: f64, eps: f64) -> f64 {
    const PHI: f64 = 0.618_033_988_749_894_9; // 1/φ
    let (mut x1, mut x2) = (PHI.mul_add(-(hi - lo), hi), PHI.mul_add(hi - lo, lo));
    let (mut f1, mut f2) = (f(x1), f(x2));
    // The interval shrinks by φ every step: a fixed count reaches eps.
    let iters = (((hi - lo) / eps).ln() / -PHI.ln()).ceil().max(0.0) as u32;
    for _ in 0..iters {
        if f1 < f2 {
            hi = x2;
            x2 = x1;
            f2 = f1;
            x1 = PHI.mul_add(-(hi - lo), hi);
            f1 = f(x1);
        } else {
            lo = x1;
            x1 = x2;
            f1 = f2;
            x2 = PHI.mul_add(hi - lo, lo);
            f2 = f(x2);
        }
    }
    f64::midpoint(lo, hi)
}

/// Newton polish of the arc foot point `θ`: the device-space distance
/// `|p − F(θ)|²` is minimised where its derivative `2(F−p)·F′` vanishes.
/// `F`, `F′`, `F″` are the affine image of the box-local arc and its
/// derivatives. The iteration is quadratic-convergent near the minimum,
/// so a few steps take the foot to machine precision; a non-finite
/// derivative (Lamé cusps at the arc ends) or a step out of the sampled
/// bracket keeps the seeded value.
#[allow(clippy::many_single_char_names)] // names follow the curve's formula
fn newton_polish(
    arc: &(f64, f64, f64, [f64; 2], [f64; 2]),
    device_from_box: &Affine,
    p: [f64; 2],
    mut theta: f64,
    lo: f64,
    hi: f64,
) -> f64 {
    let &(sx, sy, n, cc, radii) = arc;
    let e = 2.0 / n;
    let [a, b, c, d, _, _] = device_from_box.as_coeffs();
    let j = |v: [f64; 2]| [c.mul_add(v[1], a * v[0]), d.mul_add(v[1], b * v[0])];
    for _ in 0..32 {
        let (co, si) = (theta.cos(), theta.sin());
        let f = apply_to(device_from_box, arc_point(&(sx, sy, n, cc, radii), theta));
        let fp = j([
            sx * radii[0] * e * co.powf(e - 1.0) * (-si),
            sy * radii[1] * e * si.powf(e - 1.0) * co,
        ]);
        let fpp = j([
            sx * radii[0] * e * ((e - 1.0) * co.powf(e - 2.0) * si).mul_add(si, -co.powf(e)),
            sy * radii[1] * e * ((e - 1.0) * si.powf(e - 2.0) * co).mul_add(co, -si.powf(e)),
        ]);
        let g = [f[0] - p[0], f[1] - p[1]];
        let d1 = g[1].mul_add(fp[1], g[0] * fp[0]);
        let d2 = g[1].mul_add(
            fpp[1],
            g[0].mul_add(fpp[0], fp[0].mul_add(fp[0], fp[1] * fp[1])),
        );
        if !d1.is_finite() || !d2.is_finite() || d2 == 0.0 {
            break;
        }
        let next = (theta - d1 / d2).clamp(lo, hi);
        let done = (next - theta).abs() <= 1e-15;
        theta = next;
        if done {
            break;
        }
    }
    theta
}

/// `p` is inside the box `s` in box-local space: inside the extents,
/// and inside the corner curve in a corner square.
fn inside_box(s: &BoxShape, p: [f64; 2]) -> bool {
    let (ax, ay) = (p[0].abs(), p[1].abs());
    ax <= s.half[0]
        && ay <= s.half[1]
        && CORNERS.iter().enumerate().all(|(i, _)| {
            let rx = s.radii[i].max(0.0);
            let ry = rx * s.aspect;
            let (cx, cy) = (s.half[0] - rx, s.half[1] - ry);
            rx <= 0.0
                || ay <= cy
                || ax <= cx
                || ((ax - cx) / rx).powf(s.exponent) + ((ay - cy) / ry).powf(s.exponent) <= 1.0
        })
}

/// The device-space distance from `p` to one corner arc, its foot, and
/// the foot's box-local outward normal. A circular arc under a
/// similarity stays circular in device space: the foot is the radial
/// projection of `p` onto the circle, clamped to the arc's quadrant —
/// exact in one step. Anything else is polished by Newton.
#[expect(
    clippy::float_cmp,
    reason = "the closed form is only exact for literally circular arcs (exponent 2, equal radii)"
)]
#[allow(clippy::many_single_char_names)] // names follow the curve's formula
fn arc_distance(
    arc: &(f64, f64, f64, [f64; 2], [f64; 2]),
    device_from_box: &Affine,
    inv: &Affine,
    similarity: bool,
    p: [f64; 2],
) -> (f64, [f64; 2], [f64; 2]) {
    let &(sx, sy, n, cc, radii) = arc;
    if n == 2.0 && radii[0] == radii[1] && similarity {
        let q = apply_to(inv, p);
        let d = [q[0] - cc[0], q[1] - cc[1]];
        let len = d[0].hypot(d[1]);
        let in_quadrant = sx * d[0] >= 0.0 && sy * d[1] >= 0.0;
        let foot_l = if len > 0.0 && in_quadrant {
            [
                radii[0].mul_add(d[0] / len, cc[0]),
                radii[1].mul_add(d[1] / len, cc[1]),
            ]
        } else {
            // Off the quadrant (or at the arc centre): the nearer of
            // the arc's two endpoints.
            let e0 = [sx.mul_add(radii[0], cc[0]), cc[1]];
            let e1 = [cc[0], sy.mul_add(radii[1], cc[1])];
            if (q[1] - e0[1]).mul_add(q[1] - e0[1], (q[0] - e0[0]).powi(2))
                <= (q[1] - e1[1]).mul_add(q[1] - e1[1], (q[0] - e1[0]).powi(2))
            {
                e0
            } else {
                e1
            }
        };
        let f = apply_to(device_from_box, foot_l);
        let nl = [foot_l[0] - cc[0], foot_l[1] - cc[1]];
        let nl_len = nl[0].hypot(nl[1]).max(1e-12);
        (
            (p[0] - f[0]).hypot(p[1] - f[1]),
            f,
            [nl[0] / nl_len, nl[1] / nl_len],
        )
    } else {
        // Best of 512 uniform samples, then golden-section over the
        // bracketing interval and a Newton polish.
        const N: u32 = 512;
        const HALF: f64 = std::f64::consts::FRAC_PI_2;
        let dist2 = |theta: f64| {
            let f = apply_to(device_from_box, arc_point(arc, theta));
            (p[1] - f[1]).mul_add(p[1] - f[1], (p[0] - f[0]).powi(2))
        };
        let mut best_i = 0;
        let mut best_d = f64::INFINITY;
        for i in 0..=N {
            let theta = f64::from(i) * HALF / f64::from(N);
            let d = dist2(theta);
            if d < best_d {
                best_d = d;
                best_i = i;
            }
        }
        let lo = f64::from(best_i.saturating_sub(1)) * HALF / f64::from(N);
        let hi = f64::from((best_i + 1).min(N)) * HALF / f64::from(N);
        let theta = golden_section(dist2, lo, hi, 1e-13);
        let theta = newton_polish(arc, device_from_box, p, theta, lo, hi);
        let f = apply_to(device_from_box, arc_point(arc, theta));
        (dist2(theta).sqrt(), f, arc_normal(arc, theta))
    }
}

/// The member clip's signed distance and unit outward normal at device
/// point `p`.
///
/// The distance is the exact Euclidean distance to the box boundary
/// (straight segments plus Lamé corner arcs), negative inside.
/// `device_from_box` maps box-local space to device space — the caller's
/// `transform * extra` from [`box_params`].
#[expect(
    clippy::float_cmp,
    clippy::many_single_char_names,
    reason = "a bitwise-equal distance is a genuine medial-axis tie; a/b/c/d name the affine coefficients"
)]
#[must_use]
pub fn distance_and_normal(s: &BoxShape, device_from_box: &Affine, p: [f64; 2]) -> (f64, [f64; 2]) {
    let inv = device_from_box.inverse();
    // `device_from_box` is a similarity (rotation, translation and one
    // uniform scale) when its columns share a length and are orthogonal.
    // Under a similarity a circular arc stays circular in device space.
    let [a, b, c, d, _, _] = device_from_box.as_coeffs();
    let s0 = a.mul_add(a, b * b);
    let s1 = c.mul_add(c, d * d);
    let scale2 = s0.max(s1);
    let similarity = scale2 > 0.0
        && (s0 - s1).abs() <= 1e-12 * scale2
        && a.mul_add(c, b * d).abs() <= 1e-12 * scale2;
    // Sign: the inside test runs on the inverse-mapped point.
    let inside = inside_box(s, apply_to(&inv, p));
    let sign = if inside { -1.0 } else { 1.0 };
    let det = a.mul_add(d, -(b * c));
    // The piece's device-space outward normal at its foot: the to-foot
    // direction off the boundary, the inverse-transposed box-local
    // normal on it.
    let out_dir = |f: [f64; 2], n_out: [f64; 2]| {
        let tf = [p[0] - f[0], p[1] - f[1]];
        let l = tf[0].hypot(tf[1]);
        if l >= 1e-9 {
            [sign * tf[0] / l, sign * tf[1] / l]
        } else {
            let n_delta = [
                c.mul_add(-n_out[1], d * n_out[0]) / det,
                b.mul_add(-n_out[0], a * n_out[1]) / det,
            ];
            let ll = n_delta[0].hypot(n_delta[1]).max(1e-12);
            [n_delta[0] / ll, n_delta[1] / ll]
        }
    };
    let mut best = f64::INFINITY;
    let mut best_dir = [f64::NEG_INFINITY; 2];
    for piece in boundary_pieces(s) {
        let (d2, f, n_out) = match piece {
            Piece::Segment(a, b, n_out) => {
                let (a, b) = (apply_to(device_from_box, a), apply_to(device_from_box, b));
                let ab = [b[0] - a[0], b[1] - a[1]];
                let len2 = ab[0].hypot(ab[1]).powi(2);
                let t = if len2 > 0.0 {
                    ((p[1] - a[1]).mul_add(ab[1], (p[0] - a[0]) * ab[0]) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let f = [ab[0].mul_add(t, a[0]), ab[1].mul_add(t, a[1])];
                ((p[0] - f[0]).hypot(p[1] - f[1]), f, n_out)
            }
            Piece::Arc {
                cc,
                radii,
                exponent,
                signs,
            } => arc_distance(
                &(signs[0], signs[1], exponent, cc, radii),
                device_from_box,
                &inv,
                similarity,
                p,
            ),
        };
        if d2 < best {
            best = d2;
            best_dir = out_dir(f, n_out);
        } else if d2 == best {
            // A medial-axis tie: the distance is not differentiable
            // here. Pick the piece whose outward normal is the larger
            // direction (+x, then +y) — the convention the independent
            // reference uses on symmetry lines. The epsilon absorbs the
            // foot's rounding: tied feet are only equal to ~1e-14.
            let dir = out_dir(f, n_out);
            let dd = [dir[0] - best_dir[0], dir[1] - best_dir[1]];
            if dd[0] > 1e-9 || (dd[0].abs() <= 1e-9 && dd[1] > 0.0) {
                best_dir = dir;
            }
        }
    }
    (sign * best, best_dir)
}

/// `m * (x, y)` as a point pair.
fn apply_to(m: &Affine, p: [f64; 2]) -> [f64; 2] {
    let q = *m * kurbo::Point::new(p[0], p[1]);
    [q.x, q.y]
}

/// Bilinear sample of a capture at device point `q`, a literal port of the
/// WGSL `backdrop_sample`.
///
/// Texel centres sit at `n + 0.5`, clamped to
/// `[origin, origin + size - 1]`, values unclamped. The oracle captures the
/// full canvas (origin `(0,0)`, size the canvas); the GPU captures the
/// group's bounded region — both regions contain every point a member's
/// reach can sample, so the clamp is a no-op on both sides.
#[allow(clippy::many_single_char_names)] // names mirror the WGSL `backdrop_sample`
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "texel coordinates are clamped into the capture before indexing"
)]
#[must_use]
pub fn bilinear(capture: &[[f64; 4]], width: usize, height: usize, q: [f64; 2]) -> [f64; 4] {
    let (w, h) = (width as f64, height as f64);
    let f = [
        (q[0] - 0.5).clamp(0.0, w - 1.0),
        (q[1] - 0.5).clamp(0.0, h - 1.0),
    ];
    let lo = [f[0].floor() as usize, f[1].floor() as usize];
    let hi = [(lo[0] + 1).min(width - 1), (lo[1] + 1).min(height - 1)];
    let t = [f[0] - f[0].floor(), f[1] - f[1].floor()];
    let c00 = capture[lo[1] * width + lo[0]];
    let c10 = capture[lo[1] * width + hi[0]];
    let c01 = capture[hi[1] * width + lo[0]];
    let c11 = capture[hi[1] * width + hi[0]];
    let mix = |c0: [f64; 4], c1: [f64; 4], t: f64| {
        [
            (c1[0] - c0[0]).mul_add(t, c0[0]),
            (c1[1] - c0[1]).mul_add(t, c0[1]),
            (c1[2] - c0[2]).mul_add(t, c0[2]),
            (c1[3] - c0[3]).mul_add(t, c0[3]),
        ]
    };
    mix(mix(c00, c10, t[0]), mix(c01, c11, t[0]), t[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::RoundedRect;

    #[test]
    fn bilinear_at_texel_centres_is_the_texel() {
        let px: Vec<[f64; 4]> = (0..16)
            .map(|i| [f64::from(i), f64::from(i * 3), 0.5, 1.0])
            .collect();
        for y in 0..4u32 {
            for x in 0..4u32 {
                let v = bilinear(&px, 4, 4, [f64::from(x) + 0.5, f64::from(y) + 0.5]);
                assert_eq!(v, px[y as usize * 4 + x as usize]);
            }
        }
        // Off-centre mixes the neighbours.
        let v = bilinear(&px, 4, 4, [1.0, 0.5]);
        assert_eq!(v[0], 0.5);
        // Clamped outside the capture.
        let v = bilinear(&px, 4, 4, [-3.0, 0.5]);
        assert_eq!(v, px[0]);
    }

    /// The exact signed distance and outward unit normal of an
    /// axis-aligned rounded rect, computed independently of the ported
    /// SDF: the closest-point construction `q = |p - c| - (half - r)`,
    /// `d = |max(q, 0)| + min(max(qx, qy), 0) - r`, the normal the outward
    /// direction of the closest feature (the corner arc, or the axis of
    /// the nearer straight edge). Also returns `q` for the caller's
    /// ambiguity checks.
    #[allow(clippy::many_single_char_names)] // c/q/r follow the formula's names
    fn exact_rounded_rect(
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        r: f64,
        p: [f64; 2],
    ) -> (f64, [f64; 2], [f64; 2]) {
        let c = [f64::midpoint(x0, x1), f64::midpoint(y0, y1)];
        let half = [(x1 - x0) / 2.0, (y1 - y0) / 2.0];
        let rel = [p[0] - c[0], p[1] - c[1]];
        let q = [rel[0].abs() - (half[0] - r), rel[1].abs() - (half[1] - r)];
        let d = q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - r;
        // The outward normal in the first quadrant: along the arc when
        // both components overflow, along the nearer edge's axis inside.
        let nq = if q[0] > 0.0 && q[1] > 0.0 {
            let len = q[0].hypot(q[1]);
            [q[0] / len, q[1] / len]
        } else if q[0] > q[1] {
            [1.0, 0.0]
        } else {
            [0.0, 1.0]
        };
        (d, [nq[0] * rel[0].signum(), nq[1] * rel[1].signum()], q)
    }

    /// The ported SDF is checked against an independently computed exact
    /// rounded-rect distance and normal over the effect scenes' member
    /// clips: a literal port cannot catch a GPU-side formula bug by
    /// agreement alone.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::many_single_char_names,
        reason = "the grid bounds are integral halves of the rect coords; names mirror the coordinates"
    )]
    #[test]
    fn effect_sdf_matches_the_exact_rounded_rect() {
        // Every member clip of the backdrop effect scenes: the three
        // refraction members, the tint and rim members — all rounded
        // rects, one radius each.
        let clips = [
            RoundedRect::new(40.0, 32.0, 88.0, 80.0, 16.0),
            RoundedRect::new(112.0, 96.0, 208.0, 192.0, 16.0),
            RoundedRect::new(80.0, 200.0, 240.0, 250.0, 16.0),
            RoundedRect::new(24.0, 24.0, 140.0, 124.0, 16.0),
            RoundedRect::new(116.0, 132.0, 232.0, 232.0, 16.0),
            RoundedRect::new(32.0, 48.0, 224.0, 176.0, 20.0),
            RoundedRect::new(80.0, 160.0, 240.0, 240.0, 24.0),
        ];
        for clip in clips {
            let (shape, extra) =
                box_params(&Shape::RoundedRect(clip)).expect("a rounded rect has an analytic box");
            let clip_tf = Affine::IDENTITY * extra;
            let r = clip.rect();
            let radius = clip.radii().as_single_radius().unwrap_or_default();
            // The scan steps are exact halves of integers.
            for xi in 2.0f64.mul_add(r.x0, -40.0) as i32..=2.0f64.mul_add(r.x1, 40.0) as i32 {
                let x = f64::from(xi) / 2.0;
                for yi in 2.0f64.mul_add(r.y0, -40.0) as i32..=2.0f64.mul_add(r.y1, 40.0) as i32 {
                    let y = f64::from(yi) / 2.0;
                    let p = [x, y];
                    let (want_d, want_n, q) = exact_rounded_rect(r.x0, r.y0, r.x1, r.y1, radius, p);
                    let (d, n) = distance_and_normal(&shape, &clip_tf, p);
                    assert!(
                        (d - want_d).abs() < 1e-9,
                        "distance mismatch at {p:?}: {d} vs {want_d} for {clip:?}"
                    );
                    // The interior closest feature is ambiguous on the
                    // quadrant diagonal, and the normal is undefined at
                    // the arc centre; skip the normal check there.
                    let ambiguous = (q[0] - q[1]).abs() < 1e-6 && q[0] <= 0.0 && q[1] <= 0.0;
                    let at_arc_centre = q[0].abs() < 1e-6 && q[1].abs() < 1e-6;
                    if !ambiguous && !at_arc_centre {
                        assert!(
                            (n[0] - want_n[0]).abs() < 1e-9 && (n[1] - want_n[1]).abs() < 1e-9,
                            "normal mismatch at {p:?}: {n:?} vs {want_n:?} for {clip:?}"
                        );
                    }
                }
            }
        }
    }

    /// A dense boundary sample of `s` mapped through `device_from_box`,
    /// in clockwise boundary order: the independent reference the exact
    /// distance is checked against.
    fn boundary_cloud(s: &BoxShape, device_from_box: &Affine) -> Vec<[f64; 2]> {
        const ARC_SAMPLES: u32 = 50_000;
        const EDGE_SAMPLES: u32 = 1_000;
        let (hx, hy) = s.half.into();
        // Each arc's corner data: quadrant signs, centre, radii.
        let arc_of = |i: usize| {
            let (sx, sy) = CORNERS[i];
            let rx = s.radii[i].max(0.0);
            (
                sx,
                sy,
                s.exponent,
                [sx * (hx - rx), sy * rx.mul_add(-s.aspect, hy)],
                [rx, rx * s.aspect],
            )
        };
        // The endpoint on the x = ±hx edge (θ = 0) and on the y = ±hy
        // edge (θ = π/2).
        let end_x = |i: usize| {
            let (sx, sy, _, _, radii) = arc_of(i);
            [sx * hx, sy * (hy - radii[1])]
        };
        let end_y = |i: usize| {
            let (_, sy, _, cc, _) = arc_of(i);
            [cc[0], sy * hy]
        };
        // Clockwise: TL arc (left edge → top edge), top edge, TR arc
        // (top → right, sampled θ π/2 → 0), right edge, BR arc, bottom
        // edge, BL arc (bottom → left, θ π/2 → 0), left edge.
        let mut cloud = Vec::new();
        let emit_arc = |cloud: &mut Vec<[f64; 2]>, i: usize, from: f64, to: f64| {
            let arc = arc_of(i);
            for j in 0..=ARC_SAMPLES {
                let theta = from + (to - from) * f64::from(j) / f64::from(ARC_SAMPLES);
                cloud.push(apply_to(device_from_box, arc_point(&arc, theta)));
            }
        };
        let emit_edge = |cloud: &mut Vec<[f64; 2]>, a: [f64; 2], b: [f64; 2]| {
            for j in 0..=EDGE_SAMPLES {
                let t = f64::from(j) / f64::from(EDGE_SAMPLES);
                cloud.push(apply_to(
                    device_from_box,
                    [
                        a[0].mul_add(1.0 - t, b[0] * t),
                        a[1].mul_add(1.0 - t, b[1] * t),
                    ],
                ));
            }
        };
        let half = std::f64::consts::FRAC_PI_2;
        emit_arc(&mut cloud, 0, 0.0, half);
        emit_edge(&mut cloud, end_y(0), end_y(1));
        emit_arc(&mut cloud, 1, half, 0.0);
        emit_edge(&mut cloud, end_x(1), end_x(2));
        emit_arc(&mut cloud, 2, 0.0, half);
        emit_edge(&mut cloud, end_y(2), end_y(3));
        emit_arc(&mut cloud, 3, half, 0.0);
        emit_edge(&mut cloud, end_x(3), end_x(0));
        cloud
    }

    /// Even-odd point-in-polygon on the boundary-ordered sample, an
    /// independent inside test for the sign of the distance.
    fn inside_polygon(boundary: &[[f64; 2]], p: [f64; 2]) -> bool {
        let mut inside = false;
        let n = boundary.len();
        for i in 0..n {
            let (a, b) = (boundary[i], boundary[(i + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                inside = !inside;
            }
        }
        inside
    }

    /// Boundary samples per index run. Small enough that a run's
    /// bounding box prunes most of the cloud, large enough that the
    /// per-point box scan stays cheap.
    const RUN: usize = 512;

    /// An exact index over the dense cloud: the nearest-sample scan and
    /// the even-odd inside test keep their linear-scan answers, without
    /// rescanning all ~204k samples per grid point.
    struct BoundaryIndex<'a> {
        cloud: &'a [[f64; 2]],
        /// `(min, max)` bounding box of each `RUN`-sample run, in cloud
        /// order.
        runs: Vec<([f64; 2], [f64; 2])>,
        /// For every integer `y` in `-70..=70` (the test's `yi` range),
        /// the `x` coordinates where a boundary edge crosses the ray
        /// `p -> (+∞, y)` — the crossings `inside_polygon` finds one
        /// edge at a time, gathered once per row.
        crossings: Vec<Vec<f64>>,
    }

    /// The `yi` range the dense-boundary test scans.
    const Y_GRID: i32 = 70;

    impl<'a> BoundaryIndex<'a> {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::float_cmp,
            reason = "edge y-spans are a few dozen units here; a degenerate edge never straddles"
        )]
        fn new(cloud: &'a [[f64; 2]]) -> Self {
            let runs = cloud
                .chunks(RUN)
                .map(|run| {
                    let mut lo = [f64::INFINITY; 2];
                    let mut hi = [f64::NEG_INFINITY; 2];
                    for q in run {
                        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
                    }
                    (lo, hi)
                })
                .collect();
            let mut crossings = vec![Vec::new(); usize::try_from(2 * Y_GRID + 1).unwrap()];
            for (i, a) in cloud.iter().enumerate() {
                let b = cloud[(i + 1) % cloud.len()];
                if a[1] == b[1] {
                    continue;
                }
                // The edge straddles `y` exactly over
                // `min(a.y, b.y) <= y < max(a.y, b.y)` — the predicate
                // `inside_polygon` applies to each point.
                let lo = f64::min(a[1], b[1]).ceil() as i32;
                let hi = f64::max(a[1], b[1]).ceil() as i32;
                for y in lo.max(-Y_GRID)..hi.min(Y_GRID + 1) {
                    let x = (b[0] - a[0]).mul_add((f64::from(y) - a[1]) / (b[1] - a[1]), a[0]);
                    crossings[usize::try_from(y + Y_GRID).unwrap()].push(x);
                }
            }
            Self {
                cloud,
                runs,
                crossings,
            }
        }

        /// The distance to the closest cloud sample, the same value the
        /// linear `fold` returns: every scanned sample still goes through
        /// `hypot`, and only whole runs are pruned.
        fn nearest(&self, p: [f64; 2]) -> f64 {
            // A run's bounding-box distance lower-bounds every sample in
            // it: scan the closest run to seed `best`, then only runs
            // whose box can still beat it.
            let box_dist = |(lo, hi): &([f64; 2], [f64; 2])| {
                (lo[0] - p[0])
                    .max(p[0] - hi[0])
                    .max(0.0)
                    .hypot((lo[1] - p[1]).max(p[1] - hi[1]).max(0.0))
            };
            let scan = |run: usize, best: &mut f64| {
                let start = run * RUN;
                for q in &self.cloud[start..(start + RUN).min(self.cloud.len())] {
                    *best = best.min((q[0] - p[0]).hypot(q[1] - p[1]));
                }
            };
            let (mut seed, mut seed_lb) = (0, f64::INFINITY);
            let mut lower = Vec::with_capacity(self.runs.len());
            for (run, bounds) in self.runs.iter().enumerate() {
                let lb = box_dist(bounds);
                if lb < seed_lb {
                    (seed_lb, seed) = (lb, run);
                }
                lower.push((lb, run));
            }
            let mut best = f64::INFINITY;
            if !self.runs.is_empty() {
                scan(seed, &mut best);
            }
            for (lb, run) in lower {
                // `lb` and `best` are rounded; a run within an ulp of
                // the answer is still scanned, so pruning never flips
                // the minimum.
                if lb - best < 1e-9 && run != seed {
                    scan(run, &mut best);
                }
            }
            best
        }

        /// The even-odd inside test: the parity of crossings right of
        /// `p` is the same parity `inside_polygon` toggles to. Integral
        /// `y` rows come from the table; anything else falls back to
        /// the linear scan.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::float_cmp,
            reason = "the fallback covers non-integral y; integral y stays small"
        )]
        fn inside(&self, p: [f64; 2]) -> bool {
            let y = p[1] as i32;
            if f64::from(y) != p[1] || !(-Y_GRID..=Y_GRID).contains(&y) {
                return inside_polygon(self.cloud, p);
            }
            self.crossings[usize::try_from(y + Y_GRID).unwrap()]
                .iter()
                .filter(|x| p[0] < **x)
                .count()
                % 2
                == 1
        }
    }

    /// Elliptical and Lamé corners (#173's second-order path) against an
    /// independent dense boundary: an ellipse 40x20 and a Continuous box
    /// (exponent 3, corner radius 24), each under identity and a
    /// rotated, non-uniformly scaled transform.
    #[test]
    fn effect_sdf_matches_a_dense_boundary() {
        let shapes = [
            BoxShape {
                half: [40.0, 20.0],
                aspect: 0.5,
                exponent: 2.0,
                radii: [40.0; 4],
            },
            BoxShape {
                half: [60.0, 56.0],
                aspect: 1.0,
                exponent: 3.0,
                radii: [24.0; 4],
            },
        ];
        let transforms = [
            Affine::IDENTITY,
            Affine::rotate(0.3) * Affine::scale_non_uniform(1.5, 0.8),
        ];
        for s in shapes {
            for tf in transforms {
                let cloud = boundary_cloud(&s, &tf);
                let index = BoundaryIndex::new(&cloud);
                // The sampling spacing bounds the reference's error.
                let spacing = (0..cloud.len() - 1)
                    .map(|i| (cloud[i + 1][0] - cloud[i][0]).hypot(cloud[i + 1][1] - cloud[i][1]))
                    .fold(0.0, f64::max);
                let tolerance = 2.0f64.mul_add(spacing, 1e-9);
                for xi in -70..=70 {
                    let x = f64::from(xi);
                    for yi in -70..=70 {
                        let y = f64::from(yi);
                        let p = [x, y];
                        let nearest = index.nearest(p);
                        let want = if index.inside(p) { -nearest } else { nearest };
                        let (d, _) = distance_and_normal(&s, &tf, p);
                        assert!(
                            (d - want).abs() < tolerance,
                            "distance mismatch at {p:?}: {d} vs {want} for {s:?}"
                        );
                    }
                }
            }
        }
    }
}
