// Cherenkov GPU slice: instanced coverage geometry for every primitive family.
//
// The painter path uses quads. Eligible solid passes first write opaque
// interiors (rectangles or inscribed octagons) with depth, then replay the
// original quads in painter order with depth tests and source-over blending.
// The vertex shader places the geometry; the fragment shader
// computes analytic coverage (an SDF, a Gaussian-blurred rounded box, or an
// atlas texel), multiplies by the instance's clip coverage and opacity, and
// evaluates the paint at the pixel centre in the instance's local space. The
// output is premultiplied linear Display P3 into an rgba16float target with
// fixed-function src-over blending.
//
// `shared.wgsl` precedes this file in every build: the consts, instance
// layout, vertex stage and coverage machinery live there.

// The Rust side prepends `const VARIANT: u32 = <n>u;` when building each
// module; the file stays compilable standalone.
const VARIANT_SIMPLE: u32 = 0u;
const VARIANT_SHADOW: u32 = 1u;
const VARIANT_FULL: u32 = 2u;

@group(1) @binding(0) var source: texture_2d<f32>;
// The blend backdrop: a copy of the target's region, sampled like `source`.
@group(1) @binding(1) var backdrop: texture_2d<f32>;
@group(1) @binding(2) var image_tex: texture_2d<f32>;

// Abramowitz & Stegun 7.1.26, |error| < 1.5e-7.
fn erf(x: f32) -> f32 {
    let s = sign(x);
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.5066282746 * sigma);
}

// Horizontal inset of a circular corner of radius `r` at distance `dy` past
// the start of the corner (dy <= 0 means the straight part of the edge).
fn corner_inset(r: f32, dy: f32) -> f32 {
    if dy <= 0.0 || r <= 0.0 {
        return 0.0;
    }
    let dd = min(dy, r);
    return r - sqrt(max(r * r - dd * dd, 0.0));
}

// One corner-band row of the shadow integrand: the analytic x integral of
// row `y` (sides inset by the corners of radii `rl`, `rr`) times the
// Gaussian weight of its distance to `p.y`.
fn corner_row(s: Shape, p: vec2<f32>, sigma: f32, k: f32, rl: f32, rr: f32, y: f32) -> f32 {
    let ay = abs(y);
    let xl = -s.half.x + corner_inset(rl, ay - (s.half.y - rl));
    let xr = s.half.x - corner_inset(rr, ay - (s.half.y - rr));
    if xr <= xl {
        return 0.0;
    }
    let row = 0.5 * (erf((xr - p.x) * k) - erf((xl - p.x) * k));
    return row * gaussian(y - p.y, sigma);
}

// The symmetric pair of Gauss–Legendre nodes `mid ± hw * x` with weight `w`.
fn corner_pair(
    s: Shape, p: vec2<f32>, sigma: f32, k: f32, rl: f32, rr: f32,
    mid: f32, hw: f32, x: f32, w: f32,
) -> f32 {
    return (corner_row(s, p, sigma, k, rl, rr, mid - hw * x)
        + corner_row(s, p, sigma, k, rl, rr, mid + hw * x)) * w * hw;
}

