// Motion blur shader - directional spatial blur

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

    let radius = max(i32(round(param(0u))), 0);
    if radius == 0 {
        let center = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims_i - vec2<i32>(1));
        textureStore(output_texture, coord, textureLoad(input_texture, center, 0));
        return;
    }

    let angle_radians = param(1u) * 0.017453292519943295;
    let direction = vec2<f32>(cos(angle_radians), sin(angle_radians));

    var sum = vec4<f32>(0.0);
    var total_weight = 0.0;

    for (var i = -radius; i <= radius; i++) {
        let offset = direction * f32(i);
        let sample_pos = mapped + offset;
        let sample_coord = clamp(
            vec2<i32>(sample_pos),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        );
        let weight = 1.0 - abs(f32(i)) / f32(radius + 1);
        sum += textureLoad(input_texture, sample_coord, 0) * weight;
        total_weight += weight;
    }

    textureStore(output_texture, coord, sum / max(total_weight, 0.0001));
}
