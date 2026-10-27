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
const VARIANT_UNION: u32 = 3u;
const VARIANT_LONG_GRADIENT: u32 = 4u;
// Every span but the union's: GPU-routed path draws (#2543).
const VARIANT_CELL: u32 = 5u;
const VARIANT_IMAGE: u32 = 6u;
const VARIANT_GLYPH: u32 = 7u;
const VARIANT_COMPOSITE: u32 = 8u;

@group(1) @binding(0) var source: texture_2d<f32>;
// The blend backdrop: a copy of the target's region, sampled like `source`.
@group(1) @binding(1) var backdrop: texture_2d<f32>;
@group(1) @binding(2) var image_tex: texture_2d<f32>;

// image-source
// The sRGB transfer decode of each 8-bit code (`image_storage::transfer_table`).
struct ImageDecode {
    linear: array<f32, 256>,
}
@group(0) @binding(5) var<storage, read> image_decode: ImageDecode;
// image-source

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
        // long-gradient-stub
        if VARIANT == VARIANT_LONG_GRADIENT {
            var end = first + count - 1u;
            loop {
                if end - i <= 1u {
                    break;
                }
                let mid = i + (end - i) / 2u;
                if t < stops[mid].offset {
                    end = mid;
                } else {
                    i = mid;
                }
            }
        } else {
        // long-gradient-stub
        loop {
            if i + 1u >= first + count - 1u || t < stops[i + 1u].offset {
                break;
            }
            i += 1u;
        }
        // long-gradient-stub
        }
        // long-gradient-stub
        let s0 = stops[i];
        let s1 = stops[i + 1u];
        let span = s1.offset - s0.offset;
        let f = select((t - s0.offset) / span, 0.0, span <= 0.0);
        c = mix(s0.color, s1.color, f);
    }
    var rgb = c.rgb;
    if interp == INTERP_SRGB {
        rgb = SPACE_SRGB_TO_P3 * space_srgb_decode(rgb);
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
// radial_t: the largest root of |p - (c0 + t·dc)| = r0 + t·dr whose radius
// r0 + t·dr is non-negative; `valid` is false where there is none.
// Identical circles (grad2.z set by lowering) take t = -∞ inside r0 and
// +∞ outside. WGSL rejects non-finite constants, so `side` carries that
// sign (-1 or +1, 0 for a finite `value`); lowering rejects identical
// circles under repeat and reflect.
struct RadialParameter {
    value: f32,
    valid: bool,
    side: f32,
}
fn radial_t(i: u32, p: vec2<f32>) -> RadialParameter {
    let c0 = instances[i].grad.xy;
    let c1 = instances[i].grad.zw;
    let r0 = instances[i].grad2.x;
    let r1 = instances[i].grad2.y;
    let pd = p - c0;
    if instances[i].grad2.z != 0.0 {
        return RadialParameter(0.0, true, select(1.0, -1.0, length(pd) <= r0));
    }
    let dc = c1 - c0;
    let dr = r1 - r0;
    let a = dot(dc, dc) - dr * dr;
    // b = -2·((p - c0)·dc + r0·dr)
    let b = -2.0 * (dot(pd, dc) + r0 * dr);
    let c = dot(pd, pd) - r0 * r0;
    if abs(a) < 1e-12 {
        if abs(b) < 1e-12 {
            return RadialParameter(0.0, false, 0.0);
        }
        let t = -c / b;
        return RadialParameter(t, r0 + t * dr >= 0.0, 0.0);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return RadialParameter(0.0, false, 0.0);
    }
    let sq = sqrt(disc);
    // When `a` is negative the larger root has the smaller numerator, so
    // compare the roots themselves.
    let q0 = (-b + sq) / (2.0 * a);
    let q1 = (-b - sq) / (2.0 * a);
    let hi = max(q0, q1);
    if r0 + hi * dr >= 0.0 {
        return RadialParameter(hi, true, 0.0);
    }
    let lo = min(q0, q1);
    return RadialParameter(lo, r0 + lo * dr >= 0.0, 0.0);
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
    var theta = angle_of(p.y - c.y, p.x - c.x);
    while theta < start {
        theta += tau;
    }
    while theta >= start + tau {
        theta -= tau;
    }
    return (theta - start) / span;
}

// A loaded image texel in the working representation. Only the image
// variant (and union, which carries everything) converts source texels:
// the `// image-source` spans are dropped from every other variant, whose
// image paints all sample working texels (flags 0).
fn image_working_texel(encoded: vec4<f32>, flags: u32) -> vec4<f32> {
    // image-source
    if (flags & 4096u) != 0u { return image_source_texel(encoded, flags); }
    // image-source
    return encoded;
}

// image-source
// Conversion happens separately at each tap, BEFORE interpolation. In
// particular neither straight alpha nor encoded premultiplied alpha may be
// hardware-filtered and subsequently converted to the working representation.
// Flag bits (`image_storage`): 12 source texels, 13 premultiplied, 14 the
// transfer function, 15 sRGB primaries, 16 the transfer table.
fn image_source_texel(encoded: vec4<f32>, flags: u32) -> vec4<f32> {
    let a = encoded.a;
    var rgb = encoded.rgb;
    if (flags & 65536u) != 0u {
        // A straight 8-bit code: its decode is a table entry.
        let code = vec3<u32>(round(rgb * 255.0));
        rgb = vec3<f32>(
            image_decode.linear[code.r],
            image_decode.linear[code.g],
            image_decode.linear[code.b],
        );
    } else {
        if (flags & 8192u) != 0u && a > 0.0 {
            rgb = min(rgb / a, vec3<f32>(1.0));
        }
        if (flags & 16384u) != 0u {
            rgb = select(pow(max((rgb + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), rgb / 12.92, rgb <= vec3<f32>(0.04045));
        }
    }
    if (flags & 32768u) != 0u {
        rgb = mat3x3<f32>(
            vec3<f32>(0.822461969, 0.033194199, 0.017082631),
            vec3<f32>(0.177538031, 0.966805801, 0.072397441),
            vec3<f32>(0.0, 0.0, 0.910519929)
        ) * rgb;
    }
    let linear = vec4<f32>(rgb * a, a);
    // Match the previous upload's working-f16 rounding before interpolation.
    return quantizeToF16(linear);
}
// image-source

// Samples `tex` like the oracle's sample_image: texel centres at n + 0.5,
// coordinate already in image-pixel space after the per-axis extend.
fn sample_image_tex(tex: texture_2d<f32>, u: f32, v: f32, w: f32, h: f32, bilinear: bool, flags: u32) -> vec4<f32> {
    let dims = vec2<f32>(w, h);
    if bilinear {
        // Clamp the sample coordinate into texel-centre space before the
        // fraction: taps outside the border texels collapse onto the edge.
        let f = clamp(vec2<f32>(u, v) - 0.5, vec2<f32>(0.0), dims - 1.0);
        let lo = vec2<i32>(floor(f));
        let hi = min(lo + 1, vec2<i32>(dims) - 1);
        let t = f - floor(f);
        let c00 = image_working_texel(textureLoad(tex, vec2<i32>(lo.x, lo.y), 0), flags);
        let c10 = image_working_texel(textureLoad(tex, vec2<i32>(hi.x, lo.y), 0), flags);
        let c01 = image_working_texel(textureLoad(tex, vec2<i32>(lo.x, hi.y), 0), flags);
        let c11 = image_working_texel(textureLoad(tex, vec2<i32>(hi.x, hi.y), 0), flags);
        return mix(mix(c00, c10, t.x), mix(c01, c11, t.x), t.y);
    }
    let xy = clamp(round(vec2<f32>(u, v) - 0.5), vec2<f32>(0.0), dims - 1.0);
    return image_working_texel(textureLoad(tex, vec2<i32>(xy), 0), flags);
}

// Image paint: `grad`/`grad2` carry the local→image affine [a b c d e f]
// and the image size [w, h]; meta_.w packs extend_x | extend_y<<4 |
// sampling<<8 and source conversion flags in bits 12–16. The extends run in image-pixel space, like the oracle's
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
    return sample_image_tex(image_tex, u, v, g2.z, g2.w, ((meta_w >> 8u) & 1u) != 0u, meta_w);
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
    // long-gradient-stub
    // Specialize dispatch without changing the kind used to evaluate the
    // gradient. The scoped bindings vanish from the short/union modules.
    let gradient_kind = kind;
    {
    let kind = select(kind, PAINT_LINEAR, VARIANT == VARIANT_LONG_GRADIENT);
    // long-gradient-stub
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
            // long-gradient-stub
            let kind = gradient_kind;
            // long-gradient-stub
            let meta_w = meta_.w;
            let extend = (meta_w >> 20u) & 0xfu;
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
                if radial.side != 0.0 {
                    // t = ±∞ clamps to an end stop under pad and is outside
                    // the range of every other mode lowering accepts.
                    if extend != EXTEND_PAD {
                        return vec4<f32>(0.0);
                    }
                    t = max(radial.side, 0.0);
                } else {
                    t = radial.value;
                }
            }
            // NaN (exponent all-ones, nonzero mantissa) → transparent.
            // `t != t` is not reliable under every driver.
            let tbits = bitcast<u32>(t);
            if (tbits & 0x7f800000u) == 0x7f800000u && (tbits & 0x007fffffu) != 0u {
                return vec4<f32>(0.0);
            }
            let interp = (meta_w >> 16u) & 0xfu;
            let count = meta_w & 0xffffu;
            if !extend_ok(t, extend) {
                return vec4<f32>(0.0);
            }
            return eval_stops(instances[i].meta_.z, count, interp, extend_t(t, extend));
        }
    }
    // long-gradient-stub
    }
    // long-gradient-stub
}