// Indicator of the rounded box `s` (circular corners), convolved with an
// isotropic Gaussian of standard deviation `sigma`, evaluated at `p`. The x
// integral of each row is analytic (erf); the y integral over the corner
// bands within ±3σ is an 8-point Gauss–Legendre rule.
fn shadow(s: Shape, p: vec2<f32>, sigma: f32) -> f32 {
    let k = 1.0 / (sigma * 1.4142135624);
    // Rows within ±3σ of p that intersect the box.
    let lo = max(p.y - 3.0 * sigma, -s.half.y);
    let hi = min(p.y + 3.0 * sigma, s.half.y);
    if hi <= lo {
        return 0.0;
    }
    let rmax = max(max(max(s.radii.x, s.radii.y), max(s.radii.z, s.radii.w)), 0.0);
    // Rows whose corner insets are beyond 4σ of p integrate like straight rows.
    let straight = abs(p.x) <= s.half.x - rmax - 4.0 * sigma;
    // Rows |y| < band have straight sides: the integral is separable.
    let band = select(max(s.half.y - rmax, 0.0), s.half.y, straight);
    var acc = 0.0;
    let ya = max(lo, -band);
    let yb = min(hi, band);
    if yb > ya {
        let row = 0.5 * (erf((s.half.x - p.x) * k) - erf((-s.half.x - p.x) * k));
        acc += row * 0.5 * (erf((yb - p.y) * k) - erf((ya - p.y) * k));
    }
    // Corner bands: Gauss–Legendre over the rows still inside ±3σ.
    for (var side = 0; side < 2; side++) {
        let top = side == 0;
        let a = max(select(band, lo, top), lo);
        let b = min(select(hi, -band, top), hi);
        if b <= a {
            continue;
        }
        let rl = select(s.radii.w, s.radii.x, top);
        let rr = select(s.radii.z, s.radii.y, top);
        let mid = 0.5 * (a + b);
        let hw = 0.5 * (b - a);
        acc += corner_pair(s, p, sigma, k, rl, rr, mid, hw, 0.1834346425, 0.3626837834);
        acc += corner_pair(s, p, sigma, k, rl, rr, mid, hw, 0.5255324099, 0.3137066459);
        acc += corner_pair(s, p, sigma, k, rl, rr, mid, hw, 0.7966664774, 0.2223810345);
        acc += corner_pair(s, p, sigma, k, rl, rr, mid, hw, 0.9602898565, 0.1012285363);
    }
    return clamp(acc, 0.0, 1.0);
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Returns false when EXTEND_NONE leaves t outside [0,1] — the caller
// returns transparent. NaN fails the range test and is also rejected.
fn extend_ok(t: f32, mode: u32) -> bool {
    return mode != EXTEND_NONE || (t >= 0.0 && t <= 1.0);
}

fn extend_t(t: f32, mode: u32) -> f32 {
    switch mode {
        case EXTEND_REPEAT: {
            return t - floor(t);
        }
        case EXTEND_REFLECT: {
            let m = t - 2.0 * floor(t * 0.5);
            return 1.0 - abs(m - 1.0);
        }
        default: {
            return clamp(t, 0.0, 1.0);
        }
    }
}

// Premultiplied working-space colour of the gradient at parameter `t`.
fn eval_stops(first: u32, count: u32, interp: u32, t: f32) -> vec4<f32> {
    var c: vec4<f32>;
    if count == 0u {
        return vec4<f32>(0.0);
    }
    if t <= stops[first].offset || count == 1u {
        c = stops[first].color;
    } else if t >= stops[first + count - 1u].offset {
        c = stops[first + count - 1u].color;
    } else {
        var i = first;
        loop {
            if i + 1u >= first + count - 1u || t < stops[i + 1u].offset {
                break;
            }
            i += 1u;
        }
        let s0 = stops[i];
        let s1 = stops[i + 1u];
        let span = s1.offset - s0.offset;
        let f = select((t - s0.offset) / span, 0.0, span <= 0.0);
        c = mix(s0.color, s1.color, f);
    }
    var rgb = c.rgb;
    if interp == INTERP_SRGB {
        rgb = SRGB_TO_P3 * srgb_decode(rgb);
    }
    return vec4<f32>(rgb * c.a, c.a);
}

fn linear_t(i: u32, p: vec2<f32>) -> f32 {
    let d = instances[i].grad.zw - instances[i].grad.xy;
    let dd = dot(d, d);
    if dd <= 0.0 {
        return 0.0;
    }
    return dot(p - instances[i].grad.xy, d) / dd;
}

// Two-point conical gradient parameter, a literal port of the oracle's
// radial_t: the larger real root of |p - (c0 + t·dc)| = r0 + t·dr.
// Degenerate coincident circles use the relative distance from the centre.
// Explicit validity avoids a non-finite constant, which WGSL rejects.
struct RadialParameter {
    value: f32,
    valid: bool,
}
fn radial_t(i: u32, p: vec2<f32>) -> RadialParameter {
    let c0 = instances[i].grad.xy;
    let c1 = instances[i].grad.zw;
    let r0 = instances[i].grad2.x;
    let r1 = instances[i].grad2.y;
    let dc = c1 - c0;
    let dr = r1 - r0;
    let pd = p - c0;
    let a = dot(dc, dc) - dr * dr;
    // b = -2·((p - c0)·dc + r0·dr)
    let b = -2.0 * (dot(pd, dc) + r0 * dr);
    let c = dot(pd, pd) - r0 * r0;
    if abs(a) < 1e-12 {
        if abs(b) < 1e-12 {
            // Coincident circles: distance relative to r0.
            if abs(r0) < 1e-12 {
                return RadialParameter(0.0, true);
            }
            return RadialParameter((length(pd) - r0) / abs(r0), true);
        }
        return RadialParameter(-c / b, true);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return RadialParameter(0.0, false);
    }
    let sq = sqrt(disc);
    // The cone answer is the larger root; when `a` is negative that is the
    // smaller numerator, so compare the roots themselves.
    return RadialParameter(max((-b + sq) / (2.0 * a), (-b - sq) / (2.0 * a)), true);
}

// Sweep (conic) parameter: the wrapped angle of p - center mapped into
// [start_angle, end_angle). A literal port of the oracle's sweep_t.
fn sweep_t(i: u32, p: vec2<f32>) -> f32 {
    let start = instances[i].grad2.x;
    var end = instances[i].grad2.y;
    let tau = 6.283185307179586;
    while end <= start {
        end += tau;
    }
    let span = end - start;
    let c = instances[i].grad.xy;
    var theta = atan2(p.y - c.y, p.x - c.x);
    while theta < start {
        theta += tau;
    }
    while theta >= start + tau {
        theta -= tau;
    }
    return (theta - start) / span;
}

// Samples `tex` like the oracle's sample_image: texel centres at n + 0.5,
// coordinate already in image-pixel space after the per-axis extend.
fn sample_image_tex(tex: texture_2d<f32>, u: f32, v: f32, w: f32, h: f32, bilinear: bool) -> vec4<f32> {
    let dims = vec2<f32>(w, h);
    if bilinear {
        // Clamp the sample coordinate into texel-centre space before the
        // fraction: taps outside the border texels collapse onto the edge.
        let f = clamp(vec2<f32>(u, v) - 0.5, vec2<f32>(0.0), dims - 1.0);
        let lo = vec2<i32>(floor(f));
        let hi = min(lo + 1, vec2<i32>(dims) - 1);
        let t = f - floor(f);
        let c00 = textureLoad(tex, vec2<i32>(lo.x, lo.y), 0);
        let c10 = textureLoad(tex, vec2<i32>(hi.x, lo.y), 0);
        let c01 = textureLoad(tex, vec2<i32>(lo.x, hi.y), 0);
        let c11 = textureLoad(tex, vec2<i32>(hi.x, hi.y), 0);
        return mix(mix(c00, c10, t.x), mix(c01, c11, t.x), t.y);
    }
    let xy = clamp(round(vec2<f32>(u, v) - 0.5), vec2<f32>(0.0), dims - 1.0);
    return textureLoad(tex, vec2<i32>(xy), 0);
}

// Image paint: `grad`/`grad2` carry the local→image affine [a b c d e f]
// and the image size [w, h]; meta_.w packs extend_x | extend_y<<4 |
// sampling<<8. The extends run in image-pixel space, like the oracle's
// eval_image_paint.
fn paint_image(i: u32, local: vec2<f32>) -> vec4<f32> {
    let g = instances[i].grad;
    let g2 = instances[i].grad2;
    let q = vec2<f32>(
        g.x * local.x + g.z * local.y + g2.x,
        g.y * local.x + g.w * local.y + g2.y,
    );
    let meta_w = instances[i].meta_.w;
    let ex = meta_w & 0xfu;
    let ey = (meta_w >> 4u) & 0xfu;
    let tu = q.x / g2.z;
    let tv = q.y / g2.w;
    if !extend_ok(tu, ex) || !extend_ok(tv, ey) {
        return vec4<f32>(0.0);
    }
    let u = extend_t(tu, ex) * g2.z;
    let v = extend_t(tv, ey) * g2.w;
    return sample_image_tex(image_tex, u, v, g2.z, g2.w, ((meta_w >> 8u) & 1u) != 0u);
}

// Bilinear inverse in f32, solving for v. The f64 oracle solves for u.
fn mesh_cross(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

fn mesh_uv(point: vec2<f32>, top: vec4<f32>, bottom: vec4<f32>) -> vec3<f32> {
    let horizontal = top.zw - top.xy;
    let vertical = bottom.xy - top.xy;
    let bend = bottom.zw - bottom.xy - horizontal;
    let delta = point - top.xy;
    let qa = -mesh_cross(vertical, bend);
    let qb = mesh_cross(delta, bend) - mesh_cross(vertical, horizontal);
    let qc = mesh_cross(delta, horizontal);
    var roots = vec2<f32>(-1.0);
    if qa == 0.0 {
        if qb == 0.0 { return vec3<f32>(0.0); }
        roots = vec2<f32>(-qc / qb);
    } else {
        let discriminant = qb * qb - 4.0 * qa * qc;
        if discriminant < 0.0 { return vec3<f32>(0.0); }
        let signed_root = select(-sqrt(discriminant), sqrt(discriminant), qb >= 0.0);
        let numerator = -0.5 * (qb + signed_root);
        roots = vec2<f32>(numerator / qa);
        if numerator != 0.0 { roots.y = qc / numerator; }
    }
    var answer = vec3<f32>(0.0);
    for (var index = 0u; index < 2u; index += 1u) {
        let v = roots[index];
        if !(v >= 0.0 && v <= 1.0) { continue; }
        let direction = horizontal + v * bend;
        let remainder = delta - v * vertical;
        var u: f32;
        if abs(direction.x) >= abs(direction.y) {
            if direction.x == 0.0 { continue; }
            u = remainder.x / direction.x;
        } else {
            u = remainder.y / direction.y;
        }
        if !(u >= 0.0 && u <= 1.0) { continue; }
        if mesh_cross(direction, vertical + u * bend) == 0.0 { continue; }
        if answer.z == 0.0 || v > answer.y || (v == answer.y && u > answer.x) {
            answer = vec3<f32>(u, v, 1.0);
        }
    }
    return answer;
}

fn paint_mesh(first: u32, count: u32, point: vec2<f32>, smooth_color: bool) -> vec4<f32> {
    for (var remaining = count; remaining > 0u; remaining -= 1u) {
        let base = first + (remaining - 1u) * 6u;
        let uv = mesh_uv(point, stops[base].color, stops[base + 1u].color);
        if uv.z != 0.0 {
            var weight = uv.xy;
            if smooth_color { weight = weight * weight * (3.0 - 2.0 * weight); }
            let top = mix(stops[base + 2u].color, stops[base + 3u].color, weight.x);
            let bottom = mix(stops[base + 4u].color, stops[base + 5u].color, weight.x);
            return mix(top, bottom, weight.y);
        }
    }
    return vec4<f32>(0.0);
}

fn paint(i: u32, meta_: vec4<u32>, color: vec4<f32>, local: vec2<f32>, device: vec2<f32>) -> vec4<f32> {
    let kind = meta_.y & 0xffffu;
    var point = local;
    if (meta_.y & 0x10000u) != 0u {
        let linear = stops[meta_.z - 2u].color;
        let offset = stops[meta_.z - 1u].color.xy;
        point = vec2<f32>(linear.x * local.x + linear.z * local.y,
                          linear.y * local.x + linear.w * local.y) + offset;
    }
    switch kind {
        case PAINT_SOLID: {
            return vec4<f32>(color.rgb * color.a, color.a);
        }
        case PAINT_TEXTURE: {
            // `grad.xy` carries the source region's device-space origin.
            return textureLoad(source, vec2<i32>(floor(device - instances[i].grad.xy)), 0);
        }
        case PAINT_MESH: {
            return paint_mesh(meta_.z, meta_.w & 0x00ffffffu, point, (meta_.y & 0x20000u) != 0u);
        }
        case PAINT_IMAGE: {
            return paint_image(i, point);
        }
        default: {
            var t: f32;
            if kind == PAINT_LINEAR {
                t = linear_t(i, point);
            } else if kind == PAINT_SWEEP {
                t = sweep_t(i, point);
            } else {
                let radial = radial_t(i, point);
                if !radial.valid {
                    return vec4<f32>(0.0);
                }
                t = radial.value;
            }
            // NaN (exponent all-ones, nonzero mantissa) → transparent.
            // `t != t` is not reliable under every driver.
            let tbits = bitcast<u32>(t);
            if (tbits & 0x7f800000u) == 0x7f800000u && (tbits & 0x007fffffu) != 0u {
                return vec4<f32>(0.0);
            }
            let meta_w = meta_.w;
            let extend = (meta_w >> 20u) & 0xfu;
            let interp = (meta_w >> 16u) & 0xfu;
            let count = meta_w & 0xffffu;
            if !extend_ok(t, extend) {
                return vec4<f32>(0.0);
            }
            return eval_stops(instances[i].meta_.z, count, interp, extend_t(t, extend));
        }
    }
}

// Per-member backdrop effects: a member composite bilinearly samples the
// bound capture and applies the effect packed in meta_.w's low bits.
// `backdrop_origin`/`backdrop_size` are set per instance by
// paint_backdrop; registered effect shaders call backdrop_sample.
var<private> backdrop_origin: vec2<f32>;
var<private> backdrop_size: vec2<f32>;

// Bilinear sample of the bound capture at device point `q`: the four
// texels around `q - 0.5` (texel centres), clamped to the capture
// region, values unclamped.
fn backdrop_sample(q: vec2<f32>) -> vec4<f32> {
    let f = clamp(q - backdrop_origin - 0.5, vec2<f32>(0.0), backdrop_size - 1.0);
    let lo = vec2<i32>(floor(f));
    let hi = min(lo + 1, vec2<i32>(backdrop_size) - 1);
    let t = f - floor(f);
    let c00 = textureLoad(source, lo, 0);
    let c10 = textureLoad(source, vec2<i32>(hi.x, lo.y), 0);
    let c01 = textureLoad(source, vec2<i32>(lo.x, hi.y), 0);
    let c11 = textureLoad(source, hi, 0);
    return mix(mix(c00, c10, t.x), mix(c01, c11, t.x), t.y);
}

// backdrop-effect-stub
fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>, size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32> {
    return backdrop_sample(p);
}
// backdrop-effect-stub

// The member composite for a PAINT_BACKDROP instance: the effect in
// meta_.w's low bits (`kind | stop count << 8`) evaluated at the device
// pixel centre `pixel`. grad.xy is the capture origin, grad2.xy its
// size, grad2.zw the member's device size.
fn paint_backdrop(i: u32, pixel: vec2<f32>) -> vec4<f32> {
    let inst = instances[i];
    backdrop_origin = inst.grad.xy;
    backdrop_size = inst.grad2.xy;
    let kind = inst.meta_.w & 0xffu;
    let first = inst.meta_.z;
    if kind == EFFECT_COLOR {
        // 3x4 premultiplied matrix, filtrate ColorMatrix layout:
        // dot(row, c) per channel, alpha passes through.
        let c = backdrop_sample(pixel);
        return vec4<f32>(
            dot(stops[first].color, c),
            dot(stops[first + 1u].color, c),
            dot(stops[first + 2u].color, c),
            c.a,
        );
    }
    // Refraction and effect shaders need the member clip's SDF: signed
    // distance and unit outward normal in device space, the same
    // J^-T math the clip coverage block uses.
    let pc = apply(inst.clip_inv, pixel);
    let sample = sdf_sample(inst.clip, pc, false);
    let g = sample.gradient;
    let ci = inst.clip_inv;
    let dg = vec2<f32>(ci[0].x * g.x + ci[0].y * g.y, ci[0].z * g.x + ci[0].w * g.y);
    let len = max(length(dg), 1e-6);
    let d = sample.distance / len;
    let n = dg / len;
    if kind == EFFECT_REFRACTION {
        // stops[first].color.xy = (depth, strength).
        let p0 = stops[first].color;
        let t = clamp(1.0 + d / p0.x, 0.0, 1.0);
        return backdrop_sample(pixel - n * p0.y * t * t);
    }
    if kind == EFFECT_RIM {
        // stops[first].color = (width, r, g, b); stops[first+1].color =
        // (a, gain). The rim is an additive term on the sampled colour
        // (alpha unchanged — gaining alpha would cancel under src-over).
        let p0 = stops[first].color;
        let p1 = stops[first + 1u].color;
        let t = clamp(1.0 + d / p0.x, 0.0, 1.0);
        let k = p1.x * p1.y * t * t;
        let c = backdrop_sample(pixel);
        return vec4<f32>(c.rgb + p0.yzw * k, c.a);
    }
    var params = array<vec4<f32>, 16>();
    let count = (inst.meta_.w >> 8u) & 0xffu;
    for (var j = 0u; j < count; j = j + 1u) {
        params[j] = stops[first + j].color;
    }
    return backdrop_effect(pixel, d, n, inst.grad2.zw, params);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if VARIANT == VARIANT_SIMPLE {
        return fs_simple(in, false);
    }
    if VARIANT == VARIANT_SHADOW {
        return fs_shadow(in);
    }
    return fs_full(in);
}

// The tile build profile replaces these two interfaces with attachment
// inputs. Coverage, colour conversion and blend arithmetic remain shared.
fn read_composite_source(coord: vec2<i32>) -> vec4<f32> {
    return textureLoad(source, coord, 0);
}

fn read_composite_backdrop(coord: vec2<i32>) -> vec4<f32> {
    return textureLoad(backdrop, coord, 0);
}

// Solid fill/span/glyph coverage: no clip, mask, inner, or paint()
// evaluation, and no `instances` reads at all.
fn fs_simple(in: VsOut, classified: bool) -> vec4<f32> {
    let s = Shape(in.shape_a.xy, in.shape_a.z, in.shape_a.w, in.shape_radii);
    let m = array<vec4<f32>, 2>(in.affine0, in.affine1);
    var cov: f32;
    switch in.meta_.x {
        case KIND_GLYPH: {
            var texel: vec2<i32>;
            if classified {
                texel = vec2<i32>(floor(in.local));
            } else {
                texel = vec2<i32>(floor(in.device - in.cell.xy)) + vec2<i32>(in.cell.zw);
            }
            cov = textureLoad(atlas, texel, 0).r;
        }
        case KIND_SPAN: {
            cov = 1.0;
        }
        default: {
            if classified && in.affine1.w > 0.0 {
                let r = vec2<f32>(s.half.x, s.half.x * s.aspect);
                let u = abs(in.local) / r;
                let rr = dot(u, u);
                if rr < in.affine1.z {
                    cov = 1.0;
                } else if rr > in.affine1.w {
                    cov = 0.0;
                } else {
                    cov = shape_coverage(s, in.local, m, classified);
                }
            } else {
                cov = shape_coverage(s, in.local, m, classified);
            }
        }
    }
    cov = clamp(cov, 0.0, 1.0) * in.params.y;
    return move_space(vec4<f32>(in.color.rgb * in.color.a, in.color.a) * cov, SPACE_LINEAR, globals.space);
}

// Retained path spans already describe full coverage. Their opacity is
// uniform over the primitive, so the vertex stage can select them without
// fragment discard (which would prevent early hidden-surface removal).
fn opaque_span(inst: Instance) -> bool {
    return inst.meta_.x == KIND_SPAN && inst.color.a * inst.params.y == 1.0;
}

struct OpaqueOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec3<f32>,
}

// Coverage replay needs only the original local/atlas coordinate, the
// instance identity, and the conservative ellipse bounds. Pull the uniform
// primitive fields in the fragment stage instead of duplicating them in
// every vertex's rasterizer payload.
struct PartialOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) instance: u32,
    @location(2) @interpolate(flat) coverage: vec2<f32>,
}

