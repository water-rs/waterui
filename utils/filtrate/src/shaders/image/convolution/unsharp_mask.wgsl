// Unsharp mask shader

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

    let coord = vec2<i32>(global_id.xy);
    let input_dims_u = vec2<u32>(uniforms.input_dimensions);
    let input_dims_i = vec2<i32>(input_dims_u);
    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims_i - vec2<i32>(1));

    let radius = max(i32(round(param(0u))), 1);
    let amount = max(param(1u), 0.0);
    let kernel_width = radius * 2 + 1;
    let kernel_area = max(kernel_width * kernel_width, 1);
    let sample_weight = 1.0 / f32(kernel_area);

    var blurred_sum = vec4<f32>(0.0);
    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let sample_coord = clamp(
                center + vec2<i32>(x, y),
                vec2<i32>(0),
                input_dims_i - vec2<i32>(1),
            );
            blurred_sum += textureLoad(input_texture, sample_coord, 0) * sample_weight;
        }
    }

    let base = textureLoad(input_texture, center, 0);
    let blurred_color = blurred_sum;
    let sharpened = base.rgb + (base.rgb - blurred_color.rgb) * amount;
    textureStore(output_texture, coord, vec4<f32>(sharpened, base.a));
}