// Per-member backdrop effects: a member composite bilinearly samples the
// bound capture and applies the effect packed in meta_.w's low bits.
// `backdrop_origin`/`backdrop_size`/`backdrop_scale`/`backdrop_levels`
// are set per instance by paint_backdrop; registered effect shaders call
// backdrop_sample and backdrop_sample_level.
var<private> backdrop_origin: vec2<f32>;
var<private> backdrop_size: vec2<f32>;
var<private> backdrop_scale: f32;
var<private> backdrop_levels: f32;

// Bilinear sample of the bound capture's level `k` at device point `q`:
// on the level-k grid `q` is `q · scale / 2^k`, the four texels around
// it minus 0.5 (texel centres) are read at mip `k`, clamped to the
// level's extent `ceil(backdrop_size / 2^k)`, values unclamped. The
// region's texel origin is aligned to `2^k`, so the level-k origin is
// `backdrop_origin / 2^k` exactly.
fn backdrop_sample_at(q: vec2<f32>, k: u32) -> vec4<f32> {
    let div = f32(1u << k);
    let size = (vec2<u32>(backdrop_size) + vec2<u32>((1u << k) - 1u)) >> vec2<u32>(k);
    let f = clamp(
        (q * backdrop_scale - backdrop_origin) / div - 0.5,
        vec2<f32>(0.0),
        vec2<f32>(size) - 1.0,
    );
    let lo = vec2<i32>(floor(f));
    let hi = min(lo + 1, vec2<i32>(size) - 1);
    let t = f - floor(f);
    let c00 = textureLoad(source, lo, i32(k));
    let c10 = textureLoad(source, vec2<i32>(hi.x, lo.y), i32(k));
    let c01 = textureLoad(source, vec2<i32>(lo.x, hi.y), i32(k));
    let c11 = textureLoad(source, hi, i32(k));
    return mix(mix(c00, c10, t.x), mix(c01, c11, t.x), t.y);
}

