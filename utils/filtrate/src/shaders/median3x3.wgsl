// 3x3 median filter.
//
// Per-channel median of the 3x3 neighbourhood. Useful for salt-and-pepper
// noise removal while preserving edges better than a box blur. Alpha is
// taken from the centre pixel.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

fn median9(values: array<f32, 9>) -> f32 {
    var v = values;
    // Insertion sort — only 9 elements, the constant factor wins over
    // anything fancier here.
    for (var i: u32 = 1u; i < 9u; i = i + 1u) {
        var j: u32 = i;
        loop {
            if j == 0u { break; }
            if v[j - 1u] <= v[j] { break; }
            let tmp = v[j];
            v[j] = v[j - 1u];
            v[j - 1u] = tmp;
            j = j - 1u;
        }
    }
    return v[4u];
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

    var rs: array<f32, 9>;
    var gs: array<f32, 9>;
    var bs: array<f32, 9>;
    var idx: u32 = 0u;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            let p = clamp(coord + vec2<i32>(dx, dy), vec2<i32>(0), max_xy);
            let texel = textureLoad(input_texture, p, 0);
            rs[idx] = texel.r;
            gs[idx] = texel.g;
            bs[idx] = texel.b;
            idx = idx + 1u;
        }
    }

    let centre_alpha = textureLoad(input_texture, clamp(coord, vec2<i32>(0), max_xy), 0).a;
    let result = vec4<f32>(median9(rs), median9(gs), median9(bs), centre_alpha);
    textureStore(output_texture, coord, result);
}
