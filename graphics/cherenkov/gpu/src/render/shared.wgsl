// Shared engine module prelude: the instanced-quad machinery every fixed
// pipeline builds on — instance layout, vertex stage, SDF coverage and the
// clip/mask coverage helper. `build.rs` concatenates this file ahead of the
// engine tail (`shader.wgsl`) and the external-frame tail (`external.wgsl`);
// it is not a module on its own.
//
// Layouts here mirror `render::instance` on the CPU side exactly.

const KIND_FILL: u32 = 0u;
const KIND_STROKE_OFFSET: u32 = 1u; // coverage(outer shape) - coverage(inner shape)
const KIND_STROKE_DIST: u32 = 2u;   // coverage(d - hw) - coverage(d + hw)
const KIND_SHADOW: u32 = 3u;        // Gaussian-blurred rounded box
const KIND_GLYPH: u32 = 4u;         // coverage from the glyph atlas
const KIND_SPAN: u32 = 5u;          // a full-coverage device-space run
const KIND_REGION: u32 = 6u;        // atlas cell with a retained full interval

const PAINT_SOLID: u32 = 0u;
const PAINT_LINEAR: u32 = 1u;
const PAINT_RADIAL: u32 = 2u;
const PAINT_TEXTURE: u32 = 3u;      // composite: sample the bound texture at the device pixel
const PAINT_SWEEP: u32 = 4u;
const PAINT_IMAGE: u32 = 5u;
const PAINT_MESH: u32 = 6u;
const PAINT_BACKDROP: u32 = 7u;   // member composite with a per-member effect

const EFFECT_COLOR: u32 = 1u;
const EFFECT_REFRACTION: u32 = 2u;
const EFFECT_SHADER: u32 = 3u;
const EFFECT_RIM: u32 = 4u;
const EFFECT_SAMPLE: u32 = 5u;
const EFFECT_LEVEL: u32 = 6u;

const EXTEND_PAD: u32 = 0u;
const EXTEND_REPEAT: u32 = 1u;
const EXTEND_REFLECT: u32 = 2u;
const EXTEND_NONE: u32 = 3u;

const INTERP_WORKING: u32 = 0u;
const INTERP_SRGB: u32 = 1u;

const FLAG_HAS_CLIP: u32 = 1u;
const FLAG_HAS_INNER: u32 = 2u;
const FLAG_HAS_MASK: u32 = 4u;      // clip coverage x atlas mask cell
const FLAG_MASK_TEXTURE: u32 = 8u;  // mask sampled from `mask_tex`, not the atlas
const FLAG_TEX_SRGB: u32 = 16u;     // PAINT_TEXTURE source stores encoded pixels
const FLAG_BLEND_SRC: u32 = 32u;    // the composite blends in the source's space
const FLAG_UNION: u32 = 64u;        // PAINT_BACKDROP member reads the union field

// A rounded box centred at the origin. `radii` are the corner radii along x
// in the order top-left, top-right, bottom-right, bottom-left; the radius
// along y is `radius * aspect`. `exponent` is the Lamé exponent of the corner
// curve (2 = circular / elliptical).
struct Shape {
    half: vec2<f32>,
    aspect: f32,
    exponent: f32,
    radii: vec4<f32>,
}

struct Instance {
    // local -> device, kurbo coefficient order [a, b, c, d, e, f]:
    // x' = a x + c y + e ; y' = b x + d y + f.
    affine: array<vec4<f32>, 2>,
    // Quad rectangle (x0, y0, x1, y1). Local space, except KIND_GLYPH and
    // KIND_SPAN where it is the device-space atlas cell rectangle.
    bounds: vec4<f32>,
    shape: Shape,
    inner: Shape,
    // device -> clip-local affine, same coefficient order.
    clip_inv: array<vec4<f32>, 2>,
    // The clip shape; for a masked clip (always a sharp rect) `aspect` and
    // `exponent` — unread by its SDF — carry the mask cell size.
    clip: Shape,
    // Straight-alpha working-space colour (solid paint, glyph, shadow).
    color: vec4<f32>,
    // Linear: start.xy, end.xy. Radial: start centre.xy, end centre.xy.
    grad: vec4<f32>,
    // Radial: start radius, end radius.
    grad2: vec4<f32>,
    // Glyph/cell: atlas cell origin (x, y) in texels. zw: mask atlas origin.
    uv: vec4<f32>,
    // x: stroke half width (STROKE_DIST) or shadow sigma. y: opacity.
    // zw: mask device origin.
    params: vec4<f32>,
    // x: kind, y: paint, z: first stop index, w: stops | interp << 16 | extend << 20 | flags << 24
    meta_: vec4<u32>,
}

