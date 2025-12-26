// Sharpen shader - standalone pass (spatial filter, cannot fuse)

struct Uniforms {
    dimensions: vec2<f32>,
    amount: f32,
    _padding: f32,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = vec2<u32>(uniforms.dimensions);
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let coord = vec2<i32>(global_id.xy);
    let center = textureLoad(input_texture, coord, 0);

    // Sample neighbors
    let top = textureLoad(input_texture, coord + vec2<i32>(0, -1), 0);
    let bottom = textureLoad(input_texture, coord + vec2<i32>(0, 1), 0);
    let left = textureLoad(input_texture, coord + vec2<i32>(-1, 0), 0);
    let right = textureLoad(input_texture, coord + vec2<i32>(1, 0), 0);

    // Laplacian kernel
    let laplacian = center * 4.0 - top - bottom - left - right;

    // Add sharpened detail
    let result = center + laplacian * uniforms.amount;
    let color = vec4<f32>(clamp(result.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), center.a);

    textureStore(output_texture, coord, color);
}
