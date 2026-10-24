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
const KIND_POLYGON: u32 = 7u;       // appended opaque interior tier (opaque phase only)

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
    // The local point of a local-space quad. A device-space quad
    // (`kind >= KIND_GLYPH`) has none: a fragment that paints it derives it
    // from the pixel (`device_quad_pixel`).
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
    // A GPU-routed buffer cell's texel word address in the coverage
    // arena; zero for every other draw.
    @location(11) @interpolate(flat) cell_texel: u32,
}

// The angle of the vector (x, y) in [-pi, pi], the range of the oracle's
// and the CPU kernel's `atan2(y, x)`: a zero `y` of either sign takes the
// upper branch, so (x < 0, y = 0) is pi, and the zero vector is 0.
//
// Not `atan2`, whose result at the zero vector is unspecified: the sweep
// gradient's centre pixel drew a stray colour from it (#3234), and the
// compute stroker's round join between perpendicular tangents became a
// bevel with it (#2544). The angle comes from the normalized x instead.
fn angle_of(y: f32, x: f32) -> f32 {
    let r = length(vec2<f32>(y, x));
    if r == 0.0 {
        return 0.0;
    }
    return select(-1.0, 1.0, y >= 0.0) * acos(clamp(x / r, -1.0, 1.0));
}

fn apply(m: array<vec4<f32>, 2>, p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        m[0].x * p.x + m[0].z * p.y + m[1].x,
        m[0].y * p.x + m[0].w * p.y + m[1].y,
    );
}

// cell-buffer
const FLAG_CELL_BUFFER: u32 = 0x80u;
// The coverage arena (water-rs/waterui#3081): one block per GPU-routed
// entry — count header, emitted geometry records, packed R8 texels.
@group(0) @binding(4) var<storage, read> coverage_arena: array<u32>;
// cell-buffer

// cell-buffer
// A GPU-routed draw joins its block's geometry records to the anchor
// instance's drawing state in the vertex stage: `vertex_index` encodes
// the anchor in its high bits ((A + 1) * 8 + corner), `instance_index`
// addresses the geometry slot. The anchor's `shape.half` carries the
// draw's shift and `uv.x` the block's texel base word, which leaves the
// join as `texel`, the cell's absolute texel word address.
struct GpuJoin {
    inst: Instance,
    anchor: u32,
    corner: u32,
    texel: u32,
}
fn join_gpu_instance(vi: u32, ii: u32) -> GpuJoin {
    let a = (vi >> 3u) - 1u;
    var inst = instances[a];
    let shift = inst.shape.half;
    let base = ii * 8u + 4u;
    let bounds = bitcast<vec4<f32>>(vec4<u32>(
        coverage_arena[base], coverage_arena[base + 1u],
        coverage_arena[base + 2u], coverage_arena[base + 3u]));
    let data = vec4<u32>(
        coverage_arena[base + 4u], coverage_arena[base + 5u],
        coverage_arena[base + 6u], coverage_arena[base + 7u]);
    inst.bounds = bounds + shift.xyxy;
    var kind = data.w;
    var texel = 0u;
    if kind == KIND_GLYPH {
        inst.shape.half = vec2<f32>(f32(data.z & 0xffffu), f32(data.z >> 16u));
        texel = bitcast<u32>(inst.uv.x) + data.x;
        inst.uv.y = f32(data.y);
        inst.meta_.w = inst.meta_.w | (FLAG_CELL_BUFFER << 24u);
        if data.z != 0u && inst.meta_.y == PAINT_SOLID {
            kind = KIND_REGION;
            inst.meta_.z = data.z;
        }
    }
    inst.meta_.x = kind;
    return GpuJoin(inst, a, vi & 7u, texel);
}
// cell-buffer

// Buffer cells carry local pixel coordinates, keeping large word offsets
// out of interpolated f32 values; CPU atlas cells carry atlas coordinates.
fn cell_texel_origin(origin: vec2<f32>, packed: u32) -> vec2<f32> {
    // cell-buffer
    if (packed & 0x80000000u) != 0u { return vec2<f32>(0.0); }
    // cell-buffer
    return origin;
}