struct Stop {
    color: vec4<f32>, // straight alpha, in the interpolation space
    offset: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

struct Globals {
    size: vec2<f32>,
    // Device-space origin of this pass's target region.
    origin: vec2<f32>,
    // The space the target's premultiplied pixels are stored in:
    // SPACE_LINEAR or SPACE_SRGB.
    space: u32,
    pad0: u32,
    attachment_origin: vec2<f32>,
}

const SPACE_LINEAR: u32 = 0u;
const SPACE_SRGB: u32 = 1u;

@group(0) @binding(0) var<uniform> globals: Globals;
alias InstanceBuffer = array<Instance>;
alias StopBuffer = array<Stop>;
@group(0) @binding(1) var<storage, read> instances: InstanceBuffer;
@group(0) @binding(2) var<storage, read> stops: StopBuffer;
@group(0) @binding(3) var atlas: texture_2d<f32>;

// A clip mask too large for the atlas, on its own R8Unorm texture. Both
// pipelines bind a clip-mask texture at this slot.
@group(1) @binding(3) var mask_tex: texture_2d<f32>;
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) device: vec2<f32>,
    @location(2) @interpolate(flat) instance: u32,
    @location(3) @interpolate(flat) meta_: vec4<u32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
    // half.xy, aspect, exponent.
    @location(6) @interpolate(flat) shape_a: vec4<f32>,
    @location(7) @interpolate(flat) shape_radii: vec4<f32>,
    @location(8) @interpolate(flat) affine0: vec4<f32>,
    @location(9) @interpolate(flat) affine1: vec4<f32>,
    // bounds.xy, uv.xy (glyph/mask cell texel origin).
    @location(10) @interpolate(flat) cell: vec4<f32>,
}

fn apply(m: array<vec4<f32>, 2>, p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        m[0].x * p.x + m[0].z * p.y + m[1].x,
        m[0].y * p.x + m[0].w * p.y + m[1].y,
    );
}

fn apply_inverse(m: array<vec4<f32>, 2>, p: vec2<f32>) -> vec2<f32> {
    let a = m[0].x;
    let b = m[0].y;
    let c = m[0].z;
    let d = m[0].w;
    let det = a * d - b * c;
    let inv_det = select(1.0 / det, 0.0, abs(det) < 1e-12);
    let q = p - m[1].xy;
    return vec2<f32>((d * q.x - c * q.y) * inv_det, (-b * q.x + a * q.y) * inv_det);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    return quad_vertex(vi, ii);
}

fn quad_vertex(vi: u32, ii: u32) -> VsOut {
    return instance_vertex(vi, ii, instances[ii]);
}

