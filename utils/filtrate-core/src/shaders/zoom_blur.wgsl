// Zoom blur shader - radial blur around a focal point

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
    let uv = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) / uniforms.output_dimensions;

    let amount = max(param(0u), 0.0);
    let center = vec2<f32>(param(1u), param(2u));
    if amount <= 0.0001 {
        let mapped = clamp(vec2<i32>(uv * uniforms.input_dimensions), vec2<i32>(0), input_dims_i - vec2<i32>(1));
        textureStore(output_texture, coord, textureLoad(input_texture, mapped, 0));
        return;
    }

    let direction = center - uv;
    let samples: i32 = 12;
    var sum = vec4<f32>(0.0);
    var total_weight = 0.0;

    for (var i = 0; i < samples; i++) {
        let t = f32(i) / f32(samples - 1);
        let sample_uv = uv + direction * amount * t;
        let mapped = clamp(
            vec2<i32>(sample_uv * uniforms.input_dimensions),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        );
        let weight = 1.0 - t * 0.65;
        sum += textureLoad(input_texture, mapped, 0) * weight;
        total_weight += weight;
    }

    textureStore(output_texture, coord, sum / max(total_weight, 0.0001));
}
