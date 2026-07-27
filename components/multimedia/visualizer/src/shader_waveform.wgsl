// Uniforms struct layout matches encase-generated layout
struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    sensitivity: f32,
    // encase automatically adds padding before vec3
    bg_color: vec3<f32>,
    line_width: f32,
    glow_intensity: f32,
    // encase automatically adds padding before vec3
    line_color: vec3<f32>,
    // encase automatically adds padding before vec3
    glow_color: vec3<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var samples: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: VertexOutput;
    out.position = vec4<f32>(positions[idx], 0.0, 1.0);
    out.uv = positions[idx] * 0.5 + 0.5;
    return out;
}

// Catmull-Rom Cubic Spline Interpolation
fn get_sample(t: f32) -> f32 {
    let len = f32(textureDimensions(samples).x);
    let idx_f = t * (len - 1.0);
    let i = u32(idx_f);
    let f = fract(idx_f);

    let i0 = select(i - 1u, 0u, i == 0u);
    let i1 = i;
    let i2 = min(i + 1u, u32(len - 1.0));
    let i3 = min(i + 2u, u32(len - 1.0));

    let s0 = textureLoad(samples, vec2<i32>(i32(i0), 0), 0).r;
    let s1 = textureLoad(samples, vec2<i32>(i32(i1), 0), 0).r;
    let s2 = textureLoad(samples, vec2<i32>(i32(i2), 0), 0).r;
    let s3 = textureLoad(samples, vec2<i32>(i32(i3), 0), 0).r;

    let t2 = f * f;
    let t3 = t2 * f;

    let a = -0.5 * s0 + 1.5 * s1 - 1.5 * s2 + 0.5 * s3;
    let b = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
    let c = -0.5 * s0 + 0.5 * s2;
    let d = s1;

    return a * t3 + b * t2 + c * f + d;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    var color = uniforms.bg_color;

    // Amplitude scaling
    let scale = 0.4 * uniforms.sensitivity;
    
    // Get interpolated sample value (-1.0 to 1.0)
    let val = get_sample(uv.x);
    let y_pos = 0.5 + val * scale;
    
    let dist = abs(uv.y - y_pos);
    
    // Dynamic line width relative to resolution
    let width = max(uniforms.line_width, 1.0) / uniforms.resolution.y;
    
    // Core line
    let core = smoothstep(width, width * 0.5, dist);
    
    // Glow effect
    let glow_width = width * 8.0;
    let glow = exp(-dist / glow_width) * uniforms.glow_intensity;
    
    // Composite
    if (core > 0.0) {
        color = mix(color, uniforms.line_color, core);
    }
    
    // Additive glow
    color = color + uniforms.glow_color * glow;

    return vec4<f32>(color, 1.0);
}