fn instance_vertex(vi: u32, ii: u32, inst: Instance) -> VsOut {
    // Two triangles: 0 1 2, 2 1 3 over the corners (x0,y0) (x1,y0) (x0,y1) (x1,y1).
    let corner = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u)[vi];
    let sx = f32(corner & 1u);
    let sy = f32(corner >> 1u);
    let p = vec2<f32>(mix(inst.bounds.x, inst.bounds.z, sx), mix(inst.bounds.y, inst.bounds.w, sy));
    var out: VsOut;
    if inst.meta_.x >= KIND_GLYPH {
        out.device = p;
        out.local = apply_inverse(inst.affine, p);
    } else {
        out.local = p;
        out.device = apply(inst.affine, p);
    }
    let ndc = (out.device - globals.origin) / globals.size * 2.0 - 1.0;
    out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.instance = ii;
    // Hoist the constants every fragment reads into flat varyings so the
    // fragment shader only touches `instances` for kind-specific fields.
    out.meta_ = inst.meta_;
    // Region metadata belongs to the opaque vertex stage. Sampled coverage
    // uses the original atlas-cell fragment path for both cell kinds.
    out.meta_.x = select(inst.meta_.x, KIND_GLYPH, inst.meta_.x == KIND_REGION);
    out.color = inst.color;
    out.params = inst.params;
    out.shape_a = vec4<f32>(inst.shape.half, inst.shape.aspect, inst.shape.exponent);
    out.shape_radii = inst.shape.radii;
    out.affine0 = inst.affine[0];
    out.affine1 = inst.affine[1];
    out.cell = vec4<f32>(inst.bounds.xy, inst.uv.xy);
    return out;
}

// Distance evaluations `lame_corner` spends on one point.
const LAME_EVALS: i32 = 6;
const QUARTER_PI: f32 = 0.7853981633974483;
const SQRT_HALF: f32 = 0.7071067811865476;

// A point on the Lamé quarter curve (x/r.x)^n + (y/r.y)^n = 1 at the
// parameter w = (y/r.y) / (x/r.x), w in [0, 1] from the x axis to the
// diagonal of the normalised curve: c = r·(1, w)·s with
// s = (1 + w^n)^(-1/n).
struct LamePoint {
    c: vec2<f32>,
    // (r.y, r.x·w^(n-1)): the outward normal, unnormalised.
    normal: vec2<f32>,
    // w^(n-2).
    wm2: f32,
    s: f32,
    // 1 + w^n.
    one: f32,
}

fn lame_point(r: vec2<f32>, n: f32, w: f32) -> LamePoint {
    // w^(n-2) is 1 at w = 0 for an ellipse (n = 2) and 0 for n > 2.
    let wm2 = pow(max(w, 1e-30), n - 2.0);
    let p = w * wm2;
    let one = 1.0 + w * p;
    let s = pow(one, -1.0 / n);
    return LamePoint(r * vec2<f32>(1.0, w) * s, vec2<f32>(r.y, r.x * p), wm2, s, one);
}

fn cbrt(x: f32) -> f32 {
    return sign(x) * pow(max(abs(x), 1e-30), 1.0 / 3.0);
}

// The largest real root of c3·x³ + c1·x + c0 = 0 with c3 > 0: Cardano's
// form where the cubic has one real root, the trigonometric form where it
// has three.
fn cubic_root(c3: f32, c1: f32, c0: f32) -> f32 {
    let p = c1 / c3;
    let h = 0.5 * c0 / c3;
    let disc = h * h + p * p * p / 27.0;
    if disc >= 0.0 {
        let e = sqrt(disc);
        return cbrt(e - h) + cbrt(-e - h);
    }
    let m = sqrt(-p / 3.0);
    return 2.0 * m * cos(acos(clamp(-h / (m * m * m), -1.0, 1.0)) / 3.0);
}

// Where the solve of an elliptical corner starts: its w = 0 end is a
// vertex of the ellipse. There the foot condition is
// F(w) = r.y·q.y - w·(r.x·q.x - Δ/√(1 + w²)), Δ = r.x² - r.y². Across a
// major vertex (Δ > 0) -F is convex and its Taylor cubic
// (Δ/2)w³ + (r.x·q.x - Δ)w - r.y·q.y bounds it from above, so the cubic's
// largest root lies just below the foot, with an error of order w⁵: the
// evolute cusp, where the foot is a near-triple root that Newton's method
// approaches only linearly, sits at small w. Across a minor vertex -F is
// concave with a slope bounded away from zero and the linear term is the
// start.
fn ellipse_start(q: vec2<f32>, r: vec2<f32>) -> f32 {
    let delta = r.x * r.x - r.y * r.y;
    let c1 = r.x * q.x - delta;
    if delta <= 0.0 {
        return clamp(r.y * q.y / c1, 0.0, 1.0);
    }
    return clamp(cubic_root(0.5 * delta, c1, -r.y * q.y), 0.0, 1.0);
}

