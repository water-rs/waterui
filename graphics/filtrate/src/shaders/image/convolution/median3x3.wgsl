// 3x3 median: the per-channel median of the neighbourhood (all four
// channels, so the result stays consistent on premultiplied colour).

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
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let pixel = floor(uv * size);

    var samples: array<vec4<f32>, 9>;
    var idx: u32 = 0u;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            samples[idx] = load(input, input_point_sampler, size, pixel + vec2<f32>(f32(dx), f32(dy)));
            idx = idx + 1u;
        }
    }
    return median9(samples);
}
