// Guided smooth: an edge-preserving blur of `radius` pixels whose weights
// fall off with the guide image's (`aux0`) colour distance from the centre,
// mixed in by `amount`.

struct Params {
    radius: f32,
    range_sigma: f32,
    amount: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let radius = i32(round(max(params.radius, 0.0)));
    let inv_sigma = 1.0 / max(params.range_sigma, 0.0001);
    let amount = clamp(params.amount, 0.0, 1.0);

    var smoothed = base;
    if radius > 0 {
        let center_guide = texel_at(aux0, uv).rgb;
        var weighted_sum = vec4<f32>(0.0);
        var weight_total = 0.0;
        for (var y = -radius; y <= radius; y++) {
            for (var x = -radius; x <= radius; x++) {
                let sample_uv = uv + vec2<f32>(f32(x), f32(y)) / size;
                let diff = length(texel_at(aux0, sample_uv).rgb - center_guide);
                let weight = exp(-diff * inv_sigma);
                weighted_sum += textureSampleLevel(input, input_point_sampler, sample_uv, 0.0) * weight;
                weight_total += weight;
            }
        }
        smoothed = weighted_sum / max(weight_total, 0.0001);
    }
    return mix(base, smoothed, amount);
}