// Where the solve of a continuous corner (r.x = r.y = r) starts: its w = 1
// end is the corner's diagonal, a vertex whose evolute cusp lies on the
// diagonal. The corner's support function about the diagonal normal is
// h(π/4 + ψ) = h0·(1 + K2·ψ² + K4·ψ⁴) with p = n/(n - 1) the dual
// exponent, so the stationarity of q·m - h(ψ) is, to third order,
// B - (A - h0 + ρ0)·ψ - ((h4 - A)/6)·ψ³ = 0 with A, B the components of q
// along and across the diagonal, ρ0 = h0·(1 + 2·K2) the radius of
// curvature and h4 = 24·h0·K4. Its largest root is the normal angle ψ away
// from the diagonal, and w = tan(π/4 - ψ)^(1/(n - 1)).
fn diagonal_start(q: vec2<f32>, r: f32, n: f32) -> f32 {
    let p = n / (n - 1.0);
    let c2 = 0.5 * p * (p - 1.0);
    let c4 = p * (p - 1.0) * (p - 2.0) * (p - 3.0) / 24.0;
    let k2 = c2 / p - 0.5;
    let k4 = (2.0 * c2 / 3.0 + c4) / p + 0.5 * (1.0 / p - 1.0) * c2 * c2 / p - 0.5 * c2 / p + 1.0 / 24.0;
    let h0 = r * exp2(1.0 / p - 0.5);
    let along = (q.x + q.y) * SQRT_HALF;
    let across = (q.x - q.y) * SQRT_HALF;
    let k3 = (24.0 * h0 * k4 - along) / 6.0;
    let k1 = along - h0 + h0 * (1.0 + 2.0 * k2);
    var psi = QUARTER_PI;
    if k3 > 0.0 {
        psi = cubic_root(k3, k1, -across);
    } else if k1 > 0.0 {
        psi = across / k1;
    }
    let theta = clamp(QUARTER_PI - psi, 0.0, QUARTER_PI);
    return clamp(pow(max(tan(theta), 1e-30), 1.0 / (n - 1.0)), 0.0, 1.0);
}

// Exact signed distance from `q` (corner-local, both components >= 0, in
// the units of `r`) to the Lamé quarter curve (q.x/r.x)^n + (q.y/r.y)^n = 1,
// for an elliptical corner (n = 2) or a continuous one (r.x = r.y, n > 2).
// Returns (d, unit normal in q space, radius of curvature at the foot).
//
// The foot is the root of F(w) = (q - c)·T, T the tangent. Folding the
// quarter at w = 1 puts it in [0, 1] with F(0) >= 0 >= F(1): an ellipse
// quarter runs between two vertices, and a continuous corner is symmetric
// about its diagonal, so the half that holds q's foot has monotone
// curvature and F changes sign on it once. The solve starts from a
// closed-form model of F about the half's vertex, where the evolute cusp
// makes the foot a near-triple root (`ellipse_start`, `diagonal_start`),
// then takes Newton steps clamped to the sign bracket, bisecting where F
// is not falling. The box is convex, so the tangent at every curve point
// supports it: the result, the distance to the tangent at the last
// iterate, never exceeds the box's signed distance and equals it at the
// foot.
fn lame_corner(q_in: vec2<f32>, r_in: vec2<f32>, n: f32) -> vec4<f32> {
    let k = exp2(-1.0 / n);
    let swap = dot(q_in - r_in * k, vec2<f32>(-r_in.x, r_in.y)) > 0.0;
    let q = select(q_in, q_in.yx, swap);
    let r = select(r_in, r_in.yx, swap);
    var w: f32;
    if n == 2.0 {
        w = ellipse_start(q, r);
    } else {
        w = diagonal_start(q, r.x, n);
    }
    var lo = 0.0;
    var hi = 1.0;
    var foot = vec4<f32>(0.0);
    for (var i = 0; i < LAME_EVALS; i++) {
        let pt = lame_point(r, n, w);
        let len = length(pt.normal);
        let normal = pt.normal / len;
        let qc = q - pt.c;
        // Curvature from the implicit form,
        // κ = (n - 1)(w^(2n-2) + w^(n-2))·r.x·r.y / (s·|normal|³).
        let p = pt.normal.y / r.x;
        let kappa = (n - 1.0) * (p * p + pt.wm2) * r.x * r.y / (pt.s * len * len * len);
        foot = vec4<f32>(dot(qc, normal), normal, kappa);
        let t = vec2<f32>(-pt.normal.y, pt.normal.x);
        let f = dot(qc, t);
        let df = -pt.s / pt.one * dot(t, t) - qc.x * r.x * (n - 1.0) * pt.wm2;
        if f > 0.0 {
            lo = w;
        } else {
            hi = w;
        }
        w = select(0.5 * (lo + hi), clamp(w - f / df, lo, hi), df < 0.0);
    }
    return vec4<f32>(foot.x, select(foot.yz, foot.zy, swap), 1.0 / max(foot.w, 1e-6));
}

