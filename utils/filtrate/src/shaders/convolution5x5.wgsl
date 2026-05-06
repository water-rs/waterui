// 5x5 convolution with a user-supplied kernel.
//
// Parameters: 25 floats stored row-major (top-left to bottom-right).
// Per-channel; alpha is preserved from the centre pixel. Caller is
// responsible for kernel normalisation.

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

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let in_dims = vec2<i32>(vec2<u32>(uniforms.input_dimensions));
    let max_xy = in_dims - vec2<i32>(1);

    var acc = vec3<f32>(0.0);
    var idx: u32 = 0u;
    for (var dy: i32 = -2; dy <= 2; dy = dy + 1) {
        for (var dx: i32 = -2; dx <= 2; dx = dx + 1) {
            let p = clamp(coord + vec2<i32>(dx, dy), vec2<i32>(0), max_xy);
            let texel = textureLoad(input_texture, p, 0);
            acc = acc + texel.rgb * param(idx);
            idx = idx + 1u;
        }
    }

    let centre_alpha = textureLoad(input_texture, clamp(coord, vec2<i32>(0), max_xy), 0).a;
    textureStore(output_texture, coord, vec4<f32>(acc, centre_alpha));
}
