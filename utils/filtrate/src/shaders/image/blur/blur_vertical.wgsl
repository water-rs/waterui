// Separable vertical blur pass.
// Reads the horizontal-pass texture and writes final blurred result.
//
// Untiled by design: overlapping taps hit the GPU texture cache, which
// benchmarks faster than workgroup shared-memory tiling on tiler GPUs.

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

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let input_dims_i = vec2<i32>(vec2<u32>(uniforms.input_dimensions));
    let output_coord = vec2<i32>(global_id.xy);

    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims_i - vec2<i32>(1));

    let radius = max(i32(round(param(0u))), 0);
    if radius == 0 {
        textureStore(output_texture, output_coord, textureLoad(input_texture, center, 0));
        return;
    }

    var sum = textureLoad(input_texture, center, 0);
    for (var offset = 1; offset <= radius; offset++) {
        let up = clamp(
            center + vec2<i32>(0, -offset),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        );
        let down = clamp(
            center + vec2<i32>(0, offset),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        );
        sum += textureLoad(input_texture, up, 0) + textureLoad(input_texture, down, 0);
    }

    textureStore(output_texture, output_coord, sum / f32(2 * radius + 1));
}