// Distance and differential geometry evaluated at the same local point.
// In particular, a Lamé corner needs only one foot-point solve.
struct DistanceSample {
    distance: f32,
    gradient: vec4<f32>,
}

fn sdf_sample(s: Shape, p: vec2<f32>, specialize_quadratic: bool) -> DistanceSample {
    let sgn = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    let right = p.x > 0.0;
    let bottom = p.y > 0.0;
    let r = select(
        select(s.radii.x, s.radii.w, bottom),
        select(s.radii.y, s.radii.z, bottom),
        right,
    );
    let rx = max(r, 0.0);
    let ry = rx * s.aspect;
    let a = abs(p) - s.half;
    if rx <= 0.0 || ry <= 0.0 {
        let d = length(max(a, vec2<f32>(0.0))) + min(max(a.x, a.y), 0.0);
        var g: vec4<f32>;
        if a.x > 0.0 && a.y > 0.0 {
            g = vec4<f32>(a / length(a), 0.0, 1.0);
        } else {
            g = vec4<f32>(select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), a.x > a.y), 0.0, 0.0);
        }
        return DistanceSample(d, vec4<f32>(sgn * g.xy, g.zw));
    }
    let q = a + vec2<f32>(rx, ry);
    if abs(s.exponent - 2.0) < 1e-4 && abs(s.aspect - 1.0) < 1e-4 {
        if q.x > 0.0 && q.y > 0.0 {
            let u = q / vec2<f32>(rx, ry);
            let len = length(u);
            let grad = length(u / vec2<f32>(rx, ry)) / max(len, 1e-6);
            let d = (len - 1.0) / max(grad, 1e-6);
            let v = q / vec2<f32>(rx * rx, ry * ry);
            let normal = v / max(length(v), 1e-12);
            return DistanceSample(d, vec4<f32>(sgn * normal, rx, 0.0));
        }
    } else if q.x >= 0.0 && q.y >= 0.0 {
        // A Lamé corner owns the lines through its centre too: on an
        // ellipse's major axis, a point deeper than the vertex's radius
        // of curvature is nearest to the curve on either side of the
        // axis, not to the vertex the straight-edge distance measures.
        // Coverage replay knows the common ellipse exponent exactly. Give
        // compilation a constant argument without changing the solve.
        var l: vec4<f32>;
        if specialize_quadratic && s.exponent == 2.0 {
            l = lame_corner(q, vec2<f32>(rx, ry), 2.0);
        } else {
            l = lame_corner(q, vec2<f32>(rx, ry), s.exponent);
        }
        return DistanceSample(l.x, vec4<f32>(sgn * l.yz, l.w, 0.0));
    }
    let g = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), a.x > a.y);
    return DistanceSample(max(a.x, a.y), vec4<f32>(sgn * g, 0.0, 0.0));
}

