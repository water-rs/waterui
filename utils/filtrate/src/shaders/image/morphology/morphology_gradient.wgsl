// 3x3 morphological gradient: per-channel (max - min) over the neighbourhood.
// Highlights region boundaries.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let max_xy = vec2<i32>(vec2<u32>(uniforms.input_dimensions)) - vec2<i32>(1);

    var lo = vec3<f32>(1.0);
    var hi = vec3<f32>(0.0);
    var alpha = 0.0;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            let p = clamp(coord + vec2<i32>(dx, dy), vec2<i32>(0), max_xy);
            let texel = textureLoad(input_texture, p, 0);
            lo = min(lo, texel.rgb);
            hi = max(hi, texel.rgb);
            if dx == 0 && dy == 0 {
                alpha = texel.a;
            }
        }
    }
    textureStore(output_texture, coord, vec4<f32>(hi - lo, alpha));
}
