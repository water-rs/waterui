// Depth-aware blur: a depth-of-field blur whose circle of confusion grows
// with the depth map's (`aux0`, red channel) distance from `focus_depth`,
// up to `max_radius` pixels scaled by `aperture`. Samples at a different
// depth contribute less, so edges between depth planes stay sharp.

struct Params {
    focus_depth: f32,
    aperture: f32,
    max_radius: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let focus_depth = clamp(params.focus_depth, 0.0, 1.0);
    let aperture = max(params.aperture, 0.0);
    let max_radius = max(params.max_radius, 0.0);
    let depth = clamp(texel_at(aux0, uv).r, 0.0, 1.0);
    let coc = abs(depth - focus_depth) * aperture * max_radius;
    let radius = i32(round(coc));

    var blurred = base;
    if radius > 0 {
        var sum = vec4<f32>(0.0);
        var total_weight = 0.0;
        for (var y = -radius; y <= radius; y++) {
            for (var x = -radius; x <= radius; x++) {
                let sample_uv = uv + vec2<f32>(f32(x), f32(y)) / size;
                let depth_delta = abs(texel_at(aux0, sample_uv).r - depth);
                let depth_weight = 1.0 - smoothstep(0.0, 0.25, depth_delta);
                sum += textureSampleLevel(input, input_point_sampler, sample_uv, 0.0) * depth_weight;
                total_weight += depth_weight;
            }
        }
        blurred = sum / max(total_weight, 0.0001);
    }
    let mix_t = clamp(coc / max(max_radius, 0.0001), 0.0, 1.0);
    return mix(base, blurred, mix_t);
}