// The signed distance and unit outward normal at device point `pixel`
// of the clip shape `s` inverse-mapped by `ci` — the member-clip math
// `paint_backdrop` applies to the member's own clip and the union fold
// applies to every member's record.
fn device_sdf(ci: array<vec4<f32>, 2>, s: Shape, pixel: vec2<f32>) -> vec4<f32> {
    let pc = apply(ci, pixel);
    let sample = sdf_sample(s, pc, false);
    let g = sample.gradient;
    let dg = vec2<f32>(ci[0].x * g.x + ci[0].y * g.y, ci[0].z * g.x + ci[0].w * g.y);
    let len = max(length(dg), 1e-6);
    return vec4<f32>(sample.distance / len, dg / len, 0.0);
}

// Device-space gradient of a signed distance, for the local -> device
// affine `m` (J^-T g), before its length is scaled into per-pixel change.
fn device_grad_vec(m: array<vec4<f32>, 2>, g: vec2<f32>) -> vec2<f32> {
    let a = m[0].x;
    let b = m[0].y;
    let c = m[0].z;
    let d = m[0].w;
    return vec2<f32>(d * g.x - b * g.y, -c * g.x + a * g.y);
}

// Per-device-pixel scale for a gradient returned by `device_grad_vec`:
// |det^-1| of the affine's linear part.
fn device_grad_scale(m: array<vec4<f32>, 2>) -> f32 {
    let a = m[0].x;
    let b = m[0].y;
    let c = m[0].z;
    let d = m[0].w;
    let det = a * d - b * c;
    return abs(select(1.0 / det, 0.0, abs(det) < 1e-12));
}

// Area coverage of the axis-aligned half-plane `d <= 0` where `d`
// changes by `g` per device pixel.
fn coverage(d: f32, g: f32) -> f32 {
    return clamp(0.5 - d / g, 0.0, 1.0);
}

// Area of the unit pixel inside the half-plane `d <= 0`, with `v` the
// device-space gradient of `d` scaled by `scale` (g = |v|·scale per pixel)
// and `radius` the boundary's local radius of curvature in the same local
// units as `d` (0.0 = straight edge). Exact for straight edges at any
// angle, second-order in curvature for circular arcs; `ramp` forces the
// axis-aligned linear ramp where the distance is not a half-plane.
fn coverage_dir(d: f32, v: vec2<f32>, scale: f32, ramp: bool, radius: f32) -> f32 {
    let len = length(v);
    let g = max(len * scale, 1e-6);
    let ax = abs(v.x);
    let ay = abs(v.y);
    let b = min(ax, ay);
    if ramp {
        return coverage(d, g);
    }
    if b <= 1e-6 * len {
        let area = coverage(d, g);
        if radius > 0.0 && area > 0.0 && area < 1.0 {
            return clamp(area - (g / radius) / 24.0, 0.0, 1.0);
        }
        return area;
    }
    let a = max(ax, ay) / len;
    let bn = b / len;
    let t = -d / g;
    let h = 0.5 * (a + bn);
    let k = 0.5 * (a - bn);
    if t >= h {
        return 1.0;
    }
    if t <= -h {
        return 0.0;
    }
    if abs(t) <= k {
        let mid = 0.5 + t / a;
        if radius > 0.0 {
            let w = 1.0 / a;
            return clamp(mid - (g / radius) * w * w * w / 24.0, 0.0, 1.0);
        }
        return mid;
    }
    let e = h - abs(t);
    let tail = e * e / (2.0 * a * bn);
    let area = select(tail, 1.0 - tail, t > 0.0);
    if radius > 0.0 {
        let w = e / (a * bn);
        return clamp(area - (g / radius) * w * w * w / 24.0, 0.0, 1.0);
    }
    return area;
}

// Exact area of the unit device pixel (the square of side 1 centred on
// the fragment) inside the two half-planes `a.x + scale·dot(v1, u) <= 0`
// and `a.y + scale·dot(v2, u) <= 0`, `u` the device offset from the pixel
// centre. Sutherland–Hodgman: clip the 4-vertex square against line 1,
// then line 2, and take the shoelace area. Where a sharp corner (the two
// folded edges meeting at a point) lies inside the pixel, neither
// half-plane's linear ramp nor the exterior Euclidean ramp is the truth:
// the truth is the area inside both.
struct PolygonArea {
    first: vec2<f32>,
    previous: vec2<f32>,
    area: f32,
    count: u32,
}

