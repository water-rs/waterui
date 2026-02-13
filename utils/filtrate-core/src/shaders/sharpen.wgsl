// Sharpen shader - standalone pass (spatial filter, cannot fuse)

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

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let coord = vec2<i32>(global_id.xy);
    let input_dims_u = vec2<u32>(uniforms.input_dimensions);
    let input_dims_i = vec2<i32>(input_dims_u);
    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center_coord = clamp(
        vec2<i32>(mapped),
        vec2<i32>(0),
        input_dims_i - vec2<i32>(1),
    );
    let amount = param(0u);

    let center = textureLoad(input_texture, center_coord, 0);

    // Sample neighbors
    let top = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(0, -1),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );
    let bottom = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(0, 1),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );
    let left = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(-1, 0),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );
    let right = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(1, 0),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );

    // Laplacian kernel
    let laplacian = center * 4.0 - top - bottom - left - right;

    // Add sharpened detail
    let result = center + laplacian * amount;
    let color = vec4<f32>(result.rgb, center.a);

    textureStore(output_texture, coord, color);
}
