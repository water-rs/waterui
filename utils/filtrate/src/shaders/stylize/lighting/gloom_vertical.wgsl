// Gloom vertical accumulator and apply pass.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
@group(0) @binding(3) var original_texture: texture_2d<f32>;

fn param(index: u32) -> f32 {
    let v = uniforms.params[index / 4u];
    switch index % 4u {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }

    let input_dims = vec2<i32>(vec2<u32>(uniforms.input_dimensions));
    let coord = vec2<i32>(gid.xy);
    let radius = max(i32(round(param(0u))), 1);
    let intensity = max(param(1u), 0.0);

    var sum = vec4<f32>(0.0);
    var count = 0.0;
    for (var y = -radius; y <= radius; y++) {
        let sample_coord = clamp(
            coord + vec2<i32>(0, y),
            vec2<i32>(0),
            input_dims - vec2<i32>(1),
        );
        sum += textureLoad(input_texture, sample_coord, 0);
        count += 1.0;
    }

    let weighted = sum / count;
    let gloom = weighted.rgb / max(weighted.a, 0.0001);
    let base = textureLoad(original_texture, coord, 0);
    textureStore(
        output_texture,
        coord,
        vec4<f32>(max(base.rgb - gloom * intensity, vec3<f32>(0.0)), base.a),
    );
}