fn append_area(state: PolygonArea, p: vec2<f32>) -> PolygonArea {
    var next = state;
    if state.count == 0u {
        next.first = p;
    } else {
        next.area = state.area + state.previous.x * p.y - p.x * state.previous.y;
    }
    next.previous = p;
    next.count = state.count + 1u;
    return next;
}

fn corner_coverage(a: vec2<f32>, v1: vec2<f32>, v2: vec2<f32>, scale: f32) -> f32 {
    let square = array<vec2<f32>, 4>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, 0.5),
    );
    // One half-plane adds at most one vertex to a convex square.
    var clipped: array<vec2<f32>, 5>;
    var n = 0u;
    var previous = square[3];
    var previous_distance = a.x + scale * dot(v1, previous);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let p = square[i];
        let distance = a.x + scale * dot(v1, p);
        if (distance <= 0.0) != (previous_distance <= 0.0) {
            let t = previous_distance / (previous_distance - distance);
            clipped[n] = previous + t * (p - previous);
            n = n + 1u;
        }
        if distance <= 0.0 {
            clipped[n] = p;
            n = n + 1u;
        }
        previous = p;
        previous_distance = distance;
    }
    // Stream the second clip straight into the shoelace sum. This visits
    // the same vertices in the same order without a second polygon array.
    var area = PolygonArea(vec2<f32>(0.0), vec2<f32>(0.0), 0.0, 0u);
    previous = clipped[n - 1u];
    previous_distance = a.y + scale * dot(v2, previous);
    for (var i = 0u; i < n; i = i + 1u) {
        let p = clipped[i];
        let distance = a.y + scale * dot(v2, p);
        if (distance <= 0.0) != (previous_distance <= 0.0) {
            let t = previous_distance / (previous_distance - distance);
            area = append_area(area, previous + t * (p - previous));
        }
        if distance <= 0.0 {
            area = append_area(area, p);
        }
        previous = p;
        previous_distance = distance;
    }
    let sum = area.area + area.previous.x * area.first.y - area.first.x * area.previous.y;
    return clamp(abs(sum) * 0.5, 0.0, 1.0);
}

// Coverage of the shape `s` at local point `p`, `m` mapping local to device.
fn shape_coverage(s: Shape, p: vec2<f32>, m: array<vec4<f32>, 2>, specialize_quadratic: bool) -> f32 {
    let sample = sdf_sample(s, p, specialize_quadratic);
    let g = sample.gradient;
    // A sharp corner inside this pixel: the exact area inside both
    // half-planes. `a`, `sgn`, and the quadrant radius mirror `sdf_sample`;
    // the strict `<` keeps a corner exactly on a pixel boundary on the
    // old path.
    let sgn = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    let right = p.x > 0.0;
    let bottom = p.y > 0.0;
    let r = select(
        select(s.radii.x, s.radii.w, bottom),
        select(s.radii.y, s.radii.z, bottom),
        right,
    );
    let rx = max(r, 0.0);
    let ry = rx * s.aspect;
    let a = abs(p) - s.half;
    if rx <= 0.0 || ry <= 0.0 {
        let scale = device_grad_scale(m);
        let v1 = device_grad_vec(m, vec2<f32>(sgn.x, 0.0));
        let v2 = device_grad_vec(m, vec2<f32>(0.0, sgn.y));
        let h1 = 0.5 * scale * (abs(v1.x) + abs(v1.y));
        let h2 = 0.5 * scale * (abs(v2.x) + abs(v2.y));
        if abs(a.x) < h1 && abs(a.y) < h2 {
            return corner_coverage(a, v1, v2, scale);
        }
    }
    return coverage_dir(
        sample.distance,
        device_grad_vec(m, g.xy),
        device_grad_scale(m),
        g.w > 0.0,
        g.z,
    );
}


