// Masked blur: a square box blur of `radius` pixels, mixed in where the
// mask (`aux0`, red channel) is set, scaled by `strength`.

struct Params {
    radius: f32,
    strength: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let radius = i32(round(max(params.radius, 0.0)));
    let strength = clamp(params.strength, 0.0, 1.0);
    let mask = clamp(texel_at(aux0, uv).r, 0.0, 1.0);

    var blurred = base;
    if radius > 0 {
        var sum = vec4<f32>(0.0);
        var count = 0.0;
        for (var y = -radius; y <= radius; y++) {
            for (var x = -radius; x <= radius; x++) {
                let sample_uv = uv + vec2<f32>(f32(x), f32(y)) / size;
                sum += textureSampleLevel(input, input_point_sampler, sample_uv, 0.0);
                count += 1.0;
            }
        }
        blurred = sum / max(count, 1.0);
    }
    return mix(base, blurred, mask * strength);
}