@vertex
fn vs_opaque(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> OpaqueOut {
    var inst = instances[ii];
    var ellipse = false;
    var ellipse_half = vec2<f32>(0.0);
    if inst.meta_.x == KIND_REGION {
        let x = inst.bounds.x;
        inst.bounds.x = x + f32(inst.meta_.z & 0xffffu);
        inst.bounds.z = x + f32(inst.meta_.z >> 16u);
    }
    if inst.meta_.x == KIND_FILL {
        let m = inst.affine[0];
        let det = abs(m.x * m.w - m.y * m.z);
        let margin = vec2<f32>(abs(m.w) + abs(m.z), abs(m.y) + abs(m.x)) / det;
        let s = inst.shape;
        ellipse = s.exponent == 2.0 && all(s.radii == vec4<f32>(s.half.x));
        ellipse_half = s.half * max(1.0 - length(margin / s.half), 0.0);
        let radius = max(max(s.radii.x, s.radii.y), max(s.radii.z, s.radii.w));
        // The rectangle inscribed in the circular corners is inside every
        // supported Lamé corner (exponent >= 2). Inset one complete device
        // pixel on each local axis, beyond the analytic AA support.
        let half = s.half - radius * (1.0 - sqrt(0.5)) * vec2<f32>(1.0, s.aspect) - margin;
        inst.bounds = vec4<f32>(max(inst.bounds.xy, -half), min(inst.bounds.zw, half));
    }
    let corner = array<u32, 4>(0u, 1u, 2u, 5u)[min(vi, 3u)];
    var out = instance_vertex(corner, ii, inst);
    if ellipse {
        let d = sqrt(0.5);
        let polygon = array<vec2<f32>, 8>(
            vec2<f32>(1.0, 0.0), vec2<f32>(d, d),
            vec2<f32>(0.0, 1.0), vec2<f32>(-d, d),
            vec2<f32>(-1.0, 0.0), vec2<f32>(-d, -d),
            vec2<f32>(0.0, -1.0), vec2<f32>(d, -d),
        );
        let index = array<u32, 8>(0u, 1u, 7u, 2u, 6u, 3u, 5u, 4u)[vi];
        let p = polygon[index] * max(ellipse_half, vec2<f32>(0.0));
        let ndc = (apply(inst.affine, p) - globals.origin) / globals.size * 2.0 - 1.0;
        out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    }
    // Each index selects a distinct positive f32 depth starting at 0.125,
    // spaced eight ulps apart. GL's clip z runs [-1, 1]: naga remaps it as
    // z*2 - w in f32, where |z| near 0.75 quantises to 2^-24 — the index
    // spacing must exceed that quantum (here: 4 of them, landing on grid)
    // or neighbouring indices collapse and the replay's depth test fails.
    // Depths stay below 0.5 for 2^21 instances (below 1.0 for ~3.1M);
    // render/mod.rs's MAX_PASS_INSTANCES enforces the bound.
    out.position.z = bitcast<f32>(0x3e000000u + ii * 8u);
    if inst.color.a * inst.params.y != 1.0
        || (inst.meta_.x != KIND_SPAN && inst.meta_.x != KIND_FILL && inst.meta_.x != KIND_REGION)
        || any(inst.bounds.xy >= inst.bounds.zw) {
        out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return OpaqueOut(out.position, out.color.rgb);
}

@vertex
fn vs_partial(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PartialOut {
    var out = quad_vertex(array<u32, 4>(0u, 1u, 2u, 5u)[vi], ii);
    out.position.z = bitcast<f32>(0x3e000000u + ii * 8u);
    let inst = instances[ii];
    let s = inst.shape;
    if inst.meta_.x == KIND_FILL && s.exponent == 2.0 && all(s.radii == vec4<f32>(s.half.x)) {
        let m = inst.affine[0];
        let r = vec2<f32>(s.half.x, s.half.x * s.aspect);
        let h = 0.5 * device_grad_scale(inst.affine) * vec2<f32>(abs(m.w) + abs(m.z), abs(m.y) + abs(m.x));
        let margin = length(h) / min(r.x, r.y);
        out.affine1.z = max(1.0 - margin, 0.0) * max(1.0 - margin, 0.0);
        out.affine1.w = (1.0 + margin) * (1.0 + margin);
    }
    if opaque_span(instances[ii]) {
        out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let local = select(out.local, out.device - out.cell.xy + out.cell.zw, out.meta_.x == KIND_GLYPH);
    return PartialOut(out.position, local, ii, out.affine1.zw);
}

@fragment
fn fs_opaque(in: OpaqueOut) -> @location(0) vec4<f32> {
    return move_space(vec4<f32>(in.color, 1.0), SPACE_LINEAR, globals.space);
}

@fragment
fn fs_partial(in: PartialOut) -> @location(0) vec4<f32> {
    let inst = instances[in.instance];
    var data: VsOut;
    data.local = in.local;
    data.meta_.x = select(inst.meta_.x, KIND_GLYPH, inst.meta_.x == KIND_REGION);
    data.color = inst.color;
    data.params.y = inst.params.y;
    if data.meta_.x != KIND_GLYPH && data.meta_.x != KIND_SPAN {
        data.shape_a = vec4<f32>(inst.shape.half, inst.shape.aspect, inst.shape.exponent);
        data.shape_radii = inst.shape.radii;
        data.affine0 = inst.affine[0];
    }
    data.affine1 = vec4<f32>(0.0, 0.0, in.coverage);
    return fs_simple(data, true);
}

// The shadow kernel plus the same opacity/solid-colour tail.
fn fs_shadow(in: VsOut) -> vec4<f32> {
    let s = Shape(in.shape_a.xy, in.shape_a.z, in.shape_a.w, in.shape_radii);
    let m = array<vec4<f32>, 2>(in.affine0, in.affine1);
    let sigma = in.params.x;
    var cov: f32;
    if sigma < 0.25 {
        cov = shape_coverage(s, in.local, m, false);
    } else {
        cov = shadow(s, in.local, sigma);
    }
    cov = clamp(cov, 0.0, 1.0) * in.params.y;
    return move_space(vec4<f32>(in.color.rgb * in.color.a, in.color.a) * cov, SPACE_LINEAR, globals.space);
}

fn fs_full(in: VsOut) -> vec4<f32> {
    // The constants every fragment needs arrive as flat varyings; the
    // storage array is read only for kind-specific fields (inner, clip,
    // mask, gradient data).
    let i = in.instance;
    let s = Shape(in.shape_a.xy, in.shape_a.z, in.shape_a.w, in.shape_radii);
    let m = array<vec4<f32>, 2>(in.affine0, in.affine1);
    let flags = (in.meta_.w >> 24u) & 0xffu;
    var cov: f32;
    switch in.meta_.x {
        case KIND_STROKE_OFFSET: {
            cov = shape_coverage(s, in.local, m, false);
            if (flags & FLAG_HAS_INNER) != 0u {
                cov -= shape_coverage(instances[i].inner, in.local, m, false);
            }
        }
        case KIND_STROKE_DIST: {
            let sample = sdf_sample(s, in.local, false);
            let d = sample.distance;
            let g = sample.gradient;
            let v = device_grad_vec(m, g.xy);
            let scale = device_grad_scale(m);
            let ramp = g.w > 0.0;
            let hw = in.params.x;
            cov = coverage_dir(d - hw, v, scale, ramp, select(g.z + hw, 0.0, g.z <= 0.0))
                - coverage_dir(d + hw, v, scale, ramp, select(max(g.z - hw, 0.0), 0.0, g.z <= 0.0));
        }
        case KIND_SHADOW: {
            let sigma = in.params.x;
            if sigma < 0.25 {
                cov = shape_coverage(s, in.local, m, false);
            } else {
                cov = shadow(s, in.local, sigma);
            }
        }
        case KIND_GLYPH: {
            let texel = vec2<i32>(floor(in.device - in.cell.xy)) + vec2<i32>(in.cell.zw);
            cov = textureLoad(atlas, texel, 0).r;
        }
        case KIND_SPAN: {
            cov = 1.0;
        }
        default: {
            cov = shape_coverage(s, in.local, m, false);
        }
    }
    cov *= clip_mask_coverage(in);
    // Coverage before the opacity multiply is the composite's clip coverage:
    // the destructive Porter-Duff branch antialiases the clip edge between
    // the backdrop and the blended result.
    let inside_cov = clamp(cov, 0.0, 1.0);
    cov = inside_cov * in.params.y;
    // A PAINT_TEXTURE instance samples a target texture: a plain member
    // sample (mode 0, no FLAG_BLEND_SRC) converts the texel into the
    // pass's space and blends over by fixed function; a composite blends
    // in the source texture's space — the mode in meta_.w bits 16-23,
    // Normal when FLAG_BLEND_SRC only marks a space crossing — reading
    // the backdrop explicitly (the pass runs the Replace pipeline).
    if in.meta_.y == PAINT_TEXTURE {
        let mode = (in.meta_.w >> 16u) & 0xffu;
        let tspace = select(SPACE_LINEAR, SPACE_SRGB, (flags & FLAG_TEX_SRGB) != 0u);
        let coord = vec2<i32>(floor(in.device - instances[i].grad.xy));
        if mode == 0u && (flags & FLAG_BLEND_SRC) == 0u {
            return move_space(read_composite_source(coord), tspace, globals.space) * cov;
        }
        let cb = read_composite_backdrop(coord);
        if blend_is_destructive(mode) {
            // Destructive operators composite over the whole region: a
            // transparent source still writes over the backdrop.
            let cs = read_composite_source(coord) * in.params.y;
            return mix(cb, composite_space(mode, tspace, cb, cs), inside_cov);
        }
        let cs = read_composite_source(coord) * cov;
        return composite_space(mode, tspace, cb, cs);
    }
    if in.meta_.y == PAINT_BACKDROP {
        // The bound capture stores its own space (FLAG_TEX_SRGB): the
        // effect evaluates on it and the result lands in globals.space.
        let tspace = select(SPACE_LINEAR, SPACE_SRGB, (flags & FLAG_TEX_SRGB) != 0u);
        return move_space(paint_backdrop(i, in.device) * cov, tspace, globals.space);
    }
    return move_space(paint(i, in.meta_, in.color, in.local, in.device) * cov, SPACE_LINEAR, globals.space);
}