// Coverage of the instance's clip shape and mask at `in.device`: the
// geometric clip SDF times the atlas-or-texture mask texel, matching
// fs_full's own evaluation verbatim.
fn clip_mask_coverage(in: VsOut) -> f32 {
    let i = in.instance;
    let flags = (in.meta_.w >> 24u) & 0xffu;
    var cov = 1.0;
    if (flags & FLAG_HAS_CLIP) != 0u {
        // `clip_inv` maps device to clip-local: J^-T is its transpose.
        let pc = apply(instances[i].clip_inv, in.device);
        let sample = sdf_sample(instances[i].clip, pc, false);
        let g = sample.gradient;
        let ci = instances[i].clip_inv;
        let dg = vec2<f32>(ci[0].x * g.x + ci[0].y * g.y, ci[0].z * g.x + ci[0].w * g.y);
        cov = coverage_dir(sample.distance, dg, 1.0, g.w > 0.0, g.z);
    }
    if (flags & FLAG_HAS_MASK) != 0u {
        // Mask texel for this device pixel; texels outside the cell
        // contribute zero coverage.
        let mp = floor(in.device) - in.params.zw;
        let msize = vec2<f32>(instances[i].clip.aspect, instances[i].clip.exponent);
        let inside = all(mp >= vec2<f32>(0.0)) && all(mp < msize);
        var m: f32;
        if (flags & FLAG_MASK_TEXTURE) != 0u {
            m = textureLoad(mask_tex, vec2<i32>(mp), 0).r;
        } else {
            m = textureLoad(atlas, vec2<i32>(mp) + vec2<i32>(instances[i].uv.zw), 0).r;
        }
        cov *= select(0.0, m, inside);
    }
    return cov;
}

fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// The sRGB transfer pair preserving sign: a premultiplied pixel's
// channel can sit below the encoded black level only by going negative,
// so the byte-pipeline variants above clamp; the compositing converter
// matches the CPU backend's `convert_pixel` and does not.
fn srgb_encode_signed(c: vec3<f32>) -> vec3<f32> {
    let a = abs(c);
    let lo = a * 12.92;
    let hi = 1.055 * pow(a, vec3<f32>(1.0 / 2.4)) - 0.055;
    return sign(c) * select(hi, lo, a <= vec3<f32>(0.0031308));
}

fn srgb_decode_signed(c: vec3<f32>) -> vec3<f32> {
    let a = abs(c);
    let lo = a / 12.92;
    let hi = pow((a + 0.055) / 1.055, vec3<f32>(2.4));
    return sign(c) * select(hi, lo, a <= vec3<f32>(0.04045));
}

// Linear sRGB -> linear Display P3 (column-major constructor: columns).
const SRGB_TO_P3 = mat3x3<f32>(
    vec3<f32>(0.8224621, 0.0331941, 0.0170827),
    vec3<f32>(0.1775380, 0.9668058, 0.0723974),
    vec3<f32>(0.0, 0.0, 0.9105199),
);

// Linear Display-P3 -> linear sRGB primaries (column-major
// constructor: columns), the transpose of the CPU backend's
// row-major `P3_TO_SRGB`.
const P3_TO_SRGB = mat3x3<f32>(
    vec3<f32>(1.2249401, -0.0420569, -0.0196376),
    vec3<f32>(-0.2249404, 1.0420571, -0.0786361),
    vec3<f32>(0.0, 0.0, 1.0982735),
);

// One premultiplied pixel moved between spaces: unpremultiply,
// convert the straight colour (P3->sRGB then transfer-encode when
// going to sRGB, transfer-decode then sRGB->P3 back), repremultiply.
// Alpha carries through; a transparent pixel stays a transparent pixel
// so masks composite correctly.
fn move_space(px: vec4<f32>, src: u32, dst: u32) -> vec4<f32> {
    if (src == dst) {
        return px;
    }
    if (px.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let straight = px.rgb / px.a;
    if (dst == SPACE_SRGB) {
        let enc = srgb_encode_signed(P3_TO_SRGB * straight);
        return vec4<f32>(enc * px.a, px.a);
    }
    let dec = SRGB_TO_P3 * srgb_decode_signed(straight);
    return vec4<f32>(dec * px.a, px.a);
}