// Bilinear sample of the bound capture at device point `q`: on the
// capture grid `q` is `q · scale`, and the four texels around it minus
// 0.5 (texel centres) are read, clamped to the capture region, values
// unclamped.
fn backdrop_sample(q: vec2<f32>) -> vec4<f32> {
    return backdrop_sample_at(q, 0u);
}

// Trilinear sample of the capture pyramid at device point `q`: `level`
// clamps to `[0, backdrop_levels − 1]` and the read is bilinear at
// `floor(level)` and `ceil(level)`, mixed by `fract(level)`.
fn backdrop_sample_level(q: vec2<f32>, level: f32) -> vec4<f32> {
    let lc = clamp(level, 0.0, backdrop_levels - 1.0);
    let base = floor(lc);
    let k0 = u32(base);
    let lo = backdrop_sample_at(q, k0);
    let phase = lc - base;
    if phase == 0.0 { return lo; }
    let k1 = min(k0 + 1u, u32(backdrop_levels) - 1u);
    let hi = backdrop_sample_at(q, k1);
    return mix(lo, hi, phase);
}

// The effect entry's pixel input: the device-space sample point, the
// signed distance of the field the member draws against — the group's
// union field when it has one, the member's own clip distance
// otherwise — the unit outward normal of that field, the member's own
// signed distance, the member's device size, the device pixels per
// logical pixel of the member's recording, and the sample point in the
// member layer's own coordinate space (the space its clip is declared
// in).
struct BackdropPixel {
    p: vec2<f32>,
    sdf: f32,
    normal: vec2<f32>,
    own_sdf: f32,
    size: vec2<f32>,
    scale: f32,
    local: vec2<f32>,
}

