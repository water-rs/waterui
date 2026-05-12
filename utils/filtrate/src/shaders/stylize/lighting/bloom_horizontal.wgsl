// Bloom horizontal weighted accumulator.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

fn param(index: u32) -> f32 {
    let v = uniforms.params[index / 4u];
    switch index % 4u {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

fn weighted_bright_sample(sample_color: vec4<f32>, threshold: f32) -> vec4<f32> {
    let luma = dot(sample_color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let bright = max(luma - threshold, 0.0);
    let weight = bright + 0.0001;
    return vec4<f32>(sample_color.rgb * weight, weight);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }

    let input_dims = vec2<i32>(vec2<u32>(uniforms.input_dimensions));
    let mapped = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims - vec2<i32>(1));
    let radius = max(i32(round(param(0u))), 1);
    let threshold = max(param(1u), 0.0);

    var sum = vec4<f32>(0.0);
    var count = 0.0;
    for (var x = -radius; x <= radius; x++) {
        let sample_coord = clamp(
            center + vec2<i32>(x, 0),
            vec2<i32>(0),
            input_dims - vec2<i32>(1),
        );
        sum += weighted_bright_sample(textureLoad(input_texture, sample_coord, 0), threshold);
        count += 1.0;
    }

    textureStore(output_texture, vec2<i32>(gid.xy), sum / count);
}
