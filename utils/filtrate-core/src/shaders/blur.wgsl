// Blur shader - standalone pass (spatial filter, cannot fuse)

struct Uniforms {
    dimensions: vec2<f32>,
    radius: f32,
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
    let radius = i32(uniforms.radius);

    var sum = vec4<f32>(0.0);
    var count = 0.0;

    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let sample_coord = coord + vec2<i32>(x, y);
            if sample_coord.x >= 0 && sample_coord.x < i32(uniforms.dimensions.x) &&
               sample_coord.y >= 0 && sample_coord.y < i32(uniforms.dimensions.y) {
                sum += textureLoad(input_texture, sample_coord, 0);
                count += 1.0;
            }
        }
    }

    let color = sum / count;
    textureStore(output_texture, coord, color);
}
