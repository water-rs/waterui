// Fragment shader preamble for fused color-only filter shaders.
// This enables rendering to any texture format including HDR (Rgba16Float).
//
// Token contract (substituted by the runtime, never valid WGSL as-is):
// - `PARAM_VEC4S`: number of `vec4<f32>` rows in the parameter array,
//   derived from `filtrate_core::MAX_FILTER_PARAM_VEC4S`.
// - `CLAMP_MAX_BOUND`: value of the `COLOR_CLAMP_MAX` constant below —
//   1.0 when the pass targets an LDR format, the f16 maximum when it
//   targets an HDR format, so LDR-styled presets never crush an HDR
//   chain's highlights.
//
// Alpha contract: input and output textures are premultiplied alpha. The
// preamble unpremultiplies once so every fragment operates on straight-alpha
// color, and the postamble re-premultiplies the final result.

// Uniform buffer with proper alignment for WGSL.
// Note: array<f32, N> is NOT valid in uniform buffers due to 16-byte stride
// requirements, so parameters pack into vec4 rows.
struct Uniforms {
    dimensions: vec2<f32>,
    _padding: vec2<f32>,  // Pad to 16-byte alignment before array
    params: array<vec4<f32>, PARAM_VEC4S>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen quad using 6 vertices (2 triangles)
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );

    // UV coordinates with Y-flip for texture sampling
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = uvs[vertex_index];
    return output;
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

// Helper: Read parameter at index from packed vec4 array
fn param(index: u32) -> f32 {
    let vec_idx = index / 4u;
    let component = index % 4u;
    let v = uniforms.params[vec_idx];
    // Extract component (x=0, y=1, z=2, w=3)
    switch component {
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

// Display-range clamp bound for LDR-styled fragments (photo-effect
// presets). Substituted per target format: 1.0 for LDR, f16 max for HDR.
const COLOR_CLAMP_MAX: f32 = CLAMP_MAX_BOUND;

// Isotropic space: per-axis normalized uv is anisotropic on non-square
// targets. Radial fragments (vignette) convert distances into this space,
// where one unit equals the shorter output edge, so circles stay circular
// at any aspect ratio.
fn isotropic_scale() -> vec2<f32> {
    let dims = uniforms.dimensions;
    return dims / min(dims.x, dims.y);
}

fn to_isotropic(uv: vec2<f32>) -> vec2<f32> {
    return uv * isotropic_scale();
}

// Helper: Convert RGB to HSL
fn rgb_to_hsl(rgb: vec3<f32>) -> vec3<f32> {
    let max_c = max(max(rgb.r, rgb.g), rgb.b);
    let min_c = min(min(rgb.r, rgb.g), rgb.b);
    let l = (max_c + min_c) / 2.0;

    if max_c == min_c {
        return vec3<f32>(0.0, 0.0, l);
    }

    let d = max_c - min_c;
    let s = select(d / (2.0 - max_c - min_c), d / (max_c + min_c), l > 0.5);

    var h: f32;
    if max_c == rgb.r {
        h = (rgb.g - rgb.b) / d + select(0.0, 6.0, rgb.g < rgb.b);
    } else if max_c == rgb.g {
        h = (rgb.b - rgb.r) / d + 2.0;
    } else {
        h = (rgb.r - rgb.g) / d + 4.0;
    }
    h /= 6.0;

    return vec3<f32>(h, s, l);
}

// Helper: HSL to RGB
fn hue_to_rgb(p: f32, q: f32, t_in: f32) -> f32 {
    var t = t_in;
    if t < 0.0 { t += 1.0; }
    if t > 1.0 { t -= 1.0; }
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 1.0 / 2.0 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
    if hsl.y == 0.0 {
        return vec3<f32>(hsl.z, hsl.z, hsl.z);
    }

    let q = select(hsl.z + hsl.y - hsl.z * hsl.y, hsl.z * (1.0 + hsl.y), hsl.z < 0.5);
    let p = 2.0 * hsl.z - q;

    return vec3<f32>(
        hue_to_rgb(p, q, hsl.x + 1.0 / 3.0),
        hue_to_rgb(p, q, hsl.x),
        hue_to_rgb(p, q, hsl.x - 1.0 / 3.0)
    );
}

// Fragment shader entry point - will be followed by filter fragments and postamble
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    // Use an explicit mip level to support unfilterable float textures (e.g. HDR formats).
    var color = textureSampleLevel(input_texture, input_sampler, uv, 0.0);

    // Unpremultiply once: fragments below operate on straight-alpha color.
    // Fully transparent premultiplied texels have rgb == 0, so the max()
    // guard cannot manufacture color there.
    color = vec4<f32>(color.rgb / max(color.a, 1e-6), color.a);

    // Parameter index tracker (incremented by each filter fragment)
    var param_idx: u32 = 0u;
