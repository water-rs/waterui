// 3x3 median filter.
//
// Per-channel median of the 3x3 neighbourhood (all four channels, so the
// result stays consistent on premultiplied-alpha data). Useful for
// salt-and-pepper noise removal while preserving edges better than a box
// blur.

fn compare_swap(values: ptr<function, array<vec4<f32>, 9>>, a: u32, b: u32) {
    let va = (*values)[a];
    let vb = (*values)[b];
    (*values)[a] = min(va, vb);
    (*values)[b] = max(va, vb);
}

fn median9(values: array<vec4<f32>, 9>) -> vec4<f32> {
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

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);

    var samples: array<vec4<f32>, 9>;
    var idx: u32 = 0u;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            samples[idx] = load_input(center + vec2<i32>(dx, dy));
            idx = idx + 1u;
        }
    }
    textureStore(output_texture, vec2<i32>(gid.xy), median9(samples));
}