// The field a member draws against, read at any device point: its signed
// distance, the unit outward normal there and the member's ownership
// weight — its share of the point under a union, 1 otherwise.
struct BackdropField {
    sdf: f32,
    normal: vec2<f32>,
    weight: f32,
}

// The instance paint_backdrop is compositing: backdrop_field reads the
// member's clip or union record from it.
var<private> backdrop_member: u32;

// The member's field at device point `q`: the group's union field — the
// same fold and ownership the composite evaluates at its own pixel —
// for a union member, the member's own clip distance otherwise. At
// `q = px.p` it is exactly `px.sdf` and `px.normal`.
fn backdrop_field(q: vec2<f32>) -> BackdropField {
    let inst = instances[backdrop_member];
    // union-stub
    if ((inst.meta_.w >> 24u) & FLAG_UNION) != 0u {
        let u = union_field(bitcast<u32>(inst.uv.x), bitcast<u32>(inst.uv.y), q);
        return BackdropField(u.d, u.n, u.w_own);
    }
    // union-stub
    let own = device_sdf(inst.clip_inv, inst.clip, q);
    return BackdropField(own.x, own.yz, 1.0);
}

// backdrop-effect-stub
fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
    return backdrop_sample(px.p);
}
// backdrop-effect-stub

// The member composite for a PAINT_BACKDROP instance: the effect in
// meta_.w's low bits (`kind | stop count << 8`) evaluated at the device
// pixel centre `pixel`. grad.xy is the capture origin, grad.z the capture
// scale, grad.w its level count, grad2.xy its size, grad2.zw the member's
// device size, color.x the member's recording scale, the instance
// affine the member layer's local-to-device transform.
fn paint_backdrop(i: u32, pixel: vec2<f32>) -> vec4<f32> {
    let inst = instances[i];
    backdrop_member = i;
    backdrop_origin = inst.grad.xy;
    backdrop_scale = inst.grad.z;
    backdrop_levels = inst.grad.w;
    backdrop_size = inst.grad2.xy;
    let kind = inst.meta_.w & 0xffu;
    let first = inst.meta_.z;
    if kind == EFFECT_SAMPLE {
        return backdrop_sample(pixel);
    }
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
    // J^-T math the clip coverage block uses. A union member's own clip
    // lives in its union record — the instance clip then carries the
    // ancestors only — and the shared field replaces both `sdf` and
    // `normal` for every effect kind.
    var px: BackdropPixel;
    px.p = pixel;
    px.size = inst.grad2.zw;
    px.scale = inst.color.x;
    px.local = apply_inverse(inst.affine, pixel);
    // union-stub
    if ((inst.meta_.w >> 24u) & FLAG_UNION) != 0u {
        let field = member_field;
        px.own_sdf = field.own;
        px.sdf = field.d;
        px.normal = field.n;
    } else
    // union-stub
    {
        let own = device_sdf(inst.clip_inv, inst.clip, pixel);
        px.own_sdf = own.x;
        px.sdf = own.x;
        px.normal = own.yz;
    }
    let d = px.sdf;
    let n = px.normal;
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
    if kind == EFFECT_LEVEL {
        // stops[first].color = (depth, edge, interior): the pyramid
        // level ramps from `edge` at the clip's edge (t = 1) to
        // `interior` deep inside (t = 0).
        let p0 = stops[first].color;
        let t = clamp(1.0 + d / p0.x, 0.0, 1.0);
        return backdrop_sample_level(pixel, p0.z + (p0.y - p0.z) * t);
    }
    var params = array<vec4<f32>, 16>();
    let count = (inst.meta_.w >> 8u) & 0xffu;
    for (var j = 0u; j < count; j = j + 1u) {
        params[j] = stops[first + j].color;
    }
    return backdrop_effect(px, params);
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
                texel = vec2<i32>(floor(in.device - in.cell.xy)) + vec2<i32>(cell_texel_origin(in.cell.zw, in.meta_.w));
            }
            cov = path_cell_coverage(texel, in.cell.zw, in.cell_texel, in.shape_a.xy, in.meta_.w);
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
    // cell-buffer
    // Buffer-cell stamps joined in the vertex stage (water-rs/waterui#3081).
    @location(3) @interpolate(flat) cell: vec4<f32>,
    @location(4) @interpolate(flat) cell_texel: u32,
    @location(5) @interpolate(flat) interior: vec2<f32>,
    @location(6) @interpolate(flat) cell_meta: vec2<u32>,
    // cell-buffer
}

