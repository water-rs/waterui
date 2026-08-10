// Gradient shader for WaterUI
// Supports linear, radial, angular, and mesh gradients

// Constants matching Rust side
const GRADIENT_LINEAR: u32 = 0u;
const GRADIENT_RADIAL: u32 = 1u;
const GRADIENT_ANGULAR: u32 = 2u;
const GRADIENT_MESH: u32 = 3u;

const MAX_COLOR_STOPS: u32 = 16u;
const MAX_MESH_VERTICES: u32 = 64u;

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;

// Uniform buffer
struct GradientUniforms {
    gradient_type: u32,
    num_stops: u32,
    mesh_width: u32,
    mesh_height: u32,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
    start_value: f32,
    end_value: f32,
    smooths_colors: u32,
    _padding: u32,
}

// Color stop structure
// Note: Uses three separate f32 fields instead of vec3<f32> for padding
// because vec3<f32> has 16-byte alignment in WGSL, which would cause
// struct size mismatch with Rust's [f32; 3] (4-byte alignment).
struct ColorStop {
    color: vec4<f32>,
    position: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

// Mesh vertex structure
struct MeshVertex {
    position: vec2<f32>,
    _padding1: vec2<f32>,
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: GradientUniforms;
@group(0) @binding(1) var<storage, read> stops: array<ColorStop>;
@group(0) @binding(2) var<storage, read> mesh_vertices: array<MeshVertex>;

// Vertex shader output
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Full-screen quad vertex shader
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );

    let pos = positions[vertex_index];
    var output: VertexOutput;
    output.position = vec4<f32>(pos, 0.0, 1.0);
    // UV: (0,0) at top-left, (1,1) at bottom-right
    output.uv = vec2<f32>((pos.x + 1.0) * 0.5, (1.0 - pos.y) * 0.5);
    return output;
}

// Sample color from stops at a given t value (0.0 to 1.0)
fn sample_gradient(t: f32) -> vec4<f32> {
    let clamped_t = clamp(t, 0.0, 1.0);

    if uniforms.num_stops == 0u {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    if uniforms.num_stops == 1u {
        return stops[0].color;
    }

    // Find the two stops to interpolate between
    var lower_idx: u32 = 0u;
    var upper_idx: u32 = 0u;

    for (var i: u32 = 0u; i < uniforms.num_stops; i++) {
        if stops[i].position <= clamped_t {
            lower_idx = i;
        }
        if stops[i].position >= clamped_t && upper_idx == 0u {
            upper_idx = i;
            break;
        }
    }

    // Handle edge cases
    if upper_idx == 0u {
        upper_idx = uniforms.num_stops - 1u;
    }
    if lower_idx == upper_idx {
        return stops[lower_idx].color;
    }

    // Interpolate between the two stops
    let lower_pos = stops[lower_idx].position;
    let upper_pos = stops[upper_idx].position;
    let range = upper_pos - lower_pos;

    if range <= 0.0 {
        return stops[lower_idx].color;
    }

    let local_t = (clamped_t - lower_pos) / range;
    return mix(stops[lower_idx].color, stops[upper_idx].color, local_t);
}

// Linear gradient
fn linear_gradient(uv: vec2<f32>) -> vec4<f32> {
    let start = vec2<f32>(uniforms.start_x, uniforms.start_y);
    let end = vec2<f32>(uniforms.end_x, uniforms.end_y);
    let dir = end - start;
    let len_sq = dot(dir, dir);

    if len_sq < 0.0001 {
        return sample_gradient(0.0);
    }

    let t = dot(uv - start, dir) / len_sq;
    return sample_gradient(t);
}

// Radial gradient
fn radial_gradient(uv: vec2<f32>) -> vec4<f32> {
    let center = vec2<f32>(uniforms.start_x, uniforms.start_y);
    let dist = length(uv - center);

    let start_r = uniforms.start_value;
    let end_r = uniforms.end_value;
    let range = end_r - start_r;

    if abs(range) < 0.0001 {
        return sample_gradient(0.0);
    }

    let t = (dist - start_r) / range;
    return sample_gradient(t);
}

// Angular (conic) gradient
fn angular_gradient(uv: vec2<f32>) -> vec4<f32> {
    let center = vec2<f32>(uniforms.start_x, uniforms.start_y);
    let delta = uv - center;

    // atan2 returns angle in [-PI, PI], we normalize to [0, 1]
    var angle = atan2(delta.y, delta.x);

    let start_angle = uniforms.start_value;
    let end_angle = uniforms.end_value;
    let range = end_angle - start_angle;

    if abs(range) < 0.0001 {
        return sample_gradient(0.0);
    }

    // Normalize angle to the range [start_angle, end_angle]
    let t = (angle - start_angle) / range;
    return sample_gradient(fract(t)); // fract for wrapping
}

// Bilinear interpolation for mesh gradient
fn mesh_gradient(uv: vec2<f32>) -> vec4<f32> {
    let w = uniforms.mesh_width;
    let h = uniforms.mesh_height;

    if w < 2u || h < 2u {
        return vec4<f32>(0.5, 0.5, 0.5, 1.0);
    }

    // Find the cell containing this UV
    let cell_x = uv.x * f32(w - 1u);
    let cell_y = uv.y * f32(h - 1u);

    let ix = u32(floor(cell_x));
    let y_index = u32(floor(cell_y));

    let fx = fract(cell_x);
    let fy = fract(cell_y);

    // Clamp indices
    let x0 = min(ix, w - 2u);
    let y0 = min(y_index, h - 2u);
    let x1 = x0 + 1u;
    let y1 = y0 + 1u;

    // Get the four corner vertices
    let idx00 = y0 * w + x0;
    let idx10 = y0 * w + x1;
    let idx01 = y1 * w + x0;
    let idx11 = y1 * w + x1;

    let c00 = mesh_vertices[idx00].color;
    let c10 = mesh_vertices[idx10].color;
    let c01 = mesh_vertices[idx01].color;
    let c11 = mesh_vertices[idx11].color;

    // Bilinear interpolation
    var tx = fx;
    var ty = fy;

    // Smooth interpolation if enabled
    if uniforms.smooths_colors != 0u {
        tx = tx * tx * (3.0 - 2.0 * tx); // smoothstep
        ty = ty * ty * (3.0 - 2.0 * ty);
    }

    let top = mix(c00, c10, tx);
    let bottom = mix(c01, c11, tx);
    return mix(top, bottom, ty);
}

// Fragment shader
@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    switch uniforms.gradient_type {
        case GRADIENT_LINEAR: {
            return linear_gradient(uv);
        }
        case GRADIENT_RADIAL: {
            return radial_gradient(uv);
        }
        case GRADIENT_ANGULAR: {
            return angular_gradient(uv);
        }
        case GRADIENT_MESH: {
            return mesh_gradient(uv);
        }
        default: {
            return vec4<f32>(1.0, 0.0, 1.0, 1.0); // Magenta for unknown
        }
    }
}
