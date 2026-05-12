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

fn compare_swap(values: ptr<function, array<vec3<f32>, 9>>, a: u32, b: u32) {
    let va = (*values)[a];
    let vb = (*values)[b];
    (*values)[a] = min(va, vb);
    (*values)[b] = max(va, vb);
}

fn median9(values: array<vec3<f32>, 9>) -> vec3<f32> {
    var v = values;
    compare_swap(&v, 1u, 2u);
    compare_swap(&v, 4u, 5u);
    compare_swap(&v, 7u, 8u);
    compare_swap(&v, 0u, 1u);
    compare_swap(&v, 3u, 4u);
    compare_swap(&v, 6u, 7u);
    compare_swap(&v, 1u, 2u);
    compare_swap(&v, 4u, 5u);
    compare_swap(&v, 7u, 8u);
    compare_swap(&v, 0u, 3u);
    compare_swap(&v, 5u, 8u);
    compare_swap(&v, 4u, 7u);
    compare_swap(&v, 3u, 6u);
    compare_swap(&v, 1u, 4u);
    compare_swap(&v, 2u, 5u);
    compare_swap(&v, 4u, 7u);
    compare_swap(&v, 4u, 2u);
    compare_swap(&v, 6u, 4u);
    compare_swap(&v, 4u, 2u);
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

    var samples: array<vec3<f32>, 9>;
    var idx: u32 = 0u;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            let p = clamp(coord + vec2<i32>(dx, dy), vec2<i32>(0), max_xy);
            samples[idx] = textureLoad(input_texture, p, 0).rgb;
            idx = idx + 1u;
        }
    }

    let centre_alpha = textureLoad(input_texture, clamp(coord, vec2<i32>(0), max_xy), 0).a;
    textureStore(output_texture, coord, vec4<f32>(median9(samples), centre_alpha));
}
