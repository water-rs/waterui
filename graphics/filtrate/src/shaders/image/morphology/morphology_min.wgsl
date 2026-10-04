// 3x3 erosion: the per-channel minimum of the neighbourhood. Seeded from
// the centre texel, so extended values survive. All four channels erode
// together, consistent with premultiplied colour.
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let pixel = floor(uv * size);

    var acc = load(input, input_point_sampler, size, pixel);
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            acc = min(acc, load(input, input_point_sampler, size, pixel + vec2<f32>(f32(dx), f32(dy))));
        }
    }
    return acc;
}