// Decode the cell coordinate convention shared by the raster entry points.
fn path_cell_coverage(texel: vec2<i32>, origin: vec2<f32>, texel_base: u32, interior: vec2<f32>, packed: u32) -> f32 {
    // cell-buffer
    if (packed & 0x80000000u) != 0u {
        let local = vec2<u32>(texel);
        if local.x >= u32(interior.x) && local.x < u32(interior.y) { return 1.0; }
        let omitted = u32(interior.y - interior.x);
        let column = local.x - select(0u, omitted, local.x >= u32(interior.y));
        let pixel = local.y * (u32(origin.y) - omitted) + column;
        let word = coverage_arena[texel_base + pixel / 4u];
        return f32((word >> ((pixel % 4u) * 8u)) & 255u) / 255.0;
    }
    // cell-buffer
    return textureLoad(atlas, texel, 0).r;
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
    // cell-buffer
    // CPU draws always issue vi < 8; a GPU draw's first vertex encodes
    // its anchor, so vi >= 8 joins the geometry record at `ii`.
    if vi >= 8u {
        let joined = join_gpu_instance(vi, ii);
        var out = instance_vertex(joined.corner, joined.anchor, joined.inst);
        out.cell_texel = joined.texel;
        return out;
    }
    // cell-buffer
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
        out.local = vec2<f32>(0.0);
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

// A device-space quad's fragment (`kind >= KIND_GLYPH`: glyph and path
// cells, regions, spans) shades the pixel whose centre is its position in
// the attachment grid, offset by that grid's device-space origin — every
// pass's viewport maps device space onto the grid at
// `globals.attachment_origin`. Interpolating the device or local point
// across the quad instead would round by where the quad's corners lie, so
// the same pixel would paint differently whenever emission cut a path
// into other cells. The device point and the local point (`apply_inverse`
// of it) are therefore functions of the pixel alone. Local-space quads
// keep their interpolated points: their corners are the shape's own.
fn device_quad_pixel(in: VsOut) -> VsOut {
    var out = in;
    if in.meta_.x >= KIND_GLYPH {
        out.device = in.position.xy + globals.attachment_origin;
        out.local = apply_inverse(array<vec4<f32>, 2>(in.affine0, in.affine1), out.device);
    }
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
    var wm2 = 1.0;
    if n != 2.0 {
        wm2 = pow(max(w, 1e-30), n - 2.0);
    }
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
    // Only the last evaluated point contributes distance and curvature.
    // Refine to it without computing discarded foot geometry or taking
    // a final Newton step whose result would never be used.
    for (var i = 0; i < LAME_EVALS - 1; i++) {
        let pt = lame_point(r, n, w);
        let qc = q - pt.c;
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
    let pt = lame_point(r, n, w);
    let qc = q - pt.c;
    let len = length(pt.normal);
    let normal = pt.normal / len;
    // Curvature from the implicit form,
    // κ = (n - 1)(w^(2n-2) + w^(n-2))·r.x·r.y / (s·|normal|³).
    let p = pt.normal.y / r.x;
    let kappa = (n - 1.0) * (p * p + pt.wm2) * r.x * r.y / (pt.s * len * len * len);
    let foot = vec4<f32>(dot(qc, normal), normal, kappa);
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

// Exact area of the unit device pixel inside the half-planes
// `p[i].x + dot(p[i].yz, u) <= 0`, `u` the device offset from the pixel
// centre: each `x` the signed outside-positive distance at the centre in
// its own local units, `yz` the distance's device-space gradient in local
// units per device pixel. Sutherland–Hodgman clips the pixel square by
// every plane in turn, so where a draw edge and a clip edge coincide the
// two planes describe the same boundary and the joint coverage is the
// single edge's — not the product of two half-covered ramps that
// multiplying the shapes' separate coverages produces (#2933). Each clip
// adds at most one vertex to the convex square: six planes need ten.
alias JointPlanes = array<vec3<f32>, 6>;

fn planes_coverage(planes: JointPlanes, count: u32) -> f32 {
    var poly: array<vec2<f32>, 10>;
    poly[0] = vec2<f32>(-0.5, -0.5);
    poly[1] = vec2<f32>(0.5, -0.5);
    poly[2] = vec2<f32>(0.5, 0.5);
    poly[3] = vec2<f32>(-0.5, 0.5);
    var n = 4u;
    for (var k = 0u; k < count; k = k + 1u) {
        let pl = planes[k];
        var clipped: array<vec2<f32>, 10>;
        var m = 0u;
        var previous = poly[n - 1u];
        var previous_distance = pl.x + dot(pl.yz, previous);
        for (var i = 0u; i < n; i = i + 1u) {
            let p = poly[i];
            let distance = pl.x + dot(pl.yz, p);
            if (distance <= 0.0) != (previous_distance <= 0.0) {
                let t = clamp(previous_distance / (previous_distance - distance), 0.0, 1.0);
                clipped[m] = previous + t * (p - previous);
                m = m + 1u;
            }
            if distance <= 0.0 {
                clipped[m] = p;
                m = m + 1u;
            }
            previous = p;
            previous_distance = distance;
        }
        if m < 3u {
            return 0.0;
        }
        for (var i = 0u; i < m; i = i + 1u) {
            poly[i] = clipped[i];
        }
        n = m;
    }
    var twice = 0.0;
    var previous = poly[n - 1u];
    for (var i = 0u; i < n; i = i + 1u) {
        let p = poly[i];
        twice = twice + previous.x * p.y - previous.y * p.x;
        previous = p;
    }
    return clamp(abs(twice) * 0.5, 0.0, 1.0);
}

// A shape's boundary at the pixel as up to two half-planes in the
// `planes_coverage` form: both faces at a sharp-corner pixel, the SDF's
// tangent plane everywhere else. The gradient is J^-T times the local
// one — `device_grad_vec(m, g) / det`, signed so a mirroring keeps the
// outward side — which `coverage_dir`'s magnitude-only use never sees.
struct ShapePlanes {
    n: u32,
    p: array<vec3<f32>, 2>,
}

struct ShapeEval {
    cov: f32,
    planes: ShapePlanes,
}

fn shape_eval(s: Shape, p: vec2<f32>, m: array<vec4<f32>, 2>, specialize_quadratic: bool) -> ShapeEval {
    var out: ShapeEval;
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
    let det = m[0].x * m[0].w - m[0].y * m[0].z;
    if rx <= 0.0 || ry <= 0.0 {
        let scale = device_grad_scale(m);
        let v1 = device_grad_vec(m, vec2<f32>(sgn.x, 0.0));
        let v2 = device_grad_vec(m, vec2<f32>(0.0, sgn.y));
        let h1 = 0.5 * scale * (abs(v1.x) + abs(v1.y));
        let h2 = 0.5 * scale * (abs(v2.x) + abs(v2.y));
        if abs(a.x) < h1 && abs(a.y) < h2 {
            out.cov = corner_coverage(a, v1, v2, scale);
            out.planes.n = 2u;
            out.planes.p[0] = vec3<f32>(a.x, v1 / det);
            out.planes.p[1] = vec3<f32>(a.y, v2 / det);
            return out;
        }
    }
    out.cov = coverage_dir(
        sample.distance,
        device_grad_vec(m, g.xy),
        device_grad_scale(m),
        g.w > 0.0,
        g.z,
    );
    out.planes.n = 1u;
    out.planes.p[0] = vec3<f32>(sample.distance, device_grad_vec(m, g.xy) / det);
    return out;
}

// Coverage of the shape `s` at local point `p`, `m` mapping local to device.
fn shape_coverage(s: Shape, p: vec2<f32>, m: array<vec4<f32>, 2>, specialize_quadratic: bool) -> f32 {
    return shape_eval(s, p, m, specialize_quadratic).cov;
}

// The clip's boundary half-planes at the fragment, in the
// `planes_coverage` form: `clip_inv` is device → clip-local, so J^-T is
// its own transpose — the same mapping `clip_mask_coverage` applies to
// the SDF gradient.
fn clip_planes(in: VsOut) -> ShapePlanes {
    var out: ShapePlanes;
    let i = in.instance;
    let s = instances[i].clip;
    let ci = instances[i].clip_inv;
    let pc = apply(ci, in.device);
    let sample = sdf_sample(s, pc, false);
    let g = sample.gradient;
    let sgn = select(vec2<f32>(-1.0), vec2<f32>(1.0), pc >= vec2<f32>(0.0));
    let right = pc.x > 0.0;
    let bottom = pc.y > 0.0;
    let r = select(
        select(s.radii.x, s.radii.w, bottom),
        select(s.radii.y, s.radii.z, bottom),
        right,
    );
    let rx = max(r, 0.0);
    let ry = rx * s.aspect;
    let a = abs(pc) - s.half;
    if rx <= 0.0 || ry <= 0.0 {
        let v1 = vec2<f32>(ci[0].x * sgn.x, ci[0].z * sgn.x);
        let v2 = vec2<f32>(ci[0].y * sgn.y, ci[0].w * sgn.y);
        let h1 = 0.5 * (abs(v1.x) + abs(v1.y));
        let h2 = 0.5 * (abs(v2.x) + abs(v2.y));
        if abs(a.x) < h1 && abs(a.y) < h2 {
            out.n = 2u;
            out.p[0] = vec3<f32>(a.x, v1);
            out.p[1] = vec3<f32>(a.y, v2);
            return out;
        }
    }
    out.n = 1u;
    out.p[0] = vec3<f32>(
        sample.distance,
        vec2<f32>(ci[0].x * g.x + ci[0].y * g.y, ci[0].z * g.x + ci[0].w * g.y),
    );
    return out;
}


// The clip's atlas-or-texture mask texel at the fragment, or 1 where the
// clip carries no mask. The mask stays a coverage product: a raster cell
// has no signed-distance boundary a half-plane could intersect.
fn mask_coverage(in: VsOut) -> f32 {
    let i = in.instance;
    let flags = (in.meta_.w >> 24u) & 0xffu;
    if (flags & FLAG_HAS_MASK) == 0u {
        return 1.0;
    }
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
    return select(0.0, m, inside);
}

// Coverage of the instance's clip shape and mask at `in.device`: the
// geometric clip SDF times the atlas-or-texture mask texel, matching
// fs_full's own evaluation verbatim.
fn clip_mask_coverage(in: VsOut) -> f32 {
    return clip_mask_coverage_shape(in, instances[in.instance].clip);
}

fn clip_mask_coverage_shape(in: VsOut, clip: Shape) -> f32 {
    let i = in.instance;
    let flags = (in.meta_.w >> 24u) & 0xffu;
    var cov = 1.0;
    if (flags & FLAG_HAS_CLIP) != 0u {
        // `clip_inv` maps device to clip-local: J^-T is its transpose.
        let pc = apply(instances[i].clip_inv, in.device);
        let sample = sdf_sample(clip, pc, false);
        let g = sample.gradient;
        let ci = instances[i].clip_inv;
        let dg = vec2<f32>(ci[0].x * g.x + ci[0].y * g.y, ci[0].z * g.x + ci[0].w * g.y);
        cov = coverage_dir(sample.distance, dg, 1.0, g.w > 0.0, g.z);
    }
    cov *= mask_coverage(in);
    return cov;
}

// One premultiplied pixel moved between spaces: the space fragment's
// `space_to_srgb` / `space_from_srgb` (filtrate-core's `space::WGSL`,
// which precedes this file in every build), the conversion the CPU
// backend and the filter stages run too. Alpha carries through; a pixel
// with zero alpha becomes transparent black.
fn move_space(px: vec4<f32>, src: u32, dst: u32) -> vec4<f32> {
    if (src == dst) {
        return px;
    }
    if (dst == SPACE_SRGB) {
        return space_to_srgb(px);
    }
    return space_from_srgb(px);
}
