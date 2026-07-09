// Sharpen shader - standalone pass (spatial filter, cannot fuse)
//
// Untiled by design: the five overlapping laplacian taps hit the GPU
// texture cache, which benchmarks as fast as workgroup shared-memory
// tiling without the barriers and threadgroup-memory occupancy cost.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

fn param(index: u32) -> f32 {
    let vec_idx = index / 4u;
    let component = index % 4u;
    let v = uniforms.params[vec_idx];
    switch component {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

fn load_clamped(coord: vec2<i32>, input_dims: vec2<i32>) -> vec4<f32> {
    let clamped = clamp(coord, vec2<i32>(0), input_dims - vec2<i32>(1));
    return textureLoad(input_texture, clamped, 0);
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let input_dims_i = vec2<i32>(vec2<u32>(uniforms.input_dimensions));
    let coord = vec2<i32>(global_id.xy);
    let amount = param(0u);

    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center_coord = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims_i - vec2<i32>(1));

    let center = textureLoad(input_texture, center_coord, 0);
    let top = load_clamped(center_coord + vec2<i32>(0, -1), input_dims_i);
    let bottom = load_clamped(center_coord + vec2<i32>(0, 1), input_dims_i);
    let left = load_clamped(center_coord + vec2<i32>(-1, 0), input_dims_i);
    let right = load_clamped(center_coord + vec2<i32>(1, 0), input_dims_i);

    // Laplacian kernel
    let laplacian = center * 4.0 - top - bottom - left - right;

    // Add sharpened detail
    let result = center + laplacian * amount;
    textureStore(output_texture, coord, vec4<f32>(result.rgb, center.a));
}