@vertex
fn vs_opaque(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> OpaqueOut {
    var inst = instances[ii];
    var ellipse = false;
    var ellipse_half = vec2<f32>(0.0);
    var inst_index = ii;
    var corner_vi = vi;
    // cell-buffer
    // A GPU draw's vertices decode its anchor and join the arena record.
    if vi >= 8u {
        let joined = join_gpu_instance(vi, ii);
        inst = joined.inst;
        inst_index = joined.anchor;
        corner_vi = joined.corner;
    }
    // cell-buffer
    var tier = 0u;
    var promoted = false;
    if inst.meta_.x == KIND_REGION {
        let x = inst.bounds.x;
        inst.bounds.x = x + f32(inst.meta_.z & 0xffffu);
        inst.bounds.z = x + f32(inst.meta_.z >> 16u);
    }
    if inst.meta_.x == KIND_POLYGON {
        // An appended interior tier: grad.x bitcast holds the painter
        // index it stands for, meta_.w its polygon side count.
        inst_index = bitcast<u32>(inst.grad.x);
        tier = inst.meta_.w & 3u;
        inst.meta_.x = KIND_FILL;
    } else if inst.meta_.x == KIND_FILL {
        // meta_.w's low bits mark a fill whose interior an appended
        // polygon covers: the polygon contains the octagon's every
        // vertex, so this interior emits nothing.
        promoted = (inst.meta_.w & 3u) != 0u;
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
    let corner = array<u32, 4>(0u, 1u, 2u, 5u)[min(corner_vi, 3u)];
    var out = instance_vertex(corner, inst_index, inst);
    if ellipse {
        var ndc: vec2<f32>;
        if tier == 0u {
            let d = sqrt(0.5);
            let polygon = array<vec2<f32>, 8>(
                vec2<f32>(1.0, 0.0), vec2<f32>(d, d),
                vec2<f32>(0.0, 1.0), vec2<f32>(-d, d),
                vec2<f32>(-1.0, 0.0), vec2<f32>(-d, -d),
                vec2<f32>(0.0, -1.0), vec2<f32>(d, -d),
            );
            let index = array<u32, 8>(0u, 1u, 7u, 2u, 6u, 3u, 5u, 4u)[corner_vi];
            let p = polygon[index] * max(ellipse_half, vec2<f32>(0.0));
            ndc = (apply(inst.affine, p) - globals.origin) / globals.size * 2.0 - 1.0;
        } else {
            // Zigzag order covers a regular n-gon as a strip.
            let n = 8u << tier;
            let k = select((n - vi / 2u) % n, (vi + 1u) / 2u, (vi & 1u) == 1u);
            let a = f32(k) * 6.283185307179586 / f32(n);
            let p = vec2<f32>(cos(a), sin(a)) * max(ellipse_half, vec2<f32>(0.0));
            ndc = (apply(inst.affine, p) - globals.origin) / globals.size * 2.0 - 1.0;
        }
        out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    }
    // Each index selects a distinct positive f32 depth starting at 0.125,
    // spaced eight ulps apart. GL's clip z runs [-1, 1]: naga remaps it as
    // z*2 - w in f32, where |z| near 0.75 quantises to 2^-24 — the index
    // spacing must exceed that quantum (here: 4 of them, landing on grid)
    // or neighbouring indices collapse and the replay's depth test fails.
    // Depths stay below 0.5 for 2^21 instances (below 1.0 for ~3.1M);
    // render/mod.rs's MAX_PASS_INSTANCES enforces the bound.
    out.position.z = bitcast<f32>(0x3e000000u + inst_index * 8u + select(0u, 4u, tier != 0u));
    if inst.color.a * inst.params.y != 1.0
        || (inst.meta_.x != KIND_SPAN && inst.meta_.x != KIND_FILL && inst.meta_.x != KIND_REGION)
        || any(inst.bounds.xy >= inst.bounds.zw)
        || promoted {
        out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return OpaqueOut(out.position, out.color.rgb);
}

@vertex
fn vs_partial(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PartialOut {
    var out: VsOut;
    var inst: Instance;
    var depth_index = ii;
    // cell-buffer
    if vi >= 8u {
        let joined = join_gpu_instance(vi, ii);
        inst = joined.inst;
        depth_index = joined.anchor;
        out = instance_vertex(array<u32, 8>(0u, 1u, 2u, 5u, 5u, 5u, 5u, 5u)[joined.corner], joined.anchor, inst);
        out.cell_texel = joined.texel;
    } else {
    // cell-buffer
        out = quad_vertex(array<u32, 8>(0u, 1u, 2u, 5u, 5u, 5u, 5u, 5u)[vi], ii);
        inst = instances[ii];
    // cell-buffer
    }
    // cell-buffer
    out.position.z = bitcast<f32>(0x3e000000u + depth_index * 8u);
    let s = inst.shape;
    if inst.meta_.x == KIND_FILL && s.exponent == 2.0 && all(s.radii == vec4<f32>(s.half.x)) {
        let m = inst.affine[0];
        let r = vec2<f32>(s.half.x, s.half.x * s.aspect);
        let h = 0.5 * device_grad_scale(inst.affine) * vec2<f32>(abs(m.w) + abs(m.z), abs(m.y) + abs(m.x));
        let margin = length(h) / min(r.x, r.y);
        out.affine1.z = max(1.0 - margin, 0.0) * max(1.0 - margin, 0.0);
        out.affine1.w = (1.0 + margin) * (1.0 + margin);
    }
    if opaque_span(inst) {
        out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let local = select(out.local, out.device - out.cell.xy + cell_texel_origin(out.cell.zw, out.meta_.w), out.meta_.x == KIND_GLYPH);
    return PartialOut(
        out.position,
        local,
        out.instance,
        out.affine1.zw,
        // cell-buffer
        out.cell,
        out.cell_texel,
        out.shape_a.xy,
        vec2<u32>(out.meta_.x, out.meta_.w),
        // cell-buffer
    );
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
    var kind = inst.meta_.x;
    // cell-buffer
    // A buffer cell's origin, interior run and flag arrive as varyings:
    // `in.instance` is the draw's anchor, which carries none of the
    // stamps compute wrote per emitted instance (water-rs/waterui#3081).
    data.cell = in.cell;
    data.cell_texel = in.cell_texel;
    data.shape_a = vec4<f32>(in.interior, 0.0, 0.0);
    data.meta_.w = in.cell_meta.y;
    kind = in.cell_meta.x;
    // cell-buffer
    data.meta_.x = select(kind, KIND_GLYPH, kind == KIND_REGION);
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

fn glyph_coverage(in: VsOut) -> f32 {
    let texel = vec2<i32>(floor(in.device - in.cell.xy)) + vec2<i32>(cell_texel_origin(in.cell.zw, in.meta_.w));
    return path_cell_coverage(texel, in.cell.zw, in.cell_texel, in.shape_a.xy, in.meta_.w);
}

fn plain_composite(in: VsOut, cov: f32) -> vec4<f32> {
    let flags = (in.meta_.w >> 24u) & 0xffu;
    let tspace = select(SPACE_LINEAR, SPACE_SRGB, (flags & FLAG_TEX_SRGB) != 0u);
    let coord = vec2<i32>(floor(in.device - instances[in.instance].grad.xy));
    return move_space(read_composite_source(coord), tspace, globals.space) * cov;
}

// full-fragment
fn fs_full(fragment: VsOut) -> vec4<f32> {
    // A device-space quad paints, clips and samples at its pixel, never at
    // a point interpolated across the quad.
    let in = device_quad_pixel(fragment);
    // The constants every fragment needs arrive as flat varyings; the
    // storage array is read only for kind-specific fields (inner, clip,
    // mask, gradient data).
    let i = in.instance;
    let s = Shape(in.shape_a.xy, in.shape_a.z, in.shape_a.w, in.shape_radii);
    let m = array<vec4<f32>, 2>(in.affine0, in.affine1);
    let flags = (in.meta_.w >> 24u) & 0xffu;
    var cov: f32;
    // Where the draw coverage is a signed-distance evaluation, its
    // boundary half-planes join the clip's and the joint draw∩clip pixel
    // area replaces the coverage product (#2933). Coverages without a
    // signed-distance edge — path cells, the shadow kernel — keep the
    // product.
    var planes: JointPlanes;
    var nplanes = 0u;
    var joint = true;
    switch in.meta_.x {
        case KIND_STROKE_OFFSET: {
            let outer = shape_eval(s, in.local, m, false);
            cov = outer.cov;
            planes[nplanes] = outer.planes.p[0];
            nplanes += 1u;
            if outer.planes.n == 2u {
                planes[nplanes] = outer.planes.p[1];
                nplanes += 1u;
            }
            if (flags & FLAG_HAS_INNER) != 0u {
                let inner = shape_eval(instances[i].inner, in.local, m, false);
                cov -= inner.cov;
                // Outside the inner shape. A corner's outside is a union
                // of half-planes — not the intersection a polygon clip
                // expresses — so only a single-plane inner joins.
                if inner.planes.n == 1u {
                    planes[nplanes] = -inner.planes.p[0];
                    nplanes += 1u;
                } else {
                    joint = false;
                }
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
            let det = m[0].x * m[0].w - m[0].y * m[0].z;
            let dv = v / det;
            planes[0] = vec3<f32>(d - hw, dv);
            planes[1] = vec3<f32>(-d - hw, -dv);
            nplanes = 2u;
        }
        case KIND_SHADOW: {
            let sigma = in.params.x;
            if sigma < 0.25 {
                let se = shape_eval(s, in.local, m, false);
                cov = se.cov;
                planes[nplanes] = se.planes.p[0];
                nplanes += 1u;
                if se.planes.n == 2u {
                    planes[nplanes] = se.planes.p[1];
                    nplanes += 1u;
                }
            } else {
                cov = shadow(s, in.local, sigma);
                joint = false;
            }
        }
        case KIND_GLYPH: {
            cov = glyph_coverage(in);
            joint = false;
        }
        case KIND_SPAN: {
            cov = 1.0;
        }
        default: {
            let de = shape_eval(s, in.local, m, false);
            cov = de.cov;
            planes[nplanes] = de.planes.p[0];
            nplanes += 1u;
            if de.planes.n == 2u {
                planes[nplanes] = de.planes.p[1];
                nplanes += 1u;
            }
        }
    }
    if joint && (flags & FLAG_HAS_CLIP) != 0u {
        let cp = clip_planes(in);
        for (var k = 0u; k < cp.n; k = k + 1u) {
            planes[nplanes + k] = cp.p[k];
        }
        cov = planes_coverage(planes, nplanes + cp.n) * mask_coverage(in);
    } else {
        cov *= clip_mask_coverage(in);
    }
    // union-stub
    if in.meta_.y == PAINT_BACKDROP && (flags & FLAG_UNION) != 0u {
        // A union member's instance clip carries the ancestors only; the
        // member's own coverage term is the ownership-weighted union
        // field `w_own · AA(field < outer)`, `outer` in params.x. The
        // record base and member index arrive bitcast in `uv.xy` — read
        // from `instances` storage, not a varying: the small u32s are
        // subnormals as f32 and could flush to zero.
        let base = bitcast<u32>(instances[i].uv.x);
        let ord = bitcast<u32>(instances[i].uv.y);
        member_field = union_field(base, ord, in.device);
        cov *= union_coverage(member_field, instances[i].params.x);
    }
    // union-stub
    // Coverage before the opacity multiply is the composite's clip coverage:
    // the destructive Porter-Duff branch antialiases the clip edge between
    // the backdrop and the blended result.
    let inside_cov = clamp(cov, 0.0, 1.0);
    cov = inside_cov * in.params.y;
    // long-gradient-stub
    // LongGradient is selected only for linear, radial and sweep paints;
    // texture composites and backdrop effects cannot reach this module.
    if VARIANT == VARIANT_LONG_GRADIENT {
        return move_space(paint(i, in.meta_, in.color, in.local, in.device) * cov, SPACE_LINEAR, globals.space);
    }
    // long-gradient-stub
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
            return plain_composite(in, cov);
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
        if (flags & FLAG_BLEND_SRC) != 0u {
            // The member blends in its group's space, unlike the pass's
            // storage (two spaces exist, so it is the other one): the
            // canvas comes from the backdrop copy at device origin
            // `color.yz`, the sample source-over in the member space,
            // the result written verbatim. An uncovered pixel keeps the
            // canvas untouched.
            let coord = vec2<i32>(floor(in.device - instances[i].color.yz));
            let cb = read_composite_backdrop(coord);
            if cov <= 0.0 {
                return cb;
            }
            let space = select(SPACE_SRGB, SPACE_LINEAR, globals.space == SPACE_SRGB);
            let cs = move_space(paint_backdrop(i, in.device) * cov, tspace, space);
            let over = cs + move_space(cb, globals.space, space) * (1.0 - cs.a);
            return move_space(over, space, globals.space);
        }
        return move_space(paint_backdrop(i, in.device) * cov, tspace, globals.space);
    }
    return move_space(paint(i, in.meta_, in.color, in.local, in.device) * cov, SPACE_LINEAR, globals.space);
}

// full-fragment
