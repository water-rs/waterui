// Shared preamble for spatial compute filter shaders.
//
// The runtime prepends this file to every spatial shader source before
// compilation (see `specialize_spatial_shader`), so individual shaders only
// contain their `@compute` entry point plus filter-specific helpers.
//
// Token contract (substituted by the runtime, never valid WGSL as-is):
// - `OUTPUT_STORAGE_FORMAT`: storage texture format of the output binding.
// - `WORKGROUP_X` / `WORKGROUP_Y`: workgroup shape, referenced by each
//   shader's `@workgroup_size(WORKGROUP_X, WORKGROUP_Y)` declaration.
// - `PARAM_VEC4S`: number of `vec4<f32>` rows in the parameter array,
//   derived from `filtrate_core::MAX_FILTER_PARAM_VEC4S`.
//
// Alpha contract: every texture is premultiplied alpha. Linear operations
// (blurs, convolutions, resampling) are correct on premultiplied data as-is
// and must process all four channels together.
//
// Untiled by design: spatial shaders read through the GPU texture cache with
// no workgroup shared-memory tiling, which benchmarks at the memory-bandwidth
// floor on tiler GPUs. Do not add `var<workgroup>` storage.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    // Dimensions of `original_texture` (only meaningful for shaders recorded
    // via `spatial_shader_with_original`).
    original_dimensions: vec2<f32>,
    _padding: vec2<f32>,
    params: array<vec4<f32>, PARAM_VEC4S>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
// Bound only for `spatial_shader_with_original` passes: the texture that fed
// this filter's first stage. Shaders that do not reference it compile without
// the binding being present in the layout.
@group(0) @binding(3) var original_texture: texture_2d<f32>;

// Reads parameter `index` from the packed vec4 array.
fn param(index: u32) -> f32 {
    let v = uniforms.params[index / 4u];
    switch index % 4u {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

// Rec. 709 luma coefficients — the single luminance definition shared by all
// filtrate shaders. Do not introduce per-shader alternatives.
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

fn luminance(rgb: vec3<f32>) -> f32 {
    return dot(rgb, LUMA);
}

const DEGREES_TO_RADIANS: f32 = 0.017453292519943295;

// Center of the output pixel in normalized [0, 1] output space.
fn output_uv(gid: vec2<u32>) -> vec2<f32> {
    return (vec2<f32>(gid) + vec2<f32>(0.5)) / uniforms.output_dimensions;
}

// Clamped integer load from the input texture.
fn load_input(coord: vec2<i32>) -> vec4<f32> {
    let max_xy = vec2<i32>(uniforms.input_dimensions) - vec2<i32>(1);
    return textureLoad(input_texture, clamp(coord, vec2<i32>(0), max_xy), 0);
}

// Clamped integer load from the original texture.
fn load_original(coord: vec2<i32>) -> vec4<f32> {
    let max_xy = vec2<i32>(uniforms.original_dimensions) - vec2<i32>(1);
    return textureLoad(original_texture, clamp(coord, vec2<i32>(0), max_xy), 0);
}

// Maps an output pixel to the corresponding input texel (center-mapped,
// clamped). Identity when input and output dimensions match.
fn map_to_input(gid: vec2<u32>) -> vec2<i32> {
    let mapped = (vec2<f32>(gid) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let max_xy = vec2<i32>(uniforms.input_dimensions) - vec2<i32>(1);
    return clamp(vec2<i32>(mapped), vec2<i32>(0), max_xy);
}

// Maps an output pixel to the corresponding original texel (center-mapped,
// clamped). Identity when original and output dimensions match.
fn map_to_original(gid: vec2<u32>) -> vec2<i32> {
    let mapped = (vec2<f32>(gid) + vec2<f32>(0.5)) * uniforms.original_dimensions
        / uniforms.output_dimensions;
    let max_xy = vec2<i32>(uniforms.original_dimensions) - vec2<i32>(1);
    return clamp(vec2<i32>(mapped), vec2<i32>(0), max_xy);
}

// Manual bilinear sample of the input texture at a normalized [0, 1]
// coordinate. Spatial passes bind no sampler (storage-format outputs are
// unfilterable), so warps interpolate here instead of truncating to the
// nearest texel.
fn sample_input_bilinear(uv: vec2<f32>) -> vec4<f32> {
    let pos = uv * uniforms.input_dimensions - vec2<f32>(0.5);
    let base = floor(pos);
    let t = pos - base;
    let b = vec2<i32>(base);
    let c00 = load_input(b);
    let c10 = load_input(b + vec2<i32>(1, 0));
    let c01 = load_input(b + vec2<i32>(0, 1));
    let c11 = load_input(b + vec2<i32>(1, 1));
    return mix(mix(c00, c10, t.x), mix(c01, c11, t.x), t.y);
}

// --- Isotropic space -------------------------------------------------------
//
// Per-axis normalized uv is anisotropic on non-square outputs: a distance of
// 0.1 covers different pixel counts on each axis. Radial filters (vignette,
// twirl, bump, …) convert into this isotropic space, where one unit equals
// the shorter output edge, so circles stay circular at any aspect ratio.

fn isotropic_scale() -> vec2<f32> {
    let dims = uniforms.output_dimensions;
    return dims / min(dims.x, dims.y);
}

fn to_isotropic(uv: vec2<f32>) -> vec2<f32> {
    return uv * isotropic_scale();
}

fn from_isotropic(p: vec2<f32>) -> vec2<f32> {
    return p / isotropic_scale();
}

// Rotates `v` by `angle` radians.
fn rotate2(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let s = sin(angle);
    let c = cos(angle);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

// --- Homography ------------------------------------------------------------
//
// Projective mapping from the unit square to an arbitrary quad. Columns are
// the matrix columns of H such that H * (u, v, 1) projects to the quad point
// for square coordinates (u, v).

fn unit_square_homography(
    tl: vec2<f32>,
    tr: vec2<f32>,
    br: vec2<f32>,
    bl: vec2<f32>,
) -> mat3x3<f32> {
    let s = tl - tr + br - bl;
    let d1 = tr - br;
    let d2 = bl - br;
    let denom = d1.x * d2.y - d1.y * d2.x;
    var g = 0.0;
    var h = 0.0;
    if abs(s.x) > 1e-6 || abs(s.y) > 1e-6 {
        g = (s.x * d2.y - s.y * d2.x) / denom;
        h = (d1.x * s.y - d1.y * s.x) / denom;
    }
    let a = tr.x - tl.x + g * tr.x;
    let b = bl.x - tl.x + h * bl.x;
    let c = tl.x;
    let d = tr.y - tl.y + g * tr.y;
    let e = bl.y - tl.y + h * bl.y;
    let f = tl.y;
    return mat3x3<f32>(
        vec3<f32>(a, d, g),
        vec3<f32>(b, e, h),
        vec3<f32>(c, f, 1.0),
    );
}

// Applies a homography with the projective divide.
fn apply_homography(m: mat3x3<f32>, p: vec2<f32>) -> vec2<f32> {
    let q = m * vec3<f32>(p, 1.0);
    return q.xy / q.z;
}

// Inverts a 3x3 matrix via the adjugate. Homographies are scale-invariant,
// so the determinant divide can be skipped by callers that immediately
// re-divide by w — kept here for clarity since this runs per pixel constant.
fn inverse3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let c0 = cross(m[1], m[2]);
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    let det = dot(m[0], c0);
    // The crosses are the ROWS of the inverse; transpose into columns.
    return transpose(mat3x3<f32>(c0, c1, c2)) * (1.0 / det);
}
