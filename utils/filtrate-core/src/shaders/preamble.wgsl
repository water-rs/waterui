// Preamble for fused color-only filter shaders
// This file is included at the start of compiled filter shaders.

struct Uniforms {
    dimensions: vec2<f32>,
    // Params are packed as f32 array, up to 64 floats
    params: array<f32, 64>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

// Helper: Read parameter at index
fn param(index: u32) -> f32 {
    return uniforms.params[index];
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

// Main entry point - will be followed by filter fragments and postamble
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = vec2<u32>(uniforms.dimensions);
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let coord = vec2<i32>(global_id.xy);
    let uv = vec2<f32>(global_id.xy) / uniforms.dimensions;
    var color = textureLoad(input_texture, coord, 0);

    // Parameter index tracker (incremented by each filter fragment)
    var param_idx: u32 = 0u;
