// Unsharp mask, second pass: finishes the separable box blur vertically
// (the first pass is the shared horizontal box blur), then sharpens the
// original — the input of the first pass, bound as `aux0` — against it:
// `original + (original - blurred) * amount`. The separable pair costs
// 2(2r + 1) taps per pixel instead of (2r + 1)^2.

struct Params {
    radius: f32,
    amount: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let pixel = floor(uv * size);
    let radius = max(i32(round(params.radius)), 0);
    let amount = max(params.amount, 0.0);

    var sum = load(input, input_point_sampler, size, pixel);
    for (var offset = 1; offset <= radius; offset++) {
        let delta = vec2<f32>(0.0, f32(offset));
        sum += load(input, input_point_sampler, size, pixel - delta)
            + load(input, input_point_sampler, size, pixel + delta);
    }
    let blurred = sum / f32(2 * radius + 1);

    let original = texel_at(aux0, uv);
    let sharpened = original.rgb + (original.rgb - blurred.rgb) * amount;
    return vec4<f32>(sharpened, original.a);
}
